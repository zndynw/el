pub struct TemplateDef {
    pub id: &'static str,
    pub description: &'static str,
    pub content: &'static str,
}

const POSTGRESQL_IMPORT: &str = r#"# PostgreSQL import template

[database]
db_type = "postgresql"
connection_string = "localhost:5432/mydb"
username = "postgres"
# password = "secret"

[logging]
# tag = "gp-risk-import"

[import]
schema = "public"
table = "users"
input_file = "users.csv"
format = "csv"
delimiter = ","
has_header = true
batch_size = 1000
on_error = "skip"
transaction_mode = "per_batch"
show_progress = true
progress_interval_secs = 30
"#;

const POSTGRESQL_EXPORT: &str = r#"# PostgreSQL export template

[database]
db_type = "postgresql"
connection_string = "localhost:5432/mydb"
username = "postgres"
# password = "secret"

[vars]
# batch_date = "20260329"
# schema = "public"
# table_name = "users"

[logging]
# tag = "pg-export"

[export]
query = "select * from {schema}.{table_name} where dt = '{batch_date}'"
output_file = "out/{table_name}_{batch_date}.csv"
format = "csv"
include_header = true
progress_interval_secs = 30
"#;

const MYSQL_IMPORT: &str = r#"# MySQL import template

[database]
db_type = "mysql"
connection_string = "localhost:3306/mydb"
username = "root"
password = "secret"

[logging]
# tag = "mysql-import"

[import]
schema = "mydb"
table = "users"
input_file = "users.csv"
format = "csv"
delimiter = ","
has_header = true
batch_size = 1000
"#;

const MYSQL_EXPORT: &str = r#"# MySQL export template

[database]
db_type = "mysql"
connection_string = "localhost:3306/mydb"
username = "root"
password = "secret"

[vars]
# batch_date = "20260329"
# table_name = "users"

[logging]
# tag = "mysql-export"

[export]
query = "select * from {table_name} where biz_date = '{batch_date}'"
output_file = "out/{table_name}_{batch_date}.csv"
format = "csv"
include_header = true
progress_interval_secs = 30
"#;

const ORACLE_IMPORT: &str = r#"# Oracle import template

[database]
db_type = "oracle"
connection_string = "localhost:1521/ORCL"
username = "scott"
password = "tiger"

[logging]
# tag = "oracle-import"

[import]
schema = "SCOTT"
table = "EMP"
input_file = "emp.csv"
format = "csv"
delimiter = ","
has_header = true
batch_size = 1000
"#;

const ORACLE_EXPORT: &str = r#"# Oracle export template

[database]
db_type = "oracle"
connection_string = "localhost:1521/ORCL"
username = "scott"
password = "tiger"

[vars]
# batch_date = "20260329"
# table_name = "emp"

[logging]
# tag = "oracle-export"

[export]
query = "select * from {table_name} where trunc(crt_date) = to_date('{batch_date}', 'yyyymmdd')"
output_file = "out/{table_name}_{batch_date}.csv"
format = "csv"
include_header = true
progress_interval_secs = 30
"#;

const GREENPLUM_IMPORT: &str = r#"# Greenplum gpfdist direct import template

[database]
db_type = "greenplum"
connection_string = "localhost:5432/gpdb"
username = "gpadmin"
# password = "secret"
gpfdist_host = "etl"
gpfdist_port = 9000

[vars]
# sync_mode = "full"
# datasource = "crm"
# batch_date = "20260329"
# start_date = "2026-03-01"

[logging]
# tag = "gp-direct-import"

[import]
schema = "public"
table = "fact_sales"
input_file = "{sync_mode}/{datasource}/fact_sales_{batch_date}.dat"
format = "custom"
delimiter = "\u0003"
# escape = "\\"
has_header = false
source_columns = ["c1", "c2", "c3", "c4", "c5"]
target_columns = ["sale_id", "month_id", "amount", "sale_time"]
truncate_table = false
# pre_sql = "delete from public.fact_sales t using {ext_table} e where t.sale_id = e.c1 and t.biz_date >= '{start_date}'"
# post_sql = "analyze public.fact_sales"

[import.column_mapping]
c1 = "sale_id"
c4 = "amount"
c5 = "sale_time"

[import.column_expressions]
month_id = "to_char(c2, 'yyyy-mm')"

[import.column_types]
c1 = "bigint"
c2 = "timestamp"
c3 = "text"
c4 = "numeric"
c5 = "timestamp"
"#;

const GREENPLUM_INCREMENTAL: &str = r#"# Greenplum incremental import template

[database]
db_type = "greenplum"
connection_string = "localhost:5432/gpdb"
username = "gpadmin"
# password = "secret"
gpfdist_host = "etl"
gpfdist_port = 9000

[vars]
# sync_mode = "inc"
# datasource = "crm"
# batch_date = "20260329"
# start_date = "2026-03-01"

[logging]
# tag = "gp-incremental"

[import]
schema = "public"
table = "fact_sales"
input_file = "{sync_mode}/{datasource}/fact_sales_{batch_date}.dat"
format = "custom"
delimiter = "\u0003"
has_header = false
source_columns = ["c1", "c2", "c3", "c4", "c5"]
target_columns = ["sale_id", "month_id", "amount", "sale_time"]
pre_sql = "delete from public.fact_sales t using {ext_table} e where t.sale_id = e.c1 and t.biz_date >= '{start_date}'"
post_sql = "analyze public.fact_sales"

[import.column_mapping]
c1 = "sale_id"
c4 = "amount"
c5 = "sale_time"

[import.column_expressions]
month_id = "to_char(c2, 'yyyy-mm')"

[import.column_types]
c1 = "bigint"
c2 = "timestamp"
c3 = "text"
c4 = "numeric"
c5 = "timestamp"
"#;

const ADVANCED_IMPORT: &str = r#"# Advanced import template

[database]
db_type = "postgresql"
connection_string = "localhost:5432/mydb"
username = "postgres"
# password = "secret"

[logging]
# tag = "advanced-import"

[import]
schema = "public"
table = "target_table"
input_file = "source_data.csv"
format = "csv"
delimiter = ","
has_header = true
target_columns = ["id", "name", "age", "email", "created_at"]
skip_columns = ["temp_field"]
truncate_table = true

[import.column_mapping]
user_id = "id"
user_name = "name"
created = "created_at"

[import.column_types]
id = "integer"
age = "integer"
created_at = "timestamp"
"#;

static TEMPLATES: &[TemplateDef] = &[
    TemplateDef {
        id: "postgresql-import",
        description: "PostgreSQL import",
        content: POSTGRESQL_IMPORT,
    },
    TemplateDef {
        id: "postgresql-export",
        description: "PostgreSQL export",
        content: POSTGRESQL_EXPORT,
    },
    TemplateDef {
        id: "mysql-import",
        description: "MySQL import",
        content: MYSQL_IMPORT,
    },
    TemplateDef {
        id: "mysql-export",
        description: "MySQL export",
        content: MYSQL_EXPORT,
    },
    TemplateDef {
        id: "oracle-import",
        description: "Oracle import",
        content: ORACLE_IMPORT,
    },
    TemplateDef {
        id: "oracle-export",
        description: "Oracle export",
        content: ORACLE_EXPORT,
    },
    TemplateDef {
        id: "greenplum-import",
        description: "Greenplum direct gpfdist import",
        content: GREENPLUM_IMPORT,
    },
    TemplateDef {
        id: "greenplum-incremental",
        description: "Greenplum incremental import",
        content: GREENPLUM_INCREMENTAL,
    },
    TemplateDef {
        id: "advanced-import",
        description: "Advanced import with mapping",
        content: ADVANCED_IMPORT,
    },
];

pub fn all() -> &'static [TemplateDef] {
    TEMPLATES
}

pub fn get(id: &str) -> Option<&'static TemplateDef> {
    TEMPLATES.iter().find(|template| template.id == id)
}

pub fn resolve_shortcut(db_type: &str, mode: &str) -> Option<&'static str> {
    match (db_type, mode) {
        ("postgresql", "import") => Some("postgresql-import"),
        ("postgresql", "export") => Some("postgresql-export"),
        ("mysql", "import") => Some("mysql-import"),
        ("mysql", "export") => Some("mysql-export"),
        ("oracle", "import") => Some("oracle-import"),
        ("oracle", "export") => Some("oracle-export"),
        ("greenplum", "import") => Some("greenplum-import"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{get, resolve_shortcut};

    #[test]
    fn resolves_template_shortcuts() {
        assert_eq!(
            resolve_shortcut("greenplum", "import"),
            Some("greenplum-import")
        );
        assert_eq!(
            resolve_shortcut("postgresql", "export"),
            Some("postgresql-export")
        );
    }

    #[test]
    fn template_lookup_finds_known_id() {
        let template = get("advanced-import").expect("template should exist");
        assert!(template.content.contains("[import]"));
    }
}
