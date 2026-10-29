# mdtui3 核心库

只追加的 Markdown 解析器，以及按视口读取语义、维护排版高度和恢复排版状态的数据结构。零运行时依赖，禁止 unsafe。这里交付的是核心库，不包含终端绘制、HTML 输出、字体塑形或自动换行实现。

目标是：即使输入逐字符到达、一个段落长达数百万字、分隔符很晚才闭合、引用定义出现在文末，也不反复扫描旧文档。支持的方言有明确限制，以换取可证明的复杂度。

## 接入

```rust
use mdtui3::{Document, HeightIndex, Style};

let mut doc = Document::new();
let mut heights = HeightIndex::new(1)?;

for chunk in ["Hello **world** and [guide].\n\n", "[guide]: /help\n"] {
    let update = doc.append(chunk)?;
    heights.apply(&doc, &update)?;
}
let update = doc.finish();
heights.apply(&doc, &update)?;

// 只访问估计行号 100..124，前后各多看 4 行，最多取 32 个片段。
for tile in heights.viewport(100..124, 4, 32) {
    let ticket = heights.ticket(tile.tile).unwrap();
    for run in doc.runs(tile.tile).unwrap() {
        if run.style.contains(Style::HIDDEN) {
            continue;
        }
        let target = run.link.and_then(|id| doc.link_destination(id));
        // 将 run.text / run.style / run.block / target 交给自己的渲染器。
        // run.source 保留原文位置，可用于选择、点击和滚动锚点。
    }
    // 真实排版后：heights.measure(ticket, 实际归属于此片段的行数)?;
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

可执行示例在 `examples/stream.rs`。库 crate 名称为 `mdtui3`。

`append(&str)` 接收有效 UTF-8，可以在任意 Unicode 标量边界分块；传输层的任意字节分块需要先做 UTF-8 解码。`finish()` 表示真正的 EOF、完成尾部解析，重复调用无副作用，此后不能再追加。空追加不改变 revision。

所有 ID 和源码 byte offset 都只在所属文档中有效。已分配的 `BlockId`、`TileId`、`LinkId`、`SymbolId`、`TableId`、`HeadingId` 不会重排；当前块和最后一个片段可以增长。库返回的是借用、区间和增量通知，不复制历史 AST。`Update` 应当逐次处理后释放，不需要保存事件历史。

## 数据结构与复杂度

记 `n` 为累计源码字节数，`a` 为本次实际产生的语义更新数，`T` 为片段数，`V` 为读取的源码字节数，`K` 为视口返回的片段数。以下使用 `log(n + 1)` 的含义；字符串和 Vec 扩容计入摊还成本。

| 操作 | 时间上界 |
| --- | --- |
| 整个输入流的块分类、行内字节扫描、分隔符栈操作 | O(n) |
| 整个输入流的解析，包含区间树和引用符号表 | O(n log n)，摊还 |
| 一次强调/代码配对的区间标记 | O(log n)，不访问区间内部 |
| 查找/登记有长度上限的引用标签 | O(log n)，不使用依赖平均性能的 HashMap |
| 一条新定义生效 | O(log n)，与已有引用使用次数无关 |
| Setext 将长段落改成标题 | O(log n)，块类型本身修改为 O(1) |
| 定位源码所属块、片段 | O(log n) |
| 读取可见片段的语义 runs | O(V log n)，迭代器本身 O(1) 额外空间 |
| 查找下一个未隐藏的位置 | O(log n)，跨越任意长度隐藏区间 |
| 高度修正、区间失效、区间置零、行号定位、前缀高度 | O(log T) |
| 按行号获取一屏及 overscan | O((K + 1) log T)，输出空间 O(K) |
| 宽度改变后重置所有高度估计 | O(1) |
| 检查点写入、区间失效、查找最近有效前驱 | O(log T) |
| 清空所有检查点的有效性 | O(1) |
| 标题索引追加/按 ID 读取 | 摊还 O(1) / O(1)，不复制标题文字 |
| 按级别查找前后标题 | O(log H)，H 为标题数，至多查六个级别 |
| 逐行、翻页、首尾导航 | O(log T)，基于当前高度估计 |
| 标题导航 | O(log H + log n + log T) |
| 表格按行列读取 / 按源码定位行 | O(1) / O(log R)，R 为该表行数 |
| 按视口查询链接/图片对象 | O(log L + 结果数)，L 为链接和图片总数 |

核心文档空间 O(n)。高度树最多 O(T) 节点，但未测量的整片区域只用一个均匀节点。检查点树按需分配：若曾缓存 C 个位置，空间 O(min(T, C log T))，另外保存调用者提供的状态。所有树的递归深度受机器字长约束，不随 Markdown 嵌套深度增长；释放文档不会递归释放一个深层 AST。

单次 `append` **不是最坏情况 O(新增字节数)**：Vec/String 可能扩容；一个新闭合符可能弹出许多旧 opener；换行可能提交之前缓冲的有界行前缀与一个候选表头。保证针对整个流的摊还复杂度，累计不会变成 O(n²)。没有使用局部无界二次算法。调用者主动在每次追加后遍历 `source()` 或整段 `runs_in()`，仍会自己制造二次工作；常规显示应使用有界 `runs(TileId)`。

### 为什么不会重复做旧工作

1. 原文保存在 append-only String 中，所有语义对象引用 byte span。输入游标只向前走，块分类最多查看一次完整短行或长行的有界前缀。
2. 行内解析是状态机。分隔符的同类 opener 索引允许直接定位候选；每个 opener 至多入栈、出栈各一次。失败的候选不会回退重扫。反引号 run 按长度配对，无论 run 多长都只保存区间。
3. 样式和隐藏标记保存在动态二叉区间树。闭合分隔符只向覆盖区间的 O(log n) 个节点写 tag；查询继承这些 tag。源码不需要移动、包裹或重新分词。节点同时聚合最新 revision 和“全部隐藏”信息。
4. 引用保留 `LinkId -> SymbolId -> destination` 间接关系。定义只更新一个符号和一个全局引用 epoch，没有反向引用列表遍历。旧缓存通过 `TileKey` 按需发现变化。
5. 高度树支持 lazy assignment、子树和以及按累计高度选择。零高度区域可整段跳过。重复 resize 重用节点，不累计废弃树。
6. 检查点子树记录有效计数。失效只把覆盖子树的计数置零，查找前驱直接剪枝。失效时不逐个运行调用者状态的析构函数；旧状态在该位置被覆盖或整个索引销毁时释放。

`Document::work()` 暴露输入扫描量、块分类输入量、行内扫描量、分隔符操作数、标签规范化字节数、区间树实际写入访问次数、表格行扫描字节数。分类字节量是分类器接收的字节数，分类器内部对这些字节只做固定次数的线性扫描。它不是 CPU 指令计数。测试使用这些指标约束增长，不依赖机器跑分来“证明”复杂度。

## 视口与排版契约

源码按最多 **256 字节**切成片段，绝不切断 UTF-8 标量。一个片段可跨块边界；一个巨大段落也会被切成很多小片段。`runs()` 给每个 run 附带块 ID，未提交的尾部原文则是 `None`。样式可从任意片段直接查询，不需要从段落开头重放 Markdown 语法。

片段边界**不是强制换行，也不保证是 grapheme、单词或 shaping 边界**。渲染器需要定义行归属约定，并携带 column、词尾、字素尾部等恢复状态。`CheckpointIndex<S>` 的 S 完全由渲染器决定，表示开始某个片段前的状态：

```rust
use mdtui3::{CheckpointIndex, Document, HeightIndex, TileId};

let mut doc = Document::new();
let mut heights = HeightIndex::default();
let mut checkpoints = CheckpointIndex::<usize>::new(); // 示例：usize 表示列号

let update = doc.append("some text\n")?;
heights.apply(&doc, &update)?;
checkpoints.apply(&doc, &update)?;
checkpoints.insert(TileId(0), 0)?;

let target = TileId(0);
if let Some((from, state)) = checkpoints.previous(target) {
    // 只有 from 离 target 足够近时，才在本帧预算内从该状态恢复。
    // 距离过远：从本地 source anchor 以临时行起点开始，或分帧建立状态。
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

每次追加及 EOF 后，立即对高度索引和检查点调用 `apply`。检查点从最早变化的片段起保守失效到末尾，因为换行状态可能向后传播。漏掉 update 会返回 `OutOfOrderUpdate`，可以调用 `rebuild` 恢复估计；不能把一个文档的 ID/update/ticket 用于另一个文档或索引。更换宽度、字体或其它排版参数时，调用 `heights.reset(...)` 和 `checkpoints.clear()`。

开始排版前取得 `Measurement` ticket，完成后通过 `measure` 回填行高。期间如果发生了失效或 resize，旧 ticket 会被拒绝。零行高是合法结果，例如一个只含隐藏语法或不拥有任何完整显示行的片段。`Anchor` 保存片段和片段内行号，`row_of` 可以在高度修正后重新求滚动位置。

**未排版区域只有估计高度。** 在从未访问、也没有邻近检查点的超长软换行段落中，第一次跳转无法同时保证绝对行号精确和只做一屏工作。这不是通过完整预排版掩盖的成本：本库明确选择估计滚动位置及源码锚点。渲染器应限制恢复距离和每帧工作量，必要时采用局部临时起点，之后逐步修正；不要为找到一个遥远的精确检查点回扫整个段落。

大量连续的隐藏源码也无需逐片段访问。以下跳过从一个完全隐藏片段开始的整个隐藏区间：

```rust
# use mdtui3::{Document, HeightIndex, TileId};
# let doc = Document::new();
# let mut heights = HeightIndex::default();
# let tile_id = TileId(0);
if let Some(span) = doc.tile(tile_id) {
    let next = doc.next_visible(span.start);
    if next.is_none_or(|offset| offset >= span.end) {
        let end = next.and_then(|offset| doc.tile_at(offset))
            .map_or(doc.tile_count(), |tile| tile.0);
        heights.suppress(tile_id.0..end)?;
        // 重新查询 viewport，就会跳过这些零高度片段。
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

不完整的行首也在片段中，立即以原文可见。`pending()` 最多 1024 + 3 字节；在换行、EOF 或达到上限时提交块分类。超过上限的行后续直接流式消费，不积累一个待重新解析的巨型尾巴。缓存使用 `tile_key()`，不要只按片段 ID 永久缓存；`Update::dirty` 是源码区间通知，不是展开的片段 ID 列表。

## 表格、任务列表与图片

表格和任务标记的基本写法参考 [GFM 表格](https://github.github.com/gfm/#tables-extension-) 和 [任务列表](https://github.github.com/gfm/#task-list-items-extension-)。这是项目方言的扩展，不声明完整 GFM 合规。

`BlockKind::Table { id }` 指向 `Table`，`Update.new_tables` 给出新创建的 ID 范围：`columns` 保存默认/左/中/右对齐和列宽提示，`rows()[0]` 是表头，`row(i)` / `cell(i, j)` 可直接访问。`row_at(byte_offset)` 二分定位所在行，分隔行没有数据行编号。单元格保存裁剪首尾空白后的原文区间，内部语法用 `runs_in()` 或与可见片段相交后查询。每个单元格使用独立行内状态，强调不会跨越单元格。

```rust
use mdtui3::{Alignment, Document, TableId};
let mut doc = Document::new();
doc.append("| Name | Value |\n| :--- | ---: |\n| CPU | **42** |\n")?;
let table = doc.table(TableId(0)).unwrap();
assert_eq!(table.columns[1].alignment, Alignment::Right);
assert_eq!(doc.text(table.cell(1, 1).unwrap().source), Some("**42**"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

表头/分隔行允许可选首尾竖线，列数必须一致；正文缺列为隐式空单元格，多余列隐藏，不分配 rows × columns 的空矩阵。`\|` 在文本和代码 span 中均表示单元格内的竖线。表格在空行或其它块级结构处结束，可以直接跟在段落后。表头和分隔行各自须在约 1 KiB 分类前缀内完成；正文行和单元格没有此长度限制，会持续流式解析。为了判定表格，解析器额外保留一个有界候选表头，期间以原文预览；`pending()` 仍只报告当前行前缀。判定失败时该候选只解析一次，不回扫历史段落。

`TableRow::complete` 区分完整行与正在增长的行。当前行的最后一个空单元格可能在确认尾部竖线后被移除；先前完整行保持稳定。`source_width_hint` 是已完成单元格的最大原文字节数，单调增长，**不是终端字符宽度**。渲染器需要结合窗口预算、字体/Unicode 宽度、padding 和内容换行决定最终列宽；列宽提示不要求读取旧单元格。提示变大时，`Update.tables_changed` 通知具体表格，同时 `dirty` 覆盖该表范围，旧高度/检查点按现有接口惰性失效。

进入表格视口时先定位行，再读取可见列的单元格；超长单元格仍按片段和检查点恢复。表格一行的显示高度由各单元格排版高度的最大值决定，渲染器应约定显示行归属于哪个源码片段，把总行高回填一次，不能把每列高度相加。标题行、边框、横向滚动、列宽冻结/修正策略属于后续渲染器工作，见 [TODO](TODO.md)。

任务列表通过 `BlockKind::ListItem { task: Some(TaskMarker { checked, source }), .. }` 提供状态；识别列表项开头的 `[ ]`、`[x]`、`[X]`，标记后需要空白或行结束。任务标记不会成为引用链接，不会污染符号表。渲染器据此绘制复选框；当前解析器不修改原文中的勾选状态。

图片复用链接目标解析，但 `link_info(LinkId)` 的 `LinkKind::Image` 与普通链接明确区分，保留 alt 区间和完整语法区间。支持直接目标、引用式图片、延迟定义。可以先把 alt 渲染成链接；空 alt 图片也会保留对象，用 `links_in(visible_span)` 在 O(log L + 结果数) 内查询，避免因没有可见 label run 而遗漏。图片解码和绘制未实现。

## 标题索引与导航

`doc.headings()` 提供文档顺序的 `HeadingIndex`。每个 `Heading` 包含稳定 ID 对应的块、级别、整个标题的源码区间和不含标题标记的内容区间。标题文字不复制；超长 Setext 标题也只保存区间。`at_level(level)` 直接返回该级别的 ID 列表；`next(offset, max_level)` / `previous(...)` 查找严格位于指定源码位置之后/之前、级别不超过上限的标题。`Update.new_headings` 给出新加入的 ID 范围。长 ATX 标题的区间会随追加更新；索引不含当前方言尚未解析的容器内标题。

```rust
use mdtui3::{Document, HeadingId, HeightIndex, NavAction, Navigator};
let mut doc = Document::new();
doc.append("# One\n## Two\n### Three\n")?;
let mut heights = HeightIndex::default();
heights.rebuild(&doc)?;
let mut nav = Navigator::new();
nav.navigate(&doc, &heights, NavAction::Heading(HeadingId(0)), 24);
let target = nav.navigate(&doc, &heights, NavAction::NextHeading { max_level: 2 }, 24).unwrap();
assert_eq!(target.source, Some(6)); // 两个标题即使在同一个片段里也能区分
nav.navigate(&doc, &heights, NavAction::PageDown, 24);
nav.navigate(&doc, &heights, NavAction::End, 24);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Navigator` 支持 `LineUp/LineDown`、`PageUp/PageDown`、`Home/End`、`Heading(id)` 和按级别前后标题跳转。翻页移动一个视口高度，首尾及越界动作自动钳制到有效范围；空文档也可安全使用。高度修正后通过保存的 `Anchor` 恢复位置。

标题动作返回的 `ScrollTarget.source` 是精确目标位置，`top` 是当前高度索引下的估计视口顶行；渲染器需要用局部排版把源码位置映射成实际显示行，再调用 `reconcile`。普通逐行/翻页之后，用 `reconcile_source` 回填实际视口的源码锚点，后续标题查找才会从精确位置出发；未回填时使用片段起点估计。这些接口不通过预排版整篇内容获取精确行号。键盘事件映射和目录选择器列入 [TODO](TODO.md)。

## 明确支持的 Markdown 方言

- 段落与空行；ATX 标题；Setext 标题；分隔线。
- 缩进代码；反引号/波浪线围栏代码，opening fence 长度 3..32；代码 info 保留源码区间。
- 引用行和有序/无序列表项的块信息。容器按物理行展平，提供 quote depth 或列表 indent/number，不构建嵌套列表树，不推断列表续行或 lazy blockquote continuation。
- 管道表格、列对齐、单元格行内语法；任务列表复选框状态。
- `*em*`、`_em_`、`**strong**`、`__strong__`、`~~strike~~`，以及精确长度匹配的反引号代码 span。支持跨同一段落的多行输入。
- ASCII 标点反斜线转义；内联链接 `[text](destination)`；shortcut/full/collapsed 引用链接；单行引用定义 `[label]: destination` 或 `[label]: <destination>`。
- 图片 `![alt](destination)` 及引用式图片，保留独立类型和 alt 信息，可由渲染器降级为链接。
- CRLF 的 CR 隐藏，LF 保留。裸 CR 按原文处理。

取舍：

- 不是完整 CommonMark/GFM 实现。行内 `***` 等超过支持长度的强调 run 按原文处理；flanking 使用 ASCII 空白规则，单词内下划线不匹配。交叉闭合采用最近同类 opener，并丢弃中间未闭合 opener，不做回溯。
- 未闭合的代码 span 在块结束后保留为原文，期间跳过的其它语法不会重新解析。嵌套链接不支持：新 `[` 替换尚未闭合的外层 `[`。
- 已识别的引用立即显示 label，并隐藏引用语法；尚未定义时没有可跳转目标。定义到来后只增加目标。第一个有效定义生效，后续重复定义不覆盖。标签最多 999 字节，折叠 Unicode 空白并转小写，不做完整 Unicode case folding。
- URL 最多 4096 字节，支持平衡括号和转义括号的定界，但返回的 destination 是原文；不提供 title、多行 destination、URL 解码或安全过滤。渲染器自行决定协议策略和转义解码。
- 引用定义、Setext、分隔线、闭合 fence 必须在块分类前缀内完成。超长候选行按普通内容/代码处理；超长 opening fence 的 info 只保留已分类前缀，其余该行隐藏。超长标题末尾的 closing hashes 不再回看识别。
- 不处理 HTML、实体解码、自动链接、脚注、数学公式等额外语义。它们可能包含上述方言仍能识别的普通行内标记。HTML 不在当前需求范围。
- 支持只追加，不支持任意位置编辑、撤回已有源码或移除定义。文档销毁会释放 O(n) 状态，这是一次性清理成本。

## 源码入口

| 文件 | 职责 |
| --- | --- |
| `src/lib.rs` | 流式提交、稳定块/片段、按范围惰性查询、缓存 key |
| `src/block.rs` | 有界块分类 |
| `src/inline.rs` | 单遍分隔符、代码和链接状态机 |
| `src/annotations.rs` | 区间样式、隐藏聚合、最新 revision |
| `src/references.rs` | 有长度上限的标签规范化和 BTreeMap 符号表 |
| `src/layout.rs` | lazy 高度树、viewport、锚点和过期测量拒绝 |
| `src/checkpoints.rs` | 有效检查点的范围失效和前驱查询 |
| `src/table.rs` | 表格行/单元格、对齐、流式单元格解析和列宽提示 |
| `src/headings.rs` | 标题 arena 和六个级别的索引 |
| `src/navigation.rs` | 逐行/翻页/首尾/标题导航及源码目标 |
| `tests/` | 语义样例、随机模型对照、分块一致性、复杂度回归 |

## 验证与开发

Nix flake 锁定工具链和 nextest；构建产物按 workspace 路径隔离。已有 Rust 工具链也可以直接执行对应的 cargo 命令。

```sh
nix develop -c cargo check --all-targets
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo nextest run --release --no-fail-fast
nix develop -c cargo test --doc
nix develop -c cargo fmt --all -- --check
nix develop -c cargo run -r --example stream
nix develop -c cargo run -r --example structure
nix develop -c cargo run -r --example scaling
```

测试包含任意单切点和随机 UTF-8 分块、与朴素数组/有序映射对照的区间/高度/检查点随机测试、深层 opener、百万字 Setext 转换、百万字隐藏区间、十亿片段的稀疏高度索引、逐字符超长段落、延迟定义成本和 resize 节点复用。没有把 wall-clock 阈值当作正确性条件。`scaling` 示例输出时间及操作计数，便于在目标机器上复查。

本 checkout 已安装 Rust skill 提供的本地 pre-commit：运行 `cargo fmt --all` 后只重新 stage 原本暂存的 Rust 文件。格式化可能改变提交中的行号；其它未暂存文件可能被格式化，但不会自动暂存。钩子不随 Git 分发，紧急情况下可用 `git commit --no-verify` 跳过。
