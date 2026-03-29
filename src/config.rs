use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database: DatabaseConfig,
    pub export: ExportConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default)]
    pub log_file: Option<String>,
    #[serde(default)]
    pub verbose: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            log_file: None,
            verbose: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub db_type: String,
    pub connection_string: String,
    pub username: String,
    pub password: String,
    #[serde(default = "default_fetch_size")]
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
    #[serde(default = "default_progress_interval")]
    pub progress_interval: u64,
    #[serde(default)]
    pub skip_errors: bool,
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

fn default_progress_interval() -> u64 {
    1_000_000
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            db_type: "oracle".to_string(),
            connection_string: "localhost:1521/ORCL".to_string(),
            username: String::new(),
            password: String::new(),
            fetch_size: default_fetch_size(),
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
