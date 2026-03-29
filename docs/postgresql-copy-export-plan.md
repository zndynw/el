# PostgreSQL COPY 导出方案

## 概述

为 `el` 工具添加 PostgreSQL 支持，使用 `COPY TO STDOUT` 实现高性能数据导出。

## 技术方案

### 1. 依赖库选择

使用 `postgres` crate（tokio-postgres 的同步版本）：

```toml
[dependencies]
postgres = "0.19"
```

### 2. 连接字符串格式

支持两种格式：
- 标准格式：`host:port/database`
- PostgreSQL URL：`postgresql://host:port/database`

示例：
```
localhost:5432/mydb
postgresql://localhost:5432/mydb
```

### 3. 核心实现

#### 3.1 PostgreSQL Database 实现

创建 `src/db/postgresql.rs`，实现 `Database` trait：

```rust
pub struct PostgreSqlDatabase {
    config: DatabaseConfig,
    connection: Option<Client>,
}
```

关键方法：
- `connect()`: 建立连接
- `stream_query()`: 使用 COPY 流式导出

#### 3.2 COPY 导出策略

**方案 A：COPY TO STDOUT（推荐）**

```sql
COPY (SELECT * FROM table) TO STDOUT WITH (FORMAT CSV, DELIMITER E'\x03', NULL '', HEADER false)
```

优势：
- 服务端直接流式输出，性能最优
- 无需临时文件
- 支持任意 SELECT 查询
- 自动处理转义和引用

实现：
```rust
let copy_query = format!(
    "COPY ({}) TO STDOUT WITH (FORMAT CSV, DELIMITER E'\\x{:02x}', NULL '', HEADER {})",
    user_query,
    delimiter_byte,
    if include_header { "true" } else { "false" }
);

let reader = conn.copy_out(&copy_query)?;
// 直接写入输出文件
```

**方案 B：传统 SELECT（备选）**

使用普通 SELECT + 逐行处理，与 MySQL/Oracle 保持一致。

适用场景：
- COPY 权限受限
- 需要自定义格式化逻辑

### 4. 类型映射

PostgreSQL → DbValue：

| PostgreSQL 类型 | DbValue 类型 |
|----------------|-------------|
| TEXT, VARCHAR, CHAR | Text |
| INTEGER, BIGINT, SMALLINT | Integer |
| NUMERIC, DECIMAL | Decimal |
| REAL, DOUBLE PRECISION | Float |
| BOOLEAN | Boolean |
| DATE | Date |
| TIMESTAMP, TIMESTAMPTZ | DateTime |
| TIME, TIMETZ | Time |
| INTERVAL | Interval |
| JSON, JSONB | Json |
| BYTEA | Binary |
| UUID | Text |
| ARRAY | Text (序列化) |

### 5. 配置示例

```toml
[database]
db_type = "postgresql"
connection_string = "localhost:5432/mydb"
username = "postgres"
password = "password"
fetch_size = 1000

[export]
query = "SELECT * FROM users WHERE created_at > '2024-01-01'"
output_file = "users.csv"
format = "csv"
delimiter = "\u0003"
```

### 6. 实现步骤

1. **添加依赖**
   - 在 `Cargo.toml` 添加 `postgres = "0.19"`

2. **创建 PostgreSQL 模块**
   - 创建 `src/db/postgresql.rs`
   - 实现连接字符串解析
   - 实现 `Database` trait
   - 实现 COPY TO STDOUT 逻辑

3. **注册数据库类型**
   - 在 `src/db/mod.rs` 添加 `pub mod postgresql;`
   - 在 `src/app.rs` 添加 PostgreSQL 分支

4. **类型转换**
   - 在 `src/value.rs` 添加 PostgreSQL 类型映射（如需要）

5. **测试**
   - 单元测试：连接字符串解析
   - 集成测试：实际导出验证

### 7. COPY 格式选项

```sql
COPY (...) TO STDOUT WITH (
    FORMAT CSV,              -- 使用 CSV 格式
    DELIMITER E'\x03',       -- 自定义分隔符（ASCII 3）
    NULL '',                 -- NULL 值表示为空字符串
    HEADER false,            -- 不包含表头（由程序控制）
    QUOTE '"',               -- 引用字符
    ESCAPE '"',              -- 转义字符
    ENCODING 'UTF8'          -- 字符编码
)
```

### 8. 性能优化

- **流式处理**：COPY 输出直接写入文件，无内存缓冲
- **批量读取**：使用 `BufReader` 包装 COPY reader
- **压缩支持**：与现有 gzip 压缩逻辑集成
- **进度显示**：统计已读取字节数估算进度

### 9. 错误处理

- 连接失败：返回清晰错误信息
- 查询语法错误：捕获 PostgreSQL 错误码
- 权限不足：提示 COPY 权限要求
- 类型转换失败：记录警告，输出 NULL（可选 skip_errors）

### 10. 兼容性

- PostgreSQL 9.6+
- 支持所有标准 SQL 查询
- 兼容现有 CSV/TSV/Custom 格式配置
- 与 Oracle/MySQL 导出行为一致

## 实现优先级

**P0（核心功能）**
- PostgreSQL 连接
- COPY TO STDOUT 导出
- 基本类型映射
- CSV 格式输出

**P1（完善功能）**
- 自定义分隔符支持
- 压缩支持
- 进度显示
- 错误处理

**P2（增强功能）**
- 复杂类型支持（ARRAY, HSTORE）
- SSL 连接
- 连接池
- 性能统计

## 预期效果

- 导出性能：与 `psql \copy` 相当
- 内存占用：恒定（流式处理）
- 文件格式：与 MySQL/Oracle 导出一致
- 用户体验：配置方式统一
