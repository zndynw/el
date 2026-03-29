# export 子命令基于 PostgreSQL COPY 的方案

## 1. 结论

可以做，而且如果目标是“PostgreSQL 导出尽可能快”，`COPY ... TO STDOUT` 比沿用当前 Oracle/MySQL 那套“查询结果逐列解码成 `DbValue`，再统一格式化写文件”的路线更合理。

但这不是“新增一个 PostgreSQL 适配器”那么简单，而是两条导出路线并存：

- 通用路线：面向 Oracle / MySQL / 未来其他库，走 `Database -> DbValue -> Exporter`
- PostgreSQL COPY 路线：直接让 PostgreSQL 产出最终文本流，程序只负责把字节流写到文件

如果接受这点，方案完全可落地。

## 2. 为什么 COPY 更适合 PostgreSQL

当前项目的导出主链路是：

1. 数据库驱动执行查询
2. Rust 逐行读取结果
3. 每列转换成 `DbValue`
4. `ValueFormatter` 转成字符串
5. `Exporter` 再写 CSV/TSV/custom 文件

这条链路的优点是跨库统一，缺点是 PostgreSQL 不能发挥自身导出能力。

`COPY (SELECT ...) TO STDOUT` 的优势：

- 数据库服务端直接序列化结果，CPU 开销更低
- Rust 侧不需要逐列解码/重编码
- 对大结果集通常吞吐更高
- PostgreSQL 原生支持 `CSV`、`TEXT`、`HEADER`、自定义分隔符、NULL 表示

所以如果你的核心目标是 PostgreSQL 导出性能，COPY 路线值得优先考虑。

## 3. 和当前项目的冲突点

COPY 快，但它和当前项目抽象并不完全兼容。

### 3.1 当前 `Exporter` 假设“我拿到的是结构化行列”

当前 `src/export.rs` 的核心前提是：

- 先 `on_columns()`
- 再多次 `on_row(&[DbValue])`

COPY 路线下，Rust 很可能只能拿到一段段已经格式化好的字节流，而不是结构化的列值。

### 3.2 当前 `ValueFormatter` 会失去作用

如果直接用 COPY 输出：

- `bytea` 怎么表现，取决于 PostgreSQL COPY 文本格式
- `timestamp/timestamptz` 的字符串格式，由 PostgreSQL 控制
- `json/jsonb` 直接按 PostgreSQL 输出
- 不再走当前 `Binary -> hex` 这套项目内统一格式化逻辑

这意味着 PostgreSQL 导出结果会更“原生”，但和 Oracle / MySQL 的语义一致性会下降。

### 3.3 现有 `custom` 格式不一定还能保留全部行为

PostgreSQL COPY 原生擅长：

- `FORMAT csv`
- 默认 text/copy 文本格式
- 自定义单字符分隔符

但当前项目的 `custom` 实际上更像“裸拼接分隔符，不做 CSV quoting”，这个行为和 PostgreSQL COPY CSV/TEXT 都不完全一样。

所以必须先定策略：是保性能，还是保现有导出语义完全一致。

## 4. 推荐方案

建议不要把 PostgreSQL COPY 硬塞进现有 `Database` trait，而是给 `export` 子命令新增一条“PostgreSQL 原生 COPY 导出模式”。

推荐设计：

- 默认仍保留通用导出模式
- PostgreSQL 可额外启用 `copy` 模式
- COPY 模式下绕开 `Database` trait 和 `Exporter` 的逐行格式化链路

建议最终形态：

```text
el export --db-type postgresql --pg-export-mode copy ...
```

或者：

```text
el export --db-type postgresql --native-copy ...
```

我更建议 `--pg-export-mode copy`，因为后续还可以扩成：

- `row`
- `copy`

这样不把 PostgreSQL 路线写死。

## 5. 总体设计

## 5.1 配置层

建议在 `DatabaseConfig` 或新建 PostgreSQL 专属配置中加入：

```toml
[database]
db_type = "postgresql"
connection_string = "127.0.0.1:5432/reporting"
username = "postgres"
password = "secret"

[postgresql]
export_mode = "copy"
copy_format = "csv"
copy_null = ""
copy_quote = "\""
copy_escape = "\""
copy_force_quote = false
```

如果不想改 TOML 结构太多，也可以先把这些参数放在 CLI，首版只支持 CLI 覆盖。

## 5.2 CLI 设计

建议新增参数：

- `--pg-export-mode row|copy`
- `--pg-copy-format csv|text`
- `--pg-copy-null <value>`
- `--pg-copy-quote <char>`
- `--pg-copy-escape <char>`
- `--pg-copy-force-quote`

首版也可以收敛，只做：

- `--pg-export-mode`
- `--pg-copy-format`

其余参数先用固定值。

## 5.3 执行分流

当前 `src/app.rs` 的执行路径大致是：

1. 解析配置
2. `build_database()`
3. `Exporter::export(db.as_mut())`

建议改成：

1. 解析配置
2. 判断：
   - 若 `db_type != postgresql`，走现有通用路径
   - 若 `db_type == postgresql && export_mode == row`，走 PostgreSQL 行模式
   - 若 `db_type == postgresql && export_mode == copy`，走 PostgreSQL COPY 路径

也就是在 `src/app.rs` 增加类似：

```rust
fn run_postgresql_copy_export(...) -> Result<ExportStats>
```

不要强行把 COPY 伪装成 `Database` 实现，否则抽象会越来越别扭。

## 6. COPY 路线的实现建议

## 6.1 依赖选择

建议使用同步 `postgres` crate。

原因：

- 项目当前是同步架构
- `COPY OUT` 能通过同步客户端实现
- 接入成本最低

建议依赖：

```toml
postgres = "0.19"
```

## 6.2 核心执行方式

PostgreSQL 侧执行：

```sql
COPY (
  SELECT ...
) TO STDOUT WITH (
  FORMAT CSV,
  HEADER true,
  DELIMITER ',',
  NULL '',
  QUOTE '"',
  ESCAPE '"'
)
```

Rust 侧逻辑：

1. 连接 PostgreSQL
2. 拼装 COPY SQL
3. 调用 copy-out 接口获得 reader/stream
4. 将字节流写入目标文件
5. 如果配置了 gzip，就在文件写入端套 `GzEncoder`
6. 统计输出文件大小、耗时

这条链路非常短：

`PostgreSQL COPY OUT -> BufWriter/Gzip -> file`

## 6.3 SQL 拼装方式

不建议直接把用户传入的 SQL 原样拼在 COPY 命令里且不做边界处理说明。

推荐：

```rust
let copy_sql = format!(
    "COPY ({query}) TO STDOUT WITH (FORMAT CSV, HEADER {header}, DELIMITER {delimiter}, NULL {null})"
);
```

但要明确约束：

- `query` 必须是单条可作为子查询包裹的 SELECT
- 不支持多语句
- 不支持返回多个结果集

这和当前项目约束并不冲突，甚至更清晰。

## 6.4 文件输出

这部分可以最大程度复用现有 `src/export.rs` 的 writer 设计思路，但不复用 `ExportSink`。

建议抽出一个更底层的 writer 工具，例如：

- `OutputWriter::new(export_config)`
- 支持：
  - 普通文件
  - gzip 压缩
  - 缓冲区大小

然后：

- 通用导出模式继续使用 `ExportSink`
- PostgreSQL COPY 模式直接把 copy reader 的字节写入 `OutputWriter`

这样能减少重复代码。

## 7. 格式兼容策略

这里是方案的关键。

## 7.1 建议只支持 COPY 能稳定表达的格式

COPY 模式下建议明确限制：

- 支持 `csv`
- 支持 `tsv`
- 可选支持 `custom`，但要收窄定义

推荐首版规则：

- `csv` -> `COPY ... WITH (FORMAT CSV, DELIMITER ',')`
- `tsv` -> `COPY ... WITH (FORMAT CSV, DELIMITER E'\\t')`
- `custom` -> 仅当分隔符长度为 1 时支持，用 `FORMAT CSV` + 自定义 `DELIMITER`

注意：

- 这里的 `tsv/custom` 仍然是“CSV quoting 语义 + 不同分隔符”
- 不再等价于当前 `write_custom_record()` 那种“纯文本直接拼接”

这点必须在文档里说清楚。

## 7.2 如果必须保留“裸 custom 分隔符输出”

那就不适合走 COPY CSV。

可以考虑 PostgreSQL `COPY ... FORMAT TEXT`，但它的 escaping 规则仍然是 PostgreSQL 自己的，不等价于当前项目的 custom writer。

所以建议结论非常明确：

- 要极致性能，就接受 PostgreSQL 原生输出语义
- 要完全保留项目当前 custom 行为，就只能走 row 模式

不要试图两头都占。

## 8. 统计与进度设计

## 8.1 能保留的统计

COPY 模式下可以保留：

- 总耗时
- 输出文件大小
- 吞吐 MB/s
- 输出文件路径

## 8.2 很难准确保留的统计

当前 `ExportStats` 里有：

- `rows_exported`
- `db_read_time_secs`
- `io_write_time_secs`
- `avg_row_size_bytes`

COPY 模式下问题在于：

- 字节流是连续的，Rust 不再天然知道行数
- 很难准确拆分“DB read time”和“I/O write time”

建议：

- COPY 模式首版允许 `rows_exported = 0` 或 `None`
- 或者新增模式化统计结构

更合理的做法是改造 `ExportStats`：

```rust
pub struct ExportStats {
    pub rows_exported: Option<u64>,
    pub duration_secs: f64,
    pub file_size_bytes: u64,
    pub output_file: String,
    pub mode: ExportMode,
}
```

但这会影响现有代码。

如果想最小改动，建议：

- COPY 模式下通过按 `\n` 计数估算行数
- 若 `include_header = true`，最终减 1

这个近似值通常够用，但要注明：

- 如果字段内包含换行，行数统计会失真

所以我更建议：

- COPY 模式首版不要承诺准确行数统计

## 8.3 进度显示

通用模式现在按“每 N 行”打印进度。

COPY 模式下建议改成按字节进度：

- 每读取/写出 N MB 打一条日志
- 例如：`Progress: 512 MB written`

因为 COPY 流天然更适合做字节级进度，而不是行级进度。

## 9. 兼容性取舍

## 9.1 COPY 模式下建议保留

- `--output`
- `--compression`
- `--buffer-size`
- `--header`
- `--format`
- `--delimiter`
- `--query`
- `--log-file`

## 9.2 COPY 模式下建议弱化或禁用

- `--fetch`
  - COPY 模式没有意义
- `--progress-interval`
  - 当前按行定义，不适合 COPY
- `skip_errors`
  - COPY 过程中出现行级错误时，数据库通常直接失败，不存在通用“跳过坏行”语义

建议做法：

- COPY 模式检测到这些参数时打印 warn
- 或直接返回错误提示“不支持该参数”

## 10. 推荐架构落地方式

建议新增以下模块，而不是硬改现有抽象：

- `src/db/postgresql.rs`
  - 负责 PostgreSQL 连接与基础能力
- `src/export/postgresql_copy.rs`
  - 负责 COPY 导出
- `src/export/output.rs`
  - 负责统一输出 writer

如果暂时不想重构目录，也可以先收敛成：

- `src/db/postgresql.rs`
- `src/export_postgresql_copy.rs`

但从后续维护看，拆到 `export/` 子模块会更清晰。

## 11. 分阶段实施建议

### 第一阶段：可用版

目标：

- `--db-type postgresql --pg-export-mode copy`
- 支持 CSV
- 支持 header
- 支持 gzip
- 支持自定义单字符 delimiter

不做：

- 准确行数统计
- 复杂 COPY 参数
- row/copy 双模式完整打磨

### 第二阶段：增强版

目标：

- 支持 `text`
- 支持更多 COPY 参数
- 字节级进度日志
- 更清晰的统计输出

### 第三阶段：双模式稳定版

目标：

- PostgreSQL 同时支持：
  - `row` 模式：兼容现有导出语义
  - `copy` 模式：优先性能
- README 和配置文档清楚描述两者差异

## 12. 风险

### 风险 1：PostgreSQL 导出结果和其他数据库不再完全一致

这是最大的产品风险。

因为 COPY 输出格式是 PostgreSQL 原生格式，不再受项目 `DbValue` 统一格式化控制。

应对：

- 明确把 COPY 模式定义为“原生高速模式”
- 文档写清楚和通用模式的差异

### 风险 2：`custom` 格式语义变化

如果用户当前依赖的是“ASCII 3 裸拼接，不加 quoting”，COPY 模式不一定能保持一致。

应对：

- COPY 模式下限制 `custom` 的语义
- 需要旧行为时使用 row 模式

### 风险 3：统计信息退化

COPY 模式天然更难拿到准确行数和 DB/IO 拆分耗时。

应对：

- 换成更适合流式 copy 的统计口径

### 风险 4：抽象分裂

项目会从“统一导出框架”变成“通用框架 + PostgreSQL 特化通道”。

应对：

- 在 `app.rs` 做清晰分流
- 不要把 COPY 路线勉强塞进 `Database` trait

## 13. 我对当前项目的建议

如果你的优先级是性能，我建议不要继续沿用我之前那份“PostgreSQL 适配到 `DbValue`”的方案做首版，而是改成：

1. 先实现 PostgreSQL COPY 导出模式
2. 明确它是“原生高速模式”
3. 接受和 Oracle/MySQL 输出语义不完全一致
4. 若后面确实有“跨库导出结果格式尽量统一”的需求，再补 PostgreSQL row 模式

也就是说，PostgreSQL 最合理的路线不是“照抄 Oracle/MySQL”，而是“按 PostgreSQL 的强项单独设计”。

## 14. 最小可交付口径

建议对外描述为：

“`export` 子命令新增 PostgreSQL 原生 COPY 导出模式，优先面向大数据量高速导出场景。该模式复用 PostgreSQL 服务端序列化能力，显著减少客户端逐列解码与重编码开销；同时接受其输出语义更接近 PostgreSQL 原生 COPY，而非项目现有跨库统一格式化行为。”。

## 15. 结论

这个方向可做，而且从性能角度看是更对的方向。

但它本质上不是“给当前 `Database` trait 再接一个 PostgreSQL 实现”，而是：

- 在当前通用导出框架旁边
- 为 PostgreSQL 新增一条原生高速导出通道

如果你接受“性能优先，统一语义次之”的取舍，这条路线比纯 `DbValue` 适配路线更值得做。
