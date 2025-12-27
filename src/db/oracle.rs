use crate::config::DatabaseConfig;
use crate::db::{Database, QueryResult};
use anyhow::{Context, Result};
use oracle::{Connection, Row};

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
        // connection_string格式: host:port/service_name
        // 添加//前缀以符合Oracle连接字符串格式
        format!("//{}", self.config.connection_string)
    }

    fn row_to_strings(&self, row: &Row, col_count: usize) -> Result<Vec<String>> {
        let mut values = Vec::with_capacity(col_count);
        
        for i in 0..col_count {
            let value: Option<String> = row.get(i)?;
            values.push(value.unwrap_or_default());
        }
        
        Ok(values)
    }

    pub fn get_column_info(&mut self, query: &str) -> Result<Vec<String>> {
        let conn = self
            .connection
            .as_ref()
            .context("Database not connected")?;

        let mut stmt = conn.statement(query)
            .fetch_array_size(1)
            .build()?;

        let rows = stmt.query(&[])?;
        
        let columns: Vec<String> = rows
            .column_info()
            .iter()
            .map(|col| col.name().to_string())
            .collect();

        Ok(columns)
    }

    pub fn execute_query_streaming<F>(&mut self, query: &str, mut callback: F) -> Result<Vec<String>>
    where
        F: FnMut(Vec<String>) -> Result<()>,
    {
        let conn = self
            .connection
            .as_ref()
            .context("Database not connected")?;

        let mut stmt = conn.statement(query)
            .fetch_array_size(self.config.fetch_size as u32)
            .build()?;

        let rows = stmt.query(&[])?;
        
        let columns: Vec<String> = rows
            .column_info()
            .iter()
            .map(|col| col.name().to_string())
            .collect();

        let col_count = columns.len();

        for row_result in rows {
            let row = row_result?;
            let values = self.row_to_strings(&row, col_count)?;
            callback(values)?;
        }

        Ok(columns)
    }
}

impl Database for OracleDatabase {
    fn connect(&mut self) -> Result<()> {
        let conn_str = self.build_connection_string();
        let conn = Connection::connect(
            &self.config.username,
            &self.config.password,
            &conn_str,
        )
        .context("Failed to connect to Oracle database")?;

        self.connection = Some(conn);
        Ok(())
    }

    fn execute_query(&mut self, query: &str) -> Result<QueryResult> {
        let conn = self
            .connection
            .as_ref()
            .context("Database not connected")?;

        let mut stmt = conn.statement(query).build()?;
        let rows = stmt.query(&[])?;
        
        let columns: Vec<String> = rows
            .column_info()
            .iter()
            .map(|col| col.name().to_string())
            .collect();
        
        let col_count = columns.len();
        
        let mut result = QueryResult::new();
        result.columns = columns;
        
        for row_result in rows {
            let row = row_result?;
            let values = self.row_to_strings(&row, col_count)?;
            result.rows.push(values);
        }

        Ok(result)
    }
}
