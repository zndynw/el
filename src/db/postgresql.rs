use crate::config::{DatabaseConfig, ExportConfig, ExportFormat};
use crate::db::{Database, ImportSession, ImportStats, QuerySink};
use crate::value::DbValue;
use anyhow::{Context, Result, anyhow};
use postgres::{Client, NoTls};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::time::Instant;
use tracing::info;

pub struct PostgreSqlDatabase {
    config: DatabaseConfig,
    connection: Option<Client>,
}

impl PostgreSqlDatabase {
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

struct PostgreSqlConnectionTarget {
    host: String,
    port: u16,
    database: String,
}

fn parse_connection_target(value: &str) -> Result<PostgreSqlConnectionTarget> {
    let (host_port, database) = value.rsplit_once('/').ok_or_else(|| {
        anyhow!("PostgreSQL connection string must be host:port/database or host/database")
    })?;

    let database = database.trim();
    if database.is_empty() {
        return Err(anyhow!(
            "PostgreSQL connection string must include database"
        ));
    }

    let (host, port) = if let Some((h, p)) = host_port.rsplit_once(':') {
        let host = h.trim();
        if host.is_empty() {
            return Err(anyhow!("PostgreSQL connection string must include host"));
        }
        (
            host.to_string(),
            p.trim().parse::<u16>().context("Invalid PostgreSQL port")?,
        )
    } else {
        let host = host_port.trim();
        if host.is_empty() {
            return Err(anyhow!("PostgreSQL connection string must include host"));
        }
        (host.to_string(), 5432)
    };

    Ok(PostgreSqlConnectionTarget {
        host,
        port,
        database: database.to_string(),
    })
}

impl Database for PostgreSqlDatabase {
    fn connect(&mut self) -> Result<()> {
        let conn_str = self.build_connection_string()?;
        let client = Client::connect(&conn_str, NoTls)
            .context("Failed to connect to PostgreSQL database")?;
        self.connection = Some(client);
        Ok(())
    }

    fn stream_query(&mut self, query: &str, sink: &mut dyn QuerySink) -> Result<()> {
        let conn = self.connection.as_mut().context("Database not connected")?;

        let copy_query = format!(
            "COPY ({}) TO STDOUT WITH (FORMAT CSV, NULL '', HEADER false)",
            query
        );
        let mut reader = conn.copy_out(&copy_query)?;

        let mut first_line = true;
        let mut buffer = Vec::new();
        let mut line_buffer = Vec::new();

        loop {
            buffer.clear();
            buffer.resize(8192, 0);
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }

            for &byte in &buffer[..n] {
                if byte == b'\n' {
                    if first_line {
                        let columns = parse_csv_line(&line_buffer)?;
                        sink.on_columns(&columns)?;
                        first_line = false;
                    } else {
                        let values = parse_csv_line(&line_buffer)?
                            .into_iter()
                            .map(|s| {
                                if s.is_empty() {
                                    DbValue::Null
                                } else {
                                    DbValue::Text(s)
                                }
                            })
                            .collect::<Vec<_>>();
                        sink.on_row(&values)?;
                    }
                    line_buffer.clear();
                } else {
                    line_buffer.push(byte);
                }
            }
        }

        Ok(())
    }

    fn execute_sql(&mut self, sql: &str) -> Result<u64> {
        let conn = self.connection.as_mut().context("Database not connected")?;
        Ok(conn.execute(sql, &[])?)
    }

    fn direct_export(
        &mut self,
        query: &str,
        writer: &mut dyn Write,
        format: &ExportConfig,
    ) -> Result<(u64, u64)> {
        let conn = self.connection.as_mut().context("Database not connected")?;

        // Count rows if requested
        let row_count = if format.count_rows {
            let count_query = format!("SELECT COUNT(*) FROM ({}) AS __count_subquery", query);
            conn.query_one(&count_query, &[])?.get::<_, i64>(0) as u64
        } else {
            0
        };

        let delimiter = if format.delimiter.len() == 1 {
            format.delimiter.chars().next().unwrap()
        } else {
            return Err(anyhow!(
                "PostgreSQL COPY only supports single-byte delimiters"
            ));
        };

        let copy_query = match format.format {
            ExportFormat::Csv => {
                format!(
                    "COPY ({}) TO STDOUT WITH (FORMAT CSV, DELIMITER '{}', NULL '', HEADER {})",
                    query, delimiter, format.include_header
                )
            }
            ExportFormat::Tsv => {
                format!(
                    "COPY ({}) TO STDOUT WITH (FORMAT CSV, DELIMITER E'\\t', NULL '', HEADER {})",
                    query, format.include_header
                )
            }
            ExportFormat::Custom => {
                format!(
                    "COPY ({}) TO STDOUT WITH (FORMAT TEXT, DELIMITER '{}', NULL '', HEADER {})",
                    query, delimiter, format.include_header
                )
            }
        };

        let mut reader = conn.copy_out(&copy_query)?;
        let mut buffer = [0u8; 8192];
        let mut total_bytes = 0u64;
        let export_start = Instant::now();
        let mut last_progress_time = Instant::now();

        loop {
            let n = reader.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buffer[..n])?;
            total_bytes += n as u64;

            if last_progress_time.elapsed().as_secs_f64() >= format.progress_interval_secs as f64 {
                let elapsed = export_start.elapsed().as_secs_f64();
                let bytes_per_sec = total_bytes as f64 / elapsed;
                info!(
                    bytes_written = total_bytes,
                    speed_bytes_per_sec = bytes_per_sec as u64,
                    elapsed_secs = elapsed as u64,
                    progress_interval_secs = format.progress_interval_secs,
                    "export_progress"
                );
                last_progress_time = Instant::now();
            }
        }

        Ok((total_bytes, row_count))
    }

    fn prepare_import(
        &mut self,
        table: &str,
        _external_columns: &[String],
        _selected_source_columns: &[String],
        target_columns: &[String],
        _column_types: &HashMap<String, String>,
        _config: &crate::config::ImportConfig,
    ) -> Result<Box<dyn ImportSession>> {
        let conn = self.connection.take().context("Database not connected")?;

        Ok(Box::new(PostgresCopySession {
            conn,
            table: table.to_string(),
            columns: target_columns.to_vec(),
            rows_inserted: 0,
            start_time: Instant::now(),
        }))
    }
}

struct PostgresCopySession {
    conn: Client,
    table: String,
    columns: Vec<String>,
    rows_inserted: u64,
    start_time: Instant,
}

impl ImportSession for PostgresCopySession {
    fn insert_batch(&mut self, rows: &[Vec<DbValue>]) -> Result<usize> {
        use crate::value::ValueFormatter;
        let formatter = ValueFormatter::default();

        let cols = self.columns.join(", ");
        let copy_sql = format!(
            "COPY {} ({}) FROM STDIN WITH (FORMAT CSV)",
            self.table, cols
        );
        let mut writer = self.conn.copy_in(&copy_sql)?;

        for row in rows {
            let mut line = String::new();
            for (i, value) in row.iter().enumerate() {
                if i > 0 {
                    line.push(',');
                }
                let s = formatter.format(value);
                if s.contains(',') || s.contains('"') || s.contains('\n') {
                    line.push('"');
                    line.push_str(&s.replace('"', "\"\""));
                    line.push('"');
                } else {
                    line.push_str(&s);
                }
            }
            line.push('\n');
            writer.write_all(line.as_bytes())?;
        }

        writer.finish()?;
        self.rows_inserted += rows.len() as u64;
        Ok(rows.len())
    }

    fn commit(&mut self) -> Result<()> {
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

fn parse_csv_line(line: &[u8]) -> Result<Vec<String>> {
    let line_str = String::from_utf8_lossy(line);
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line_str.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                result.push(current.clone());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    result.push(current);

    Ok(result)
}
