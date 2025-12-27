use crate::config::{CompressionType, ExportConfig, ExportFormat};
use crate::db::oracle::OracleDatabase;
use anyhow::{Context, Result};
use csv::WriterBuilder;
use flate2::write::GzEncoder;
use flate2::Compression;
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub struct Exporter {
    config: ExportConfig,
}

impl Exporter {
    pub fn new(config: ExportConfig) -> Self {
        Self { config }
    }

    pub fn export(&mut self, db: &mut OracleDatabase) -> Result<ExportStats> {
        let start_time = Instant::now();
        let row_count = Arc::new(AtomicU64::new(0));
        let mut io_write_time = 0.0;
        
        let progress_bar = if self.config.show_progress {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.green} [{elapsed_precise}] {msg}")
                    .unwrap()
            );
            Some(pb)
        } else {
            None
        };

        let file = File::create(&self.config.output_file)
            .context("Failed to create output file")?;
        
        let writer: Box<dyn Write> = match self.config.compression {
            CompressionType::Gzip => {
                Box::new(BufWriter::with_capacity(
                    self.config.buffer_size,
                    GzEncoder::new(file, Compression::default())
                ))
            }
            CompressionType::None => {
                Box::new(BufWriter::with_capacity(self.config.buffer_size, file))
            }
        };
        let mut writer = writer;

        let delimiter = self.get_delimiter();
        
        // 先获取列信息
        let columns = db.get_column_info(&self.config.query)?;
        
        // 如果需要表头，先写入
        if self.config.include_header {
            self.write_row(&mut *writer, &columns, delimiter)?;
        }
        
        // 流式写入数据
        let row_count_clone = Arc::clone(&row_count);
        let pb_clone = progress_bar.clone();
        
        let db_start = Instant::now();
        db.execute_query_streaming(&self.config.query, |row_values| {
            let count = row_count_clone.fetch_add(1, Ordering::Relaxed) + 1;
            
            if let Some(ref pb) = pb_clone {
                if count % 1000 == 0 {
                    pb.set_message(format!("Exported {} rows", count));
                }
            }
            
            let io_start = Instant::now();
            self.write_row(&mut *writer, &row_values, delimiter)?;
            io_write_time += io_start.elapsed().as_secs_f64();
            Ok(())
        })?;
        let db_read_time = db_start.elapsed().as_secs_f64();
        
        writer.flush()?;

        if let Some(pb) = progress_bar {
            pb.finish_with_message(format!("Export completed: {} rows", row_count.load(Ordering::Relaxed)));
        }

        let duration = start_time.elapsed();
        let file_size = std::fs::metadata(&self.config.output_file)?.len();
        let rows = row_count.load(Ordering::Relaxed);
        let avg_row_size = if rows > 0 {
            file_size as f64 / rows as f64
        } else {
            0.0
        };

        Ok(ExportStats {
            rows_exported: rows,
            duration_secs: duration.as_secs_f64(),
            file_size_bytes: file_size,
            db_read_time_secs: db_read_time,
            io_write_time_secs: io_write_time,
            avg_row_size_bytes: avg_row_size,
        })
    }

    fn get_delimiter(&self) -> u8 {
        match self.config.format {
            ExportFormat::Csv => {
                if self.config.delimiter.len() == 1 {
                    self.config.delimiter.as_bytes()[0]
                } else {
                    b','
                }
            },
            ExportFormat::Tsv => b'\t',
            ExportFormat::Custom => {
                if self.config.delimiter.len() == 1 {
                    self.config.delimiter.as_bytes()[0]
                } else {
                    b','
                }
            }
        }
    }

    fn write_row(&self, writer: &mut dyn Write, values: &[String], delimiter: u8) -> Result<()> {
        let buffer = Vec::with_capacity(1024);
        let mut csv_writer = WriterBuilder::new()
            .delimiter(delimiter)
            .from_writer(buffer);
        
        csv_writer.write_record(values)?;
        let data = csv_writer.into_inner()?;
        writer.write_all(&data)?;
        
        Ok(())
    }
}

pub struct ExportStats {
    pub rows_exported: u64,
    pub duration_secs: f64,
    pub file_size_bytes: u64,
    pub db_read_time_secs: f64,
    pub io_write_time_secs: f64,
    pub avg_row_size_bytes: f64,
}

impl ExportStats {
    pub fn print_summary(&self) {
        println!("\n=== Export Summary ===");
        println!("Rows exported: {}", self.rows_exported);
        println!("Duration: {:.2} seconds", self.duration_secs);
        println!("File size: {} bytes ({:.2} MB)", 
            self.file_size_bytes, 
            self.file_size_bytes as f64 / 1024.0 / 1024.0
        );
        
        if self.duration_secs > 0.0 {
            let rows_per_sec = self.rows_exported as f64 / self.duration_secs;
            println!("Speed: {:.2} rows/second", rows_per_sec);
        }
        
        println!("\n=== Performance Details ===");
        println!("DB read time: {:.2} seconds ({:.1}%)", 
            self.db_read_time_secs,
            (self.db_read_time_secs / self.duration_secs) * 100.0
        );
        println!("I/O write time: {:.2} seconds ({:.1}%)", 
            self.io_write_time_secs,
            (self.io_write_time_secs / self.duration_secs) * 100.0
        );
        println!("Average row size: {:.2} bytes", self.avg_row_size_bytes);
        
        if self.rows_exported > 0 {
            let mb_per_sec = (self.file_size_bytes as f64 / 1024.0 / 1024.0) / self.duration_secs;
            println!("Throughput: {:.2} MB/second", mb_per_sec);
        }
    }
}
