use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    pub export: ExportConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub db_type: String,
    pub connection_string: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub fetch_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportConfig {
    pub query: String,
    pub output_file: String,
    pub format: ExportFormat,
    #[serde(default = "default_delimiter")]
    pub delimiter: String,
    #[serde(default)]
    pub show_progress: bool,
    #[serde(default)]
    pub include_header: bool,
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
    #[serde(default)]
    pub compression: CompressionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Csv,
    Tsv,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressionType {
    None,
    Gzip,
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
    1024 * 1024  // 1MB
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            db_type: "oracle".to_string(),
            connection_string: "localhost:1521/ORCL".to_string(),
            username: String::new(),
            password: String::new(),
            fetch_size: 1000,
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
