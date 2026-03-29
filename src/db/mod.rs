pub mod mysql;
pub mod oracle;

use crate::value::DbValue;
use anyhow::Result;

pub trait Database {
    fn connect(&mut self) -> Result<()>;
    fn stream_query(&mut self, query: &str, sink: &mut dyn QuerySink) -> Result<()>;
}

pub trait QuerySink {
    fn on_columns(&mut self, columns: &[String]) -> Result<()>;
    fn on_row(&mut self, values: &[DbValue]) -> Result<()>;
}
