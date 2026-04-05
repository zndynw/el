use crate::config::{CompressionType, ErrorStrategy, ImportConfig, TransactionMode};
use crate::db::Database;
use crate::value::DbValue;
use anyhow::{Context, Result, anyhow};
use csv::{Reader, StringRecord};
use flate2::read::GzDecoder;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::time::Instant;

pub struct Importer {
    db: Box<dyn Database>,
    config: ImportConfig,
}

impl Importer {
    pub fn new(db: Box<dyn Database>, config: ImportConfig) -> Self {
        Self { db, config }
    }

    pub fn import(&mut self) -> Result<()> {
        let import_start = Instant::now();

        tracing::trace!(
            input_file = %self.config.input_file,
            table = %self.config.qualified_target_table(),
            batch_size = self.config.batch_size,
            "import_config_details"
        );

        if let Err(e) = self.db.connect() {
            tracing::error!(error = %e, "db_connect_failed");
            return Err(e);
        }

        if let Some(stats) = self.db.direct_import(&self.config)? {
            tracing::debug!(
                table = %self.config.qualified_target_table(),
                method = "direct_import",
                "import_method_selected"
            );
            tracing::info!(
                status = if stats.rows_failed > 0 { "partial" } else { "success" },
                table = %self.config.qualified_target_table(),
                rows_inserted = stats.rows_inserted,
                rows_failed = stats.rows_failed,
                duration_ms = stats.duration.as_millis() as u64,
                "import_summary"
            );
            return Ok(());
        }

        tracing::debug!(method = "stream_import", "import_method_selected");

        if self.config.truncate_table {
            let truncate_start = Instant::now();
            self.truncate_table()?;
            tracing::trace!(
                duration_ms = truncate_start.elapsed().as_millis() as u64,
                "truncate_completed"
            );
        }

        let file_open_start = Instant::now();
        let reader = self.open_input_file().map_err(|e| {
            tracing::error!(
                input_file = %self.config.input_file,
                error = %e,
                "file_open_failed"
            );
            e
        })?;
        tracing::trace!(
            duration_ms = file_open_start.elapsed().as_millis() as u64,
            "file_opened"
        );

        let mut csv_reader = self.create_csv_reader(reader)?;

        let (source_columns, target_columns, column_indices) =
            self.resolve_columns(&mut csv_reader)?;
        let selected_source_columns = column_indices
            .iter()
            .map(|&idx| source_columns[idx].clone())
            .collect::<Vec<_>>();

        tracing::debug!(
            source_columns = ?source_columns,
            target_columns = ?target_columns,
            selected_source_columns = ?selected_source_columns,
            column_indices = ?column_indices,
            "column_projection_resolved"
        );

        if let Some(sql) = self.config.pre_sql.clone() {
            tracing::debug!(phase = "pre_sql", sql = %sql, "sql_preview");
            let pre_sql_start = Instant::now();
            self.execute_sql("pre_sql", &sql)?;
            tracing::trace!(
                phase = "pre_sql",
                duration_ms = pre_sql_start.elapsed().as_millis() as u64,
                "sql_completed"
            );
        }

        let column_types = self.config.column_types.clone().unwrap_or_default();
        let qualified_table = self.config.qualified_target_table();

        let prepare_start = Instant::now();
        let mut session = self.db.prepare_import(
            &qualified_table,
            &source_columns,
            &selected_source_columns,
            &target_columns,
            &column_types,
            &self.config,
        )?;
        tracing::trace!(
            duration_ms = prepare_start.elapsed().as_millis() as u64,
            "import_session_prepared"
        );

        let mut batch = Vec::new();
        let mut row_count = 0u64;
        let mut error_count = 0u64;
        let mut batch_count = 0u64;
        let mut last_progress_time = Instant::now();
        let progress_interval_secs = self.config.progress_interval_secs as f64;

        for result in csv_reader.records() {
            match result {
                Ok(record) => match self.parse_record(&record, &column_indices, &target_columns) {
                    Ok(values) => {
                        batch.push(values);
                        if batch.len() >= self.config.batch_size {
                            batch_count += 1;
                            let batch_start = Instant::now();
                            tracing::debug!(
                                batch_no = batch_count,
                                batch_size = batch.len(),
                                "batch_flush_start"
                            );
                            session.insert_batch(&batch)?;
                            let batch_duration = batch_start.elapsed();
                            tracing::trace!(
                                batch_no = batch_count,
                                duration_ms = batch_duration.as_millis() as u64,
                                rows_per_sec =
                                    (batch.len() as f64 / batch_duration.as_secs_f64()) as u64,
                                "batch_inserted"
                            );

                            if self.config.transaction_mode == TransactionMode::PerBatch {
                                let commit_start = Instant::now();
                                tracing::debug!(
                                    batch_no = batch_count,
                                    transaction_mode = ?self.config.transaction_mode,
                                    "transaction_commit"
                                );
                                session.commit()?;
                                tracing::trace!(
                                    duration_ms = commit_start.elapsed().as_millis() as u64,
                                    "transaction_committed"
                                );
                            }
                            row_count += batch.len() as u64;

                            // Time-based progress reporting
                            if self.config.show_progress {
                                let elapsed = last_progress_time.elapsed().as_secs_f64();
                                if elapsed >= progress_interval_secs {
                                    let total_elapsed = import_start.elapsed().as_secs_f64();
                                    let speed = row_count as f64 / total_elapsed;
                                    tracing::info!(
                                        rows_inserted = row_count,
                                        rows_failed = error_count,
                                        speed_rows_per_sec = speed as u64,
                                        elapsed_secs = total_elapsed as u64,
                                        progress_interval_secs = self.config.progress_interval_secs,
                                        "import_progress"
                                    );
                                    last_progress_time = Instant::now();
                                }
                            }

                            batch.clear();
                        }
                    }
                    Err(e) => {
                        error_count += 1;
                        if self.config.on_error == ErrorStrategy::Abort {
                            tracing::error!(
                                row_num = row_count + batch.len() as u64 + 1,
                                error = %e,
                                "row_parse_failed_abort"
                            );
                            return Err(e);
                        } else {
                            tracing::warn!(
                                row_num = row_count + batch.len() as u64 + 1,
                                rows_failed = error_count,
                                on_error = ?self.config.on_error,
                                reason = %e,
                                "row_skipped"
                            );
                        }
                    }
                },
                Err(e) => {
                    error_count += 1;
                    if self.config.on_error == ErrorStrategy::Abort {
                        tracing::error!(
                            error = %e,
                            "record_read_failed_abort"
                        );
                        return Err(e.into());
                    } else {
                        tracing::warn!(
                            rows_failed = error_count,
                            on_error = ?self.config.on_error,
                            reason = %e,
                            "record_skipped"
                        );
                    }
                }
            }
        }

        if !batch.is_empty() {
            batch_count += 1;
            let batch_start = Instant::now();
            tracing::debug!(
                batch_no = batch_count,
                batch_size = batch.len(),
                "batch_flush_start"
            );
            session.insert_batch(&batch)?;
            tracing::trace!(
                batch_no = batch_count,
                duration_ms = batch_start.elapsed().as_millis() as u64,
                "batch_inserted"
            );
            row_count += batch.len() as u64;
        }

        if self.config.transaction_mode == TransactionMode::All {
            let commit_start = Instant::now();
            tracing::debug!(
                transaction_mode = ?self.config.transaction_mode,
                "transaction_commit"
            );
            session.commit()?;
            tracing::trace!(
                duration_ms = commit_start.elapsed().as_millis() as u64,
                "transaction_committed"
            );
        }

        let _stats = session.finish()?;

        if let Some(sql) = self.config.post_sql.clone() {
            tracing::debug!(phase = "post_sql", "db_reconnect_start");
            if let Err(e) = self.db.connect() {
                tracing::error!(phase = "post_sql", error = %e, "db_reconnect_failed");
                return Err(e);
            }
            tracing::debug!(phase = "post_sql", sql = %sql, "sql_preview");
            let post_sql_start = Instant::now();
            self.execute_sql("post_sql", &sql)?;
            tracing::trace!(
                phase = "post_sql",
                duration_ms = post_sql_start.elapsed().as_millis() as u64,
                "sql_completed"
            );
        }

        let total_duration = import_start.elapsed();
        let speed = if total_duration.as_secs_f64() > 0.0 {
            row_count as f64 / total_duration.as_secs_f64()
        } else {
            0.0
        };

        tracing::info!(
            status = if error_count > 0 { "partial" } else { "success" },
            table = %self.config.qualified_target_table(),
            rows_inserted = row_count,
            rows_failed = error_count,
            batch_count = batch_count,
            duration_ms = total_duration.as_millis() as u64,
            speed_rows_per_sec = speed as u64,
            "import_summary"
        );

        Ok(())
    }

    fn truncate_table(&mut self) -> Result<()> {
        let table = self.config.qualified_target_table();
        let sql = format!("TRUNCATE TABLE {}", table);
        if let Err(e) = self.db.execute_sql(&sql) {
            tracing::error!(table = %table, error = %e, "table_truncate_failed");
            return Err(e);
        }
        tracing::info!(table = %table, "table_truncated");
        Ok(())
    }

    fn execute_sql(&mut self, phase: &str, sql: &str) -> Result<()> {
        let affected = match self.db.execute_sql(sql) {
            Ok(n) => n,
            Err(e) => {
                tracing::error!(phase = phase, sql = %sql, error = %e, "sql_execution_failed");
                return Err(e);
            }
        };
        tracing::info!(phase = phase, affected_rows = affected, sql = %sql, "sql_executed");
        Ok(())
    }

    fn open_input_file(&self) -> Result<Box<dyn Read>> {
        let file = File::open(&self.config.input_file).context("Failed to open input file")?;

        tracing::trace!(
            input_file = %self.config.input_file,
            compression = ?self.config.compression,
            "file_opening"
        );

        let reader: Box<dyn Read> = match self.config.compression {
            CompressionType::Gzip => Box::new(GzDecoder::new(file)),
            CompressionType::None => Box::new(file),
        };

        Ok(reader)
    }

    fn create_csv_reader(&self, reader: Box<dyn Read>) -> Result<Reader<BufReader<Box<dyn Read>>>> {
        let delimiter = if self.config.delimiter.is_empty() {
            b','
        } else {
            self.config.delimiter.as_bytes()[0]
        };

        let reader = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(self.config.has_header)
            .from_reader(BufReader::new(reader));

        Ok(reader)
    }

    fn resolve_columns(
        &self,
        csv_reader: &mut Reader<BufReader<Box<dyn Read>>>,
    ) -> Result<(Vec<String>, Vec<String>, Vec<usize>)> {
        let csv_columns = if self.config.has_header {
            csv_reader
                .headers()?
                .iter()
                .map(|s| s.to_string())
                .collect()
        } else {
            self.config
                .source_columns
                .clone()
                .context("source_columns must be specified when has_header is false")?
        };

        let (final_target_columns, column_indices) =
            resolve_column_projection(&csv_columns, &self.config)?;

        if column_indices.is_empty() {
            return Err(anyhow!("No columns to import after filtering"));
        }

        Ok((csv_columns, final_target_columns, column_indices))
    }

    fn parse_record(
        &self,
        record: &StringRecord,
        column_indices: &[usize],
        target_columns: &[String],
    ) -> Result<Vec<DbValue>> {
        let mut values = Vec::new();
        for (i, &idx) in column_indices.iter().enumerate() {
            let s = record.get(idx).unwrap_or("");

            let type_hint = self
                .config
                .column_types
                .as_ref()
                .and_then(|types| types.get(&target_columns[i]))
                .map(|s| s.as_str());

            match DbValue::from_str(s, &self.config.null_value, type_hint) {
                Ok(v) => values.push(v),
                Err(e) => {
                    tracing::trace!(
                        column = %target_columns[i],
                        column_index = idx,
                        value = %s,
                        type_hint = ?type_hint,
                        error = %e,
                        "value_parse_failed"
                    );
                    return Err(e);
                }
            }
        }
        Ok(values)
    }
}

fn resolve_column_projection(
    csv_columns: &[String],
    config: &ImportConfig,
) -> Result<(Vec<String>, Vec<usize>)> {
    if !config.has_header {
        let configured_columns = config
            .source_columns
            .as_ref()
            .context("source_columns must be specified when has_header is false")?;

        if configured_columns.len() != csv_columns.len() {
            return Err(anyhow!(
                "source_columns count ({}) must match input column count ({}) when has_header is false",
                configured_columns.len(),
                csv_columns.len()
            ));
        }

        let available: HashMap<String, usize> = configured_columns
            .iter()
            .enumerate()
            .map(|(idx, col)| (col.clone(), idx))
            .collect();

        if let Some(target_columns) = &config.target_columns {
            let mut column_indices = Vec::with_capacity(target_columns.len());
            for target_col in target_columns {
                let source_col = config
                    .column_mapping
                    .as_ref()
                    .and_then(|mapping| {
                        mapping
                            .iter()
                            .find_map(|(source, target)| (target == target_col).then_some(source))
                    })
                    .unwrap_or(target_col);
                let idx = available.get(source_col).copied().ok_or_else(|| {
                    anyhow!(
                        "target column '{}' is not mapped from source_columns",
                        target_col
                    )
                })?;
                column_indices.push(idx);
            }
            return Ok((target_columns.clone(), column_indices));
        }

        let mut final_target_columns = Vec::new();
        let mut column_indices = Vec::new();
        for (idx, source_col) in configured_columns.iter().enumerate() {
            if config
                .skip_columns
                .as_ref()
                .map(|v| v.iter().any(|col| col == source_col))
                .unwrap_or(false)
            {
                continue;
            }

            let target_col = config
                .column_mapping
                .as_ref()
                .and_then(|mapping| mapping.get(source_col))
                .unwrap_or(source_col)
                .clone();
            column_indices.push(idx);
            final_target_columns.push(target_col);
        }

        return Ok((final_target_columns, column_indices));
    }

    let skip_set: HashSet<&str> = config
        .skip_columns
        .as_ref()
        .map(|v| v.iter().map(|s| s.as_str()).collect())
        .unwrap_or_default();
    let mapping = config.column_mapping.as_ref();

    let mut available: HashMap<String, usize> = HashMap::new();
    for (idx, csv_col) in csv_columns.iter().enumerate() {
        if skip_set.contains(csv_col.as_str()) {
            continue;
        }

        let target_col = mapping
            .and_then(|m| m.get(csv_col))
            .unwrap_or(csv_col)
            .clone();

        if available.insert(target_col.clone(), idx).is_some() {
            return Err(anyhow!(
                "multiple input columns map to the same target column: {}",
                target_col
            ));
        }
    }

    if let Some(target_columns) = &config.target_columns {
        let mut column_indices = Vec::with_capacity(target_columns.len());
        for target_col in target_columns {
            let idx = available.get(target_col).copied().ok_or_else(|| {
                anyhow!(
                    "target column '{}' is not present in input headers after mapping/filtering",
                    target_col
                )
            })?;
            column_indices.push(idx);
        }
        return Ok((target_columns.clone(), column_indices));
    }

    let mut column_indices = Vec::new();
    let mut final_target_columns = Vec::new();
    for (idx, csv_col) in csv_columns.iter().enumerate() {
        if skip_set.contains(csv_col.as_str()) {
            continue;
        }

        let target_col = mapping
            .and_then(|m| m.get(csv_col))
            .unwrap_or(csv_col)
            .clone();
        column_indices.push(idx);
        final_target_columns.push(target_col);
    }

    Ok((final_target_columns, column_indices))
}

#[cfg(test)]
mod tests {
    use super::resolve_column_projection;
    use crate::config::{
        CompressionType, ErrorStrategy, ImportConfig, ImportFormat, TransactionMode,
    };
    use std::collections::HashMap;

    fn base_import_config() -> ImportConfig {
        ImportConfig {
            schema: None,
            table: "t".to_string(),
            input_file: "input.dat".to_string(),
            source_columns: None,
            target_columns: None,
            column_mapping: None,
            column_expressions: None,
            skip_columns: None,
            column_types: None,
            format: ImportFormat::Csv,
            delimiter: ",".to_string(),
            escape: None,
            has_header: true,
            batch_size: 1000,
            null_value: "".to_string(),
            on_error: ErrorStrategy::Skip,
            transaction_mode: TransactionMode::PerBatch,
            show_progress: false,
            progress_interval_secs: 30,
            truncate_table: false,
            pre_sql: None,
            post_sql: None,
            error_log_table: None,
            compression: CompressionType::None,
        }
    }

    #[test]
    fn header_mode_respects_target_columns_order_and_subset() {
        let csv_columns = vec![
            "user_id".to_string(),
            "user_name".to_string(),
            "temp_field".to_string(),
            "email".to_string(),
            "created".to_string(),
        ];
        let mut config = base_import_config();
        config.target_columns = Some(vec![
            "id".to_string(),
            "name".to_string(),
            "email".to_string(),
            "created_at".to_string(),
        ]);
        config.skip_columns = Some(vec!["temp_field".to_string()]);
        config.column_mapping = Some(HashMap::from([
            ("user_id".to_string(), "id".to_string()),
            ("user_name".to_string(), "name".to_string()),
            ("created".to_string(), "created_at".to_string()),
        ]));

        let (target_columns, indices) =
            resolve_column_projection(&csv_columns, &config).expect("projection should resolve");

        assert_eq!(target_columns, vec!["id", "name", "email", "created_at"]);
        assert_eq!(indices, vec![0, 1, 3, 4]);
    }

    #[test]
    fn no_header_mode_requires_target_columns_count_to_match_input() {
        let csv_columns = vec!["c1".to_string(), "c2".to_string(), "c3".to_string()];
        let mut config = base_import_config();
        config.has_header = false;
        config.source_columns = Some(vec!["a".to_string(), "b".to_string()]);

        let err = resolve_column_projection(&csv_columns, &config)
            .expect_err("mismatched column count should fail");

        assert!(
            err.to_string()
                .contains("source_columns count (2) must match input column count (3)")
        );
    }

    #[test]
    fn no_header_mode_allows_five_source_columns_and_four_target_columns() {
        let csv_columns = vec![
            "c1".to_string(),
            "c2".to_string(),
            "c3".to_string(),
            "c4".to_string(),
            "c5".to_string(),
        ];
        let mut config = base_import_config();
        config.has_header = false;
        config.source_columns = Some(csv_columns.clone());
        config.target_columns = Some(vec![
            "id".to_string(),
            "name".to_string(),
            "amount".to_string(),
            "created_at".to_string(),
        ]);
        config.column_mapping = Some(HashMap::from([
            ("c1".to_string(), "id".to_string()),
            ("c2".to_string(), "name".to_string()),
            ("c4".to_string(), "amount".to_string()),
            ("c5".to_string(), "created_at".to_string()),
        ]));

        let (target_columns, indices) =
            resolve_column_projection(&csv_columns, &config).expect("projection should resolve");

        assert_eq!(target_columns, vec!["id", "name", "amount", "created_at"]);
        assert_eq!(indices, vec![0, 1, 3, 4]);
    }

    #[test]
    fn qualified_target_table_uses_separate_schema_and_table() {
        let mut config = base_import_config();
        config.schema = Some("htdw_bak".to_string());
        config.table = "test_d_risk".to_string();

        assert_eq!(config.qualified_target_table(), "htdw_bak.test_d_risk");
    }
}
