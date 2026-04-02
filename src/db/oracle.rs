use crate::config::DatabaseConfig;
use crate::db::{Database, ImportSession, ImportStats, QuerySink};
use crate::value::DbValue;
use anyhow::{Context, Result, anyhow};
use oracle::Batch;
use oracle::sql_type::{Blob, Clob, IntervalDS, IntervalYM, Nclob, OracleType, Timestamp};
use oracle::{Connection, Row};
use std::collections::HashMap;
use std::io::Read;
use std::time::Instant;
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

    fn execute_sql(&mut self, sql: &str) -> Result<u64> {
        let conn = self.connection.as_ref().context("Database not connected")?;
        let stmt = conn.execute(sql, &[])?;
        Ok(stmt.row_count()?)
    }

    fn prepare_import(
        &mut self,
        table: &str,
        _external_columns: &[String],
        _selected_source_columns: &[String],
        target_columns: &[String],
        column_types: &HashMap<String, String>,
        _config: &crate::config::ImportConfig,
    ) -> Result<Box<dyn ImportSession>> {
        let conn = self.connection.take().context("Database not connected")?;
        Ok(Box::new(OracleImportSession {
            conn,
            table: table.to_string(),
            columns: target_columns.to_vec(),
            column_types: target_columns
                .iter()
                .map(|col| {
                    (
                        col.clone(),
                        column_types
                            .get(col)
                            .and_then(|hint| oracle_type_from_hint(hint)),
                    )
                })
                .collect(),
            rows_inserted: 0,
            start_time: Instant::now(),
        }))
    }
}

struct OracleImportSession {
    conn: Connection,
    table: String,
    columns: Vec<String>,
    column_types: HashMap<String, Option<OracleType>>,
    rows_inserted: u64,
    start_time: Instant,
}

impl ImportSession for OracleImportSession {
    fn insert_batch(&mut self, rows: &[Vec<DbValue>]) -> Result<usize> {
        if rows.is_empty() {
            return Ok(0);
        }

        let sql = build_single_row_insert_sql(&self.table, &self.columns);
        let mut batch = self.conn.batch(&sql, rows.len()).build()?;
        self.apply_batch_bind_types(&mut batch, rows)?;

        for row in rows {
            let bind_values = self.build_bind_values(row)?;
            let params = bind_values
                .iter()
                .map(OracleBindValue::as_to_sql)
                .collect::<Vec<_>>();
            batch.append_row(&params)?;
        }

        batch.execute()?;
        self.rows_inserted += rows.len() as u64;
        Ok(rows.len())
    }

    fn commit(&mut self) -> Result<()> {
        self.conn.commit()?;
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<ImportStats> {
        Ok(ImportStats {
            rows_inserted: self.rows_inserted,
            rows_failed: 0,
            duration: self.start_time.elapsed(),
        })
    }
}

impl OracleImportSession {
    fn apply_batch_bind_types(&self, batch: &mut Batch<'_>, rows: &[Vec<DbValue>]) -> Result<()> {
        for (index, column) in self.columns.iter().enumerate() {
            let bind_type = self.column_types.get(column).and_then(|ty| ty.clone()).or_else(|| {
                rows.iter()
                    .filter_map(|row| row.get(index))
                    .find_map(oracle_type_from_value)
            });

            if let Some(bind_type) = bind_type {
                batch.set_type(index + 1, &bind_type)?;
            }
        }
        Ok(())
    }

    fn build_bind_values(&self, row: &[DbValue]) -> Result<Vec<OracleBindValue>> {
        if row.len() != self.columns.len() {
            return Err(anyhow!(
                "row column count {} does not match target column count {}",
                row.len(),
                self.columns.len()
            ));
        }

        row.iter()
            .enumerate()
            .map(|(index, value)| {
                let column_name = &self.columns[index];
                let column_type = self
                    .column_types
                    .get(column_name)
                    .and_then(|ty| ty.clone());
                OracleBindValue::from_db_value(value, column_type)
            })
            .collect()
    }
}

enum OracleBindValue {
    Null(OracleType),
    Boolean(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Binary(Vec<u8>),
}

impl OracleBindValue {
    fn from_db_value(value: &DbValue, hinted_type: Option<OracleType>) -> Result<Self> {
        let bind_value = match value {
            DbValue::Null => Self::Null(hinted_type.unwrap_or(OracleType::Varchar2(1))),
            DbValue::Boolean(value) => Self::Boolean(*value),
            DbValue::Integer(value) => Self::Integer(*value),
            DbValue::UnsignedInteger(value) => Self::Integer(i64::try_from(*value).map_err(|_| {
                anyhow!("unsigned integer value {} exceeds Oracle i64 bind range", value)
            })?),
            DbValue::Float(value) => Self::Float(*value),
            DbValue::Decimal(value)
            | DbValue::Text(value)
            | DbValue::Date(value)
            | DbValue::DateTime(value)
            | DbValue::Time(value)
            | DbValue::Interval(value)
            | DbValue::Json(value) => Self::Text(value.clone()),
            DbValue::Binary(value) => Self::Binary(value.clone()),
        };

        Ok(bind_value)
    }

    fn as_to_sql(&self) -> &dyn oracle::sql_type::ToSql {
        match self {
            Self::Null(value) => value,
            Self::Boolean(value) => value,
            Self::Integer(value) => value,
            Self::Float(value) => value,
            Self::Text(value) => value,
            Self::Binary(value) => value,
        }
    }
}

fn build_single_row_insert_sql(table: &str, columns: &[String]) -> String {
    let placeholders = (1..=columns.len())
        .map(|i| format!(":{}", i))
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "INSERT INTO {} ({}) VALUES ({})",
        table,
        columns.join(","),
        placeholders
    )
}

fn oracle_type_from_value(value: &DbValue) -> Option<OracleType> {
    match value {
        DbValue::Null => None,
        DbValue::Boolean(_) => Some(OracleType::Boolean),
        DbValue::Integer(_) | DbValue::UnsignedInteger(_) => Some(OracleType::Number(0, 0)),
        DbValue::Float(_) => Some(OracleType::BinaryDouble),
        DbValue::Decimal(_) => Some(OracleType::Number(38, 10)),
        DbValue::Text(_) | DbValue::Json(_) => Some(OracleType::Varchar2(4000)),
        DbValue::Binary(_) => Some(OracleType::Raw(2000)),
        DbValue::Date(_) => Some(OracleType::Date),
        DbValue::DateTime(_) => Some(OracleType::Timestamp(6)),
        DbValue::Time(_) | DbValue::Interval(_) => Some(OracleType::Varchar2(4000)),
    }
}

fn oracle_type_from_hint(hint: &str) -> Option<OracleType> {
    let normalized = hint.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "bool" | "boolean" => Some(OracleType::Boolean),
        "smallint" | "int" | "integer" | "bigint" => Some(OracleType::Number(0, 0)),
        "float" | "double" | "real" => Some(OracleType::BinaryDouble),
        "decimal" | "numeric" => Some(OracleType::Number(38, 10)),
        "date" => Some(OracleType::Date),
        "datetime" | "timestamp" => Some(OracleType::Timestamp(6)),
        "binary" | "blob" | "raw" => Some(OracleType::Raw(2000)),
        "text" | "string" | "json" => Some(OracleType::Varchar2(4000)),
        _ if normalized.starts_with("varchar2(")
            || normalized.starts_with("varchar(")
            || normalized.starts_with("nvarchar2(")
            || normalized.starts_with("char(")
            || normalized.starts_with("nchar(") =>
        {
            parse_sized_oracle_type(&normalized)
        }
        _ if normalized.starts_with("raw(") => parse_raw_type(&normalized),
        _ if normalized.starts_with("timestamp(") => parse_timestamp_type(&normalized),
        _ if normalized.starts_with("number(") || normalized.starts_with("numeric(") => {
            parse_number_type(&normalized)
        }
        _ => None,
    }
}

fn parse_sized_oracle_type(value: &str) -> Option<OracleType> {
    let size = parse_type_size(value)?;
    if value.starts_with("nvarchar2(") {
        Some(OracleType::NVarchar2(size))
    } else if value.starts_with("char(") {
        Some(OracleType::Char(size))
    } else if value.starts_with("nchar(") {
        Some(OracleType::NChar(size))
    } else {
        Some(OracleType::Varchar2(size))
    }
}

fn parse_raw_type(value: &str) -> Option<OracleType> {
    parse_type_size(value).map(OracleType::Raw)
}

fn parse_timestamp_type(value: &str) -> Option<OracleType> {
    parse_type_size(value).and_then(|size| u8::try_from(size).ok()).map(OracleType::Timestamp)
}

fn parse_number_type(value: &str) -> Option<OracleType> {
    let start = value.find('(')? + 1;
    let end = value.rfind(')')?;
    let parts = value[start..end]
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();

    match parts.as_slice() {
        [precision] => Some(OracleType::Number(precision.parse().ok()?, 0)),
        [precision, scale] => Some(OracleType::Number(
            precision.parse().ok()?,
            scale.parse().ok()?,
        )),
        _ => None,
    }
}

fn parse_type_size(value: &str) -> Option<u32> {
    let start = value.find('(')? + 1;
    let end = value.rfind(')')?;
    value[start..end].trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{
        build_single_row_insert_sql, oracle_type_from_hint, oracle_type_from_value,
        OracleBindValue,
    };
    use crate::value::DbValue;
    use oracle::sql_type::OracleType;

    #[test]
    fn oracle_insert_sql_uses_single_row_placeholders() {
        let sql = build_single_row_insert_sql("SCOTT.EMP", &["EMPNO".to_string(), "ENAME".to_string()]);

        assert_eq!(sql, "INSERT INTO SCOTT.EMP (EMPNO,ENAME) VALUES (:1,:2)");
    }

    #[test]
    fn oracle_type_hint_parses_common_types() {
        assert_eq!(oracle_type_from_hint("integer"), Some(OracleType::Number(0, 0)));
        assert_eq!(oracle_type_from_hint("timestamp"), Some(OracleType::Timestamp(6)));
        assert_eq!(oracle_type_from_hint("varchar2(128)"), Some(OracleType::Varchar2(128)));
        assert_eq!(oracle_type_from_hint("number(18,2)"), Some(OracleType::Number(18, 2)));
    }

    #[test]
    fn oracle_type_can_be_inferred_from_db_value() {
        assert_eq!(oracle_type_from_value(&DbValue::Integer(1)), Some(OracleType::Number(0, 0)));
        assert_eq!(oracle_type_from_value(&DbValue::DateTime("2026-04-02 12:00:00".to_string())), Some(OracleType::Timestamp(6)));
        assert_eq!(oracle_type_from_value(&DbValue::Null), None);
    }

    #[test]
    fn oracle_null_bind_uses_hint_when_available() {
        let bind_value = OracleBindValue::from_db_value(&DbValue::Null, Some(OracleType::Date))
            .expect("null bind should be created");

        match bind_value {
            OracleBindValue::Null(OracleType::Date) => {}
            other => panic!("unexpected bind value: {:?}", std::mem::discriminant(&other)),
        }
    }
}
