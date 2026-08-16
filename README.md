# EL

`el` 是一个面向 Oracle、MySQL、PostgreSQL、Greenplum 的数据导出/导入工具，重点支持大文件导出、列映射导入，以及 Greenplum 基于 `gpfdist` 的外部表导入。

## 功能概览

### 导出

- 支持 `csv`、`tsv`、`custom` 三种格式
- 支持 `gzip` 压缩输出
- 支持直接写 SQL，也支持从 SQL 文件读取
- 支持进度日志、日志文件、日志标签
- 支持模板变量，用于动态生成 `query` 和 `output_file`

### 导入

- 支持 `csv`、`tsv`、`custom` 三种格式
- 支持普通文件导入和 `gzip` 压缩文件导入
- 支持 `source_columns`、`target_columns`、`column_mapping`、`skip_columns`
- 支持 `column_expressions` 做显式投影表达式
- 支持 `pre_sql`、`post_sql`
- 支持错误策略和事务模式
- Greenplum 支持基于 `gpfdist` 的直接外部表导入

## 安装

```bash
cargo build --release
```

可执行文件位置：

```text
target/release/el
```

## 快速开始

### 生成模板

查看可用模板：

```bash
el init --list
```

生成 Greenplum 导入模板：

```bash
el init --db-type greenplum --mode import --output greenplum-import.toml
```

也可以直接指定模板 ID：

```bash
el init --template greenplum-incremental --output greenplum-incremental.toml
```

### 导出

使用配置文件：

```bash
el export -c export.toml
```

使用命令行：

```bash
el export \
  --db-type postgresql \
  --conn "127.0.0.1:5432/mydb" \
  --username app \
  --query "select * from public.users" \
  --output users.csv \
  --format csv \
  --header \
  --progress
```

### 导入

使用配置文件：

```bash
el import -c import.toml
```

使用命令行：

```bash
el import \
  --db-type postgresql \
  --conn "127.0.0.1:5432/mydb" \
  --username app \
  --table users \
  --input users.csv \
  --format csv \
  --header \
  --progress
```

## 配置文件结构

标准配置文件包含这些段：

```toml
[database]

[vars]

[logging]

[export]
```

或者：

```toml
[database]

[vars]

[logging]

[import]
```

### `database`

通用字段：

```toml
[database]
db_type = "postgresql"
connection_string = "127.0.0.1:5432/mydb"
username = "app"
# password = "secret"
```

说明：

- PostgreSQL/Greenplum 的 `password` 可省略，import 和 export 使用相同的凭据解析规则
- 凭据优先级为：显式 `--password` 或配置文件 `password` > PostgreSQL URL 内嵌密码 > `PGPASSWORD` > password file
- `--dry-run` 和 `--print-resolved-config` 不读取环境密码或 password file
- 密码不会写入日志；resolved config 只显示空值或 `***`

password file 使用 libpq 的 pgpass 格式，默认路径为：

- Linux/macOS：`~/.pgpass`
- Windows：`%APPDATA%\postgresql\pgpass.conf`
- 设置 `PGPASSFILE` 可覆盖默认路径

每条记录包含五个字段：

```text
hostname:port:database:username:password
```

前四个字段可使用 `*` 通配符，冒号和反斜杠分别写成 `\:` 和 `\\`。按文件顺序使用第一条匹配记录。Unix 下文件不能授予 group/other 任何权限，通常应设置为 `0600`；权限过宽时该文件会被忽略。

Greenplum 还需要：

```toml
gpfdist_host = "etl"
gpfdist_port = 9000
```

### `vars`

用于模板变量替换：

```toml
[vars]
batch_date = "20260330"
schema = "public"
table_name = "orders"
sync_mode = "inc"
datasource = "crm"
start_date = "2026-03-01"
```

### `logging`

```toml
[logging]
# log_file = "logs/el.log"
# tag = "nightly-batch"
# verbose = true
```

说明：

- `log_file` 指定日志输出文件
- `tag` 会追加到日志行中
- 未指定 `tag` 时，日志格式保持原样

## 模板变量

当前支持模板变量的字段：

- `import.input_file`
- `import.pre_sql`
- `import.post_sql`
- `export.query`
- `export.output_file`

语法：

```toml
input_file = "{sync_mode}/{datasource}/orders_{batch_date}.txt"
```

命令行可覆盖配置文件中的变量：

```bash
el import -c import.toml --var batch_date=20260330 --var sync_mode=full
```

规则：

- 优先级：命令行 `--var` 高于配置文件 `[vars]`
- 缺失变量会直接报错
- `pre_sql`、`post_sql` 中的 `{ext_table}` 会保留给 Greenplum 外部表导入阶段替换

### `export.query` 的路径模板

`export.query` 支持两层模板：

1. `query` 字段本身是 SQL 文本时，直接替换其中变量
2. `query` 字段是 SQL 文件路径时：
   - 先替换路径里的变量
   - 再读取文件内容
   - 再替换文件内容里的变量

示例：

```toml
[vars]
table_name = "orders"
batch_date = "20260330"
schema = "public"

[export]
query = "sql/{table_name}.sql"
output_file = "out/{table_name}_{batch_date}.csv"
format = "csv"
include_header = true
```

`sql/orders.sql`：

```sql
select *
from {schema}.orders
where dt = '{batch_date}'
```

## 导出配置示例

```toml
[database]
db_type = "postgresql"
connection_string = "127.0.0.1:5432/mydb"
username = "app"
# password = "secret"

[vars]
schema = "public"
table_name = "users"
batch_date = "20260330"

[logging]
tag = "user-export"

[export]
query = "select * from {schema}.{table_name} where dt = '{batch_date}'"
output_file = "out/{table_name}_{batch_date}.csv"
format = "csv"
include_header = true
progress_interval_secs = 30
compression = "none"
```

## 导入配置示例

### PostgreSQL / MySQL / Oracle 通用导入

```toml
[database]
db_type = "postgresql"
connection_string = "127.0.0.1:5432/mydb"
username = "app"
# password = "secret"

[logging]
tag = "user-import"

[import]
schema = "public"
table = "users"
input_file = "users.csv"
format = "csv"
delimiter = ","
has_header = true
target_columns = ["id", "name", "email", "created_at"]
batch_size = 1000
on_error = "skip"
transaction_mode = "per_batch"
progress_interval_secs = 30

[import.column_mapping]
user_id = "id"
created = "created_at"
```

### Greenplum 直接导入

```toml
[database]
db_type = "greenplum"
connection_string = "127.0.0.1:5432/huatai"
username = "htods"
# password = "secret"
gpfdist_host = "etl"
gpfdist_port = 9000

[vars]
sync_mode = "inc"
datasource = "crm"
batch_date = "20260330"
start_date = "2026-03-01"

[logging]
tag = "gp-risk-import"

[import]
schema = "htdw_bak"
table = "d_risk"
input_file = "{sync_mode}/{datasource}/d_risk_{batch_date}.txt"
format = "custom"
delimiter = "\u0003"
# escape = "\\"
has_header = false
source_columns = ["riskcode", "riskname", "classcode", "classname", "datestamp"]
target_columns = ["riskcode", "riskname", "classcode", "classname", "datestamp"]
pre_sql = "delete from htdw_bak.d_risk t using {ext_table} e where t.riskcode = e.riskcode and t.datestamp >= '{start_date}'"
post_sql = "analyze htdw_bak.d_risk"

[import.column_types]
riskcode = "VARCHAR(30)"
riskname = "VARCHAR(200)"
classcode = "VARCHAR(30)"
classname = "VARCHAR(200)"
datestamp = "TIMESTAMP"
```

说明：

- Greenplum 模式下，`input_file` 表示 `gpfdist` 服务根目录下的相对路径
- `schema` 和 `table` 必须分开配置，`table` 不能写成 `schema.table`
- `source_columns` 用于外部表列定义
- `target_columns` 用于目标表插入列顺序
- `column_types` 在 Greenplum 中用于外部表列类型定义，不做自动推断

### Greenplum 列表达式

如果目标表字段需要显式表达式，可以使用 `column_expressions`：

```toml
[import]
target_columns = ["riskcode", "riskname", "datestamp"]

[import.column_expressions]
riskcode = "'12' || riskcode"
riskname = "riskname"
datestamp = "current_date"
```

规则：

- `column_expressions` 的 key 必须是目标列名
- 这是显式 SQL 投影，不会根据类型自动猜测转换规则

## 命令行参数

### 全局参数

```text
-v, --verbose             启用详细日志
--quiet                   关闭详细日志
--log-tag <TAG>           指定日志标签
--var <KEY=VALUE>         指定模板变量，可重复传入
```

### `export`

```text
-c, --config <FILE>           配置文件
--query <SQL|FILE>            SQL 文本或 SQL 文件路径
-o, --output <FILE>           输出文件
--format <csv|tsv|custom>     导出格式
--delimiter <CHAR>            自定义分隔符
--header / --no-header        是否输出表头

--db-type <TYPE>              数据库类型
--conn <STRING>               连接串
--username <USER>             用户名
--password <PASS>             密码

--fetch <N>                   拉取批次大小
--buffer-size <N>             输出缓冲大小
--compression <TYPE>          压缩类型：none/gzip
--progress-interval-secs <N>  进度输出间隔（秒）
--count-rows / --no-count-rows
--log-file <FILE>             日志文件
```

### `import`

```text
-c, --config <FILE>           配置文件
--schema <SCHEMA>             目标 schema
--table <TABLE>               目标表名
-i, --input <FILE>            输入文件；Greenplum 下为 gpfdist 相对路径
--format <csv|tsv|custom>     导入格式
--delimiter <CHAR>            分隔符
--escape <CHAR>               Greenplum 外部表 ESCAPE
--header / --no-header        是否有表头
--progress / --no-progress    是否显示进度

--db-type <TYPE>              数据库类型
--conn <STRING>               连接串
--username <USER>             用户名
--password <PASS>             密码

--source-columns <COLS>       源列，逗号分隔
--target-columns <COLS>       目标列，逗号分隔
--column-mapping <MAP>        列映射，格式 source:target,...
--skip-columns <COLS>         跳过列，逗号分隔
--column-types <TYPES>        列类型，格式 col:type,...
--batch-size <N>              批次大小
--null-value <VALUE>          空值标记
--on-error <skip|abort>       错误策略
--transaction <MODE>          事务模式：per_batch/all/none
--truncate                    导入前清空目标表
--pre-sql <SQL>               导入前执行 SQL
--post-sql <SQL>              导入后执行 SQL
--compression <TYPE>          压缩类型：none/gzip
--progress-interval-secs <N>  进度输出间隔（秒）
--log-file <FILE>             日志文件

--error-log-table <TABLE>     Greenplum 错误日志表
--gpfdist-host <HOST>         Greenplum gpfdist 主机
--gpfdist-port <PORT>         Greenplum gpfdist 端口
--gpfdist-dir <DIR>           旧版重写路径使用的 gpfdist 目录
```

## 连接串格式

- Oracle: `host:port/service` 或 `host/service`
- MySQL: `host:port/database`、`host/database` 或 `mysql://...`
- PostgreSQL: `host:port/database`、`host/database` 或 `postgresql://...`
- Greenplum: 与 PostgreSQL 相同

## 日志

默认输出到标准输出，也可写入文件：

```bash
el import -c import.toml --log-file logs/import.log --log-tag gp-risk -v
```

说明：

- `--log-tag` 会在日志中附加自定义标签
- `RUST_LOG` 仍然可以覆盖日志级别过滤
- 调试日志中的 `log.file` 如果显示编译机路径，这是编译时记录的源码路径，不是运行机实际访问的文件路径

## Greenplum 使用说明

### 1. 启动 gpfdist

```bash
gpfdist -d /data/gpfdist -p 9000
```

### 2. 准备数据文件

如果配置：

```toml
input_file = "inc/crm/d_risk_20260330.txt"
```

那么该文件应位于：

```text
/data/gpfdist/inc/crm/d_risk_20260330.txt
```

### 3. 执行导入

```bash
el import -c greenplum-import.toml --var batch_date=20260330
```

### 4. 关于外部表

- 外部表名会自动生成，默认放在目标 schema 下
- `pre_sql` 和 `post_sql` 中可使用 `{ext_table}`
- `pre_sql`、`post_sql` 执行后会输出影响行数

## 注意事项

- `table` 不能写成 `schema.table`，请使用单独的 `schema` 字段
- `has_header = false` 时，必须提供 `source_columns`
- Greenplum `custom`/`tsv` 直接导入时不支持 `has_header = true`
- Greenplum 的外部表列数应与文件列数一致；真正插入哪些列由 `target_columns` 和投影规则决定
- `column_types` 不做自动类型猜测，用户需要自己保证外部表定义与数据一致

## 开发

运行测试：

```bash
cargo test
```

格式化：

```bash
cargo fmt
```

## 许可证

MIT
