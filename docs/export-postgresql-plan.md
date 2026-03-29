# export 子命令支持 PostgreSQL 方案

## 1. 背景

当前项目的 `export` 主流程已经具备较好的数据库无关性：

- CLI、配置文件和 README 已声明 `postgresql` 是可选数据库类型。
- `Exporter` 只依赖 `Database` / `QuerySink` 抽象，不感知具体数据库实现。
- `OracleDatabase`、`MySqlDatabase` 已经分别实现了“连接 + 流式读取 + `DbValue` 映射”。

但从代码现状看，PostgreSQL 仍未真正接入：

- `src/app.rs` 的 `build_database()` 只分发到 `oracle` 和 `mysql`。
- `src/db/mod.rs` 只暴露了 `mysql`、`oracle` 模块。
- 缺少 `src/db/postgresql.rs` 实现。
- README 仍标记 PostgreSQL 导出“待实现”。

这意味着项目已经具备接入 PostgreSQL 的骨架，缺的是数据库适配层和少量接线工作。

## 2. 目标

本次方案目标：

- 让 `el export` 支持 `--db-type postgresql`。
- 复用现有 `Exporter`、输出格式、压缩、进度日志、表头能力。
- 保持与 Oracle / MySQL 一致的使用方式。
- 尽量不改动现有导出主链路，控制实现风险。

非目标：

- 不在本阶段实现 `import`。
- 不在本阶段做 PostgreSQL 专用高速 COPY 导出优化。
- 不在本阶段扩展新的导出格式或新的 `DbValue` 类型体系。

## 3. 现状分析

### 3.1 当前调用链

当前导出链路如下：

1. `src/cli.rs`
   解析 `export` 子命令参数，已允许 `oracle/mysql/postgresql`。
2. `src/app.rs`
   合并 CLI 与配置，构造 `DatabaseConfig` / `ExportConfig`。
3. `build_database()`
   根据 `db_type` 创建具体数据库对象。
4. `src/export.rs`
   调用 `db.stream_query()`，逐行写入目标文件。
5. `src/value.rs`
   将统一的 `DbValue` 转成输出字符串。

### 3.2 适配 PostgreSQL 的最佳落点

PostgreSQL 接入点应完全落在数据库适配层：

- 新增 `src/db/postgresql.rs`
- 在 `src/db/mod.rs` 中暴露模块
- 在 `src/app.rs` 中接入 `PostgreSqlDatabase`
- 在 `Cargo.toml` 中增加 PostgreSQL 客户端依赖
- 补充示例配置、README 和测试

也就是说，不需要重写 `Exporter`，只需要让 PostgreSQL 像 Oracle / MySQL 一样输出：

- 列名：`Vec<String>`
- 行值：`Vec<DbValue>`

## 4. 总体方案

## 4.1 方案选型

建议首版采用同步 `postgres` crate，而不是 `tokio-postgres`。

原因：

- 当前项目整体是同步架构，没有 Tokio runtime。
- `Database` trait 是同步接口，使用同步客户端改动最小。
- 首版目标是“尽快补齐支持”，而不是引入异步重构。
- `postgres` crate 可以满足连接、查询、逐行读取、类型提取的核心需求。

建议依赖：

```toml
postgres = { version = "0.19", default-features = false, features = ["with-chrono-0_4"] }
```

如后续需要更丰富类型支持，可再按需增加 feature。

## 4.2 实现结构

新增结构体：

```rust
pub struct PostgreSqlDatabase {
    config: DatabaseConfig,
    client: Option<postgres::Client>,
}
```

实现接口：

- `connect()`
- `stream_query()`

内部职责划分建议：

- `build_config()` 或 `build_connection_string()`
  统一处理 PostgreSQL 连接串格式。
- `stream_query_impl()`
  执行查询、读取列信息、逐行转换为 `DbValue`。
- `row_to_values()` / `read_value()`
  负责 PostgreSQL 类型到 `DbValue` 的映射。

## 5. 连接设计

### 5.1 连接串兼容策略

建议支持两类输入：

1. URL 形式

```text
postgresql://host:5432/dbname
postgres://host:5432/dbname
```

2. 简写形式

```text
host:5432/dbname
host/dbname
```

与 MySQL 一样，最终统一构造成 PostgreSQL 可识别的连接参数。

建议默认规则：

- 默认端口：`5432`
- 必须包含数据库名
- 用户名、密码仍使用现有 `username` / `password` 字段注入

### 5.2 实现建议

优先复用 PostgreSQL 原生 URL：

- 若 `connection_string` 以 `postgres://` 或 `postgresql://` 开头，直接使用。
- 否则解析 `host[:port]/database`，再拼成：

```text
host=127.0.0.1 port=5432 dbname=reporting user=xxx password=yyy
```

这样可以保持与现有 CLI/配置习惯一致，也减少 README 变更成本。

## 6. 查询与流式导出设计

### 6.1 首版实现方式

建议首版使用：

- `Client::prepare(query)` 获取列元数据
- `query_raw()` 或等价可迭代接口逐行拉取
- 对每一行进行 `DbValue` 映射后交给 `sink.on_row()`

目标是逻辑上保持与 Oracle / MySQL 一致：

- 先 `sink.on_columns()`
- 再循环 `sink.on_row()`

### 6.2 关于“流式”定义

这里的“流式”不是数据库服务端 COPY 流，而是：

- 不将全量结果集读入 Rust 内存
- 逐行消费、逐行写文件

只要 PostgreSQL 客户端能以 iterator / row stream 的方式消费结果，就满足当前项目的导出模型。

### 6.3 `fetch_size` 的处理

当前 `DatabaseConfig.fetch_size` 对 Oracle 有直接作用，对 MySQL 作用较弱。

PostgreSQL 首版建议：

- 保留配置字段，不修改公共配置结构。
- 若所选客户端 API 不直接支持 fetch size，则在文档中明确“当前仅保留兼容字段，暂不严格生效”。
- 二期如需要真正确保分批抓取，可引入游标或事务内 server-side cursor。

这比为了“完全消费 `fetch_size`”而大改 `Database` trait 更稳妥。

## 7. PostgreSQL 到 `DbValue` 的类型映射

建议首版遵循“优先保证可导出，其次保证语义精确”的原则。

### 7.1 推荐映射表

| PostgreSQL 类型 | 建议映射到 `DbValue` | 说明 |
|------|------|------|
| `bool` | `Boolean` | 直接映射 |
| `smallint` / `integer` / `bigint` | `Integer` | 统一有符号整数 |
| `oid` / 其他无符号场景 | `UnsignedInteger` 或 `Text` | 首版可先按 `Text` 降级 |
| `real` / `double precision` | `Float` | 直接映射 |
| `numeric` / `decimal` | `Decimal(String)` | 避免精度损失 |
| `char` / `varchar` / `text` / `name` | `Text` | 直接映射 |
| `date` | `Date` | 格式化成字符串 |
| `timestamp` / `timestamptz` | `DateTime` | 统一字符串输出 |
| `time` / `timetz` | `Time` | 统一字符串输出 |
| `interval` | `Interval` | 输出标准字符串 |
| `json` / `jsonb` | `Json` | 输出 JSON 文本 |
| `bytea` | `Binary(Vec<u8>)` | 交给现有 hex formatter |
| `uuid` | `Text` | 字符串即可 |
| `xml` | `Text` | 首版按文本处理 |
| `inet` / `cidr` / `macaddr` | `Text` | 首版按文本处理 |
| `enum` | `Text` | 按显示值导出 |
| `array` | `Text` | 首版用 PostgreSQL 文本表示 |

### 7.2 首版不建议强支持的类型

以下类型建议首版不做专门对象化处理，统一降级为文本，必要时对个别类型返回明确错误：

- `money`
- `point` / `line` / `lseg` / `box` / `path` / `polygon` / `circle`
- `tsvector` / `tsquery`
- `bit` / `varbit`
- 范围类型：`int4range`、`tsrange` 等
- 复合类型
- 域类型

处理原则：

- 能稳定转字符串的，优先转 `Text`
- 无法稳定提取且风险较高的，记录 warn 并返回错误或空值

这样可以先覆盖绝大部分业务表。

## 8. 错误处理策略

沿用现有风格：

- 连接失败：返回 `Failed to connect to PostgreSQL database`
- 查询无结果集：返回明确错误
- 单列转换失败：
  - 若可局部降级，记录 `warn`，该列输出 `Null`
  - 若属于完全不支持的关键类型，可中止当前导出

建议行为与 `OracleDatabase::row_to_values()` 保持一致，即单列失败默认降级为空并打日志，避免因为单个异常值中断整表导出。

## 9. 代码改动清单

### 9.1 必改文件

- `Cargo.toml`
  - 增加 `postgres` 依赖
- `src/db/mod.rs`
  - `pub mod postgresql;`
- `src/app.rs`
  - `use crate::db::postgresql::PostgreSqlDatabase;`
  - 在 `build_database()` 中支持 `"postgresql"` 和建议兼容 `"postgres"`
- `src/db/postgresql.rs`
  - 新增 PostgreSQL 适配实现
- `config.example.toml`
  - 补充 PostgreSQL 连接串说明
- `README.md`
  - 删除“PostgreSQL 导出待实现”标记
  - 增加 PostgreSQL 使用示例

### 9.2 可选优化文件

- `src/config.rs`
  - 仅当需要新增 PostgreSQL 专属配置时再改；首版不建议改
- `src/value.rs`
  - 若后续发现 `uuid`/数组/范围类型需要更细粒度语义，再考虑扩充 `DbValue`

## 10. 分阶段实施建议

### 第一阶段：MVP 可用

目标：

- 可以连接 PostgreSQL
- 可以执行普通 `SELECT`
- 可以导出 CSV/TSV/custom
- 覆盖主流类型：整型、浮点、numeric、文本、时间、json、bytea

验收标准：

- `el export --db-type postgresql ...` 可成功导出
- `--header`、`--compression gzip`、`--progress` 均保持可用
- 对不支持类型有明确日志

### 第二阶段：兼容性增强

目标：

- 增加更多 PostgreSQL 特殊类型兼容
- 对数组、枚举、UUID、网络类型做更稳定映射
- 完善 README 和配置示例

### 第三阶段：性能优化

目标：

- 评估 server-side cursor / 分批抓取
- 在特定条件下评估 `COPY TO STDOUT`

但第三阶段应独立评估，不建议并入首版需求。因为：

- `COPY` 更适合 PostgreSQL 原生 CSV/Text 输出
- 当前项目有统一 `DbValue` 格式化、二进制 hex 编码、custom 分隔符等逻辑
- 直接走 COPY 容易绕开已有行为，导致不同数据库导出语义不一致

## 11. 测试方案

### 11.1 单元测试

建议新增以下测试：

- 连接串解析
  - `host:port/database`
  - `host/database`
  - `postgresql://...`
- 类型映射
  - `bool -> Boolean`
  - `numeric -> Decimal`
  - `bytea -> Binary`
  - `jsonb -> Json`
  - `timestamp/timestamptz -> DateTime`
- 不支持类型的降级或报错行为

### 11.2 集成测试

建议使用 Docker PostgreSQL 做集成测试，覆盖：

- 基础表导出
- 含中文、换行、引号文本
- `bytea`
- `jsonb`
- `numeric(38,10)`
- `timestamp with time zone`
- 空值

### 11.3 回归测试

至少跑：

- 现有 `cargo test`
- Oracle / MySQL 现有单元测试
- PostgreSQL 新增测试

重点确认 PostgreSQL 接入没有破坏已有数据库实现。

## 12. 风险与应对

### 风险 1：PostgreSQL 类型面过宽

问题：

- PostgreSQL 内建类型和扩展类型远多于 MySQL / Oracle 常见场景。

应对：

- 首版只承诺主流类型
- 其他类型优先降级为文本
- 用日志把不兼容点暴露出来

### 风险 2：真实“流式抓取”能力与 `fetch_size` 不完全一致

问题：

- 同步客户端 API 可能不直接暴露 fetch size 语义。

应对：

- 首版先保证功能可用
- 后续再按性能数据决定是否引入 cursor 优化

### 风险 3：时间/时区格式与现有库不一致

问题：

- PostgreSQL `timestamp` / `timestamptz` 的字符串表现可能与 Oracle / MySQL 不完全一致。

应对：

- 统一转换为稳定字符串格式
- 文档明确“导出目标是可消费文本，不承诺跨库完全同构的时间字面量”

### 风险 4：大对象和复杂类型处理成本高

问题：

- PostgreSQL 的复杂对象类型如果在首版强支持，开发和测试成本会显著上升。

应对：

- 首版不纳入范围
- 仅支持 `bytea` 这类常见二进制字段

## 13. 推荐实施顺序

1. 增加 `postgres` 依赖和 `src/db/postgresql.rs`
2. 打通 `src/db/mod.rs` 与 `src/app.rs` 接线
3. 完成连接串解析与基础查询导出
4. 补齐主流类型映射
5. 增加单元测试
6. 更新 `config.example.toml` 与 README
7. 使用真实 PostgreSQL 做联调验证

## 14. 建议的最小交付口径

建议对外宣称：

“`export` 子命令新增 PostgreSQL 支持，首版覆盖主流结构化类型与 `bytea/json/jsonb`，保持与 Oracle/MySQL 一致的导出参数和文件输出行为。复杂 PostgreSQL 专有类型先按文本降级，后续按业务需要继续补强。”。

这个口径与当前项目成熟度匹配，也能避免一次性承诺过多。

## 15. 结论

从当前代码结构看，PostgreSQL 支持属于典型的“新增数据库适配器”工作，而不是导出框架重构。

最合理的做法是：

- 保持 `Database` trait 和 `Exporter` 不变
- 通过新增 `src/db/postgresql.rs` 完成接入
- 首版聚焦主流类型和可用性
- 将 server-side cursor / COPY 优化放到后续迭代

这样能以最小改动完成 `export` 子命令对 PostgreSQL 的支持，并最大程度复用当前项目已经具备的导出能力。
