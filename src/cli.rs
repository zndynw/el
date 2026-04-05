use clap::{ArgAction, Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "el")]
#[command(about = "Data Export/Import Tool")]
#[command(version)]
pub struct Cli {
    /// Enable verbose logging
    #[arg(short, long, global = true, conflicts_with = "quiet")]
    pub verbose: bool,

    /// Custom log tag
    #[arg(long, global = true)]
    pub log_tag: Option<String>,

    /// Template variable override, format: key=value
    #[arg(long = "var", global = true)]
    pub vars: Vec<String>,

    /// Disable verbose logging
    #[arg(long, global = true, action = ArgAction::SetTrue, conflicts_with = "verbose")]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

impl Cli {
    pub fn verbose_override(&self) -> Option<bool> {
        if self.verbose {
            Some(true)
        } else if self.quiet {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Subcommand)]
pub enum Commands {
    /// Export data
    Export(ExportArgs),
    /// Import data
    Import(ImportArgs),
    /// Generate a config template
    Init(InitArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ExportArgs {
    /// Config file path
    #[arg(short, long, help_heading = "Common")]
    pub config: Option<String>,

    /// Query SQL or SQL file path
    #[arg(long, help_heading = "Common")]
    pub query: Option<String>,

    /// Output file path
    #[arg(short, long, help_heading = "Common")]
    pub output: Option<String>,

    /// Export format: csv/tsv/custom
    #[arg(long, help_heading = "Common")]
    pub format: Option<String>,

    /// Delimiter for custom format
    #[arg(long, help_heading = "Common")]
    pub delimiter: Option<String>,

    /// Include header
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_header", help_heading = "Common")]
    pub header: bool,

    /// Do not include header
    #[arg(long = "no-header", action = ArgAction::SetTrue, conflicts_with = "header", help_heading = "Common")]
    pub no_header: bool,

    /// Database type: oracle/mysql/postgresql
    #[arg(long, help_heading = "Database")]
    pub db_type: Option<String>,

    /// Database connection string
    #[arg(long, help_heading = "Database")]
    pub conn: Option<String>,

    /// Username
    #[arg(long, help_heading = "Database")]
    pub username: Option<String>,

    /// Password
    #[arg(long, help_heading = "Database")]
    pub password: Option<String>,

    /// Fetch size
    #[arg(long, help_heading = "Advanced")]
    pub fetch: Option<usize>,

    /// Buffer size in bytes
    #[arg(long, help_heading = "Advanced")]
    pub buffer_size: Option<usize>,

    /// Compression type: none/gzip
    #[arg(long, help_heading = "Advanced")]
    pub compression: Option<String>,

    /// Log file path
    #[arg(long, help_heading = "Advanced")]
    pub log_file: Option<String>,

    /// Progress output interval in seconds
    #[arg(long, help_heading = "Advanced")]
    pub progress_interval_secs: Option<u64>,

    /// Count rows before export
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_count_rows", help_heading = "Advanced")]
    pub count_rows: bool,

    /// Do not count rows before export
    #[arg(long = "no-count-rows", action = ArgAction::SetTrue, conflicts_with = "count_rows", help_heading = "Advanced")]
    pub no_count_rows: bool,

    /// Print the resolved export execution plan without connecting or writing files
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "print_resolved_config", help_heading = "Advanced")]
    pub dry_run: bool,

    /// Print the resolved export configuration without connecting or writing files
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "dry_run", help_heading = "Advanced")]
    pub print_resolved_config: bool,
}

impl ExportArgs {
    pub fn header_override(&self) -> Option<bool> {
        if self.header {
            Some(true)
        } else if self.no_header {
            Some(false)
        } else {
            None
        }
    }

    pub fn count_rows_override(&self) -> Option<bool> {
        if self.count_rows {
            Some(true)
        } else if self.no_count_rows {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct ImportArgs {
    /// Config file path
    #[arg(short, long, help_heading = "Common")]
    pub config: Option<String>,

    /// Target schema name
    #[arg(long, help_heading = "Common")]
    pub schema: Option<String>,

    /// Target table name
    #[arg(long, help_heading = "Common")]
    pub table: Option<String>,

    /// Input file path, or gpfdist relative path for Greenplum
    #[arg(short, long, help_heading = "Common")]
    pub input: Option<String>,

    /// Import format: csv/tsv/custom
    #[arg(long, help_heading = "Common")]
    pub format: Option<String>,

    /// Delimiter
    #[arg(long, help_heading = "Common")]
    pub delimiter: Option<String>,

    /// Escape character for Greenplum external table format
    #[arg(long, help_heading = "Common")]
    pub escape: Option<String>,

    /// Show progress
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_progress", help_heading = "Common")]
    pub progress: bool,

    /// Do not show progress
    #[arg(long = "no-progress", action = ArgAction::SetTrue, conflicts_with = "progress", help_heading = "Common")]
    pub no_progress: bool,

    /// Input file has header
    #[arg(long, action = ArgAction::SetTrue, conflicts_with = "no_header", help_heading = "Common")]
    pub header: bool,

    /// Input file has no header
    #[arg(long = "no-header", action = ArgAction::SetTrue, conflicts_with = "header", help_heading = "Common")]
    pub no_header: bool,

    /// Database type: oracle/mysql/postgresql/greenplum
    #[arg(long, help_heading = "Database")]
    pub db_type: Option<String>,

    /// Database connection string
    #[arg(long, help_heading = "Database")]
    pub conn: Option<String>,

    /// Username
    #[arg(long, help_heading = "Database")]
    pub username: Option<String>,

    /// Password
    #[arg(long, help_heading = "Database")]
    pub password: Option<String>,

    /// Source column names, comma separated
    #[arg(long, help_heading = "Advanced")]
    pub source_columns: Option<String>,

    /// Target column names, comma separated
    #[arg(long, help_heading = "Advanced")]
    pub target_columns: Option<String>,

    /// Column mapping: source_col:target_col,...
    #[arg(long, help_heading = "Advanced")]
    pub column_mapping: Option<String>,

    /// Skip columns, comma separated
    #[arg(long, help_heading = "Advanced")]
    pub skip_columns: Option<String>,

    /// Column types: col:type,...
    #[arg(long, help_heading = "Advanced")]
    pub column_types: Option<String>,

    /// Batch size
    #[arg(long, help_heading = "Advanced")]
    pub batch_size: Option<usize>,

    /// Null value representation
    #[arg(long, help_heading = "Advanced")]
    pub null_value: Option<String>,

    /// Error strategy: skip/abort
    #[arg(long, help_heading = "Advanced")]
    pub on_error: Option<String>,

    /// Transaction mode: per_batch/all/none
    #[arg(long, help_heading = "Advanced")]
    pub transaction: Option<String>,

    /// Truncate target table before import
    #[arg(long, action = ArgAction::SetTrue, help_heading = "Advanced")]
    pub truncate: bool,

    /// Pre-import SQL
    #[arg(long, help_heading = "Advanced")]
    pub pre_sql: Option<String>,

    /// Post-import SQL
    #[arg(long, help_heading = "Advanced")]
    pub post_sql: Option<String>,

    /// Compression type: none/gzip
    #[arg(long, help_heading = "Advanced")]
    pub compression: Option<String>,

    /// Log file path
    #[arg(long, help_heading = "Advanced")]
    pub log_file: Option<String>,

    /// Progress output interval in seconds
    #[arg(long, help_heading = "Advanced")]
    pub progress_interval_secs: Option<u64>,

    /// Error log table for Greenplum
    #[arg(long, help_heading = "Greenplum")]
    pub error_log_table: Option<String>,

    /// Greenplum gpfdist host
    #[arg(long, help_heading = "Greenplum")]
    pub gpfdist_host: Option<String>,

    /// Greenplum gpfdist port
    #[arg(long, help_heading = "Greenplum")]
    pub gpfdist_port: Option<u16>,

    /// Greenplum gpfdist directory, only used by legacy rewrite path
    #[arg(long, help_heading = "Greenplum")]
    pub gpfdist_dir: Option<String>,

    /// Print the resolved import execution plan without connecting or reading files
    #[arg(long, action = ArgAction::SetTrue, help_heading = "Advanced")]
    pub dry_run: bool,

    /// Print the resolved import configuration without connecting or reading files
    #[arg(long, action = ArgAction::SetTrue, help_heading = "Advanced")]
    pub print_resolved_config: bool,
}

impl ImportArgs {
    pub fn progress_override(&self) -> Option<bool> {
        if self.progress {
            Some(true)
        } else if self.no_progress {
            Some(false)
        } else {
            None
        }
    }

    pub fn header_override(&self) -> Option<bool> {
        if self.header {
            Some(true)
        } else if self.no_header {
            Some(false)
        } else {
            None
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct InitArgs {
    /// Template id to use
    #[arg(short, long, help_heading = "Selection")]
    pub template: Option<String>,

    /// Database type shortcut: postgresql/mysql/oracle/greenplum
    #[arg(long = "db-type", help_heading = "Selection")]
    pub db_type: Option<String>,

    /// Template mode shortcut: import/export
    #[arg(long, help_heading = "Selection")]
    pub mode: Option<String>,

    /// List available templates
    #[arg(long, action = ArgAction::SetTrue, help_heading = "Selection")]
    pub list: bool,

    /// Output file path
    #[arg(short, long, help_heading = "Output")]
    pub output: Option<String>,

    /// Overwrite output file if it already exists
    #[arg(long, action = ArgAction::SetTrue, help_heading = "Output")]
    pub force: bool,
}
