# mdtui3 核心库

只追加的 Markdown 解析器，以及按视口读取语义、维护排版高度和恢复排版状态的数据结构。零运行时依赖，禁止 unsafe。这里交付的是核心库，不包含终端绘制、HTML 输出、字体塑形或自动换行实现。

目标是：即使输入逐字符到达、一个段落长达数百万字、分隔符很晚才闭合、引用定义出现在文末，也不反复扫描旧文档。支持的方言有明确限制，以换取可证明的复杂度。

当前开发方向（2026-10-30 确认）：先把地基打牢，在既定需求和复杂度约束内支持尽量丰富、可复用的基础功能，再在其上搭建完整项目。现阶段交付核心库与可运行示例，终端 UI 留到后续阶段；按依赖关系和实现容易程度持续推进 TODO 中的核心能力。具体范围见 [开发阶段与推进原则](TODO.md#开发阶段与推进原则)。

已交付任务及验收记录见 [DONE](DONE.md)。后续优化见 [核心改进顺序与验收条件](TODO.md#核心改进顺序与验收条件)：优先保证整帧实际重排受窗口和恢复预算约束，再改善缓存复用与扩展语义支持。

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

`append(&str)` 接收有效 UTF-8，可以在任意 Unicode 标量边界分块；传输层的任意字节分块可使用 `Utf8Document`：最多暂存 3 字节不完整尾部，非法序列及 EOF 截断采用 U+FFFD 替换；偏移按解码后的文档定义。`finish()` 表示真正的 EOF、完成尾部解析，重复调用无副作用，此后不能再追加。空追加不改变 revision。

所有 ID 和源码 byte offset 都只在所属文档中有效。已分配的 `BlockId`、`TileId`、`LinkId`、`SymbolId`、`TableId`、`HeadingId` 不会重排；当前块和最后一个片段可以增长。库返回的是借用、区间和增量通知，不复制历史 AST。`Update` 可以逐次消费，或使用 `merge` 将连续更新合并后在最终 revision 消费，不需要保存事件历史。

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
| 读取可见片段的语义 runs | O(log n + V)，迭代器 O(log n) 临时空间；实际按区间节点遍历 |
| 读取显示语义片段 | O(log n + V)，普通文本借用原文，实体/代码规范化保留原始区间 |
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

核心文档空间 O(n)。高度树最多 O(T) 节点，但没有遮蔽边界的未测量整片区域只用一个均匀节点。检查点树按需分配：若曾缓存 C 个位置，空间 O(min(T, C log T))，另外保存调用者提供的状态。所有树的递归深度受机器字长约束，不随 Markdown 嵌套深度增长；释放文档不会递归释放一个深层 AST。

单次 `append` **不是最坏情况 O(新增字节数)**：Vec/String 可能扩容；一个新闭合符可能弹出许多旧 opener；换行可能提交之前缓冲的有界行前缀与一个候选表头。保证针对整个流的摊还复杂度，累计不会变成 O(n²)。没有使用局部无界二次算法。调用者主动在每次追加后遍历 `source()` 或整段 `runs_in()`，仍会自己制造二次工作；常规显示应使用有界 `runs(TileId)`。

### 为什么不会重复做旧工作

1. 原文保存在 append-only String 中，所有语义对象引用 byte span。输入游标只向前走，块分类最多查看一次完整短行或长行的有界前缀。
2. 行内解析是状态机。分隔符的同类 opener 索引允许直接定位候选；每个 opener 至多入栈、出栈各一次。失败的候选不会回退重扫。反引号 run 按长度配对，无论 run 多长都只保存区间。
3. 样式和隐藏标记保存在动态二叉区间树。闭合分隔符只向覆盖区间的 O(log n) 个节点写 tag；查询继承这些 tag。源码不需要移动、包裹或重新分词。节点同时聚合最新 revision 和“全部隐藏”信息。
4. 引用保留 `LinkId -> SymbolId -> destination` 间接关系。定义只更新一个符号版本和一个保守全局引用 epoch，没有反向引用列表遍历。`DependencyKey` 只检查自身引用符号；尺寸变化从符号最早使用位置保守失效后续布局状态。
5. 高度树支持 lazy assignment、子树和以及按累计高度选择。零高度区域可整段跳过。重复 resize 重用节点，不累计废弃树。
6. 检查点子树记录有效计数。失效只把覆盖子树的计数置零，查找前驱直接剪枝。失效时不逐个运行调用者状态的析构函数；旧状态在该位置被覆盖或整个索引销毁时释放。

`Document::work()` 暴露输入扫描量、块分类输入量、行内扫描量、分隔符操作数、标签规范化字节数、区间树实际写入访问次数、表格行扫描字节数与自动链接候选检查字节数。分类字节量是分类器接收的字节数，分类器内部对这些字节只做固定次数的线性扫描。它不是 CPU 指令计数。测试使用这些指标约束增长，不依赖机器跑分来“证明”复杂度。

`Runs::annotation_visits()` 单独统计本次查询访问的区间节点，不修改文档统计。查询只在开始时二分定位块，之后顺序跨越块边界并遍历样式区间，不对每个字符从根重查；均匀长区间可以直接跳过。片段划分直接跳到最多 256 字节处，再回退至 UTF-8 边界，无需逐字符解码整段新增输入。

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

每次追加及 EOF 后，逐次调用 `apply`，或合并连续 Update 后在最终 revision 调用一次。`apply_layout_update` / `apply_layout_update_with_policy` 先验证两个索引再修改状态。检查点从最早变化的片段起保守失效到末尾，因为换行状态可能向后传播。漏掉 update 会返回 `OutOfOrderUpdate`，可以调用 `rebuild` 恢复估计；不能把一个文档的 ID/update/ticket 用于另一个文档或索引。更换宽度、字体或其它排版参数时，调用 `heights.reset(...)` 和 `checkpoints.clear()`。

开始排版前取得 `Measurement` ticket，完成后通过 `measure` 回填行高。期间如果发生依赖范围内的失效或 resize，旧 ticket 会被拒绝；不相关的尾部追加保留旧视口 ticket。零行高是合法结果，例如一个只含隐藏语法或不拥有任何完整显示行的片段。`Anchor` 保存片段和片段内行号，`row_of` 可以在高度修正后重新求滚动位置。

**未排版区域只有估计高度。** 在从未访问、也没有邻近检查点的超长软换行段落中，第一次跳转无法同时保证绝对行号精确和只做一屏工作。这不是通过完整预排版掩盖的成本：本库明确选择估计滚动位置及源码锚点。渲染器应限制恢复距离和每帧工作量，必要时采用局部临时起点，之后逐步修正；不要为找到一个遥远的精确检查点回扫整个段落。

大量连续的隐藏源码也无需逐片段访问。以下跳过从一个完全隐藏片段开始的整个隐藏区间：

```rust
# use mdtui3::{Document, HeightIndex, TileId};
# let doc = Document::new();
# let mut heights = HeightIndex::default();
# let tile_id = TileId(0);
if let Some(span) = doc.tile(tile_id) {
    let next = doc.next_drawable(span.start);
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

链接和图片支持裸目标、`<含空格的目标>`，以及双引号、单引号、括号包围的 title；目标和 title 之间需要空格或 tab。`LinkInfo.title` 保留直接链接的 title 区间；`link_title(id)` 同时处理直接链接和延迟定义的引用 title。直接链接和引用定义共用单遍状态机，返回的字符串仍保留反斜线转义，不分配解码副本。规则参考 [CommonMark 链接](https://spec.commonmark.org/0.31.2/#links)，当前仅支持单行目标和 title。

`Style::SOFT_BREAK` / `Style::HARD_BREAK` 标记继续同一行内块的 LF。两个以上行尾空格或一个未转义的反斜线产生硬换行，普通换行为软换行；CRLF 跨 append 也有效。为避免把块末换行误判为行内换行，直到后续同块内容到达才确认，并通过 `dirty` 使旧片段失效；块末和代码内部不加这两种标记。继续行前的尾部空格及硬换行反斜线隐藏，源码仍可寻址。

```rust
use mdtui3::{Document, LinkId, Span, Style};
let mut doc = Document::new();
doc.append("[guide](<docs/my guide.md> 'Guide')  \nnext\n")?;
doc.finish();
assert_eq!(doc.link_title(LinkId(0)), Some("Guide"));
assert!(doc.runs_in(Span::new(0, doc.source().len())).unwrap()
    .any(|run| run.style.contains(Style::HARD_BREAK)));
# Ok::<(), Box<dyn std::error::Error>>(())
```

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

`Navigator` 支持 `LineUp/LineDown`、`PageUp/PageDown`、`Home/End`、`Heading(id)`、`Source(offset)` 和按级别前后标题跳转。源码目标必须位于有效 UTF-8 边界，允许当前 EOF；非法目标返回 `None`。翻页移动一个视口高度，首尾及越界动作自动钳制到有效范围；空文档也可安全使用。高度修正后通过保存的 `Anchor` 恢复位置。

标题动作返回的 `ScrollTarget.source` 是精确目标位置，`top` 是当前高度索引下的估计视口顶行；渲染器需要用局部排版把源码位置映射成实际显示行，再调用 `reconcile`。普通逐行/翻页之后，用 `reconcile_source` 回填实际视口的源码锚点，后续标题查找才会从精确位置出发；未回填时使用片段起点估计。这些接口不通过预排版整篇内容获取精确行号。键盘事件映射和目录选择器列入 [TODO](TODO.md)。

章节/代码块折叠、隐藏目标定位和源码锚点恢复见下方接口。后续的目录、文内 slug、跳转历史及更多折叠对象见 [标题导航与折叠联动规划](TODO.md#标题导航与折叠联动规划)及[折叠支持规划](TODO.md#折叠支持规划)。

## 共享预算与增量缓存

`Budget` 同时限制查询次数、tile 次数、实际交付的源码字节、语义对象、恢复步骤和依赖检查。一次帧内所有工作共用同一个实例；新建 Budget 表示开始新的帧或显式后台任务。原有 `viewport` / `runs` 是底层无整帧约束的接口，接入方不能将它们与预算接口混用后宣称整帧有界。

`viewport_budgeted` / `resume_viewport` 复用输出 Vec，返回 `Covered`、`EndOfDocument` 或 `BudgetExhausted`。继续位置保存稳定 tile 锚点，并验证视图、文档 revision 和失效版本；高度校准后重新计算累计位置。`LayoutPass` 保留半个 tile 的字节/对象位置，下一帧不会重新扫描已完成前缀。`LayoutPass::from_checkpoint` 限制恢复源码距离，每个恢复步骤也计入预算。无附近上下文时，从局部起点排版并标记为 `Local`；未知前缀的绝对行号仍是估计。

一个不可暂停的外部回调不会因为传入 Budget 就自动获得时间上界。`LayoutSink` 每次收到的文本和对象有界；昂贵的图片解码、字体排版或用户析构需另外拆成可暂停任务。`examples/budget.rs` 展示 UTF-8 字节输入、Update 合并以及跨帧推进；其中按 Unicode 标量计列，仅验证消费协议，不是终端宽度或字素排版实现。

`HeightState` 区分 `Measured`、`Local`、`Estimated`、`Expired`。`expire` 用区间操作保留旧的非零高度，并将旧零高度替换为正估计，恢复可检索性；`invalidate` 保留原来统一重置估计的低层语义。默认 `apply` 从最早可能改变尺寸的位置过期后续布局，不扫描屏外内容。渲染器确认引用目标、样式或表格宽度提示不影响尺寸时，可用 `LayoutPolicy` 保留对应测量；更换策略需重置布局。

普通 ticket 保守依赖当前 tile 之前的整个输入布局。`ticket_from` / `measure_from` 将依赖缩小到指定有效检查点，并验证该检查点的身份与版本；替换输入状态后用 `insert_for_layout` 同时过期后续高度和检查点。单次高度回填只更新当前 tile，不把后继 tile 逐个重排。

`DependencyKey` 针对直接语义依赖检查符号版本，检查数量计入 Budget；它不能替代传入排版状态的 ticket。`Document::snapshot` 显式复制受上限和字节预算约束的源码，未复制全 AST。`CheckpointIndex::drain_before` 按对象/查询预算移出旧状态，返回状态所有权；失效本身不批量执行用户析构。

## 更新事件与源码交互

Update 携带文档身份及 `base_revision -> revision`。空追加、重复 EOF 和重复应用均为无副作用操作；漏更新、乱序、错误文档或视图返回明确错误。已消费更新之后的新测量不会被重复通知清掉。`merge` 移入新事件，累计成本随事件数增长；合并后的失效可比逐步处理更保守，但不遗漏失效。`append_into` / `finish_into` 与 `viewport_into` 保留 Vec 容量用于复用。

`changes` 将文本、隐藏性、样式、块结构、引用和表格宽度提示分开。`changed_blocks`、`changed_headings`、`changed_links` 是有界增量范围，允许额外包含最后一个没有变化的旧对象；`rows_changed` 报告表格行范围。`SyntaxState` 区分暂定、增长中、局部封口，标题的状态读取其所属块；封口不代表引用定义不会晚到。Update 对象事件用于对象状态缓存，TileKey 用于显示语义缓存。

`HeadingIndex` 提供 `current`、`parent`、`first_child`、`next_sibling`、`previous_sibling`、`section` / `section_end` 和严格同级前后查询，对应动作已接入 `Navigator`。跳级标题按最近更浅级别确定父节点，兄弟不等于同级。章节末尾可随追加增长，查询不读取标题文本。

`SourceSearch` 是按字节预算推进的固定查询 KMP 任务，支持跨追加和重叠匹配；失败回退也计入预算。更换查询新建任务，历史搜索不会隐式同步完成。`Selection` 绑定文档身份及源码区间，`TaskState` 保存独立的复选框覆盖状态，二者都不修改追加源码。这些接口为后续 UI 提供基础；显示文本搜索见下文，折叠联动另行实现。

## 显示文本与源码映射

`runs` / `runs_in` 保留逐字对应源码的文本；需要实体解码和代码 span 空白规范化时使用 `display_runs` / `display_runs_in`。两种读取都返回隐藏标记，显示或复制可见文本时过滤 `Style::HIDDEN`。普通文字直接借用原文，实体只存至多 8 个 UTF-8 字节的替代值；代码 span 保存端点和流式空白摘要，不复制或在闭合时回扫正文。

```rust
use mdtui3::{Document, Span, Style};
let mut doc = Document::new();
doc.append("中 &amp; ` a\r\nb ` &#x1f642;")?;
doc.finish();
let display: String = doc.display_runs_in(Span::new(0, doc.source().len())).unwrap()
    .filter(|run| !run.style.contains(Style::HIDDEN))
    .map(|run| run.text)
    .collect();
assert_eq!(display, "中 & a b 🙂");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`DisplayRun.source` 是本次读取实际消费的源码区间；`replacement: Some(original)` 指出替代文本的完整原始区间，点击或显示选区应映射到这个整体。没有 replacement 时可按 UTF-8 字节精确映射。`Selection::text` 继续复制原始源码；复制显示文本则串接显示片段。

实体跨查询/tile 边界时只在包含 `&` 的片段输出替代文字，其余片段输出空串并保留完整映射。因此相邻查询拼接不会重复；若查询从实体中间开始，不会补发之前的文字，宿主可通过 replacement 找到实体入口。代码 CRLF 的空格归 LF，CR 片段输出空串，两者映射到同一原始 CRLF；裁掉的首尾空白也以空文本保留映射。实体解码结果不会重新参与 Markdown 解析。

需要共享整帧预算时，创建 `display_cursor(请求区间)`，反复调用 `DisplayCursor::next(&doc, &mut budget)`，处理 `Run`、`BudgetExhausted` 和 `Complete`。游标在耗尽后保存源码位置；文档 revision 改变后返回 `StaleContinuation`，需按源码锚点重建。每步预先消费一次 query、一个 object 和至多 `TILE_BYTES + 2` 源码字节的检查窗口，包含最多两个相邻 CRLF 字节；返回片段较短时仍保守计入整个检查窗口。预算为 1 query、1 object、6 bytes 即可容纳任意一个 UTF-8 标量并保证前进，替代输出最多 8 字节。调用方的排版回调成本仍需另外约束。可运行流程见 `examples/display.rs`；现有 `LayoutSink::text(RenderRun)` 仍接收源码文本。

命名引用支持固定的 2125 个 WHATWG 分号结尾名称（最长 31 字节），表在 `src/entity_names.rs`，附来源和输入 SHA-256；更新时用 `scripts/generate_entities.py` 读取另行下载的 [WHATWG 数据](https://html.spec.whatwg.org/entities.json)，构建和测试不联网。数字引用及代码 span 规范化依据 [CommonMark 0.31.2 字符引用](https://spec.commonmark.org/0.31.2/#entity-and-numeric-character-references)和[代码 span](https://spec.commonmark.org/0.31.2/#code-spans)：十进制 1..7 位、十六进制 1..6 位，必需分号；零值、代理码点及越界值替换为 U+FFFD。代码换行变为空格，首尾均为空格且并非全空格时各去掉一个；其它空白和内部连续空格保留。

实体失败、超长或未完成时保留原文，只按正常语法处理拒绝候选的当前字节；不会回扫候选。反斜线转义、代码块/span、自动链接地址、链接目标/title、引用标签匹配及 fence info 不做实体解码。未闭合代码 span 沿用已有的无回退方言。晚确认通过 Text 变化和区间 revision 使旧显示缓存及布局过期，即使宿主关闭了样式影响尺寸的策略。

## 显示文本搜索

`DisplaySearch::new(&doc, query)` 创建绑定当前文档身份及 revision 的搜索任务。搜索对象是按源码顺序串接的非 `HIDDEN` 显示片段：实体已解码、闭合代码 span 空白已规范化，隐藏标记和链接目标不参与匹配。采用区分大小写的字面匹配，支持重叠命中和跨 run/tile 匹配；空查询返回 `InvalidRange`。不增加块/表格分隔符，不把普通软换行变为空格，不做 Unicode 归一化，也不包含终端绘制的边框、占位或将来的折叠状态。

```rust
use mdtui3::{Budget, Completion, DisplaySearch, Document, Span, WorkLimit};
let mut doc = Document::new();
doc.append("a**b**&amp;c")?;
doc.finish();
let mut search = DisplaySearch::new(&doc, "ab&c")?;
let mut matches = Vec::new();
loop {
    let mut budget = Budget::new(WorkLimit {
        bytes: 16, queries: 2, objects: 2, ..Default::default()
    });
    if search.advance(&doc, &mut budget, &mut matches)? == Completion::EndOfDocument {
        break;
    }
    // UI 中在下一帧继续；也可在显式后台任务中继续推进。
}
assert_eq!(matches[0].display, 0..4);
assert_eq!(matches[0].source, Span::new(0, doc.source().len()));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`DisplayMatch.display` 是该 revision 显示文本流的 UTF-8 字节范围，不是屏幕坐标。`source` 是包含全部贡献字节的最小源码包络：中间可能含隐藏语法，端点若来自实体或 CRLF 替代则包含整个原始替代区间。例如搜索 `&fjlig;` 中的 `f` 和 `j` 会得到不同显示范围、相同源码范围。高亮可使用显示范围精确区分，源码范围用于定位、选区或复制原文；部分实体命中不能仅靠源码范围区分。

每次 `advance` 复用调用方的 Budget 和结果 Vec。读取沿用 `DisplayCursor` 的 query/object/源码检查字节计费；每次匹配比较及 KMP 失败回退再消费一个 byte，输出命中再消费一个 object。耗尽时保存未处理字节，后续不重读已完成前缀；每次提供 6 bytes、1 query、1 object 可持续前进，读取和匹配可能分属不同调用。`source_position()` 表示已读取位置，可能还有一个有界片段未匹配；`display_position()` 表示已消费显示字节数，可停在 UTF-8 标量内部。以 `EndOfDocument` 判断任务完成，重复调用不再输出命中。

源码追加可能改变旧位置的显示语义。任何 revision 变化（包括首次 `finish`）都使任务及已返回的命中失效；`advance` 在消费预算和修改结果前返回 `StaleContinuation`，错误文档返回 `WrongDocument`。调用方须丢弃旧命中并按需新建任务，库不会自动回扫全文；空追加和重复 `finish` 不使任务失效。即使文档尚未结束，也可以完整搜索当前 revision。频繁重建任务的累计重搜成本由调用方调度，不能套用 `SourceSearch` 的跨追加增量保证。

查询预处理时间及任务存储为 O(m)，m 为查询字节数；另有固定大小的片段缓冲，源码映射使用长度为 m 的环形缓冲。单 revision 的 KMP 比较总量 O(d)，d 为显示字节数；每次显示读取另有 O(log(n + 1) + 检查窗口) 的成本，窗口不超过 `TILE_BYTES + 2`，最坏累计搜索 O(n log(n + 1) + m)。输出空间由调用方管理，可逐批消费，不在任务中保存全文或全部命中。示例见 `examples/search.rs`。

## 基础折叠

`FoldView` 为每个视图保存独立的收起意愿、高度和检查点。当前支持顶层标题章节、围栏和缩进代码块：章节保留标题，正文及子章节变为一个占位；代码块整体变为占位。空章节不生成占位。父节点展开后恢复原来的子节点状态，源码、解析对象和 `Style::HIDDEN` 不变。

```rust
use mdtui3::{Document, FoldView, FoldId, HeadingId, Budget, WorkLimit};

let mut doc = Document::new();
doc.append("# Guide\n## Details\nbody\n")?;
let mut view = FoldView::new(&doc, 1)?;
view.set_collapsed(&doc, FoldId::Heading(HeadingId(1)), true)?;
view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)?;
let update = doc.append("more\n# Next\nvisible\n")?;
view.apply(&doc, &update)?;
view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), false)?;
assert!(view.is_collapsed(FoldId::Heading(HeadingId(1))));

let mut budget = Budget::new(WorkLimit {
    queries: 16, tiles: 8, bytes: 1024, objects: 16,
    ..Default::default()
});
let mut tiles = Vec::new();
view.heights().viewport_budgeted(0..24, 0, &mut budget, &mut tiles);
// 使用同一个 budget 驱动 LayoutPass::folded / advance_folded。
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Document::fold_candidate` 按稳定 `FoldId` 查询整段范围、隐藏正文、父节点、摘要源码及开放状态；`fold_candidate_at` 查询当前位置的最内层候选。`fold_candidates(span)` 二分定位范围，只枚举开头块与范围相交的候选；其 `advance` 与视口共享 Budget，每检查一个块（包括非候选）消耗一次 query 和 object，耗尽可续，完成返回 `Covered`。游标绑定文档和 revision；范围之前开始的祖先通过 `parent` 查询。摘要用 `summary_text` 限定 UTF-8 字节数并计费，不统计正文行数、任务数或复制长标题。`hidden_at` 按预算返回 `FoldLookup::{Visible, Hidden, BudgetExhausted}`，隐藏结果是最外层遮蔽者，适合占位命中和源码定位。

`LayoutPass::folded` / `advance_folded` 接收 `FoldLayoutSink`，复用已有有预算排版循环。每个可见占位只调用一次 `placeholder`，传入稳定 ID、隐藏源码、摘要区间及归属 tile；接入方将占位作为块边界绘制并在 `end_tile` 中计入其高度、重置必要的换行状态。回调只获得元数据，若要读取摘要，应另用共享 Budget。占位高度全部归属隐藏起点所在 tile；同一 tile 可有多个占位、标题和后继正文。隐藏内部的文字、图片和子占位不会交给回调。该接口仍使用 `RenderRun`，显示实体解码、终端宽度及真实软换行由接入方处理。完整示例见 `examples/folding.rs`。

高度树独立保存基础高度/有效性与区间遮蔽计数。`height`、`prefix_rows`、`locate` 和视口查询返回折叠后的高度，`base_height` 返回遮蔽前的储存值；完整隐藏的内部 tile 无须逐个清零或恢复。每个占位及末端边界 tile 保持可检索，再按预算精确测量。单节点操作为 O(log H + log F + log T)，标题祖先最多六级，H/F/T 分别是标题、已收起节点和 tile 数。父子交替收放、失效、基础高度变化、树增长及 O(1) resize 保留嵌套遮蔽。重复操作不保存历史快照，新增视图状态 O(F)，高度树仍为 O(T)。

`set_collapsed` / `toggle` 返回源码变化范围和视图版本；从受影响边界起惰性过期后续高度与检查点，旧异步测量和布局继续任务会被拒绝。`view.measure` 验证本视图的测量及检查点依赖；`insert_checkpoint` 同时使下游布局失效。`reset` 保留折叠意愿与遮蔽，只重置基础估计和检查点。保留的高度仍可能是过期估计，展开后仅测量请求窗口；选择远处 tile 时默认局部近似，不能承诺精确绝对行号。`folded_from_checkpoint` 只接受本视图的有效起点及指定字节距离内的恢复，接入方恢复 sink 状态，恢复步骤也使用同一 Budget。

每次 append/finish 或连续 Update 批处理后调用 `view.apply`，然后才能查询。对已有折叠，更新只检查最多六个开放章节和一个开放代码块；开启默认收起时还处理新增标题/块的元数据，不枚举历史折叠。新 tile 在 apply 返回前继承遮蔽。开放代码块的未分类行前缀暂属该块，闭合/下一块确认后可以收回；Setext 晚确认也能缩短前一章节的范围。普通源码追加不会读取旧正文。重复应用不清掉新测量，错误文档或不连续更新在修改视图之前被拒绝。

折叠导航、锚点恢复和有预算批量操作见下文。目录 UI、预览、表格/容器折叠、手工交叉范围继续见 [TODO](TODO.md#折叠支持规划)。

## 折叠导航与源码锚点

`FoldNavigation::new` 从当前 `Navigator` 快照创建任务，接受全部 `NavAction`。目录选择用 `Heading(id)`，搜索命中或已解析链接用 `Source(offset)`；普通滚动后的起点由 `Navigator::reconcile_source` 提供。创建任务只检查身份及参数，实际索引查询在 `advance` 中计入共享 Budget。

```rust
use mdtui3::*;
let mut doc = Document::new();
doc.append("# Guide\n## Details\nbody\n")?;
let mut view = FoldView::new(&doc, 1)?;
view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)?;
view.set_collapsed(&doc, FoldId::Heading(HeadingId(1)), true)?;
let mut nav = Navigator::new();
let mut jump = FoldNavigation::new(
    &doc, &view, &nav, NavAction::Heading(HeadingId(1)),
    FoldNavigationPolicy::Reveal, 24,
)?;
loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    let progress = jump.advance(&doc, &mut view, &mut budget)?;
    if progress.completion == Completion::Covered {
        if let Some(target) = progress.target {
            nav.apply_target(target);
        }
        break;
    }
    // 在后续帧继续；已经展开的祖先保持展开。
}
assert!(!view.is_collapsed(FoldId::Heading(HeadingId(0))));
assert!(view.is_collapsed(FoldId::Heading(HeadingId(1))));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Reveal` 是默认策略，只展开实际遮住目标的章节/代码块；目标标题自身正文和无关子树的折叠意愿保留。`VisibleOnly` 不改变折叠：`NextHeading` / `PreviousHeading` 与严格同级的 `NextAtLevel` / `PreviousAtLevel` 遇到隐藏标题时，按最外层折叠范围端点重新查索引，跳过整段。显式 `Heading` / `Source` 及父/子/兄弟等结构动作若目标隐藏，返回无目标，不改成另一种结构关系。级别上限与严格同级的语义沿用现有索引。逐行、翻页及首尾动作直接使用折叠后的高度索引。

每个有界步骤消费 1 query、1 object：解析动作、检查可见性并展开/跳过一个遮蔽范围、或查询最终估计行。当前方言的祖先最多六级标题加一个代码块；一次步骤含固定数量的索引操作，不读取正文或标题文本。只给上述最小预算也可推进。耗尽返回 `BudgetExhausted`，保存目标或已经跳过的区间；`Covered` 表示动作结束，`target: None` 表示无匹配目标。`change` 是**本次调用**已展开区间的包络和最新视图版本，已同步更新高度/检查点失效；取消或失效不会撤销已完成的展开。任务存储 O(1)，Reveal 最多打开七个节点；VisibleOnly 总工作按实际跳过的折叠区间数计费，每步 O(log H + log F + log T)，不会逐个检查被隐藏的标题。

任务绑定文档身份/revision、视图身份及折叠版本，文档追加/首次 finish、外部折叠操作和 reset 后拒绝旧任务；错误返回前不消费预算或修改视图。自身展开更新其版本，期间的正常高度测量不使任务失效；最终定位使用最新高度。完成后重复调用返回原结果且不重复展开，返回行仍是当时的估计。任务不改写原 Navigator；调用方只应用当前有效任务的结果，新的用户导航应丢弃旧任务。用 `Navigator::apply_target` 应用结果，它在 source 为 None 的普通滚动后清除旧的精确源码目标。远处定位、同 tile 多标题和折叠占位仍由局部 `LayoutPass` 在共享 Budget 下细化，随后再 reconcile；此接口不排整篇前缀，也不承诺精确绝对行号。

`FoldAnchor::new(&doc, source)` 保存文档身份和精确源码位置，`resolve(&doc, &view, viewport_rows, &mut budget)` 返回当前投影。隐藏时 `hidden_by` 标明最外层折叠，`target.source` 指向其占位起点；可见时返回原位置。保留同一个 FoldAnchor，在收起/展开、append、Setext 晚确认或 resize 后重新 resolve 并应用目标即可恢复；不要用占位位置覆盖保存的原锚点。每次 resolve 消费 1 query、1 object，预算不足返回 None，不改变折叠。锚点允许 EOF，追加后原字节偏移保持不变；不绑定 revision，错误文档或未同步的 FoldView 会被拒绝。它保存源码而非精确屏幕行/列，选区仍使用原源码范围。示例 `examples/folding.rs` 演示占位投影和分帧恢复到 120 万字节正文内的目标。

## 批量折叠与新节点默认设置

`FoldBatch` 提供当前 revision 上的显式命令：`All` 展开/收起全部候选，`Subtree` 包括根及全部后代，`AtLevel` 只修改严格等于指定级别（1–6）的标题，`Focus` 展开目标及祖先并收起所有其他候选。聚焦会修改后代意愿；仅想露出目标并保留无关状态时使用 `FoldNavigationPolicy::Reveal`。

`Siblings { target, collapsed }` 展开/收起与目标具有相同 `FoldCandidate::parent` 的其他候选，保留目标、祖先和各节点后代的原有意愿。兄弟可以是不同级别的标题或代码块；没有父节点的候选互为根层兄弟，包括首个标题之前的代码块。目标已被祖先隐藏时仍可操作，但不会自动展开祖先；没有兄弟时完成且无变化。该命令只修改已有节点，不改变新节点默认值。

```rust
use mdtui3::*;

let mut doc = Document::new();
let mut view = FoldView::new(&doc, 1)?;
view.set_defaults(FoldDefaults {
    heading_levels: [false, true, true, true, true, true],
    code: true,
});
let update = doc.append("# Guide\n## Details\nbody\n")?;
view.apply(&doc, &update)?;
assert!(view.is_collapsed(FoldId::Heading(HeadingId(1))));

let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::All { collapsed: false })?;
loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    let progress = task.advance(&doc, &mut view, &mut budget)?;
    // progress.change 是本次调用已生效的源码包络；中间状态可以直接排版。
    if progress.completion == Completion::Covered { break; }
    // UI 应在此交回事件循环，下帧使用新 Budget 继续。
}
assert!(!view.is_collapsed(FoldId::Heading(HeadingId(1))));
# Ok::<(), Box<dyn std::error::Error>>(())
```

创建任务为 O(1)，范围定位在第一次 `advance` 中消费 1 query、1 object；之后每个块同样消费 1 query、1 object，包括非候选及不匹配级别/父节点的块。子树范围通过索引定位；兄弟操作检查父章节正文范围内的块，包含后代块的元数据但不修改后代意愿，根层兄弟操作及其他命令分帧扫描整个块索引。既不扫描源码正文，也不逐 tile 更新隐藏范围。每步为 O(log B + log H + log F + log T) 的固定次数索引操作（B 为块数），完整任务按实际访问的块数计费。`visited` / `changed` 是累计块访问数/实际状态变化数，`change` 是本次调用的区间包络及最新版本，耗尽返回 `BudgetExhausted`。任务仅保存 O(1) 游标状态；不复制候选列表或折叠快照。

批量任务渐进生效；取消、文档更新、外部折叠操作、默认设置变化或 reset 后，已经完成的修改保留。旧任务会在消费预算或修改视图之前被拒绝；任务自身修改及正常高度测量允许继续，完成后重复调用不再产生修改。每次实际修改复用单节点的高度遮蔽、检查点及测量失效，调用方可以在两次 advance 之间正常绘制。需要原子切换整棵树或取消回滚的宿主，不能将该接口当成事务。

`FoldDefaults` 只规定**后续首次识别**的节点初始意愿：`heading_levels[0..6]` 对应 h1–h6，`code` 控制代码块，默认全部展开。`set_defaults` 为 O(1)，不追溯已有候选，不覆盖显式展开/收起；已有文档创建视图后仍全部展开，需要调整时使用批量命令。默认设置变化增加视图版本，但不改变当前布局；相同设置无操作。批量命令不修改默认设置，因此“全部展开”后新到来的标题仍按当前默认初始化。单节点收放继续保留子节点意愿；resize 保留默认设置和所有节点意愿。

`view.apply` 通过 `Update.new_headings` / `new_blocks` 初始化新候选，Setext 晚确认使用新 HeadingId，即使其块早已存在；重复应用不会重置意愿。`default_visits` 计量检查的新增标题/块元数据，`range_visits` 继续计量最多七个已有开放范围。初始化成本计入新增对象事件，不宣称任意大 Update 的消费具有整帧硬预算；没有新的候选时，不回访历史节点或读取旧正文。合并 Update 在 apply 时统一使用当前默认设置；需要在流中切换默认值时，先应用切换前的更新。

可运行示例：`cargo run -r --example fold_batch`。

## 临时折叠预览

`FoldView::set_preview(&doc, Some(id), &mut budget)` 暂时展开一个章节或代码块；传入 `None` 结束预览，切换 ID 则先恢复旧节点。每个视图同时只有一个预览。祖先和后代的折叠意愿保持不变，因此被父章节隐藏的节点不会因预览而自动露出；展开父章节的预览仍保留子节点占位。这里提供暂时展开整节点的视图状态，实际只排版当前窗口；按前几行/字节截断的摘要预览另行实现。

```rust
use mdtui3::*;
let mut doc = Document::new();
doc.append("# Guide\nintro\n## Details\nbody\n")?;
let mut view = FoldView::new(&doc, 1)?;
let root = FoldId::Heading(HeadingId(0));
view.set_collapsed(&doc, root, true)?;
for target in [Some(root), None] {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    let progress = view.set_preview(&doc, target, &mut budget)?;
    assert_eq!(progress.completion, Completion::Covered);
    assert_eq!(view.preview(), target);
    // 本地折叠意愿没有改变；实际可见性用 hidden_at 或有预算排版读取。
    assert!(view.is_collapsed(root));
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

`is_collapsed` 始终返回节点的本地意愿。预览期间调用 `set_collapsed`、`toggle` 或批量命令，会修改结束预览后应恢复的意愿；预览继续让本节点展开。未显式修改时恢复预览前的状态。其他节点的操作立即生效，`FoldNavigation` 按实际可见性判断必要展开路径。宿主在预览结束时显式传 `None`；预览不会因普通导航、默认设置变化、追加、finish 或 resize 自动结束。恢复使用当前结构范围，因此流式增长和 Setext 晚确认不会恢复过期的遮蔽区间。

开始、切换及结束各原子消费 1 query、1 object，支付最多两个节点的固定次数索引操作；O(log H + log F + log T)，只额外保存一个 ID 和布尔意愿，O(1) 空间。不读取正文、不枚举覆盖的 tile、不保存历史快照。相同目标为无操作且不扣预算；预算不足返回 `BudgetExhausted`，保留当前预览和所有状态，可在后续帧重试。错误文档、未同步视图和非法候选在扣费及修改前被拒绝。预览变化返回旧/新隐藏范围的包络和最新版本，并使受影响的高度、检查点、测量和继续任务失效；即使节点原已展开或范围为空也增加版本。实际绘制继续使用共享 Budget 的剩余额度，或留待后续帧。

`cargo run -r --example folding` 演示父章节预览、子折叠保留及结束恢复。`tests/fold_preview.rs` 和折叠排版模型覆盖预算原子性、显式意愿、流式范围、独立视图和旧任务拒绝；百万字节点的切换不读正文，进入预览后的排版仍受窗口及字节预算限制。

## 明确支持的 Markdown 方言

- 段落与空行；ATX 标题；Setext 标题；分隔线。
- 缩进代码；反引号/波浪线围栏代码，opening fence 长度 3..32；代码 info 保留源码区间。
- 引用行和有序/无序列表项的块信息。容器按物理行展平，提供 quote depth 或列表 indent/number，不构建嵌套列表树，不推断列表续行或 lazy blockquote continuation。
- 管道表格、列对齐、单元格行内语法；任务列表复选框状态。
- `*em*`、`_em_`、`**strong**`、`__strong__`、`~~strike~~`，以及精确长度匹配的反引号代码 span。支持跨同一段落的多行输入。
- ASCII 标点反斜线转义；内联链接 `[text](destination)`；shortcut/full/collapsed 引用链接；单行引用定义；裸目标和尖括号目标，可选 title。
- 尖括号 URI/email 自动链接；`link_destination` 借用原始地址，`link_url` 为 email 添加 `mailto:`。
- 图片 `![alt](destination)` 及引用式图片，保留独立类型和 alt 信息，可由渲染器降级为链接。
- CRLF 的 CR 隐藏，LF 保留。裸 CR 按原文处理。
- 同块行内软换行、双空格/反斜线硬换行的结构化标记。
- 显示语义读取支持实体/数字字符引用及已闭合代码 span 空白规范化，映射和上下文限制见上一节。

取舍：

- 不是完整 CommonMark/GFM 实现。行内 `***` 等超过支持长度的强调 run 按原文处理；flanking 使用 ASCII 空白规则，单词内下划线不匹配。交叉闭合采用最近同类 opener，并丢弃中间未闭合 opener，不做回溯。
- 未闭合的代码 span 在块结束后保留为原文，期间跳过的其它语法不会重新解析。嵌套链接不支持：新 `[` 替换尚未闭合的外层 `[`。
- 已识别的引用立即显示 label，并隐藏引用语法；尚未定义时没有可跳转目标。定义到来后只增加目标。第一个有效定义生效，后续重复定义不覆盖。标签最多 999 字节，折叠 Unicode 空白并转小写，不做完整 Unicode case folding。
- URL 和 title 各最多 4096 字节，左括号后的链接后缀候选最多 8256 字节（包括空白和闭合右括号）。支持平衡括号和转义定界，但返回的 destination/title 是原文；不提供多行目标/title、URL 解码或安全过滤。后缀失败时保留原文及已有 shortcut 引用，不回扫后缀中的其它语法。
- 引用定义、Setext、分隔线、闭合 fence 必须在块分类前缀内完成。超长候选行按普通内容/代码处理；超长 opening fence 的 info 只保留已分类前缀，其余该行隐藏。超长标题末尾的 closing hashes 不再回看识别。
- 自动链接候选最多 4096 字节，URI scheme 为 2..32 个 ASCII 字符；email 按 CommonMark 0.31.2 的 ASCII 地址规则。失败或 EOF 未完成的候选不重放内部行内语法；新自动链接中止未完成的外层方括号候选，保持链接源码有序且不重叠。返回原始 URI，不做 percent encoding。
- 不处理 HTML、GFM 裸 URL 自动链接、脚注、数学公式等额外语义。它们可能包含上述方言仍能识别的普通行内标记。HTML 不在当前需求范围。
- 支持只追加，不支持任意位置编辑、撤回已有源码或移除定义。文档销毁会释放 O(n) 状态，这是一次性清理成本。

## 源码入口

| 文件 | 职责 |
| --- | --- |
| `src/lib.rs` | 流式提交、稳定块/片段、按范围惰性查询、缓存 key |
| `src/block.rs` | 有界块分类 |
| `src/inline.rs` | 单遍分隔符、代码和链接状态机 |
| `src/destination.rs` | 链接/定义共享的有界目标及 title 状态机 |
| `src/display.rs` / `src/entity.rs` | 显示片段、替代文本源码映射、预算游标及字符引用解码 |
| `src/search.rs` / `src/display_search.rs` | 源码增量搜索、revision 绑定的显示文本搜索与命中映射、选区及任务状态 |
| `src/annotations.rs` | 区间样式、隐藏聚合、最新 revision |
| `src/references.rs` | 有长度上限的标签规范化和 BTreeMap 符号表 |
| `src/layout.rs` | lazy 高度与遮蔽树、viewport、锚点和过期测量拒绝 |
| `src/folding.rs` / `src/frame.rs` | 章节/代码块候选、独立折叠视图、预算排版和占位 |
| `src/checkpoints.rs` | 有效检查点的范围失效和前驱查询 |
| `src/table.rs` | 表格行/单元格、对齐、流式单元格解析和列宽提示 |
| `src/headings.rs` | 标题 arena 和六个级别的索引 |
| `src/navigation.rs` | 逐行/翻页/首尾/标题导航及源码目标 |
| `src/fold_navigation.rs` | 有预算折叠导航、可见标题跳过及源码锚点投影/恢复 |
| `src/fold_batch.rs` | 分帧全部/子树/兄弟/级别/聚焦命令及进度 |
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
nix develop -c cargo run -r --example budget
nix develop -c cargo run -r --example display
nix develop -c cargo run -r --example search
nix develop -c cargo run -r --example folding
nix develop -c cargo run -r --example fold_batch
```

测试包含任意单切点和随机 UTF-8 分块、与朴素数组/有序映射对照的区间/高度/检查点随机测试、深层 opener、百万字 Setext 转换、百万字隐藏区间、十亿片段的稀疏高度索引、逐字符超长段落、延迟定义成本和 resize 节点复用。没有把 wall-clock 阈值当作正确性条件。`scaling` 示例输出时间及操作计数，便于在目标机器上复查。

行内分块测试会特意跨过 `BLOCK_LOOKAHEAD`，避免短样例被整行缓冲后才解析。另有中间前缀一致性、TileKey 未变则语义不变、标题/链接索引与线性查询对照、目标/title 长度边界及百万字尾空格确认成本测试。自比较验证分块一致性，预期语义样例和朴素数据结构模型提供另外的判定依据，不能将这些测试视为完整规范合规率。

`tests/display_search.rs` 使用完整显示文本上的朴素搜索及独立语义样例作对照，覆盖实体部分命中、CRLF 映射、跨隐藏标记/tile、长查询和重叠匹配、真实流式逐前缀/逐切点、固定种子生成输入、极小共享预算、旧 revision 拒绝及失败回退计数。百万字隐藏区间验证有限预算不会触发整篇读取。

本 checkout 已安装 Rust skill 提供的本地 pre-commit：运行 `cargo fmt --all` 后只重新 stage 原本暂存的 Rust 文件。格式化可能改变提交中的行号；其它未暂存文件可能被格式化，但不会自动暂存。钩子不随 Git 分发，紧急情况下可用 `git commit --no-verify` 跳过。

## 存储实测与标准样例范围

`annotation_arena_stats`、高度树/检查点的 `arena_stats`、高度依赖树的 `dependency_arena_stats` 返回 Vec 容量字节数、节点数与实际容量增长分配次数。统计不包含 allocator 元数据、碎片、其它 String/Vec 或用户状态的间接堆分配。`scaling` 示例同时记录操作计数、查询分位数与容量；`one_byte_reused` 对照复用通知缓冲的逐字符输入。

2026-10-30 在 ark 上的 release 样本：524,286 字节密集标记产生 854,391 个语义节点；子索引改用 `Option<NonZeroUsize>` 后，节点大小由 88 降至 72 字节，对应 arena 容量由 92,274,688 降至 75,497,472 字节（约 18.2%）。写入访问数 8,010,100、固定视口查询访问数 13,022 均未改变，分配次数同为 20。缩小了节点常数，没有增加全树压缩过程。峰值 arena 容量等于当前容量，因为该 arena 不缩容；这不是整个进程峰值内存。语义/修订存储拆分与查询 scratch 复用尚未实施。

标准样例固定为 [CommonMark 0.31.2 autolinks](https://spec.commonmark.org/0.31.2/#autolinks)，离线夹具保存在 `tests/fixtures/commonmark_autolinks.rs`，保留上游 CC BY-SA 4.0 归属信息。19 个样例中，18 个在源码文本/目标语义上符合；606 为刻意差异：失败候选内部的反斜线不重新解析。URI percent encoding 属输出层，因此对照时归一化目标编码。其它 CommonMark 章节与 GFM 全集未评分；现有容器展平、强调/代码回退和语法长度限制仍按上文方言约定，不以自比较分块测试充当完整规范合规率。

显示语义另固定了 24 个 CommonMark 0.31.2 文本样例（25–30、35–37、39–40、328–340），离线夹具 `tests/fixtures/commonmark_display.rs` 同样保留归属信息，只对照移除 HTML 标签后的文字。HTML、属性解码、列表结构及完整代码回退未计入这组评分；不将局部样例通过表述为整个章节合规。`tests/display.rs` 另外验证源码映射、跨 tile/逐标量查询、逐前缀/逐切点一致性、百万字代码晚闭合、错误候选线性计数、预算推进和旧游标拒绝。
