# 核心性能矩阵

`crates/mdstream-core/examples/scaling.rs` 与 `crates/mdstream-core/tests/performance.rs` 共用固定工作负载 `crates/mdstream-core/tests/support/performance.rs`。矩阵只测核心解析、索引和有预算消费，不实现终端 UI，不添加运行时依赖。CSV schema 2 替代旧 scaling 示例的输出格式。

仓库现采用 workspace 布局；本文正文使用当前路径，已保存的 `*-environment.txt` 与基线 CSV 是测量时生成的原始快照，内部仍保留当时的 `src/`、`tests/`、`examples/` 路径。

## 复跑

从项目根目录使用项目锁定的 devShell：

```bash
nix develop -c bash scripts/measure-performance.sh /tmp/mdstream-matrix
# 可选参数：输出目录、基准字节数、每场景查询样本数、独立进程重复次数
nix develop -c bash scripts/measure-performance.sh /tmp/mdstream-matrix-large 262144 101 3
# 只输出一轮 CSV；规模为 base、2 × base、4 × base
nix develop -c cargo run -p mdstream-core -r --example scaling -- 65536 31
nix develop -c cargo nextest run --workspace --test performance
```

脚本拒绝已有输出目录；保存 `environment.txt`（UTC 时间、主机、内核、Git HEAD/脏状态、Rust/Cargo/CPU、输入参数及工作负载/flake 哈希）、Cargo metadata 的权威 target 目录，以及每次进程的 CSV 和构建日志。Cargo 构建不在测量区间内。默认共 17 场景 × 3 规模 × 3 次进程 = 153 行，每行预热一次后采样 31 次。工作负载不含随机数，CSV 提供实际源码字节数和 FNV-1a 指纹；`requested_bytes` 是生成参数，并非每种文档的实际长度。每个进程按固定顺序运行，不做 CPU 绑核、缓存清空或频率固定。

## 场景与测量边界

| 场景 | 工作负载与独立验证 |
| --- | --- |
| `one_byte_appends` / `one_byte_reused` / `append_64` / `append_4096` / `append_whole` | 相同 ASCII 长段落按 1/64/4096/整块输入；首项每次生成 Update，其余复用。确认扫描不随历史长度或 append 次数倍增 |
| `sparse_markup` / `dense_markup` | 固定少量加粗与密集强调/删除线/链接/代码对照；字节计数线性，区间树写入受 n log n 界约束 |
| `unicode_1` / `unicode_257` | 中文、emoji、组合音标、ZWJ 和 CRLF；使用 Utf8Document 按原始字节切分，包含 UTF-8 标量内部切点，完整语义快照对照一次性解析 |
| `wide_sparse` | 列数为 size/64，正文行数为 size/8；表头实际各列，正文每行仅一格，验证物理单元格数为 columns + rows、实际 cell Vec 容量线性 |
| `late_emphasis` / `late_code` / `late_math` | 长正文和闭合后缀分别 append，单独记录后缀加 finish 的时间/计数；百万字确认只新增 2 个 inline 字节，区间写入按对数路径计费 |
| `late_references` / `deep_openers` | 大量旧引用晚定义、长 opener 栈；晚定义只规范化一个 3 字节标签，不遍历旧引用；opener 步骤受字节线性界约束 |
| `resize` / `height_corrections` | 宽度 19/41 与估计 7/3 交替 reset；或在八个固定源码锚点写入 0/3/11/1 高度。reset 恰好一个访问且不分配，单点修正沿对数路径更新；65/130 次循环验证稀疏节点复用 |

所有解析场景都经过同一 `Utf8Document` 字节适配器。`parse_ms` / `parse_mib_s` 包括 append、解码、通知生成和 finish；不含输入生成、布局、最终容量读取、结构验收或 CSV 格式化。`closure_us` 及 `closure_*` 是独立后缀与 finish 的子区间；无后缀场景只含 finish。吞吐分母使用实际源码字节，不使用生成参数。

每个查询样本从文档中部附近的八个固定 tile 锚点选择一个位置，依次执行可选 resize/高度修正、4 行视口查询、从前一 tile 恢复的 LayoutPass、测量写回，预算允许时再按同一源码锚点重新查询。显式 reset/修正、位置查询、任务建立、视口查询、恢复、文本/对象消费和测量共用一个 Budget，限制为 queries=256、tiles=16、bytes=2048、objects=256、recovery=256、dependencies=8。重复访问也计入额度。`query_p50_us` / p95 / p99 / max 是整个样本区间的 wall-clock 时间，含该场景的 reset/修正；分位数使用 nearest-rank（排序后第 ceil(p × N / 100) 项）。各样本保留前次高度状态，首个预热样本不计入汇总。

这里的 LayoutSink 按 Unicode 标量计列，恢复时使用合成的 column=0；不代表字形、终端列宽、真实检查点命中或精确绝对行号。CSV 的 `sink_total_exhausted` 明示 LayoutPass 在预算内未完成的样本数。密集场景可能只完成恢复 tile、暂停在目标 tile；时间是**有界消费帧**的分位数，不能解释为完整视口排版时延。样本之间重新创建局部任务以测固定工作量；跨帧任务推进性继续由既有 frame 测试验收。引用依赖检查额度在此驱动中未使用，报告值为 0，不代表测过缓存依赖验证。

## 计数与容量口径

CSV 保存全部 `Document::work()` 计数、append 次数、闭合增量、每轮共享预算六项的总消费/单帧上限、sink 实际字节/标量/对象/测量次数、height/dependency 索引写入访问、reset/修正次数，以及预热后查询分位数。查询时间包含索引查找，但 `tree_visits` 是解析索引的**写入**访问；预算 `queries` 是调用次数，不是索引内部比较次数。复杂度测试不对时间设置阈值。

容量记录 annotation/container/height/dependency 四个公开 arena 的实际 Vec 容量字节、节点数和分配/扩容计数，以及表格 columns/cells 的实际 Vec 容量和复用视口 scratch 容量。这些字段在本驱动的只增长路径上是最终保留容量；reset 验证不改变高度 arena。它们**不是总堆内存、RSS 或进程峰值**，不包含源码 String、Utf8Document 解码缓冲、Update、其它符号/对象索引、表格 row Vec 本体、测试数据、allocator 元数据或临时峰值；没有通过 unsafe 全局分配器冒充完整统计。

9 项专项测试包含 17 场景的固定语料指纹/重复非时间列、完整语义快照、三规模解析计数、百万字延迟确认、百万规模稀疏表、共享预算整条消费链、反复 resize/修正与离屏正文增长。树高度上界由输入长度或 tile 数计算，不以一次机器耗时回推复杂度。原有独立区间/高度/32 位模型仍保留。

## 2026-10-30 观测记录

[完整三轮 CSV](performance-baseline.csv) 记录 153 行、4,743 个计时样本；三轮所有非时间列逐项相同。环境为 ark、AMD Ryzen 7 255（16 个逻辑 CPU）、Linux 6.18.54、x86_64-unknown-linux-gnu、Rust/Cargo 1.99.1、LLVM 23.1.1；release 默认优化配置。运行开始于 2026-10-30 14:58:01 UTC，基于 `24e37a08be67abc31fca274eaa61eea9cf96c8c8` 加本单元工作树。flake.lock SHA-256 为 `c45839979d7301e8347ffc849047e7d1290bc5e5975bfe7c09635b9761b06593`，工作负载文件 SHA-256 为 `830f2e4d43f741ca8a1edc049eca9d0184ced5408e551bce4bbaa2493a308c85`。

下表选生成参数 262,144 的行；吞吐是三次进程的最小–最大值，查询分位数只列第一轮，避免把不同分布的分位数合并。其它负载、CPU 频率和 allocator 状态会影响时间；不能据此定义性能门槛或宣称不同机器上的速度。

| 场景 | 实际字节 | 三轮解析 MiB/s | 首轮帧 p50 / p95 μs |
| --- | ---: | ---: | ---: |
| one_byte_appends | 262144 | 7.83–8.28 | 1.38 / 2.36 |
| one_byte_reused | 262144 | 10.89–11.58 | 1.40 / 2.27 |
| append_64 | 262144 | 93.07–111.54 | 1.38 / 2.19 |
| append_4096 | 262144 | 114.50–129.60 | 1.38 / 2.19 |
| append_whole | 262144 | 112.37–127.72 | 1.39 / 2.23 |
| sparse_markup | 262151 | 115.76–121.45 | 1.46 / 2.52 |
| dense_markup | 272241 | 5.32–5.89 | 62.61 / 68.55 |
| unicode_1 | 262170 | 18.72–19.22 | 19.36 / 22.73 |
| unicode_257 | 262170 | 35.29–37.23 | 17.79 / 19.24 |
| wide_sparse | 81924 | 8.09–9.15 | 6.86 / 7.89 |
| late_emphasis | 262147 | 122.09–124.08 | 1.40 / 2.31 |
| late_code | 262147 | 147.59–150.40 | 1.37 / 2.23 |
| late_math | 262147 | 158.43–182.21 | 1.36 / 2.17 |
| late_references | 262168 | 6.94–7.47 | 50.11 / 52.61 |
| deep_openers | 262152 | 72.17–91.89 | 1.46 / 2.53 |
| resize | 252801 | 21.88–22.57 | 35.62 / 39.53 |
| height_corrections | 252801 | 20.51–22.16 | 37.62 / 45.51 |

此矩阵不覆盖每个语法特性或其笛卡尔积，不含真实终端绘制、字体/emoji shaping、并发吞吐、总内存峰值或多平台性能结论；这些边界保留后续任务。
