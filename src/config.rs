use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    #[serde(default)]
    pub export: Option<ExportConfig>,
    #[serde(default)]
    pub import: Option<ImportConfig>,
    #[serde(default)]
    pub vars: HashMap<String, String>,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default)]
    pub log_file: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub verbose: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            log_file: None,
            tag: None,
            verbose: false,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub db_type: String,
    pub connection_string: String,
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_fetch_size")]
    pub fetch_size: usize,
    #[serde(default)]
    pub gpfdist_host: Option<String>,
    #[serde(default)]
    pub gpfdist_port: Option<u16>,
    #[serde(default)]
    pub gpfdist_dir: Option<String>,
}

impl fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let password = if self.password.is_empty() { "" } else { "***" };

        f.debug_struct("DatabaseConfig")
            .field("db_type", &self.db_type)
            .field("connection_string", &self.connection_string)
            .field("username", &self.username)
            .field("password", &password)
            .field("fetch_size", &self.fetch_size)
            .field("gpfdist_host", &self.gpfdist_host)
            .field("gpfdist_port", &self.gpfdist_port)
            .field("gpfdist_dir", &self.gpfdist_dir)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    pub query: String,
    pub output_file: String,
    pub format: ExportFormat,
    #[serde(default = "default_delimiter")]
    pub delimiter: String,
    #[serde(default)]
    pub include_header: bool,
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    #[serde(default)]
    pub compression: CompressionType,
    #[serde(default = "default_progress_interval_secs")]
    pub progress_interval_secs: u64,
    #[serde(default)]
    pub skip_errors: bool,
    #[serde(default)]
    pub count_rows: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Tsv,
    Custom,
}

impl FromStr for ExportFormat {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "csv" => Ok(Self::Csv),
            "tsv" => Ok(Self::Tsv),
            "custom" => Ok(Self::Custom),
            _ => Err(format!("unsupported export format: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressionType {
    None,
    Gzip,
}

impl FromStr for CompressionType {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "none" => Ok(Self::None),
            "gzip" => Ok(Self::Gzip),
            _ => Err(format!("unsupported compression type: {value}")),
        }
    }
}

impl Default for CompressionType {
    fn default() -> Self {
        CompressionType::None
    }
}

fn default_delimiter() -> String {
    "\x03".to_string()
}

fn default_buffer_size() -> usize {
    1024 * 1024
}

fn default_fetch_size() -> usize {
    1000
}

fn default_progress_interval_secs() -> u64 {
    30
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            db_type: "oracle".to_string(),
            connection_string: "localhost:1521/ORCL".to_string(),
            username: String::new(),
            password: String::new(),
            fetch_size: default_fetch_size(),
            gpfdist_host: None,
            gpfdist_port: None,
            gpfdist_dir: None,
        }
    }
}

impl Config {
    pub fn from_file(path: &str) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportConfig {
    #[serde(default)]
    pub schema: Option<String>,
    pub table: String,
    pub input_file: String,
    #[serde(default)]
    pub source_columns: Option<Vec<String>>,
    #[serde(default)]
    pub target_columns: Option<Vec<String>>,
    #[serde(default)]
    pub column_mapping: Option<HashMap<String, String>>,
    #[serde(default)]
    pub column_expressions: Option<HashMap<String, String>>,
    #[serde(default)]
    pub skip_columns: Option<Vec<String>>,
    #[serde(default)]
    pub column_types: Option<HashMap<String, String>>,
    #[serde(default)]
    pub format: ImportFormat,
    #[serde(default = "default_delimiter")]
    pub delimiter: String,
    #[serde(default)]
    pub escape: Option<String>,
    #[serde(default = "default_true")]
    pub has_header: bool,
    #[serde(default = "default_batch_size")]
    pub batch_size: usize,
    #[serde(default)]
    pub null_value: String,
    #[serde(default)]
    pub on_error: ErrorStrategy,
    #[serde(default)]
    pub transaction_mode: TransactionMode,
    #[serde(default)]
    pub show_progress: bool,
    #[serde(default = "default_progress_interval_secs")]
    pub progress_interval_secs: u64,
    #[serde(default)]
    pub truncate_table: bool,
    #[serde(default)]
    pub pre_sql: Option<String>,
    #[serde(default)]
    pub post_sql: Option<String>,
    #[serde(default)]
    pub error_log_table: Option<String>,
    #[serde(default)]
    pub compression: CompressionType,
}

impl ImportConfig {
    pub fn resolved_schema(&self) -> Option<&str> {
        self.schema.as_deref()
    }

    pub fn resolved_table(&self) -> &str {
        &self.table
    }

    pub fn qualified_target_table(&self) -> String {
        if let Some(schema) = self.resolved_schema() {
            format!("{}.{}", schema, self.resolved_table())
        } else {
            self.resolved_table().to_string()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportFormat {
    Csv,
    Tsv,
    Custom,
}

impl Default for ImportFormat {
    fn default() -> Self {
        Self::Csv
    }
}

impl FromStr for ImportFormat {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "csv" => Ok(Self::Csv),
            "tsv" => Ok(Self::Tsv),
            "custom" => Ok(Self::Custom),
            _ => Err(format!("unsupported import format: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorStrategy {
    Skip,
    Abort,
}

impl Default for ErrorStrategy {
    fn default() -> Self {
        Self::Skip
    }
}

impl FromStr for ErrorStrategy {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "skip" => Ok(Self::Skip),
            "abort" => Ok(Self::Abort),
            _ => Err(format!("unsupported error strategy: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionMode {
    PerBatch,
    All,
    None,
}

impl Default for TransactionMode {
    fn default() -> Self {
        Self::PerBatch
    }
}

impl FromStr for TransactionMode {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "per_batch" => Ok(Self::PerBatch),
            "all" => Ok(Self::All),
            "none" => Ok(Self::None),
            _ => Err(format!("unsupported transaction mode: {value}")),
        }
    }
}

fn default_batch_size() -> usize {
    1000
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::Config;
    use super::DatabaseConfig;

    #[test]
    fn parses_database_config_without_password() {
        let raw = r#"
[database]
db_type = "postgresql"
connection_string = "localhost:5432/testdb"
username = "postgres"
"#;

        let config: Config = toml::from_str(raw).expect("config should parse without password");

        assert_eq!(config.database.password, "");
    }

    #[test]
    fn database_config_debug_redacts_password_only() {
        let config = DatabaseConfig {
            db_type: "postgresql".to_string(),
            connection_string: "postgresql://user:secret@localhost:5432/testdb".to_string(),
            username: "tester".to_string(),
            password: "secret".to_string(),
            fetch_size: 1000,
            gpfdist_host: Some("localhost".to_string()),
            gpfdist_port: Some(8080),
            gpfdist_dir: Some("/tmp".to_string()),
        };

        let debug = format!("{config:?}");

        assert!(debug.contains(r#"password: "***""#));
        assert!(!debug.contains(r#"password: "secret""#));
        assert!(debug.contains("postgresql://user:secret@localhost:5432/testdb"));
    }
}
