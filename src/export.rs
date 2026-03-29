use crate::config::{CompressionType, ExportConfig, ExportFormat};
use crate::db::{Database, QuerySink};
use crate::value::{DbValue, ValueFormatter};
use anyhow::{Context, Result};
use csv::WriterBuilder;
use flate2::Compression;
use flate2::write::GzEncoder;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;
use tracing::info;

pub struct Exporter {
    config: ExportConfig,
}

impl Exporter {
    pub fn new(config: ExportConfig) -> Self {
        Self { config }
    }

    pub fn export(&mut self, db: &mut dyn Database) -> Result<ExportStats> {
        let start_time = Instant::now();

        // Try direct export first (PostgreSQL optimization)
        if let Ok((bytes_written, row_count)) = self.try_direct_export(db) {
            let duration = start_time.elapsed();
            let file_size = std::fs::metadata(&self.config.output_file)?.len();
            let avg_row_size = if row_count > 0 {
                file_size as f64 / row_count as f64
            } else {
                0.0
            };

            if self.config.show_progress {
                if row_count > 0 {
                    info!("Export completed: {} rows", row_count);
                } else {
                    info!("Export completed: {} bytes written", bytes_written);
                }
            }

            return Ok(ExportStats {
                rows_exported: row_count,
                rows_skipped: 0,
                duration_secs: duration.as_secs_f64(),
                file_size_bytes: file_size,
                db_read_time_secs: duration.as_secs_f64(),
                io_write_time_secs: 0.0,
                avg_row_size_bytes: avg_row_size,
                output_file: self.config.output_file.clone(),
            });
        }

        // Fallback to traditional stream_query
        let db_start = Instant::now();
        let (rows, skipped, io_write_time) = {
            let mut sink = ExportSink::new(&self.config)?;
            db.stream_query(&self.config.query, &mut sink)?;
            sink.finish()?;
            (
                sink.rows_exported,
                sink.rows_skipped,
                sink.io_write_time_secs,
            )
        };
        let db_read_time = db_start.elapsed().as_secs_f64();

        if self.config.show_progress {
            if skipped > 0 {
                info!("Export completed: {} rows ({} skipped)", rows, skipped);
            } else {
                info!("Export completed: {} rows", rows);
            }
        }

        let duration = start_time.elapsed();
        let file_size = std::fs::metadata(&self.config.output_file)?.len();
        let avg_row_size = if rows > 0 {
            file_size as f64 / rows as f64
        } else {
            0.0
        };

        Ok(ExportStats {
            rows_exported: rows,
            rows_skipped: skipped,
            duration_secs: duration.as_secs_f64(),
            file_size_bytes: file_size,
            db_read_time_secs: db_read_time,
            io_write_time_secs: io_write_time,
            avg_row_size_bytes: avg_row_size,
            output_file: self.config.output_file.clone(),
        })
    }

    fn try_direct_export(&self, db: &mut dyn Database) -> Result<(u64, u64)> {
        let file = File::create(&self.config.output_file)?;
        let mut writer: Box<dyn Write> = match self.config.compression {
            CompressionType::Gzip => Box::new(BufWriter::with_capacity(
                self.config.buffer_size,
                GzEncoder::new(file, Compression::default()),
            )),
            CompressionType::None => Box::new(BufWriter::with_capacity(self.config.buffer_size, file)),
        };

        db.direct_export(&self.config.query, writer.as_mut(), &self.config)
    }

    fn get_delimiter(config: &ExportConfig) -> u8 {
        match config.format {
            ExportFormat::Csv => {
                if config.delimiter.len() == 1 {
                    config.delimiter.as_bytes()[0]
                } else {
                    b','
                }
            }
            ExportFormat::Tsv => b'\t',
            ExportFormat::Custom => {
                if config.delimiter.len() == 1 {
                    config.delimiter.as_bytes()[0]
                } else {
                    b','
                }
            }
        }
    }
}

struct ExportSink {
    config: ExportConfig,
    formatter: ValueFormatter,
    writer: RecordWriter,
    rows_exported: u64,
    rows_skipped: u64,
    io_write_time_secs: f64,
    query_started_at: Instant,
}

enum RecordWriter {
    Csv(csv::Writer<Box<dyn Write>>),
    Custom {
        writer: Box<dyn Write>,
        delimiter: String,
    },
}

impl ExportSink {
    fn new(config: &ExportConfig) -> Result<Self> {
        let file = File::create(&config.output_file).context("Failed to create output file")?;

        let writer: Box<dyn Write> = match config.compression {
            CompressionType::Gzip => Box::new(BufWriter::with_capacity(
                config.buffer_size,
                GzEncoder::new(file, Compression::default()),
            )),
            CompressionType::None => Box::new(BufWriter::with_capacity(config.buffer_size, file)),
        };

        let writer = match config.format {
            ExportFormat::Custom => RecordWriter::Custom {
                writer,
                delimiter: config.delimiter.clone(),
            },
            ExportFormat::Csv | ExportFormat::Tsv => RecordWriter::Csv(
                WriterBuilder::new()
                    .delimiter(Exporter::get_delimiter(config))
                    .from_writer(writer),
            ),
        };

        Ok(Self {
            config: config.clone(),
            formatter: ValueFormatter::default(),
            writer,
            rows_exported: 0,
            rows_skipped: 0,
            io_write_time_secs: 0.0,
            query_started_at: Instant::now(),
        })
    }

    fn finish(&mut self) -> Result<()> {
        self.writer.flush()?;
        Ok(())
    }
}

impl QuerySink for ExportSink {
    fn on_columns(&mut self, columns: &[String]) -> Result<()> {
        if self.config.include_header {
            self.writer.write_strings(columns)?;
        }

        Ok(())
    }

    fn on_row(&mut self, values: &[DbValue]) -> Result<()> {
        let formatted_values: Vec<String> = values
            .iter()
            .map(|value| self.formatter.format(value))
            .collect();
        let io_start = Instant::now();
        let write_result = self.writer.write_strings(&formatted_values);
        self.io_write_time_secs += io_start.elapsed().as_secs_f64();

        match write_result {
            Ok(_) => {
                self.rows_exported += 1;

                if self.config.show_progress
                    && self.rows_exported % self.config.progress_interval == 0
                {
                    let elapsed = self.query_started_at.elapsed().as_secs_f64();
                    let speed = self.rows_exported as f64 / elapsed;
                    info!(
                        "Progress: {} rows exported ({:.2} rows/sec)",
                        self.rows_exported, speed
                    );
                }
                Ok(())
            }
            Err(e) => {
                if self.config.skip_errors {
                    self.rows_skipped += 1;
                    tracing::warn!("Skipped row due to error: {}", e);
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    }
}

impl RecordWriter {
    fn write_strings<S>(&mut self, values: &[S]) -> Result<()>
    where
        S: AsRef<str>,
    {
        match self {
            Self::Csv(writer) => {
                writer.write_record(values.iter().map(AsRef::as_ref))?;
                Ok(())
            }
            Self::Custom { writer, delimiter } => {
                write_custom_record(writer.as_mut(), delimiter, values)?;
                Ok(())
            }
        }
    }

    fn flush(&mut self) -> Result<()> {
        match self {
            Self::Csv(writer) => {
                writer.flush()?;
                Ok(())
            }
            Self::Custom { writer, .. } => {
                writer.flush()?;
                Ok(())
            }
        }
    }
}

fn write_custom_record<S>(writer: &mut dyn Write, delimiter: &str, values: &[S]) -> Result<()>
where
    S: AsRef<str>,
{
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            writer.write_all(delimiter.as_bytes())?;
        }
        writer.write_all(value.as_ref().as_bytes())?;
    }
    writer.write_all(b"\n")?;
    Ok(())
}

pub struct ExportStats {
    pub rows_exported: u64,
    pub rows_skipped: u64,
    pub duration_secs: f64,
    pub file_size_bytes: u64,
    pub db_read_time_secs: f64,
    pub io_write_time_secs: f64,
    pub avg_row_size_bytes: f64,
    pub output_file: String,
}

impl ExportStats {
    pub fn print_summary(&self) {
        info!("Export Summary:");
        info!("  Output file: {}", self.output_file);
        info!("  Rows exported: {}", self.rows_exported);
        if self.rows_skipped > 0 {
            info!("  Rows skipped: {}", self.rows_skipped);
        }
        info!("  Duration: {:.2} seconds", self.duration_secs);
        info!(
            "  File size: {} bytes ({:.2} MB)",
            self.file_size_bytes,
            self.file_size_bytes as f64 / 1024.0 / 1024.0
        );

        if self.duration_secs > 0.0 {
            let rows_per_sec = self.rows_exported as f64 / self.duration_secs;
            info!("  Speed: {:.2} rows/second", rows_per_sec);
        }

        info!("Performance Details:");
        info!(
            "  DB read time: {:.2} seconds ({:.1}%)",
            self.db_read_time_secs,
            (self.db_read_time_secs / self.duration_secs) * 100.0
        );
        info!(
            "  I/O write time: {:.2} seconds ({:.1}%)",
            self.io_write_time_secs,
            (self.io_write_time_secs / self.duration_secs) * 100.0
        );
        info!("  Average row size: {:.2} bytes", self.avg_row_size_bytes);

        if self.rows_exported > 0 {
            let mb_per_sec = (self.file_size_bytes as f64 / 1024.0 / 1024.0) / self.duration_secs;
            info!("  Throughput: {:.2} MB/second", mb_per_sec);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::write_custom_record;

    #[test]
    fn custom_writer_keeps_quotes_unchanged() {
        let mut output = Vec::new();

        write_custom_record(&mut output, "\x03", &["1", "2", "xx\"yy\"zz", "4"])
            .expect("custom record should be written");

        assert_eq!(String::from_utf8(output).unwrap(), "1\x032\x03xx\"yy\"zz\x034\n");
    }

    #[test]
    fn custom_writer_does_not_add_csv_quoting_for_headers() {
        let mut output = Vec::new();

        write_custom_record(&mut output, "\x03", &["col1", "col2", "col3", "col4"])
            .expect("header should be written");

        assert_eq!(String::from_utf8(output).unwrap(), "col1\x03col2\x03col3\x03col4\n");
    }
}
