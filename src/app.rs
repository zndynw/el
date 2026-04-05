mod init;
mod resolve;
mod run;
mod validate;

use crate::cli::{Cli, Commands};
use anyhow::Result;

pub(crate) use init::run_init;
pub(crate) use resolve::parse_cli_vars;
pub(crate) use run::{run_export, run_import};

#[cfg(test)]
pub(crate) use resolve::{
    ResolvedExportConfig, ResolvedImportConfig, apply_export_templates,
    build_database_config_from_args, build_database_config_from_args_import,
    build_import_config_from_args, merge_database_config, merge_export_config,
    merge_logging_config, render_template, resolve_export_config, resolve_export_query,
    resolve_import_config,
};
#[cfg(test)]
pub(crate) use run::{
    build_export_dry_run_plan, build_export_resolved_config_text, build_import_dry_run_plan,
    build_import_resolved_config_text,
};
#[cfg(test)]
pub(crate) use validate::{validate_resolved_export_config, validate_resolved_import_config};

pub fn run(cli: Cli) -> Result<()> {
    let verbose_override = cli.verbose_override();
    let log_tag_override = cli.log_tag.clone();
    let vars_override = parse_cli_vars(&cli.vars)?;

    match cli.command {
        Commands::Export(args) => {
            run_export(args, verbose_override, log_tag_override, vars_override)
        }
        Commands::Import(args) => {
            run_import(args, verbose_override, log_tag_override, vars_override)
        }
        Commands::Init(args) => run_init(args),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_export_templates, build_database_config_from_args,
        build_database_config_from_args_import, build_import_config_from_args,
        merge_database_config,
        merge_export_config, parse_cli_vars, render_template, resolve_export_config,
        resolve_export_query, resolve_import_config,
    };
    use crate::cli::{Cli, ExportArgs, ImportArgs};
    use crate::config::{
        CompressionType, DatabaseConfig, ExportConfig, ExportFormat, LoggingConfig,
    };
    use std::collections::{HashMap, HashSet};

    fn empty_args() -> ExportArgs {
        ExportArgs {
            config: None,
            db_type: None,
            conn: None,
            username: None,
            password: None,
            query: None,
            output: None,
            format: None,
            delimiter: None,
            fetch: None,
            header: false,
            no_header: false,
            buffer_size: None,
            compression: None,
            log_file: None,
            progress_interval_secs: None,
            count_rows: false,
            no_count_rows: false,
            dry_run: false,
            print_resolved_config: false,
        }
    }

    fn sample_resolved_export_config() -> super::ResolvedExportConfig {
        super::ResolvedExportConfig {
            config_path: Some("export.toml".to_string()),
            database: DatabaseConfig {
                db_type: "postgresql".to_string(),
                connection_string: "localhost:5432/app".to_string(),
                username: "app".to_string(),
                password: "secret".to_string(),
                fetch_size: 2000,
                gpfdist_host: None,
                gpfdist_port: None,
                gpfdist_dir: None,
            },
            export: ExportConfig {
                query: "select * from public.orders where dt = '20260405'".to_string(),
                output_file: "out/orders_20260405.csv.gz".to_string(),
                format: ExportFormat::Csv,
                delimiter: ",".to_string(),
                include_header: true,
                buffer_size: 8192,
                compression: CompressionType::Gzip,
                progress_interval_secs: 30,
                skip_errors: false,
                count_rows: true,
            },
            logging: LoggingConfig::default(),
        }
    }

    fn sample_resolved_export_config_with_logging() -> super::ResolvedExportConfig {
        let mut resolved = sample_resolved_export_config();
        resolved.logging = LoggingConfig {
            log_file: Some("logs/export.log".to_string()),
            tag: Some("nightly".to_string()),
            verbose: true,
        };
        resolved
    }

    fn sample_resolved_import_config() -> super::ResolvedImportConfig {
        super::ResolvedImportConfig {
            config_path: Some("import.toml".to_string()),
            database: DatabaseConfig {
                db_type: "postgresql".to_string(),
                connection_string: "localhost:5432/app".to_string(),
                username: "app".to_string(),
                password: "secret".to_string(),
                fetch_size: 1000,
                gpfdist_host: None,
                gpfdist_port: None,
                gpfdist_dir: None,
            },
            import: crate::config::ImportConfig {
                schema: Some("public".to_string()),
                table: "orders".to_string(),
                input_file: "orders.csv".to_string(),
                source_columns: Some(vec!["id".to_string(), "name".to_string()]),
                target_columns: None,
                column_mapping: None,
                column_expressions: None,
                skip_columns: None,
                column_types: None,
                format: crate::config::ImportFormat::Csv,
                delimiter: ",".to_string(),
                escape: None,
                has_header: true,
                batch_size: 1000,
                null_value: String::new(),
                on_error: crate::config::ErrorStrategy::Skip,
                transaction_mode: crate::config::TransactionMode::PerBatch,
                show_progress: false,
                progress_interval_secs: 30,
                truncate_table: false,
                pre_sql: None,
                post_sql: None,
                error_log_table: None,
                compression: CompressionType::None,
            },
            logging: LoggingConfig::default(),
        }
    }

    fn sample_resolved_import_config_with_greenplum() -> super::ResolvedImportConfig {
        let mut resolved = sample_resolved_import_config();
        resolved.database.db_type = "greenplum".to_string();
        resolved.database.gpfdist_host = Some("etl".to_string());
        resolved.database.gpfdist_port = Some(9000);
        resolved.import.format = crate::config::ImportFormat::Custom;
        resolved.import.delimiter = "|".to_string();
        resolved.import.has_header = false;
        resolved.import.batch_size = 5000;
        resolved.import.show_progress = true;
        resolved.import.progress_interval_secs = 15;
        resolved.import.truncate_table = true;
        resolved.import.pre_sql = Some("delete from public.orders where dt = '20260405'".to_string());
        resolved.import.post_sql = Some("analyze public.orders".to_string());
        resolved.import.error_log_table = Some("etl_errors".to_string());
        resolved
    }

    #[test]
    fn merge_database_config_overrides_only_explicit_cli_values() {
        let base = DatabaseConfig {
            db_type: "oracle".to_string(),
            connection_string: "db:1521/ORCL".to_string(),
            username: "scott".to_string(),
            password: "tiger".to_string(),
            fetch_size: 500,
            gpfdist_host: None,
            gpfdist_port: None,
            gpfdist_dir: None,
        };
        let mut args = empty_args();
        args.fetch = Some(2000);
        args.username = Some("new-user".to_string());

        let merged = merge_database_config(base, &args);

        assert_eq!(merged.db_type, "oracle");
        assert_eq!(merged.connection_string, "db:1521/ORCL");
        assert_eq!(merged.username, "new-user");
        assert_eq!(merged.password, "tiger");
        assert_eq!(merged.fetch_size, 2000);
    }

    #[test]
    fn merge_export_config_preserves_config_values_without_cli_overrides() {
        let base = ExportConfig {
            query: "SELECT 1".to_string(),
            output_file: "output.csv".to_string(),
            format: ExportFormat::Csv,
            delimiter: ",".to_string(),
            include_header: false,
            buffer_size: 1024,
            compression: CompressionType::None,
            progress_interval_secs: 10,
            skip_errors: false,
            count_rows: false,
        };
        let args = empty_args();

        let merged = merge_export_config(base, &args).expect("merge should succeed");

        assert_eq!(merged.query, "SELECT 1");
        assert_eq!(merged.output_file, "output.csv");
        assert_eq!(merged.format, ExportFormat::Csv);
        assert!(!merged.include_header);
        assert_eq!(merged.compression, CompressionType::None);
        assert_eq!(merged.progress_interval_secs, 10);
    }

    #[test]
    fn merge_export_config_allows_overriding_progress_interval_secs_from_cli() {
        let base = ExportConfig {
            query: "SELECT 1".to_string(),
            output_file: "output.csv".to_string(),
            format: ExportFormat::Csv,
            delimiter: ",".to_string(),
            include_header: true,
            buffer_size: 1024,
            compression: CompressionType::None,
            progress_interval_secs: 10,
            skip_errors: false,
            count_rows: false,
        };
        let mut args = empty_args();
        args.progress_interval_secs = Some(45);
        args.no_header = true;

        let merged = merge_export_config(base, &args).expect("merge should succeed");

        assert!(!merged.include_header);
        assert_eq!(merged.progress_interval_secs, 45);
    }

    #[test]
    fn merge_logging_config_allows_disabling_verbose_from_cli() {
        let logging = super::merge_logging_config(
            LoggingConfig {
                log_file: None,
                tag: None,
                verbose: true,
            },
            &empty_args(),
            Some(false),
            None,
        );

        assert!(!logging.verbose);
    }

    #[test]
    fn merge_logging_config_allows_global_tag_override() {
        let logging = super::merge_logging_config(
            LoggingConfig {
                log_file: None,
                tag: Some("config-tag".to_string()),
                verbose: false,
            },
            &empty_args(),
            None,
            Some("cli-tag".to_string()),
        );

        assert_eq!(logging.tag.as_deref(), Some("cli-tag"));
    }

    #[test]
    fn cli_exposes_log_tag_override() {
        let cli =
            <Cli as clap::Parser>::parse_from(["el", "--log-tag", "batch-01", "init", "--list"]);

        assert_eq!(cli.log_tag.as_deref(), Some("batch-01"));
    }

    #[test]
    fn cli_exposes_export_dry_run_flag() {
        let cli = <Cli as clap::Parser>::parse_from([
            "el",
            "export",
            "--db-type",
            "postgresql",
            "--conn",
            "localhost:5432/app",
            "--username",
            "app",
            "--query",
            "select 1",
            "--output",
            "out.csv",
            "--dry-run",
        ]);

        assert!(matches!(
            cli.command,
            crate::cli::Commands::Export(args) if args.dry_run
        ));
    }

    #[test]
    fn parse_cli_vars_accepts_repeated_key_value_pairs() {
        let vars = parse_cli_vars(&["date=20260329".to_string(), "sync_mode=full".to_string()])
            .expect("vars should parse");

        assert_eq!(vars.get("date").map(String::as_str), Some("20260329"));
        assert_eq!(vars.get("sync_mode").map(String::as_str), Some("full"));
    }

    #[test]
    fn export_dry_run_plan_includes_resolved_execution_details() {
        let plan = super::build_export_dry_run_plan(&sample_resolved_export_config());

        assert!(plan.contains("mode: export"));
        assert!(plan.contains("dry_run: true"));
        assert!(plan.contains("config_path: export.toml"));
        assert!(plan.contains("db_type: postgresql"));
        assert!(plan.contains("connection: localhost:5432/app"));
        assert!(plan.contains("query: select * from public.orders where dt = '20260405'"));
        assert!(plan.contains("output_file: out/orders_20260405.csv.gz"));
        assert!(plan.contains("format: csv"));
        assert!(plan.contains("compression: gzip"));
        assert!(plan.contains("count_rows: true"));
    }

    #[test]
    fn import_dry_run_plan_includes_resolved_execution_details() {
        let plan = super::build_import_dry_run_plan(&sample_resolved_import_config_with_greenplum());

        assert!(plan.contains("mode: import"));
        assert!(plan.contains("dry_run: true"));
        assert!(plan.contains("config_path: import.toml"));
        assert!(plan.contains("db_type: greenplum"));
        assert!(plan.contains("connection: localhost:5432/app"));
        assert!(plan.contains("schema: public"));
        assert!(plan.contains("table: orders"));
        assert!(plan.contains("input_file: orders.csv"));
        assert!(plan.contains("format: custom"));
        assert!(plan.contains("delimiter: |"));
        assert!(plan.contains("batch_size: 5000"));
        assert!(plan.contains("truncate_table: true"));
        assert!(plan.contains("show_progress: true"));
        assert!(plan.contains("gpfdist_host: etl"));
        assert!(plan.contains("gpfdist_port: 9000"));
        assert!(plan.contains("pre_sql: delete from public.orders where dt = '20260405'"));
        assert!(plan.contains("post_sql: analyze public.orders"));
        assert!(plan.contains("error_log_table: etl_errors"));
    }

    #[test]
    fn export_dry_run_allows_postgresql_without_password() {
        let mut args = empty_args();
        args.db_type = Some("postgresql".to_string());
        args.conn = Some("localhost:5432/app".to_string());
        args.username = Some("app".to_string());
        args.dry_run = true;

        let config = build_database_config_from_args(&args).expect("dry-run should not require password");

        assert_eq!(config.db_type, "postgresql");
        assert_eq!(config.password, "");
    }

    #[test]
    fn export_print_resolved_config_allows_postgresql_without_password() {
        let mut args = empty_args();
        args.db_type = Some("postgresql".to_string());
        args.conn = Some("localhost:5432/app".to_string());
        args.username = Some("app".to_string());
        args.print_resolved_config = true;

        let config = build_database_config_from_args(&args)
            .expect("print-resolved-config should not require password");

        assert_eq!(config.db_type, "postgresql");
        assert_eq!(config.password, "");
    }

    #[test]
    fn import_dry_run_allows_postgresql_without_password() {
        let args = ImportArgs {
            config: None,
            schema: Some("public".to_string()),
            table: Some("orders".to_string()),
            input: Some("orders.csv".to_string()),
            format: Some("csv".to_string()),
            delimiter: None,
            escape: None,
            progress: false,
            no_progress: false,
            header: true,
            no_header: false,
            db_type: Some("postgresql".to_string()),
            conn: Some("localhost:5432/app".to_string()),
            username: Some("app".to_string()),
            password: None,
            source_columns: None,
            target_columns: None,
            column_mapping: None,
            skip_columns: None,
            column_types: None,
            batch_size: None,
            null_value: None,
            on_error: None,
            transaction: None,
            truncate: false,
            pre_sql: None,
            post_sql: None,
            compression: None,
            log_file: None,
            progress_interval_secs: None,
            error_log_table: None,
            gpfdist_host: None,
            gpfdist_port: None,
            gpfdist_dir: None,
            dry_run: true,
            print_resolved_config: false,
        };

        let config = build_database_config_from_args_import(&args)
            .expect("import dry-run should not require password");

        assert_eq!(config.db_type, "postgresql");
        assert_eq!(config.password, "");
    }

    #[test]
    fn cli_exposes_export_print_resolved_config_flag() {
        let cli = <Cli as clap::Parser>::parse_from([
            "el",
            "export",
            "--db-type",
            "postgresql",
            "--conn",
            "localhost:5432/app",
            "--username",
            "app",
            "--query",
            "select 1",
            "--output",
            "out.csv",
            "--print-resolved-config",
        ]);

        assert!(matches!(
            cli.command,
            crate::cli::Commands::Export(args) if args.print_resolved_config
        ));
    }

    #[test]
    fn cli_exposes_import_dry_run_flag() {
        let cli = <Cli as clap::Parser>::parse_from([
            "el",
            "import",
            "--db-type",
            "postgresql",
            "--conn",
            "localhost:5432/app",
            "--username",
            "app",
            "--table",
            "orders",
            "--input",
            "orders.csv",
            "--dry-run",
        ]);

        assert!(matches!(
            cli.command,
            crate::cli::Commands::Import(args) if args.dry_run
        ));
    }

    #[test]
    fn cli_exposes_import_print_resolved_config_flag() {
        let cli = <Cli as clap::Parser>::parse_from([
            "el",
            "import",
            "--db-type",
            "postgresql",
            "--conn",
            "localhost:5432/app",
            "--username",
            "app",
            "--table",
            "orders",
            "--input",
            "orders.csv",
            "--print-resolved-config",
        ]);

        assert!(matches!(
            cli.command,
            crate::cli::Commands::Import(args) if args.print_resolved_config
        ));
    }

    #[test]
    fn export_resolved_config_includes_sections_and_redacts_password() {
        let output = super::build_export_resolved_config_text(
            &sample_resolved_export_config_with_logging(),
        );

        assert!(output.contains("mode = \"export\""));
        assert!(output.contains("config_path = \"export.toml\""));
        assert!(output.contains("[database]"));
        assert!(output.contains("db_type = \"postgresql\""));
        assert!(output.contains("connection_string = \"localhost:5432/app\""));
        assert!(output.contains("username = \"app\""));
        assert!(output.contains("password = \"***\""));
        assert!(output.contains("[logging]"));
        assert!(output.contains("log_file = \"logs/export.log\""));
        assert!(output.contains("tag = \"nightly\""));
        assert!(output.contains("verbose = true"));
        assert!(output.contains("[export]"));
        assert!(output.contains("query = \"select * from public.orders where dt = '20260405'\""));
        assert!(output.contains("output_file = \"out/orders_20260405.csv.gz\""));
        assert!(output.contains("format = \"csv\""));
        assert!(output.contains("compression = \"gzip\""));
        assert!(!output.contains("password = \"secret\""));
    }

    #[test]
    fn import_resolved_config_includes_sections_and_redacts_password() {
        let output = super::build_import_resolved_config_text(
            &sample_resolved_import_config_with_greenplum(),
        );

        assert!(output.contains("mode = \"import\""));
        assert!(output.contains("config_path = \"import.toml\""));
        assert!(output.contains("[database]"));
        assert!(output.contains("db_type = \"greenplum\""));
        assert!(output.contains("connection_string = \"localhost:5432/app\""));
        assert!(output.contains("username = \"app\""));
        assert!(output.contains("password = \"***\""));
        assert!(output.contains("gpfdist_host = \"etl\""));
        assert!(output.contains("gpfdist_port = 9000"));
        assert!(output.contains("[import]"));
        assert!(output.contains("schema = \"public\""));
        assert!(output.contains("table = \"orders\""));
        assert!(output.contains("input_file = \"orders.csv\""));
        assert!(output.contains("format = \"custom\""));
        assert!(output.contains("delimiter = \"|\""));
        assert!(output.contains("has_header = false"));
        assert!(output.contains("batch_size = 5000"));
        assert!(output.contains("show_progress = true"));
        assert!(output.contains("truncate_table = true"));
        assert!(output.contains("pre_sql = \"delete from public.orders where dt = '20260405'\""));
        assert!(output.contains("post_sql = \"analyze public.orders\""));
        assert!(output.contains("error_log_table = \"etl_errors\""));
        assert!(!output.contains("password = \"secret\""));
    }

    #[test]
    fn import_print_resolved_config_allows_postgresql_without_password() {
        let args = ImportArgs {
            config: None,
            schema: Some("public".to_string()),
            table: Some("orders".to_string()),
            input: Some("orders.csv".to_string()),
            format: Some("csv".to_string()),
            delimiter: None,
            escape: None,
            progress: false,
            no_progress: false,
            header: true,
            no_header: false,
            db_type: Some("postgresql".to_string()),
            conn: Some("localhost:5432/app".to_string()),
            username: Some("app".to_string()),
            password: None,
            source_columns: None,
            target_columns: None,
            column_mapping: None,
            skip_columns: None,
            column_types: None,
            batch_size: None,
            null_value: None,
            on_error: None,
            transaction: None,
            truncate: false,
            pre_sql: None,
            post_sql: None,
            compression: None,
            log_file: None,
            progress_interval_secs: None,
            error_log_table: None,
            gpfdist_host: None,
            gpfdist_port: None,
            gpfdist_dir: None,
            dry_run: false,
            print_resolved_config: true,
        };

        let config = build_database_config_from_args_import(&args)
            .expect("import print-resolved-config should not require password");

        assert_eq!(config.db_type, "postgresql");
        assert_eq!(config.password, "");
    }

    #[test]
    fn validate_resolved_export_config_rejects_unsupported_db_type() {
        let mut resolved = sample_resolved_export_config();
        resolved.database.db_type = "sqlite".to_string();

        let err = super::validate_resolved_export_config(&resolved)
            .expect_err("unsupported export db type should fail");

        assert!(err.to_string().contains("unsupported database type"));
    }

    #[test]
    fn validate_resolved_import_config_requires_source_columns_without_header() {
        let mut resolved = sample_resolved_import_config();
        resolved.import.has_header = false;
        resolved.import.source_columns = None;

        let err = super::validate_resolved_import_config(&resolved)
            .expect_err("source_columns should be required without header");

        assert!(err
            .to_string()
            .contains("source_columns must be specified when has_header is false"));
    }

    #[test]
    fn validate_resolved_import_config_requires_gpfdist_for_greenplum() {
        let mut resolved = sample_resolved_import_config();
        resolved.database.db_type = "greenplum".to_string();
        resolved.database.gpfdist_host = None;
        resolved.database.gpfdist_port = None;

        let err = super::validate_resolved_import_config(&resolved)
            .expect_err("greenplum import should require gpfdist config");

        assert!(err.to_string().contains("gpfdist_host"));
    }

    #[test]
    fn validate_resolved_import_config_rejects_greenplum_header_with_custom_format() {
        let mut resolved = sample_resolved_import_config();
        resolved.database.db_type = "greenplum".to_string();
        resolved.database.gpfdist_host = Some("etl".to_string());
        resolved.database.gpfdist_port = Some(9000);
        resolved.import.format = crate::config::ImportFormat::Custom;
        resolved.import.has_header = true;

        let err = super::validate_resolved_import_config(&resolved)
            .expect_err("greenplum custom import with header should fail");

        assert!(err.to_string().contains("does not support has_header=true"));
    }

    #[test]
    fn validate_resolved_import_config_rejects_empty_custom_delimiter() {
        let mut resolved = sample_resolved_import_config();
        resolved.import.format = crate::config::ImportFormat::Custom;
        resolved.import.delimiter = String::new();

        let err = super::validate_resolved_import_config(&resolved)
            .expect_err("custom import with empty delimiter should fail");

        assert!(err
            .to_string()
            .contains("custom import format requires a delimiter"));
    }

    #[test]
    fn render_template_replaces_known_variables_and_keeps_ext_table() {
        let vars = HashMap::from([
            ("start_date".to_string(), "2026-03-01".to_string()),
            ("datasource".to_string(), "crm".to_string()),
        ]);

        let rendered = render_template(
            "delete from {datasource}.t using {ext_table} where dt >= '{start_date}'",
            &vars,
            &HashSet::from(["ext_table"]),
        )
        .expect("template should render");

        assert_eq!(
            rendered,
            "delete from crm.t using {ext_table} where dt >= '2026-03-01'"
        );
    }

    #[test]
    fn render_template_errors_when_variable_is_missing() {
        let err = render_template(
            "risk/{date}/{datasource}.dat",
            &HashMap::new(),
            &HashSet::new(),
        )
        .expect_err("missing variable should fail");

        assert!(err.to_string().contains("missing template variable: date"));
    }

    #[test]
    fn apply_export_templates_replaces_query_and_output_file_variables() {
        let config = ExportConfig {
            query: "select * from {schema}.{table} where dt = '{batch_date}'".to_string(),
            output_file: "out/{table}_{batch_date}.csv".to_string(),
            format: ExportFormat::Csv,
            delimiter: ",".to_string(),
            include_header: false,
            buffer_size: 1024,
            compression: CompressionType::None,
            progress_interval_secs: 10,
            skip_errors: false,
            count_rows: false,
        };
        let vars = HashMap::from([
            ("schema".to_string(), "public".to_string()),
            ("table".to_string(), "orders".to_string()),
            ("batch_date".to_string(), "20260329".to_string()),
        ]);

        let rendered =
            apply_export_templates(config, &vars).expect("export templates should render");

        assert_eq!(
            rendered.query,
            "select * from public.orders where dt = '20260329'"
        );
        assert_eq!(rendered.output_file, "out/orders_20260329.csv");
    }

    #[test]
    fn resolve_export_query_supports_template_in_file_path_and_file_content() {
        let temp_dir = std::env::temp_dir().join(format!(
            "el_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let query_file = temp_dir.join("orders.sql");
        std::fs::write(
            &query_file,
            "select * from {schema}.orders where dt = '{batch_date}'",
        )
        .expect("query file should be written");

        let vars = HashMap::from([
            ("schema".to_string(), "public".to_string()),
            ("table_name".to_string(), "orders".to_string()),
            ("batch_date".to_string(), "20260329".to_string()),
        ]);

        let rendered =
            resolve_export_query(&temp_dir.join("{table_name}.sql").to_string_lossy(), &vars)
                .expect("query should resolve");

        assert_eq!(
            rendered,
            "select * from public.orders where dt = '20260329'"
        );

        let _ = std::fs::remove_file(query_file);
        let _ = std::fs::remove_dir(temp_dir);
    }

    #[test]
    fn import_config_rejects_schema_qualified_table_name() {
        let args = ImportArgs {
            config: None,
            db_type: Some("greenplum".to_string()),
            conn: Some("localhost:5432/db".to_string()),
            username: Some("gpadmin".to_string()),
            password: None,
            table: Some("htdw_bak.test_d_risk".to_string()),
            schema: None,
            source_columns: Some("c1,c2".to_string()),
            target_columns: Some("id,name".to_string()),
            column_mapping: None,
            skip_columns: None,
            column_types: None,
            input: Some("risk/data.dat".to_string()),
            format: Some("custom".to_string()),
            delimiter: Some("\u{3}".to_string()),
            escape: None,
            progress: false,
            no_progress: false,
            header: false,
            no_header: true,
            batch_size: None,
            null_value: None,
            on_error: None,
            transaction: None,
            truncate: false,
            pre_sql: None,
            post_sql: None,
            error_log_table: None,
            compression: None,
            log_file: None,
            progress_interval_secs: None,
            gpfdist_host: None,
            gpfdist_port: None,
            gpfdist_dir: None,
            dry_run: false,
            print_resolved_config: false,
        };

        let err =
            build_import_config_from_args(&args).expect_err("schema-qualified table should fail");

        assert!(err.to_string().contains("table must not contain schema"));
    }

    #[test]
    fn resolve_export_config_merges_file_cli_and_vars() {
        let temp_dir = std::env::temp_dir().join(format!(
            "el_export_cfg_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let config_path = temp_dir.join("export.toml");
        std::fs::write(
            &config_path,
            r#"[database]
db_type = "postgresql"
connection_string = "config-host:5432/app"
username = "config-user"

[vars]
batch_date = "20260401"
schema = "public"
table_name = "orders"

[logging]
tag = "config-tag"
verbose = true

[export]
query = "select * from {schema}.{table_name} where dt = '{batch_date}'"
output_file = "out/{table_name}_{batch_date}.csv"
format = "csv"
include_header = false
progress_interval_secs = 30
"#,
        )
        .expect("config file should be written");

        let args = ExportArgs {
            config: Some(config_path.to_string_lossy().to_string()),
            query: None,
            output: Some("cli-out/{table_name}_{batch_date}.csv.gz".to_string()),
            format: Some("custom".to_string()),
            delimiter: Some("|".to_string()),
            header: true,
            no_header: false,
            db_type: None,
            conn: Some("cli-host:5432/app".to_string()),
            username: Some("cli-user".to_string()),
            password: None,
            fetch: Some(5000),
            buffer_size: Some(4096),
            compression: Some("gzip".to_string()),
            log_file: Some("logs/cli-export.log".to_string()),
            progress_interval_secs: Some(10),
            count_rows: true,
            no_count_rows: false,
            dry_run: false,
            print_resolved_config: false,
        };

        let resolved = resolve_export_config(
            args,
            Some(false),
            Some("cli-tag".to_string()),
            HashMap::from([("batch_date".to_string(), "20260405".to_string())]),
        )
        .expect("export config should resolve");

        assert_eq!(resolved.config_path.as_deref(), Some(config_path.to_string_lossy().as_ref()));
        assert_eq!(resolved.database.connection_string, "cli-host:5432/app");
        assert_eq!(resolved.database.username, "cli-user");
        assert_eq!(resolved.database.fetch_size, 5000);
        assert_eq!(
            resolved.export.query,
            "select * from public.orders where dt = '20260405'"
        );
        assert_eq!(resolved.export.output_file, "cli-out/orders_20260405.csv.gz");
        assert_eq!(resolved.export.format, ExportFormat::Custom);
        assert_eq!(resolved.export.delimiter, "|");
        assert!(resolved.export.include_header);
        assert_eq!(resolved.export.compression, CompressionType::Gzip);
        assert_eq!(resolved.export.progress_interval_secs, 10);
        assert!(resolved.export.count_rows);
        assert_eq!(resolved.logging.log_file.as_deref(), Some("logs/cli-export.log"));
        assert_eq!(resolved.logging.tag.as_deref(), Some("cli-tag"));
        assert!(!resolved.logging.verbose);

        let _ = std::fs::remove_file(config_path);
        let _ = std::fs::remove_dir(temp_dir);
    }

    #[test]
    fn resolve_import_config_merges_file_cli_and_vars() {
        let temp_dir = std::env::temp_dir().join(format!(
            "el_import_cfg_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let config_path = temp_dir.join("import.toml");
        std::fs::write(
            &config_path,
            r#"[database]
db_type = "greenplum"
connection_string = "config-host:5432/app"
username = "config-user"
gpfdist_host = "etl"
gpfdist_port = 9000

[vars]
batch_date = "20260401"
datasource = "crm"

[logging]
tag = "config-import"
verbose = true

[import]
schema = "public"
table = "orders"
input_file = "in/{datasource}/orders_{batch_date}.dat"
format = "custom"
delimiter = "|"
has_header = false
source_columns = ["c1", "c2"]
target_columns = ["id", "name"]
show_progress = false
progress_interval_secs = 30
"#,
        )
        .expect("config file should be written");

        let args = ImportArgs {
            config: Some(config_path.to_string_lossy().to_string()),
            schema: Some("ods".to_string()),
            table: None,
            input: Some("override/{datasource}/orders_{batch_date}.csv.gz".to_string()),
            format: Some("csv".to_string()),
            delimiter: Some(",".to_string()),
            escape: None,
            progress: true,
            no_progress: false,
            header: true,
            no_header: false,
            db_type: None,
            conn: Some("cli-host:5432/app".to_string()),
            username: Some("cli-user".to_string()),
            password: None,
            source_columns: None,
            target_columns: Some("id,name".to_string()),
            column_mapping: None,
            skip_columns: None,
            column_types: None,
            batch_size: Some(2000),
            null_value: None,
            on_error: Some("abort".to_string()),
            transaction: Some("all".to_string()),
            truncate: true,
            pre_sql: None,
            post_sql: None,
            compression: Some("gzip".to_string()),
            log_file: Some("logs/cli-import.log".to_string()),
            progress_interval_secs: Some(15),
            error_log_table: Some("etl_errors".to_string()),
            gpfdist_host: Some("etl-override".to_string()),
            gpfdist_port: Some(9100),
            gpfdist_dir: None,
            dry_run: false,
            print_resolved_config: false,
        };

        let resolved = resolve_import_config(
            args,
            Some(false),
            Some("cli-import".to_string()),
            HashMap::from([("batch_date".to_string(), "20260405".to_string())]),
        )
        .expect("import config should resolve");

        assert_eq!(resolved.config_path.as_deref(), Some(config_path.to_string_lossy().as_ref()));
        assert_eq!(resolved.database.connection_string, "cli-host:5432/app");
        assert_eq!(resolved.database.username, "cli-user");
        assert_eq!(resolved.database.gpfdist_host.as_deref(), Some("etl-override"));
        assert_eq!(resolved.database.gpfdist_port, Some(9100));
        assert_eq!(resolved.import.schema.as_deref(), Some("ods"));
        assert_eq!(
            resolved.import.input_file,
            "override/crm/orders_20260405.csv.gz"
        );
        assert_eq!(resolved.import.format, crate::config::ImportFormat::Csv);
        assert_eq!(resolved.import.delimiter, ",");
        assert!(resolved.import.has_header);
        assert!(resolved.import.show_progress);
        assert_eq!(resolved.import.progress_interval_secs, 15);
        assert_eq!(resolved.import.batch_size, 2000);
        assert_eq!(resolved.import.on_error, crate::config::ErrorStrategy::Abort);
        assert_eq!(
            resolved.import.transaction_mode,
            crate::config::TransactionMode::All
        );
        assert!(resolved.import.truncate_table);
        assert_eq!(resolved.import.compression, CompressionType::Gzip);
        assert_eq!(resolved.import.error_log_table.as_deref(), Some("etl_errors"));
        assert_eq!(resolved.logging.log_file.as_deref(), Some("logs/cli-import.log"));
        assert_eq!(resolved.logging.tag.as_deref(), Some("cli-import"));
        assert!(!resolved.logging.verbose);

        let _ = std::fs::remove_file(config_path);
        let _ = std::fs::remove_dir(temp_dir);
    }

    #[test]
    fn resolve_export_config_errors_when_config_file_variables_are_missing() {
        let temp_dir = std::env::temp_dir().join(format!(
            "el_export_cfg_missing_var_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should move forward")
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_dir).expect("temp dir should be created");
        let config_path = temp_dir.join("export.toml");
        std::fs::write(
            &config_path,
            r#"[database]
db_type = "postgresql"
connection_string = "localhost:5432/app"
username = "app"

[export]
query = "select * from public.orders where dt = '{batch_date}'"
output_file = "out/orders.csv"
format = "csv"
"#,
        )
        .expect("config file should be written");

        let err = resolve_export_config(
            ExportArgs {
                config: Some(config_path.to_string_lossy().to_string()),
                ..empty_args()
            },
            None,
            None,
            HashMap::new(),
        )
        .err()
        .expect("missing template variable should fail");

        assert!(err.to_string().contains("missing template variable: batch_date"));

        let _ = std::fs::remove_file(config_path);
        let _ = std::fs::remove_dir(temp_dir);
    }
}
