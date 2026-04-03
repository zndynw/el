use crate::config::{DatabaseConfig, ImportConfig, ImportFormat};
use crate::db::{Database, ImportSession, ImportStats, QuerySink};
use crate::value::{DbValue, ValueFormatter};
use anyhow::{Context, Result, anyhow};
use csv::WriterBuilder;
use postgres::{Client, NoTls};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub struct GreenplumDatabase {
    config: DatabaseConfig,
    connection: Option<Client>,
}

impl GreenplumDatabase {
    pub fn new(config: DatabaseConfig) -> Self {
        Self {
            config,
            connection: None,
        }
    }

    fn build_connection_string(&self) -> Result<String> {
        if self.config.connection_string.starts_with("postgresql://")
            || self.config.connection_string.starts_with("postgres://")
        {
            Ok(self.config.connection_string.clone())
        } else {
            let target = parse_connection_target(&self.config.connection_string)?;
            let mut conn_str = format!(
                "host={} port={} dbname={} user={}",
                target.host, target.port, target.database, self.config.username
            );
            if !self.config.password.is_empty() {
                conn_str.push_str(&format!(" password={}", self.config.password));
            }
            Ok(conn_str)
        }
    }
}

struct GreenplumConnectionTarget {
    host: String,
    port: u16,
    database: String,
}

fn parse_connection_target(value: &str) -> Result<GreenplumConnectionTarget> {
    let (host_port, database) = value.rsplit_once('/').ok_or_else(|| {
        anyhow!("Greenplum connection string must be host:port/database or host/database")
    })?;

    let database = database.trim();
    if database.is_empty() {
        return Err(anyhow!("Greenplum connection string must include database"));
    }

    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        let host = h.trim();
        if host.is_empty() {
            return Err(anyhow!("Greenplum connection string must include host"));
        }
        (
            host.to_string(),
            p.trim().parse::<u16>().context("Invalid Greenplum port")?,
        )
    } else {
        let host = host_port.trim();
        if host.is_empty() {
            return Err(anyhow!("Greenplum connection string must include host"));
        }
        (host.to_string(), 5432)
    };

    Ok(GreenplumConnectionTarget {
        host,
        port,
        database: database.to_string(),
    })
}

impl Database for GreenplumDatabase {
    fn connect(&mut self) -> Result<()> {
        let conn_str = self.build_connection_string()?;
        let client =
            Client::connect(&conn_str, NoTls).context("Failed to connect to Greenplum database")?;
        self.connection = Some(client);
        Ok(())
    }

    fn stream_query(&mut self, _query: &str, _sink: &mut dyn QuerySink) -> Result<()> {
        Err(anyhow!(
            "Greenplum stream_query not implemented, use PostgreSQL driver"
        ))
    }

    fn execute_sql(&mut self, sql: &str) -> Result<u64> {
        let conn = self.connection.as_mut().context("Database not connected")?;
        Ok(conn.execute(sql, &[])?)
    }

    fn direct_import(&mut self, config: &ImportConfig) -> Result<Option<ImportStats>> {
        let conn = self.connection.as_mut().context("Database not connected")?;
        let gpfdist_host = self
            .config
            .gpfdist_host
            .as_ref()
            .ok_or_else(|| anyhow!("gpfdist_host not configured"))?;
        let gpfdist_port = self
            .config
            .gpfdist_port
            .ok_or_else(|| anyhow!("gpfdist_port not configured"))?;
        let source_columns = config
            .source_columns
            .as_ref()
            .context("source_columns is required for Greenplum direct import")?;

        let projection = build_projection(config, source_columns)?;
        let temp_table = build_temp_external_table_name(
            config.resolved_schema(),
            config.resolved_table(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        );
        let gpfdist_url = format!(
            "gpfdist://{}:{}/{}",
            gpfdist_host,
            gpfdist_port,
            normalize_gpfdist_path(&config.input_file)
        );
        let start_time = Instant::now();

        let column_defs = source_columns
            .iter()
            .map(|col| {
                let col_type = config
                    .column_types
                    .as_ref()
                    .and_then(|types| types.get(col).map(String::as_str))
                    .unwrap_or("TEXT");
                format!("{} {}", col, col_type)
            })
            .collect::<Vec<_>>()
            .join(", ");

        let create_sql = format!(
            "CREATE EXTERNAL TABLE {} ({}) LOCATION ('{}') {} LOG ERRORS SEGMENT REJECT LIMIT 1000 ROWS",
            temp_table,
            column_defs,
            gpfdist_url,
            external_format_clause_for_source(config)?
        );
        conn.execute(&create_sql, &[])?;

        let result = (|| -> Result<ImportStats> {
            if config.truncate_table {
                conn.execute(
                    &format!("TRUNCATE TABLE {}", config.qualified_target_table()),
                    &[],
                )?;
            }

            if let Some(sql) = &config.pre_sql {
                let sql = sql.replace("{ext_table}", &temp_table);
                let affected = conn.execute(&sql, &[])?;
                tracing::info!(phase = "pre_sql", affected_rows = affected, "sql_executed");
            }

            let insert_sql = format!(
                "INSERT INTO {} ({}) SELECT {} FROM {}",
                config.qualified_target_table(),
                projection.target_columns.join(", "),
                projection.select_expressions.join(", "),
                temp_table
            );
            let rows_inserted = conn.execute(&insert_sql, &[])?;

            if let Some(sql) = &config.post_sql {
                let sql = sql.replace("{ext_table}", &temp_table);
                let affected = conn.execute(&sql, &[])?;
                tracing::info!(phase = "post_sql", affected_rows = affected, "sql_executed");
            }

            let rows_failed = if let Some(err_table) = &config.error_log_table {
                let affected = save_greenplum_errors(conn, err_table, &temp_table)?;
                tracing::info!(
                    error_log_table = %err_table,
                    rows_failed = affected,
                    "error_log_captured"
                );
                affected
            } else {
                count_greenplum_errors(conn, &temp_table)?
            };

            Ok(ImportStats {
                rows_inserted,
                rows_failed,
                duration: start_time.elapsed(),
            })
        })();

        let drop_sql = format!("DROP EXTERNAL TABLE IF EXISTS {}", temp_table);
        let _ = conn.execute(&drop_sql, &[]);

        Ok(Some(result?))
    }

    fn prepare_import(
        &mut self,
        table: &str,
        external_columns: &[String],
        selected_source_columns: &[String],
        target_columns: &[String],
        column_types: &HashMap<String, String>,
        config: &ImportConfig,
    ) -> Result<Box<dyn ImportSession>> {
        let conn = self.connection.take().context("Database not connected")?;

        let gpfdist_host = self
            .config
            .gpfdist_host
            .as_ref()
            .ok_or_else(|| anyhow!("gpfdist_host not configured"))?;
        let gpfdist_port = self
            .config
            .gpfdist_port
            .ok_or_else(|| anyhow!("gpfdist_port not configured"))?;
        let gpfdist_dir = self
            .config
            .gpfdist_dir
            .as_ref()
            .ok_or_else(|| anyhow!("gpfdist_dir not configured"))?;

        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        let temp_filename = format!("import_{}.csv", timestamp);
        let temp_file_path = PathBuf::from(gpfdist_dir).join(&temp_filename);
        let gpfdist_url = format!(
            "gpfdist://{}:{}/{}",
            gpfdist_host, gpfdist_port, temp_filename
        );

        let temp_table = format!("temp_ext_{}", timestamp);

        Ok(Box::new(GreenplumExternalSession {
            conn,
            temp_table,
            target_table: table.to_string(),
            external_columns: external_columns.to_vec(),
            selected_source_columns: selected_source_columns.to_vec(),
            target_columns: target_columns.to_vec(),
            column_types: column_types.clone(),
            format: config.format.clone(),
            delimiter: config.delimiter.clone(),
            gpfdist_url,
            temp_file_path,
            data_file: None,
            error_log_table: config.error_log_table.clone(),
            rows_inserted: 0,
            rows_failed: 0,
            start_time: Instant::now(),
        }))
    }
}

#[derive(Debug)]
struct ProjectionPlan {
    target_columns: Vec<String>,
    select_expressions: Vec<String>,
}

struct GreenplumExternalSession {
    conn: Client,
    temp_table: String,
    target_table: String,
    external_columns: Vec<String>,
    selected_source_columns: Vec<String>,
    target_columns: Vec<String>,
    column_types: HashMap<String, String>,
    format: ImportFormat,
    delimiter: String,
    gpfdist_url: String,
    temp_file_path: PathBuf,
    data_file: Option<File>,
    error_log_table: Option<String>,
    rows_inserted: u64,
    rows_failed: u64,
    start_time: Instant,
}

impl ImportSession for GreenplumExternalSession {
    fn insert_batch(&mut self, rows: &[Vec<DbValue>]) -> Result<usize> {
        if self.data_file.is_none() {
            let file = File::create(&self.temp_file_path)
                .context("Failed to create temp file for gpfdist")?;
            self.data_file = Some(file);

            let column_defs = self
                .external_columns
                .iter()
                .map(|col| {
                    let col_type = self
                        .column_types
                        .get(col)
                        .map(String::as_str)
                        .unwrap_or("TEXT");
                    format!("{} {}", col, col_type)
                })
                .collect::<Vec<_>>()
                .join(", ");

            let create_sql = format!(
                "CREATE EXTERNAL TABLE {} ({}) LOCATION ('{}') {} LOG ERRORS SEGMENT REJECT LIMIT 1000 ROWS",
                self.temp_table,
                column_defs,
                self.gpfdist_url,
                external_format_clause(&self.format, &self.delimiter)?
            );

            self.conn.execute(&create_sql, &[])?;
        }

        let formatter = ValueFormatter::default();
        let format = self.format.clone();
        let delimiter = self.delimiter.clone();
        let file = self.data_file.as_mut().unwrap();

        for row in rows {
            write_row(file, &formatter, row, &format, &delimiter)?;
        }

        self.rows_inserted += rows.len() as u64;
        Ok(rows.len())
    }

    fn commit(&mut self) -> Result<()> {
        Ok(())
    }

    fn finish(mut self: Box<Self>) -> Result<ImportStats> {
        if let Some(mut file) = self.data_file.take() {
            file.flush()?;
            drop(file);

            let insert_sql = format!(
                "INSERT INTO {} ({}) SELECT {} FROM {}",
                self.target_table,
                self.target_columns.join(", "),
                self.selected_source_columns.join(", "),
                self.temp_table
            );
            self.conn.execute(&insert_sql, &[])?;

            if let Some(err_table) = &self.error_log_table {
                self.rows_failed =
                    save_greenplum_errors(&mut self.conn, err_table, &self.temp_table)?;
                tracing::info!(
                    error_log_table = %err_table,
                    rows_failed = self.rows_failed,
                    "error_log_captured"
                );
            } else {
                self.rows_failed = count_greenplum_errors(&mut self.conn, &self.temp_table)?;
            }

            let drop_sql = format!("DROP EXTERNAL TABLE IF EXISTS {}", self.temp_table);
            let _ = self.conn.execute(&drop_sql, &[]);

            let _ = std::fs::remove_file(&self.temp_file_path);
        }

        Ok(ImportStats {
            rows_inserted: self.rows_inserted,
            rows_failed: self.rows_failed,
            duration: self.start_time.elapsed(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{
        CompressionType, ErrorStrategy, ImportConfig, ImportFormat, TransactionMode,
    };
    use std::collections::HashMap;
    use std::time::Instant;

    fn base_import_config() -> ImportConfig {
        ImportConfig {
            schema: None,
            table: "t".to_string(),
            input_file: "risk/data.dat".to_string(),
            source_columns: Some(vec![
                "id_raw".to_string(),
                "created_raw".to_string(),
                "amount".to_string(),
            ]),
            target_columns: Some(vec![
                "id".to_string(),
                "month_id".to_string(),
                "amount".to_string(),
            ]),
            column_mapping: Some(HashMap::from([
                ("id_raw".to_string(), "id".to_string()),
                ("amount".to_string(), "amount".to_string()),
            ])),
            column_expressions: None,
            skip_columns: None,
            column_types: None,
            format: ImportFormat::Custom,
            delimiter: "\x03".to_string(),
            escape: None,
            has_header: false,
            batch_size: 1000,
            null_value: "".to_string(),
            on_error: ErrorStrategy::Skip,
            transaction_mode: TransactionMode::PerBatch,
            show_progress: false,
            progress_interval: 1_000_000,
            truncate_table: false,
            pre_sql: None,
            post_sql: None,
            error_log_table: None,
            compression: CompressionType::None,
        }
    }

    #[test]
    fn custom_format_uses_text_external_table_clause() {
        let _unused = (HashMap::<String, String>::new(), Instant::now());
        let clause = super::external_format_clause(&ImportFormat::Custom, "\x03")
            .expect("clause should build");

        assert!(clause.starts_with("FORMAT 'TEXT'"));
        assert!(clause.contains("DELIMITER E'\\003'"));
    }

    #[test]
    fn csv_format_uses_csv_external_table_clause_without_header() {
        let _unused = (HashMap::<String, String>::new(), Instant::now());
        let clause =
            super::external_format_clause(&ImportFormat::Csv, "|").expect("clause should build");

        assert_eq!(clause, "FORMAT 'CSV' (DELIMITER ',' NULL '')");
        assert!(!clause.contains("HEADER"));
    }

    #[test]
    fn projection_uses_configured_column_expressions_for_targets() {
        let mut config = base_import_config();
        config.column_expressions = Some(HashMap::from([(
            "month_id".to_string(),
            "to_char(created_raw, 'yyyy-mm')".to_string(),
        )]));

        let plan = super::build_projection(&config, config.source_columns.as_ref().unwrap())
            .expect("projection should build");

        assert_eq!(plan.target_columns, vec!["id", "month_id", "amount"]);
        assert_eq!(
            plan.select_expressions,
            vec!["id_raw", "to_char(created_raw, 'yyyy-mm')", "amount"]
        );
    }

    #[test]
    fn projection_errors_when_target_has_no_mapping_or_expression() {
        let config = base_import_config();

        let err = super::build_projection(&config, config.source_columns.as_ref().unwrap())
            .expect_err("projection should fail");

        assert!(err.to_string().contains("month_id"));
    }

    #[test]
    fn temp_external_table_name_includes_sanitized_target_table() {
        let name = super::build_temp_external_table_name(Some("htdw_bak"), "test_d_risk", 123456);

        assert_eq!(name, "htdw_bak.temp_ext_test_d_risk_123456");
    }

    #[test]
    fn greenplum_text_delimiter_literal_uses_escape_string_syntax() {
        assert_eq!(super::greenplum_text_delimiter_literal("\x03"), "E'\\003'");
        assert_eq!(super::greenplum_text_delimiter_literal("\t"), "E'\\t'");
        assert_eq!(super::greenplum_text_delimiter_literal("|"), "E'|'");
    }

    #[test]
    fn external_format_clause_includes_escape_when_configured() {
        let mut config = base_import_config();
        config.escape = Some("\\".to_string());

        let clause =
            super::external_format_clause_for_source(&config).expect("clause should build");

        assert!(clause.contains("ESCAPE E'\\\\'"));
    }

    #[test]
    fn column_types_keep_original_greenplum_type_declaration() {
        let mut config = base_import_config();
        config.column_types = Some(HashMap::from([
            ("id_raw".to_string(), "VARCHAR(30)".to_string()),
            ("created_raw".to_string(), "TIMESTAMP(7)".to_string()),
            ("amount".to_string(), "NUMERIC(16,2)".to_string()),
        ]));

        let source_columns = config.source_columns.as_ref().unwrap();
        let column_defs = source_columns
            .iter()
            .map(|col| {
                let col_type = config
                    .column_types
                    .as_ref()
                    .and_then(|types| types.get(col).map(String::as_str))
                    .unwrap_or("TEXT");
                format!("{} {}", col, col_type)
            })
            .collect::<Vec<_>>()
            .join(", ");

        assert!(column_defs.contains("id_raw VARCHAR(30)"));
        assert!(column_defs.contains("created_raw TIMESTAMP(7)"));
        assert!(column_defs.contains("amount NUMERIC(16,2)"));
    }

    #[test]
    fn base_import_config_can_enable_error_log_table() {
        let mut config = base_import_config();
        config.error_log_table = Some("ext_error_table".to_string());

        assert_eq!(config.error_log_table.as_deref(), Some("ext_error_table"));
    }
}

fn external_format_clause(format: &ImportFormat, delimiter: &str) -> Result<String> {
    match format {
        ImportFormat::Csv => Ok("FORMAT 'CSV' (DELIMITER ',' NULL '')".to_string()),
        ImportFormat::Tsv => Ok("FORMAT 'TEXT' (DELIMITER E'\\t' NULL '')".to_string()),
        ImportFormat::Custom => Ok(format!(
            "FORMAT 'TEXT' (DELIMITER {} NULL '')",
            greenplum_text_delimiter_literal(&effective_delimiter(format, delimiter)?)
        )),
    }
}

fn external_format_clause_for_source(config: &ImportConfig) -> Result<String> {
    let escape_clause = optional_escape_clause(config.escape.as_deref());
    match config.format {
        ImportFormat::Csv => {
            let header_clause = if config.has_header { " HEADER" } else { "" };
            Ok(format!(
                "FORMAT 'CSV' (DELIMITER ',' NULL ''{}{})",
                header_clause, escape_clause
            ))
        }
        ImportFormat::Tsv => {
            if config.has_header {
                return Err(anyhow!(
                    "Greenplum direct import does not support has_header=true with tsv format"
                ));
            }
            Ok(format!(
                "FORMAT 'TEXT' (DELIMITER E'\\t' NULL ''{})",
                escape_clause
            ))
        }
        ImportFormat::Custom => {
            if config.has_header {
                return Err(anyhow!(
                    "Greenplum direct import does not support has_header=true with custom format"
                ));
            }
            Ok(format!(
                "FORMAT 'TEXT' (DELIMITER {} NULL ''{})",
                greenplum_text_delimiter_literal(&effective_delimiter(
                    &config.format,
                    &config.delimiter
                )?),
                escape_clause
            ))
        }
    }
}

fn build_projection(config: &ImportConfig, source_columns: &[String]) -> Result<ProjectionPlan> {
    let skip_set = config
        .skip_columns
        .as_ref()
        .map(|cols| {
            cols.iter()
                .map(String::as_str)
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();

    let mut available_targets = std::collections::HashMap::new();
    for source_col in source_columns {
        if skip_set.contains(source_col.as_str()) {
            continue;
        }
        let target_col = config
            .column_mapping
            .as_ref()
            .and_then(|mapping| mapping.get(source_col))
            .unwrap_or(source_col)
            .clone();
        if available_targets
            .insert(target_col.clone(), source_col.clone())
            .is_some()
        {
            return Err(anyhow!(
                "multiple source columns map to the same target column: {}",
                target_col
            ));
        }
    }

    let target_columns = if let Some(target_columns) = &config.target_columns {
        target_columns.clone()
    } else {
        let mut inferred = Vec::new();
        for source_col in source_columns {
            if skip_set.contains(source_col.as_str()) {
                continue;
            }
            let target_col = config
                .column_mapping
                .as_ref()
                .and_then(|mapping| mapping.get(source_col))
                .unwrap_or(source_col)
                .clone();
            inferred.push(target_col);
        }
        inferred
    };

    let mut select_expressions = Vec::with_capacity(target_columns.len());
    for target_col in &target_columns {
        if let Some(expr) = config
            .column_expressions
            .as_ref()
            .and_then(|expressions| expressions.get(target_col))
        {
            select_expressions.push(expr.clone());
            continue;
        }

        let source_col = available_targets.get(target_col).ok_or_else(|| {
            anyhow!(
                "target column '{}' is not mapped from source_columns and has no column_expressions entry",
                target_col
            )
        })?;
        select_expressions.push(source_col.clone());
    }

    Ok(ProjectionPlan {
        target_columns,
        select_expressions,
    })
}

fn normalize_gpfdist_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_string()
}

fn build_temp_external_table_name(
    schema: Option<&str>,
    target_table: &str,
    timestamp_millis: u128,
) -> String {
    let sanitized = target_table
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();

    let suffix = if sanitized.is_empty() {
        "table".to_string()
    } else {
        sanitized
    };

    let table_name = format!("temp_ext_{}_{}", suffix, timestamp_millis);
    if let Some(schema) = schema {
        format!("{}.{}", schema, table_name)
    } else {
        table_name
    }
}

fn effective_delimiter(format: &ImportFormat, delimiter: &str) -> Result<String> {
    match format {
        ImportFormat::Csv => Ok(",".to_string()),
        ImportFormat::Tsv => Ok("\t".to_string()),
        ImportFormat::Custom => {
            if delimiter.is_empty() {
                return Err(anyhow!("custom import format requires a delimiter"));
            }
            Ok(delimiter.to_string())
        }
    }
}

fn write_row(
    file: &mut File,
    formatter: &ValueFormatter,
    row: &[DbValue],
    format: &ImportFormat,
    delimiter: &str,
) -> Result<()> {
    let values = row
        .iter()
        .map(|value| formatter.format(value))
        .collect::<Vec<_>>();

    match format {
        ImportFormat::Csv | ImportFormat::Tsv => {
            let delimiter = match format {
                ImportFormat::Csv => b',',
                ImportFormat::Tsv => b'\t',
                ImportFormat::Custom => unreachable!(),
            };
            let mut writer = WriterBuilder::new()
                .delimiter(delimiter)
                .has_headers(false)
                .from_writer(Vec::new());
            writer.write_record(values.iter().map(String::as_str))?;
            let buffer = writer
                .into_inner()
                .map_err(|err| io::Error::other(err.to_string()))?;
            file.write_all(&buffer)?;
        }
        ImportFormat::Custom => {
            let delimiter = effective_delimiter(format, delimiter)?;
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    file.write_all(delimiter.as_bytes())?;
                }
                file.write_all(value.as_bytes())?;
            }
            file.write_all(b"\n")?;
        }
    }

    Ok(())
}

fn greenplum_text_delimiter_literal(delimiter: &str) -> String {
    let mut escaped = String::new();
    for ch in delimiter.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '\'' => escaped.push_str("\\'"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            c if c.is_control() => escaped.push_str(&format!("\\{:03o}", c as u32)),
            c => escaped.push(c),
        }
    }
    format!("E'{}'", escaped)
}

fn optional_escape_clause(escape: Option<&str>) -> String {
    match escape {
        Some(value) if !value.is_empty() => {
            format!(" ESCAPE {}", greenplum_text_delimiter_literal(value))
        }
        _ => String::new(),
    }
}

fn save_greenplum_errors(conn: &mut Client, err_table: &str, ext_table: &str) -> Result<u64> {
    let save_errors_sql = format!(
        "INSERT INTO {} SELECT *, now() as log_time FROM gp_read_error_log('{}')",
        err_table, ext_table
    );
    Ok(conn.execute(&save_errors_sql, &[])?)
}

fn count_greenplum_errors(conn: &mut Client, ext_table: &str) -> Result<u64> {
    let count_errors_sql = format!("SELECT COUNT(*) FROM gp_read_error_log('{}')", ext_table);
    let row = conn.query_one(&count_errors_sql, &[])?;
    let count: i64 = row.get(0);
    Ok(count.max(0) as u64)
}
