# EL - 数据导出导入工具 (Data Export/Import Tool)

一个高性能的数据库导出导入工具，使用Rust实现，支持Oracle、MySQL、PostgreSQL等数据库。

**AI生成版,测试使用**

**当前版本优先实现Oracle导出功能。**

## 功能特性

- ✅ Oracle数据库导出
- ✅ 支持配置文件和命令行参数两种方式
- ✅ 多种导出格式：CSV、TSV、自定义分隔符
- ✅ 流式处理，优化内存占用
- ✅ 批量获取优化（fetch_size可配置）
- ✅ 进度显示（可选）
- ✅ 导出统计信息（行数、耗时、文件大小、速度）
- ✅ 结构化日志系统（基于tracing）
- ✅ 支持文件日志和控制台输出
- ✅ 环境变量控制日志级别
- 🚧 MySQL导出（待实现）
- 🚧 PostgreSQL导出（待实现）
- 🚧 数据导入功能（待实现）

## 安装

### 前置要求

- Rust 1.70+
- Oracle Instant Client（用于Oracle数据库连接）

### 编译

#### 本地编译

```bash
cargo build --release
```

编译后的可执行文件位于 `target/release/el.exe`（Windows）或 `target/release/el`（Linux/macOS）。

#### 使用Docker编译CentOS 7版本

如果需要编译适用于CentOS 7的二进制程序，可以使用Docker进行交叉编译：

**Linux/macOS:**
```bash
chmod +x build-centos7.sh
./build-centos7.sh
```

**Windows (PowerShell):**
```powershell
.\build-centos7.ps1
```

编译完成后，二进制文件位于 `target/centos7/el`。

**前置要求：**
- 已安装Docker
- Docker服务正在运行

**说明：**
- Docker镜像基于Oracle Linux 7 Slim，包含Rust工具链和Oracle Instant Client 19.23
- 首次构建会下载依赖，需要较长时间（约10-20分钟）
- 后续构建会使用缓存，速度较快
- 编译产物可直接在CentOS 7及更高版本的Linux系统上运行

## 使用方法

### 方式一：使用配置文件

1. 复制示例配置文件：
```bash
cp config.example.toml config.toml
```

2. 编辑 `config.toml`，填入你的数据库连接信息和查询SQL

3. 运行导出命令：
```bash
el export --config config.toml

# 启用详细日志（全局参数-v可以放在子命令前或后）
el -v export --config config.toml
# 或
el export --config config.toml -v
```

### 方式二：使用命令行参数

```bash
el export \
  --conn localhost:1521/ORCL \
  --username your_username \
  --password your_password \
  --query "SELECT * FROM your_table WHERE rownum <= 10000" \
  --output output.csv \
  --format csv \
  --progress \
  --fetch 1000

# 启用详细日志
el -v export \
  --conn localhost:1521/ORCL \
  --username your_username \
  --password your_password \
  --query "SELECT * FROM your_table WHERE rownum <= 10000" \
  --output output.csv
```

## 命令行参数说明

### 全局参数

| 参数 | 说明 | 必需 | 默认值 |
|------|------|------|--------|
| `-v, --verbose` | 详细日志（debug级别） | 否 | false |

### export 子命令参数

| 参数 | 说明 | 必需 | 默认值 |
|------|------|------|--------|
| `--config, -c` | 配置文件路径 | 否 | - |
| `--db-type` | 数据库类型（oracle/mysql/postgresql） | 否 | oracle |
| `--conn` | 数据库连接字符串（格式：host:port/service_name） | 是* | - |
| `--username` | 用户名 | 是* | - |
| `--password` | 密码 | 是* | - |
| `--query` | 查询SQL语句或SQL文件路径 | 是* | - |
| `--output, -o` | 输出文件路径 | 是* | - |
| `--format` | 导出格式（csv/tsv/custom） | 否 | csv |
| `--delimiter` | 自定义分隔符 | 否 | \x03 (ASCII 3) |
| `--progress` | 显示进度 | 否 | false |
| `--progress-interval` | 进度输出间隔（行数） | 否 | 1000000 |
| `--fetch` | 批量获取大小 | 否 | 1000 |
| `--header` | 包含表头 | 否 | false |
| `--buffer-size` | 缓冲区大小（字节） | 否 | 1048576 (1MB) |
| `--compression` | 压缩类型（none/gzip） | 否 | none |
| `--log-file` | 日志文件路径（追加模式） | 否 | - (控制台) |

*注：使用配置文件时，这些参数不是必需的。**命令行参数优先级高于配置文件**。

## 参数优先级

当同时使用配置文件和命令行参数时，**命令行参数的优先级高于配置文件**。这意味着：

1. **所有命令行参数都可以覆盖配置文件中的对应设置**
2. **未在命令行指定的参数将使用配置文件中的值**
3. **如果配置文件和命令行都未指定，则使用默认值**

### 使用示例

假设配置文件 `config.toml` 中设置：
```toml
[database]
fetch_size = 1000

[export]
output_file = "output.csv"
format = "csv"
show_progress = false
```

使用命令行参数覆盖部分配置：
```bash
# 覆盖fetch_size和output_file，其他参数使用配置文件中的值
el export --config config.toml --fetch 5000 --output custom_output.csv

# 覆盖format和progress，其他参数使用配置文件中的值
el export --config config.toml --format tsv --progress

# 覆盖数据库连接信息
el export --config config.toml --conn newhost:1521/NEWDB --username newuser --password newpass

# 覆盖查询SQL
el export --config config.toml --query "SELECT * FROM another_table"
```

### 优先级规则总结

| 优先级 | 来源 | 说明 |
|--------|------|------|
| 最高 | 环境变量（RUST_LOG） | 仅影响日志级别 |
| 高 | 命令行参数 | 覆盖配置文件中的所有对应设置 |
| 低 | 配置文件 | 提供默认配置 |
| 最低 | 程序默认值 | 当配置文件和命令行都未指定时使用 |

## 配置文件示例

```toml
[database]
db_type = "oracle"
connection_string = "localhost:1521/ORCL"
username = "your_username"
password = "your_password"
fetch_size = 1000

[export]
query = "SELECT * FROM your_table WHERE rownum <= 10000"
output_file = "output.csv"
format = "csv"
delimiter = "\x03"
show_progress = true
include_header = false

[logging]
# log_file = "export.log"  # 可选，默认输出到控制台
verbose = false
```

## 日志系统

本工具使用 [tracing](https://github.com/tokio-rs/tracing) 作为日志框架，支持灵活的日志配置。

### 日志级别

支持以下日志级别（从低到高）：
- `trace` - 最详细的跟踪信息
- `debug` - 调试信息
- `info` - 一般信息（默认）
- `warn` - 警告信息
- `error` - 错误信息

### 配置方式

#### 1. 通过配置文件或命令行参数

```bash
# 输出到控制台（默认info级别）
el export --config config.toml

# 输出到文件（追加模式）
el export --config config.toml --log-file export.log

# 启用详细日志（debug级别，使用全局-v参数）
el -v export --config config.toml

# 同时输出到文件和启用详细日志
el -v export --config config.toml --log-file export.log
```

#### 2. 通过环境变量（RUST_LOG）

环境变量优先级最高，可以覆盖配置文件和命令行参数：

```bash
# Linux/macOS
export RUST_LOG=debug
el export --config config.toml

# Windows PowerShell
$env:RUST_LOG="debug"
.\el.exe export --config config.toml

# Windows CMD
set RUST_LOG=debug
el.exe export --config config.toml

# 只显示特定模块的日志
RUST_LOG=el::export=debug el export --config config.toml

# 组合多个模块
RUST_LOG=el::export=debug,el::db=trace el export --config config.toml
```

### 日志输出示例

```
2024-12-27T11:20:30.123456Z  INFO el: Loading configuration from: config.toml
2024-12-27T11:20:30.234567Z  INFO el: Connecting to oracle database...
2024-12-27T11:20:31.345678Z  INFO el: Connected successfully!
2024-12-27T11:20:31.456789Z  INFO el: Starting export...
2024-12-27T11:23:32.567890Z  INFO el: 
=== Export Summary ===
2024-12-27T11:23:32.678901Z  INFO el: Output file: output.csv
2024-12-27T11:23:32.789012Z  INFO el: Rows exported: 6688425
2024-12-27T11:23:32.890123Z  INFO el: Duration: 181.89 seconds
```

## 性能优化

1. **fetch_size**：调整批量获取大小，默认1000。增大此值可以提高大数据量导出的速度，但会占用更多内存。

2. **流式处理**：工具使用流式处理方式，逐行读取和写入数据，避免一次性加载所有数据到内存。

3. **缓冲写入**：使用BufWriter进行缓冲写入，减少磁盘I/O次数。

## 示例输出

```
Loading configuration from: config.toml
Connecting to oracle database...
Connected successfully!
Starting export...
⠁ [00:00:05] Exported 5000 rows
Export completed: 10000 rows

=== Export Summary ===
Rows exported: 10000
Duration: 5.23 seconds
File size: 1048576 bytes (1.00 MB)
Speed: 1912.07 rows/second
Export completed successfully!
```

## 故障排除

### Oracle连接问题

如果遇到Oracle连接错误，请确保：
1. 已安装Oracle Instant Client
2. 设置了正确的环境变量（如LD_LIBRARY_PATH或PATH）
3. 数据库连接信息正确（连接字符串格式：host:port/service_name、用户名、密码）

### 编译问题

如果编译时遇到oracle crate相关错误，请参考：
https://github.com/kubo/rust-oracle#installation

## 许可证

MIT License

## 贡献

欢迎提交Issue和Pull Request！
