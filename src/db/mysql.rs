use crate::config::DatabaseConfig;
use crate::db::{Database, QuerySink};
use crate::value::DbValue;
use anyhow::{Context, Result, anyhow};
use mysql::consts::ColumnType;
use mysql::prelude::Queryable;
use mysql::{Column, Conn, Opts, OptsBuilder, Row, Value};
use tracing;

pub struct MySqlDatabase {
    config: DatabaseConfig,
    connection: Option<Conn>,
}

impl MySqlDatabase {
    pub fn new(config: DatabaseConfig) -> Self {
        Self {
            config,
            connection: None,
        }
    }

    fn build_opts(&self) -> Result<Opts> {
        let builder = if self.config.connection_string.starts_with("mysql://") {
            OptsBuilder::from_opts(
                Opts::from_url(&self.config.connection_string)
                    .context("Invalid MySQL connection URL")?,
            )
        } else {
            let target = parse_connection_target(&self.config.connection_string)?;
            OptsBuilder::new()
                .ip_or_hostname(Some(target.host))
                .tcp_port(target.port)
                .db_name(Some(target.database))
        };

        Ok(Opts::from(
            builder
                .user(Some(self.config.username.clone()))
                .pass(Some(self.config.password.clone())),
        ))
    }

    fn row_to_values(row: Row, columns: &[Column]) -> Vec<DbValue> {
        row.unwrap()
            .into_iter()
            .zip(columns.iter())
            .enumerate()
            .map(|(index, (value, column))| {
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    mysql_value_to_db_value(value, column)
                })) {
                    Ok(db_value) => db_value,
                    Err(_) => {
                        tracing::warn!("Failed to convert column {} value", index);
                        DbValue::Null
                    }
                }
            })
            .collect()
    }

    fn stream_query_impl(&mut self, query: &str, sink: &mut dyn QuerySink) -> Result<()> {
        let conn = self.connection.as_mut().context("Database not connected")?;
        let mut result = conn.query_iter(query)?;
        let mut saw_result_set = false;

        while let Some(result_set) = result.iter() {
            if saw_result_set {
                return Err(anyhow!("Multiple result sets are not supported"));
            }

            let columns: Vec<Column> = result_set.columns().as_ref().to_vec();
            let column_names: Vec<String> = columns
                .iter()
                .map(|column| column.name_str().into_owned())
                .collect();
            sink.on_columns(&column_names)?;

            for row_result in result_set {
                let row = row_result?;
                let values = Self::row_to_values(row, &columns);
                sink.on_row(&values)?;
            }

            saw_result_set = true;
        }

        if !saw_result_set {
            return Err(anyhow!("Query did not return a result set"));
        }

        Ok(())
    }
}

impl Database for MySqlDatabase {
    fn connect(&mut self) -> Result<()> {
        let conn = Conn::new(self.build_opts()?).context("Failed to connect to MySQL database")?;
        self.connection = Some(conn);
        Ok(())
    }

    fn stream_query(&mut self, query: &str, sink: &mut dyn QuerySink) -> Result<()> {
        self.stream_query_impl(query, sink)
    }
}

struct MySqlConnectionTarget {
    host: String,
    port: u16,
    database: String,
}

fn parse_connection_target(value: &str) -> Result<MySqlConnectionTarget> {
    let (host_port, database) = value.rsplit_once('/').ok_or_else(|| {
        anyhow!("MySQL connection string must be host:port/database or host/database")
    })?;

    let database = database.trim();
    if database.is_empty() {
        return Err(anyhow!("MySQL connection string must include database"));
    }

    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        let host = h.trim();
        if host.is_empty() {
            return Err(anyhow!("MySQL connection string must include host"));
        }
        (
            host.to_string(),
            p.trim().parse::<u16>().context("Invalid MySQL port")?,
        )
    } else {
        let host = host_port.trim();
        if host.is_empty() {
            return Err(anyhow!("MySQL connection string must include host"));
        }
        (host.to_string(), 3306)
    };

    Ok(MySqlConnectionTarget {
        host,
        port,
        database: database.to_string(),
    })
}

fn mysql_value_to_db_value(value: Value, column: &Column) -> DbValue {
    match value {
        Value::NULL => DbValue::Null,
        Value::Bytes(bytes) => match column.column_type() {
            ColumnType::MYSQL_TYPE_DATE | ColumnType::MYSQL_TYPE_NEWDATE => {
                DbValue::Date(String::from_utf8_lossy(&bytes).into_owned())
            }
            ColumnType::MYSQL_TYPE_DATETIME
            | ColumnType::MYSQL_TYPE_DATETIME2
            | ColumnType::MYSQL_TYPE_TIMESTAMP
            | ColumnType::MYSQL_TYPE_TIMESTAMP2 => {
                DbValue::DateTime(String::from_utf8_lossy(&bytes).into_owned())
            }
            ColumnType::MYSQL_TYPE_TIME | ColumnType::MYSQL_TYPE_TIME2 => {
                DbValue::Time(String::from_utf8_lossy(&bytes).into_owned())
            }
            ColumnType::MYSQL_TYPE_JSON => {
                DbValue::Json(String::from_utf8_lossy(&bytes).into_owned())
            }
            ColumnType::MYSQL_TYPE_DECIMAL | ColumnType::MYSQL_TYPE_NEWDECIMAL => {
                DbValue::Decimal(String::from_utf8_lossy(&bytes).into_owned())
            }
            _ if is_binary_column(column) => DbValue::Binary(bytes),
            _ => DbValue::Text(String::from_utf8_lossy(&bytes).into_owned()),
        },
        Value::Int(number) => DbValue::Integer(number),
        Value::UInt(number) => DbValue::UnsignedInteger(number),
        Value::Float(number) => DbValue::Float(number.into()),
        Value::Double(number) => DbValue::Float(number),
        Value::Date(year, month, day, hour, minute, second, micros) => {
            if hour == 0 && minute == 0 && second == 0 && micros == 0 {
                DbValue::Date(format!("{year:04}-{month:02}-{day:02}"))
            } else if micros == 0 {
                DbValue::DateTime(format!(
                    "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
                ))
            } else {
                DbValue::DateTime(format!(
                    "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{micros:06}"
                ))
            }
        }
        Value::Time(is_negative, days, hours, minutes, seconds, micros) => {
            let total_hours = days * 24 + u32::from(hours);
            let sign = if is_negative { "-" } else { "" };

            if micros == 0 {
                DbValue::Time(format!("{sign}{total_hours:02}:{minutes:02}:{seconds:02}"))
            } else {
                DbValue::Time(format!(
                    "{sign}{total_hours:02}:{minutes:02}:{seconds:02}.{micros:06}"
                ))
            }
        }
    }
}

fn is_binary_column(column: &Column) -> bool {
    let column_type = column.column_type();

    matches!(
        column_type,
        ColumnType::MYSQL_TYPE_BIT
            | ColumnType::MYSQL_TYPE_GEOMETRY
            | ColumnType::MYSQL_TYPE_VECTOR
    ) || column.character_set() == 63
}

#[cfg(test)]
mod tests {
    use super::parse_connection_target;
    use super::{is_binary_column, mysql_value_to_db_value};
    use crate::value::DbValue;
    use mysql::Column;
    use mysql::consts::{ColumnFlags, ColumnType};

    #[test]
    fn parses_mysql_host_port_database_connection_string() {
        let target =
            parse_connection_target("127.0.0.1:3306/reporting").expect("parse should work");

        assert_eq!(target.host, "127.0.0.1");
        assert_eq!(target.port, 3306);
        assert_eq!(target.database, "reporting");
    }

    #[test]
    fn classifies_binary_mysql_bytes_using_column_metadata() {
        let column = Column::new(ColumnType::MYSQL_TYPE_BLOB).with_character_set(63);

        assert!(is_binary_column(&column));
        assert_eq!(
            mysql_value_to_db_value(mysql::Value::Bytes(vec![0x01, 0x02]), &column),
            DbValue::Binary(vec![0x01, 0x02])
        );
    }

    #[test]
    fn treats_nonbinary_text_columns_with_binary_collation_as_text() {
        let column = Column::new(ColumnType::MYSQL_TYPE_VAR_STRING)
            .with_flags(ColumnFlags::BINARY_FLAG)
            .with_character_set(224);

        assert!(!is_binary_column(&column));
        assert_eq!(
            mysql_value_to_db_value("数据".into(), &column),
            DbValue::Text("数据".to_string())
        );
    }

    #[test]
    fn treats_text_protocol_blob_columns_with_text_charset_as_text() {
        let column = Column::new(ColumnType::MYSQL_TYPE_BLOB).with_character_set(224);

        assert!(!is_binary_column(&column));
        assert_eq!(
            mysql_value_to_db_value("导出内容".into(), &column),
            DbValue::Text("导出内容".to_string())
        );
    }
}
