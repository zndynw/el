pub mod greenplum;
pub mod mysql;
pub mod oracle;
mod pgpass;
pub mod postgresql;

use crate::value::DbValue;
use anyhow::Result;
use std::collections::HashMap;
use std::io::Write;
use std::time::Duration;

pub trait Database {
    fn connect(&mut self) -> Result<()>;
    fn stream_query(&mut self, query: &str, sink: &mut dyn QuerySink) -> Result<()>;

    fn execute_sql(&mut self, _sql: &str) -> Result<u64> {
        Err(anyhow::anyhow!("execute_sql not supported"))
    }

    fn direct_import(
        &mut self,
        _config: &crate::config::ImportConfig,
    ) -> Result<Option<ImportStats>> {
        Ok(None)
    }

    fn direct_export(
        &mut self,
        _query: &str,
        _writer: &mut dyn Write,
        _format: &crate::config::ExportConfig,
    ) -> Result<(u64, u64)> {
        Err(anyhow::anyhow!("direct export not supported"))
    }

    fn prepare_import(
        &mut self,
        _table: &str,
        _external_columns: &[String],
        _selected_source_columns: &[String],
        _target_columns: &[String],
        _column_types: &HashMap<String, String>,
        _config: &crate::config::ImportConfig,
    ) -> Result<Box<dyn ImportSession>> {
        Err(anyhow::anyhow!("import not supported"))
    }
}

pub trait QuerySink {
    fn on_columns(&mut self, columns: &[String]) -> Result<()>;
    fn on_row(&mut self, values: &[DbValue]) -> Result<()>;
}

pub trait ImportSession {
    fn insert_batch(&mut self, rows: &[Vec<DbValue>]) -> Result<usize>;
    fn commit(&mut self) -> Result<()>;
    fn finish(self: Box<Self>) -> Result<ImportStats>;
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub struct ImportStats {
    pub rows_inserted: u64,
    pub rows_failed: u64,
    pub duration: Duration,
}
