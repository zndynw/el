pub mod oracle;

use anyhow::Result;

pub trait Database {
    fn connect(&mut self) -> Result<()>;
    fn execute_query(&mut self, query: &str) -> Result<QueryResult>;
}

pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl QueryResult {
    pub fn new() -> Self {
        Self {
            columns: Vec::new(),
            rows: Vec::new(),
        }
    }
}
