# EL - 数据导出导入工具 (Data Export/Import Tool)

一个高性能的数据库导出导入工具，使用Rust实现，支持Oracle、MySQL、PostgreSQL等数据库。

**当前版本优先实现Oracle导出功能。**

## 功能特性

- ✅ Oracle数据库导出
- ✅ 支持配置文件和命令行参数两种方式
- ✅ 多种导出格式：CSV、TSV、自定义分隔符
- ✅ 流式处理，优化内存占用
- ✅ 批量获取优化（fetch_size可配置）
- ✅ 进度显示（可选）
- ✅ 导出统计信息（行数、耗时、文件大小、速度）
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
```

## 命令行参数说明

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
| `--fetch` | 批量获取大小 | 否 | 1000 |
| `--header` | 包含表头 | 否 | false |
| `--buffer-size` | 缓冲区大小（字节） | 否 | 1048576 (1MB) |
| `--compression` | 压缩类型（none/gzip） | 否 | none |

*注：使用配置文件时，这些参数不是必需的。

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
