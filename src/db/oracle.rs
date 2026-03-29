use crate::config::DatabaseConfig;
use crate::db::{Database, QuerySink};
use crate::value::DbValue;
use anyhow::{Context, Result, anyhow};
use oracle::sql_type::{Blob, Clob, IntervalDS, IntervalYM, Nclob, OracleType, Timestamp};
use oracle::{Connection, Row};
use std::io::Read;
use tracing;

pub struct OracleDatabase {
    config: DatabaseConfig,
    connection: Option<Connection>,
}

impl OracleDatabase {
    pub fn new(config: DatabaseConfig) -> Self {
        Self {
            config,
            connection: None,
        }
    }

    fn build_connection_string(&self) -> String {
        format!("//{}", self.config.connection_string)
    }

    fn row_to_values(&self, row: &Row, column_types: &[OracleType]) -> Result<Vec<DbValue>> {
        column_types
            .iter()
            .enumerate()
            .map(|(index, oracle_type)| {
                self.read_value(row, index, oracle_type).or_else(|e| {
                    tracing::warn!("Failed to read column {}: {}", index, e);
                    Ok(DbValue::Null)
                })
            })
            .collect()
    }

    fn read_value(&self, row: &Row, index: usize, oracle_type: &OracleType) -> Result<DbValue> {
        let result = match oracle_type {
            OracleType::Varchar2(_)
            | OracleType::NVarchar2(_)
            | OracleType::Char(_)
            | OracleType::NChar(_)
            | OracleType::Long
            | OracleType::Rowid
            | OracleType::Xml => row
                .get::<_, Option<String>>(index)?
                .map(DbValue::Text)
                .unwrap_or(DbValue::Null),
            OracleType::Json => row
                .get::<_, Option<String>>(index)?
                .map(DbValue::Json)
                .unwrap_or(DbValue::Null),
            OracleType::Number(_, _) | OracleType::Float(_) => row
                .get::<_, Option<String>>(index)?
                .map(DbValue::Decimal)
                .unwrap_or(DbValue::Null),
            OracleType::BinaryFloat => row
                .get::<_, Option<f32>>(index)?
                .map(|value| DbValue::Float(value.into()))
                .unwrap_or(DbValue::Null),
            OracleType::BinaryDouble => row
                .get::<_, Option<f64>>(index)?
                .map(DbValue::Float)
                .unwrap_or(DbValue::Null),
            OracleType::Int64 => row
                .get::<_, Option<i64>>(index)?
                .map(DbValue::Integer)
                .unwrap_or(DbValue::Null),
            OracleType::UInt64 => row
                .get::<_, Option<u64>>(index)?
                .map(DbValue::UnsignedInteger)
                .unwrap_or(DbValue::Null),
            OracleType::Boolean => row
                .get::<_, Option<bool>>(index)?
                .map(DbValue::Boolean)
                .unwrap_or(DbValue::Null),
            OracleType::Date => row
                .get::<_, Option<Timestamp>>(index)?
                .map(|value| DbValue::DateTime(value.to_string()))
                .unwrap_or(DbValue::Null),
            OracleType::Timestamp(_) | OracleType::TimestampTZ(_) | OracleType::TimestampLTZ(_) => {
                row.get::<_, Option<Timestamp>>(index)?
                    .map(|value| DbValue::DateTime(value.to_string()))
                    .unwrap_or(DbValue::Null)
            }
            OracleType::IntervalDS(_, _) => row
                .get::<_, Option<IntervalDS>>(index)?
                .map(|value| DbValue::Interval(value.to_string()))
                .unwrap_or(DbValue::Null),
            OracleType::IntervalYM(_) => row
                .get::<_, Option<IntervalYM>>(index)?
                .map(|value| DbValue::Interval(value.to_string()))
                .unwrap_or(DbValue::Null),
            OracleType::Raw(_) | OracleType::LongRaw => row
                .get::<_, Option<Vec<u8>>>(index)?
                .map(DbValue::Binary)
                .unwrap_or(DbValue::Null),
            OracleType::CLOB => read_optional_clob(row.get::<_, Option<Clob>>(index)?)?,
            OracleType::NCLOB => read_optional_nclob(row.get::<_, Option<Nclob>>(index)?)?,
            OracleType::BLOB => read_optional_blob(row.get::<_, Option<Blob>>(index)?)?,
            OracleType::BFILE | OracleType::Object(_) | OracleType::RefCursor => {
                return Err(anyhow!("unsupported Oracle type for export: {oracle_type}"));
            }
        };
        Ok(result)
    }

    fn stream_query_impl(&mut self, query: &str, sink: &mut dyn QuerySink) -> Result<()> {
        let conn = self.connection.as_ref().context("Database not connected")?;

        let mut stmt = conn
            .statement(query)
            .fetch_array_size(self.config.fetch_size as u32)
            .build()?;

        let rows = stmt.query(&[])?;

        let columns: Vec<String> = rows
            .column_info()
            .iter()
            .map(|col| col.name().to_string())
            .collect();
        let column_types: Vec<OracleType> = rows
            .column_info()
            .iter()
            .map(|col| col.oracle_type().clone())
            .collect();

        sink.on_columns(&columns)?;

        for row_result in rows {
            let row = row_result?;
            let values = self.row_to_values(&row, &column_types)?;
            sink.on_row(&values)?;
        }

        Ok(())
    }
}

fn read_optional_blob(blob: Option<Blob>) -> Result<DbValue> {
    match blob {
        Some(mut blob) => {
            let mut bytes = Vec::new();
            blob.read_to_end(&mut bytes)?;
            Ok(DbValue::Binary(bytes))
        }
        None => Ok(DbValue::Null),
    }
}

fn read_optional_clob(clob: Option<Clob>) -> Result<DbValue> {
    match clob {
        Some(mut clob) => {
            let mut text = String::new();
            clob.read_to_string(&mut text)?;
            Ok(DbValue::Text(text))
        }
        None => Ok(DbValue::Null),
    }
}

fn read_optional_nclob(clob: Option<Nclob>) -> Result<DbValue> {
    match clob {
        Some(mut clob) => {
            let mut text = String::new();
            clob.read_to_string(&mut text)?;
            Ok(DbValue::Text(text))
        }
        None => Ok(DbValue::Null),
    }
}

impl Database for OracleDatabase {
    fn connect(&mut self) -> Result<()> {
        let conn_str = self.build_connection_string();
        let conn = Connection::connect(&self.config.username, &self.config.password, &conn_str)
            .context("Failed to connect to Oracle database")?;

        self.connection = Some(conn);
        Ok(())
    }

    fn stream_query(&mut self, query: &str, sink: &mut dyn QuerySink) -> Result<()> {
        self.stream_query_impl(query, sink)
    }
}
