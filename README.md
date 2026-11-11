# mdstream 核心库

只追加的 Markdown 解析器，以及按视口读取语义、维护排版高度和恢复排版状态的数据结构。零运行时依赖，禁止 unsafe。这里交付的是核心库，不包含终端绘制、HTML 输出、字体塑形或自动换行实现。

目标是：即使输入逐字符到达、一个段落长达数百万字、分隔符很晚才闭合、引用定义出现在文末，也不反复扫描旧文档。支持的方言有明确限制，以换取可证明的复杂度。

当前开发方向（2026-10-30 确认）：先把地基打牢，在既定需求和复杂度约束内支持尽量丰富、可复用的基础功能，再在其上搭建完整项目。现阶段交付核心库与可运行示例，终端 UI 留到后续阶段；按依赖关系和实现容易程度持续推进 TODO 中的核心能力。具体范围见 [开发阶段与推进原则](TODO.md#开发阶段与推进原则)。

已交付任务及验收记录见 [DONE](DONE.md)。后续优化见 [核心改进顺序与验收条件](TODO.md#核心改进顺序与验收条件)：优先保证整帧实际重排受窗口和恢复预算约束，再改善缓存复用与扩展语义支持。

## 工作区结构

仓库是 Cargo workspace。当前只有核心 crate `crates/mdstream-core`：package 名称为 `mdstream-core`，库 crate 名称保持 `mdstream`。终端渲染 crate 留到后续阶段加入；根目录保留跨 crate 的 README、TODO/DONE、docs、scripts 和 flake。

## 接入

```rust
use mdstream::{Document, HeightIndex, Style};

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

可执行示例在 `crates/mdstream-core/examples/stream.rs`。package 名称为 `mdstream-core`，库 crate 名称仍为 `mdstream`。

`append(&str)` 接收有效 UTF-8，可以在任意 Unicode 标量边界分块；传输层的任意字节分块可使用 `Utf8Document`：最多暂存 3 字节不完整尾部，非法序列及 EOF 截断采用 U+FFFD 替换；偏移按解码后的文档定义。`finish()` 表示真正的 EOF、完成尾部解析，重复调用无副作用，此后不能再追加。空追加不改变 revision。

所有 ID 和源码 byte offset 都只在所属文档中有效。已分配的 `BlockId`、`TileId`、`LinkId`、`SymbolId`、`TableId`、`HeadingId`、`ContainerId` 不会重排；当前块和最后一个片段可以增长。库返回的是借用、区间和增量通知，不复制历史 AST。`Update` 可以逐次消费，或使用 `merge` 将连续更新合并后在最终 revision 消费，不需要保存事件历史。

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
| 按级别查找前后标题、同作用域父子/兄弟/范围 | O(log H)，H 为标题数，至多六级 |
| 按源码找当前章节 | O(log n)，上下文边界二分，不遍历容器祖先 |
| 逐行、翻页、首尾导航 | O(log T)，基于当前高度估计 |
| 标题导航 | O(log H + log n + log T) |
| 容器按 ID 读取范围、父 ID、深度 | O(1)，开放末端共用已解析位置 |
| 有预算地向上读取 D 个容器 | O(D)，游标 O(1) 空间 |
| 表格按行列读取 / 按源码定位行 | O(1) / O(log R)，R 为该表行数 |
| 按视口查询链接/图片对象 | O(log L + 结果数)，L 为链接和图片总数 |

核心文档空间 O(n)。高度树最多 O(T) 节点，但没有遮蔽边界的未测量整片区域只用一个均匀节点。检查点树按需分配：若曾缓存 C 个位置，空间 O(min(T, C log T))，另外保存调用者提供的状态。所有树的递归深度受机器字长约束，不随 Markdown 嵌套深度增长；释放文档不会递归释放一个深层 AST。

单次 `append` **不是最坏情况 O(新增字节数)**：Vec/String 可能扩容；一个新闭合符可能弹出许多旧 opener；换行可能提交之前缓冲的有界行前缀；长表头确认时会扫描候选的单元格边界，确认或失败时会把延迟内容语义消费一次，长表格分隔行晚失败也会把延迟尾部普通消费一次。保证针对整个流的摊还复杂度，累计不会变成 O(n²)。没有使用局部无界二次算法。调用者主动在每次追加后遍历 `source()` 或整段 `runs_in()`，仍会自己制造二次工作；常规显示应使用有界 `runs(TileId)`。

### 为什么不会重复做旧工作

1. 原文保存在 append-only String 中，所有语义对象引用 byte span。输入游标只向前走，块分类最多查看一次完整短行或长行的有界前缀。
2. 行内解析是状态机。分隔符的同类 opener 索引允许直接定位候选；每个 opener 至多入栈、出栈各一次。失败的候选不会回退重扫。反引号 run 按长度配对，无论 run 多长都只保存区间。
3. 样式和隐藏标记保存在动态二叉区间树。闭合分隔符只向覆盖区间的 O(log n) 个节点写 tag；查询继承这些 tag。源码不需要移动、包裹或重新分词。节点同时聚合最新 revision 和“全部隐藏”信息。
4. 引用保留 `LinkId -> SymbolId -> destination` 间接关系。定义只更新一个符号版本和一个保守全局引用 epoch，没有反向引用列表遍历。`DependencyKey` 只检查自身引用符号；尺寸变化从符号最早使用位置保守失效后续布局状态。
5. 高度树支持 lazy assignment、子树和以及按累计高度选择。零高度区域可整段跳过。重复 resize 重用节点，不累计废弃树。
6. 容器独立 arena 只保存父 ID、深度和属性；活动栈每保留一层必须消费本行的显式标记或正长度缩进，空格串只扫描一次。开放范围共用一个已解析末端，长行追加不更新祖先；容器各开闭一次，不复制祖先链。标题作用域保存独立六级索引和进入时的外层章节摘要；上下文只在容器切换时追加边界，范围由下一同作用域标题及容器末端派生。
7. 检查点子树记录有效计数。失效只把覆盖子树的计数置零，查找前驱直接剪枝。失效时不逐个运行调用者状态的析构函数；旧状态在该位置被覆盖或整个索引销毁时释放。

`Document::work()` 暴露输入扫描量、块分类输入量、行内扫描量、分隔符操作数、标签规范化字节数、区间树实际写入访问次数、表格行扫描字节数与自动链接候选检查字节数；Setext 首尾原文端点在既有行内字节扫描中维护，计入 `inline_bytes`，确认不另扫正文；`atx_bytes` 计 ATX 正文与待确认尾缀的单遍摘要扫描；`table_header_bytes` 计候选表头摘要扫描，`table_candidate_bytes` 计长分隔行探测；`container_bytes` 计容器前缀检查，`container_steps` 计活动帧匹配及单个容器开闭，`list_steps` 计条目直接内容/兄弟检查及一次性 Tight → Loose 转换；`heading_steps` 计作用域开闭、已分类行的上下文检查和标题插入，每步至多六个固定索引操作，不含查询和标题区间的 O(1) 延长。分类字节量是分类器接收的字节数，分类器内部对这些字节只做固定次数的线性扫描。它不是 CPU 指令计数。测试使用这些指标约束增长，不依赖机器跑分来“证明”复杂度。

`Runs::annotation_visits()` 单独统计本次查询访问的区间节点，不修改文档统计。查询只在开始时二分定位块，之后顺序跨越块边界并遍历样式区间，不对每个字符从根重查；均匀长区间可以直接跳过。片段划分直接跳到最多 256 字节处，再回退至 UTF-8 边界，无需逐字符解码整段新增输入。

## 视口与排版契约

源码按最多 **256 字节**切成片段，绝不切断 UTF-8 标量。一个片段可跨块边界；一个巨大段落也会被切成很多小片段。`runs()` 给每个 run 附带块 ID，未提交的尾部原文则是 `None`。样式可从任意片段直接查询，不需要从段落开头重放 Markdown 语法。

片段边界**不是强制换行，也不保证是 grapheme、单词或 shaping 边界**。渲染器需要定义行归属约定，并携带 column、词尾、字素尾部等恢复状态。`CheckpointIndex<S>` 的 S 完全由渲染器决定，表示开始某个片段前的状态：

```rust
use mdstream::{CheckpointIndex, Document, HeightIndex, TileId};

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
# use mdstream::{Document, HeightIndex, TileId};
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

不完整的行首也在片段中，立即以原文可见。`pending()` 最多 1024 + 3 字节；通常在换行、EOF 或达到上限时提交块分类。超过上限的行后续直接流式消费；符合条件的长表头与长分隔行延迟语义提交，按新增字节维护识别状态。长表头已完成块分类，`pending()` 为空；长分隔行的 `pending()` 只保留最初分类前缀，其后原文仍可由片段读取，见下方表格规则。不复制或反复解析增长中的尾部。缓存使用 `tile_key()`，不要只按片段 ID 永久缓存；`Update::dirty` 是源码区间通知，不是展开的片段 ID 列表。

## 顶层段落行首缩进

已分类的顶层 `BlockKind::Paragraph` 每个物理行开头的 1–3 个 ASCII 空格作为缩进隐藏，不送入行内解析；原始源码、块范围和字节映射保持。该规则也作用于跨行强调、代码 span、链接和数学，以及稍后提升为 Setext 的段落。行内普通空格沿用原规则；段末空白见下节。容器正文及缩进代码不属于此段落扩展；顶层 tab/混合缩进代码见[独立规则](#顶层-tab-缩进代码)，缩进代码仍可中断段落。

短尾部在 LF/EOF 分类前按原文显示；达到 `BLOCK_LOOKAHEAD` 的长行提交后立即隐藏前导空格，后续只消费新增字节。候选表头沿用既有延迟决定规则：早管道失败后在首次行内消费时隐藏，长行先消费的前缀不因后续管道、确认或回退而重放。每行最多检查三个空格及一个后继字节，隐藏空格计入 `Work.inline_bytes`，只新增一个有界区间标记；不增加解析状态，不扫描旧正文、历史块或祖先，保持累计 O(n log n)、空间 O(n)。

隐藏缩进仍由 `DisplayCursor` 按源码窗口和前瞻计费；6 bytes / 1 query / 1 object 可推进任意 UTF-8 字符，零预算不移动。分类会失效旧显示/布局缓存和 revision 绑定任务。`tests/paragraph_indent.rs` 覆盖 UTF-8/CRLF/EOF、全切点与逐前缀、跨行行内语法、Setext、长表头成功/失败、原始映射和小预算、百万字后逐字追加及 4096 个缩进行；官方 222/224 的独立评分见文末。完整容器/缩进规则仍在 TODO。

## 顶层段落末尾空白

顶层 `BlockKind::Paragraph` 在 EOF、空行或其它块开始而关闭时，隐藏最后一个物理 LF/CRLF 之前（无换行时为 EOF 之前）的原始 space/tab 尾部。源码、块范围及映射不变，末尾物理 LF 仍可见且不标记 `HARD_BREAK`/`SOFT_BREAK`，末尾反斜杠仍是字面内容。依据 [CommonMark Paragraphs 样例 226](https://spec.commonmark.org/0.31.2/#example-226)及[块末不产生硬换行](https://spec.commonmark.org/0.31.2/#hard-line-breaks)的规则；不是对显示输出执行 trim：实体解码出的空格、已闭合代码/数学/链接内部的空白仍保留。未闭合行内候选按原始尾部裁剪；孤立 CR、Unicode 空白及 form feed 不是 space/tab。已识别容器段落也使用相同的末尾裁剪规则，见下节；内部换行的混合尾空白另见后文。代码块保留原始空白，Setext 沿用其整段原始内容规则。

开放段落尾部保持暂定可见，包括已读取 LF 的最后一行；同段下一行提交后才按既有规则隐藏换行前空格并确认软/硬换行。关闭时通过 `Update::dirty`/可见性变化失效旧 tile key 和 revision 绑定的显示任务；调用方应消费更新，不能将开放尾部当作最终布局。早管道候选沿用延迟分类；长表头在移交行内状态前仅保存旧段尾区间，表格确认时隐藏，候选失败则恢复原有续行语义。

复用行内单遍扫描的原始非空白端点，无尾部反向扫描或正文重放；每个段落关闭仅新增一次 O(log n) 区间标记，长表头仅增加 O(1) 范围状态，累计解析 O(n log n)、空间 O(n) 不变。隐藏字节仍按源码读取预算计费。`tests/paragraph_tail.rs` 的 6 项回归覆盖全切点/逐前缀、硬换行、原始映射、6 字节预算、表头成功/失败、百万空格后 1000 次追加及 4096 次段落关闭；226 的官方夹具与投影规则保持不变。

## 容器段落末尾空白

已识别引用、列表（含任务条目）及其嵌套作用域内的普通逻辑段落，在 EOF、空行、兄弟/作用域变化或新块确认关闭时，隐藏原始 space/tab 尾部。既有显式和 lazy 续行保持内部软硬换行语义；物理叶块的 `Sealed` 状态不等于逻辑段落关闭，开放段尾仍暂定可见。代码块、已闭合行内对象中的空白和实体生成空白保持原语义；Setext 仍使用其专属首尾裁剪。此扩展不改变容器匹配、行首缩进或不透明失败候选的内部回退方言。

关闭只读取逻辑段落首叶类型和现有原文端点摘要，增加一次隐藏区间更新，不回扫正文、历史叶块或祖先。早/晚表头候选共用原有 `paragraph_tail` 区间保存前一段的尾部，成功时裁剪、失败时继续普通行内序列；无新增持久状态，累计 O(n log n)、空间 O(n) 和共享读取预算保持。`tests/container_tail.rs` 覆盖 1512 组混合空白/作用域/行尾模型、UTF-8 全切点/逐前缀、表头成功/失败、稳定块/对象与缓存失效、跨 tile 映射和 6 字节预算、千层容器中的百万字追加及 4096 次逻辑段落关闭。

## 内部换行前的混合尾空白

普通行内解析路径中，已确认内部 LF/CRLF 前连续的原始 ASCII space/tab 全部隐藏，LF 保留原始源码映射。适用于顶层段落和既有同作用域显式/lazy 容器续行，包括随后确认为 Setext 的正文。软硬判定沿用现有方言：紧邻换行的至少两个 ASCII 空格或反斜线产生 `HARD_BREAK`，其余为 `SOFT_BREAK`；tab 不按列扩展成硬换行空格。因此 `a\t  \nb` 是硬换行，`a  \t\nb` 是软换行，两者显示均为 `a\nb`。参照 [CommonMark 换行规则](https://spec.commonmark.org/0.31.2/#hard-line-breaks)；此处明确的是项目混合空白方言，不表示完整 CommonMark 空白规则已完成。

同一行内序列的下一行确认前，尾空白与换行仍暂定可见；晚表头沿用待确认换行的隔离、成功丢弃和失败提交。裁剪只作用于普通换行前的原始尾部，不修改正文内部 tab、实体解码的 tab、代码 span/数学及代码块的字面内容。孤立 CR 会中断尾部，反斜线前的空白沿用原行为；容器段末裁剪见上节；顶层 tab/混合缩进代码见[独立规则](#顶层-tab-缩进代码)，容器叶块 tab 缩进及失败不透明候选的完整回退规则仍待实现。

新增一个 O(1) 尾部起点，与原有连续空格起点分别维护，复用单遍 `inline_bytes` 扫描；换行确认仅扩大既有隐藏区间，不遍历尾空白、正文或祖先，不新增显示替换或依赖。累计解析 O(n log n)、空间 O(n) 和共享读取预算保持。`tests/internal_tab_breaks.rs` 的 6 项验收覆盖 1008 组混合空白/作用域/换行模型、UTF-8 全切点及逐前缀、晚表头成功/失败、缓存失效、源码映射和 6 字节预算，以及百万字混合尾部后 1000 次追加和 4096 个换行。

## 显式列表与引用容器

`ContainerId` 指向独立 arena，`ContainerKind` 区分 Quote、List 和 Item；列表节点保存 bullet/有序分隔符，条目保存 number 和可选 TaskMarker。`Document::container(id)` O(1) 返回当前 revision 的 `Container` 快照：父 ID、深度（根为 1）、完整物理行源码包络、Growing/Sealed 状态。每个叶块只增加 `Block.container` 指向最内层容器，叶块索引仍按源码排列；`BlockKind::Quote/ListItem` 保留本行标记信息，不再代表整个容器范围。ID 只在所属 Document 内有效，关闭后不复用。

```rust
use mdstream::{Budget, ContainerKind, Document, SyntaxState, WorkLimit};
let mut doc = Document::new();
let update = doc.append("> - [x] first\n>   continued\n")?;
assert_eq!(update.new_containers, 0..3); // quote -> list -> item
let item = doc.blocks()[0].container.unwrap();
assert_eq!(doc.blocks()[1].container, Some(item));
assert_eq!(doc.container(item).unwrap().depth, 3);
assert!(matches!(doc.container(item).unwrap().kind, ContainerKind::Item { .. }));
let mut path = doc.container_ancestors(item).unwrap();
let mut budget = Budget::new(WorkLimit { queries: 1, objects: 1, ..Default::default() });
assert_eq!(path.advance(&mut budget).unwrap().unwrap().0, item);
let update = doc.append("# outside\n")?;
assert_eq!(update.closed_containers.len(), 3);
assert_eq!(doc.container(item).unwrap().state, SyntaxState::Sealed);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`container_ancestors` 借用文档，每次 `advance` 消费 1 query / 1 object 返回一层，预算不足返回 `Err(BudgetExhausted)` 且不移动，换预算继续；根之后为 `Ok(None)`。借用期间不能追加，游标不复制路径也不允许静默跨 revision 使用。简单上下文读取可以直接使用 O(1) 的父 ID 和深度摘要；不要每帧自行无预算地遍历所有容器。

流式协议：只在换行、EOF 或 `BLOCK_LOOKAHEAD` 前缀达到上限时提交结构。未分类尾部不属于开放范围；`Update.container_end` 是所有 Growing 容器共用的已解析末端，解析长行时随实际消费增长。`new_containers` 给新 ID 范围；`closed_containers` 按实际弹栈次序报告关闭 ID，每次从最内层向外关闭，节点末端固定为离开该容器的行首或 EOF。结束一个叶块不意味着容器关闭；同一 List 的兄弟 Item 使用不同 ID，List ID 保持。容器范围/状态属于对象元数据，不能仅凭旧 `tile_key` 复用这些快照。追加仅改变共享末端，不逐祖先生成增长事件；消费方同样可以保存共享末端，不能因收到更新就自动遍历全部开放节点。`merge` 只移动新事件，`append_into/finish_into` 复用关闭缓冲区；空追加和重复 finish 不产生开闭。

当前方言与边界：

- 支持显式 `>`、`>>`、`> >`，以及 `-`/`+`/`*` 和 1..9 位数字加 `.`/`)` 列表；可混合嵌套。每层容器标记前允许 0..3 列水平空白，tab 按物理行每四列对齐；`>` 后最多消费一个可选空格，或恰好占一列的 tab。相同父级、相同列缩进、相同 bullet/有序分隔符延续 List；改变数字只创建新 Item，改变标记类型创建新 List。
- 列表标记后须为空格、tab 或完整行结束；通常消费标记后的全部水平空白；含 tab 且达到五列的代码首行按下述[列表首行 tab 缩进代码](#列表首行-tab-缩进代码)消费一列分隔和四列代码缩进。续行须用空格/tab 补足该 Item 从父前缀到正文的列宽（包含标记前缩进；空条目按标记末端列宽），再匹配子层。每个兄弟按自己的数字/空白宽度确定续行要求；任务标记及其后空白不计入条目续行宽度。通常在真实字节边界匹配；已有正文、已开启代码/数学围栏及下述新叶块分类另支持部分 tab，失败和源码保留规则见[容器 tab 列对齐](#容器-tab-列对齐)。
- 显式引用续行重复 `>`，列表续行补足缩进；连续非空正文也支持省略部分或全部前缀的 [lazy continuation](#容器-lazy-continuation)。完整空行允许省略列表缩进并保留身份，见[列表跨无前缀空行](#列表跨无前缀空行)；引用仍需显式 `>`，缺失引用及其子容器关闭。不满足空行或 lazy 条件时，缺失某层前缀只保留已经匹配的外层，内层关闭。空条目和空任务保留条目身份；任务只识别最内层新条目的正文开头，任务标记之后继续识别标题或普通行内正文。
- 容器正文保留逐物理行叶块身份；同一确切容器的显式续行可衔接强调、代码 span 及软/硬换行，见[容器内行内续行](#容器内行内续行)。容器内 ATX 和多物理行 Setext 标题及章节作用域见下节，表格识别及索引见[容器内表格](#容器内表格)。容器内围栏/缩进代码见[容器内代码块](#容器内代码块)；链接与脚注定义见[容器内定义](#容器内定义)；有界完整行内的独立分隔线生成 `ThematicBreak`，见[容器内分隔线](#容器内分隔线)。顶层表格分隔行确认、分隔线/Setext 和已打开围栏的优先级保持。列表/引用整段折叠见[容器折叠](#容器折叠)；容器内标题独立折叠见[容器内章节折叠](#容器内章节折叠)，tight/loose 元数据见[列表 tight/loose](#列表-tightloose)，终端布局仍待接入。
- 结构仅从既有 1024 字节分类前缀（UTF-8 边界最多额外 3 字节）识别。前缀中的完整 `>` 可以开层，列表/任务候选必须在前缀内有分隔符或完整行末；截断候选和前缀后的标记保留正文，不在后续追加中重新分类。深度受前缀字节限制；不声称支持无限深度或完整 CommonMark/GFM 容器规则。

通常的容器匹配工作累计 O(n + 开闭次数)，省略缩进空行另用下述 O(log(Q + 1)) 引用深度查询；容器/活动栈空间 O(n)。匹配一层必定消费正长度前缀；首次失配后，有界 lazy 判定成立则 O(1) 保留剩余活动路径，空白行按下述规则保留列表，其它情况批量弹栈。列表续行共享水平空白的末端字节/列缓存；部分 tab 探测和原分类回退各至多一次，扫描次数不随容器深度增加。整库仍累计 O(n log n)。`container_arena_stats` 统计节点、活动栈和引用深度索引的实际容量字节及 Vec 增长次数，不含标题索引/作用域、源码、分配器元数据或 Update。批量关闭和 EOF 仍可能一次处理整个活动栈；解析不承诺单次调用硬预算。有预算的查询不会枚举正文或子树。

`cargo run -p mdstream-core -r --example containers` 展示混合嵌套、续行、兄弟切换、未分类尾部及分帧祖先读取。`crates/mdstream-core/tests/containers.rs` 验证独立范围模型、增量快照重建、1000 层引用、深列表单次空格扫描、百万字长续行、空行延迟关闭及分块一致性；现有分块/fuzz 快照也包含容器。

## 容器内分隔线

既有显式 Quote/Item 路径内，完整物理行的正文若只有至少三个同种 `*`、`-` 或 `_`，允许标记间及末尾 ASCII space/tab；正文前 0–3 个 ASCII 空格，或[tab 列宽缩进](#分隔线的-tab-缩进)规则内的至多三列，生成带确切容器 ID 的 `BlockKind::ThematicBreak`。整行源码范围和稳定 BlockId 保留，容器前缀隐藏；分隔线本身保持顶层分隔线的原文显示约定，不经过行内强调解析。确认会关闭前一逻辑段落并裁剪其尾空白。

分隔线优先于同层兄弟/新嵌套列表标记，例如 `> - - -` 属于引用中的分隔线，`- * * *` 属于一个列表条目中的分隔线；已有列表的缩进续行归原条目。同作用域普通段落后的 `---` 仍优先成为 Setext，表头的合法管道分隔行仍优先确认表格，已打开的围栏和四空格代码保持字面内容。缺省前缀按既有 lazy 中断规则处理，不把分隔线吸收到旧内层段落。

沿用包含容器前缀的 `BLOCK_LOOKAHEAD`（1024 字节）窗口：短未完成输入暂定显示，LF/CRLF 或 EOF 确认；达到窗口而仍未完成的候选按既有正文/列表规则提交，后续即使补足分隔线也不回扫重分类。ASCII 输入在第 1024 字节为 LF 时可确认，无 LF 的 1024 字节候选已按未完成行提交。已识别前缀后的 tab/部分 tab 分隔线缩进见[后续子项](#分隔线的-tab-缩进)；完整缩进、容器匹配和超长分隔线仍待办。

容器匹配前只反向检查一次当前有界行的同种标记后缀，保存首标记和倒数第三个标记两个位置；每层以位置和至多三格缩进判断是否停止列表解析。扫描计入 `Work.container_bytes`，没有逐层剩余行扫描、旧正文/叶块回访或新增持久状态；各层操作按新前缀/开闭计费，累计解析与空间边界保持。

`tests/container_rules.rs` 的 7 项回归覆盖 648 组标记/间隔/失败模型、优先级与确切作用域、UTF-8 全切点/逐前缀、EOF/分类窗口边界、旧对象及缓存失效、原始源码映射和 6 字节共享查询预算；400 层引用/列表、百万字候选后千次追加、4096 条分隔线验证工作量。CommonMark 样例 61 的官方输入/期望与投影算法不变，评分由未支持变为支持；完整容器与标准覆盖总项保持待办。

## 容器 tab 列对齐

一般容器前缀支持**完整 tab 边界**上的缩进匹配；已开启围栏、一般正文及所选新叶块的部分 tab 扩展见下文，不声明完整 CommonMark tab 语义。物理行从第 0 列开始；前缀里的 ASCII 字符占一列，tab 前进到下一个 4 的倍数列。嵌套时不重置列号，源码偏移仍是 UTF-8 字节偏移。`BlockKind::ListItem.indent` 表示相对父前缀的列缩进。

- 列表标记后的全部 space/tab 照常属于前缀，Item 保存其相对父前缀的正文列距离；例如 `-\tfirst` 正文从第 4 列开始，续行可用四个空格、一个 tab 或等价混合空白。有序编号和每个兄弟独立计算；新任务的 `[x]` 及其后的空白不会加到续行距离中。
- 未进入下述部分 tab 扩展的续行每匹配一个列表层，都要求其目标列恰好落在某个字节之后。若目标位于 tab 内部，该层整个匹配失败，连同该层 tab 前的空格都不消费；已成功匹配的外层仍保留。活动正文可按原 lazy 规则保持整条路径，否则关闭未匹配作用域。例如 `- first\n\t> literal` 保留原始 tab；`- > $$\n\tpayload` 因缺失引用前缀而结束条目内数学块，第二行回到根。符合下文正文续行条件时，一个 tab 可以拆给两个各需两列的嵌套 Item。
- `>` 后的可选分隔只消费一列：`  >\t# title` 的 tab 恰好占一列，随后识别标题；新建引用首行 `>\t# title` 的三列 tab 留给叶块，现由[ATX tab 缩进](#atx-标题的-tab-缩进)识别为标题；已有引用续行也可按下述新叶块扩展识别标题。剩余空白可以作为下一层容器标记前的至多三列缩进，例如 `> \t-\tfirst`；后续 `> \t\tcontinued` 匹配同一 Item。这些完整边界匹配只隐藏真实源码字节；围栏代码和正文续行的部分 tab 使用下述替代空格。
- 完整前缀之后的独立 tab 不自动展开：缩进代码的此阶段方言要求四个 ASCII 空格，脚注续行现已支持[四列及部分 tab 终点](#脚注续行的部分-tab-终点)；仅下述新叶块子项允许前缀内部分 tab 的余量参与标题、围栏和代码缩进分类。定义起始行的部分 tab 扩展见[链接定义](#引用链接定义的-tab-缩进)及[脚注定义](#脚注定义起始行的-tab-缩进)；Setext 下划线见[后续子项](#setext-下划线的-tab-缩进)。正文里的 tab 保留原文。完整前缀后的既有块、表格分隔行探测、内联对象、tight/loose、折叠与源码映射均使用相同容器身份。

```rust
use mdstream::{BlockKind, Document};
let mut doc = Document::new();
doc.append("-\t```\n\t中🙂\n")?;
let item = doc.blocks()[0].container;
doc.append("\t```\n-\t# sibling\n")?;
doc.finish();
assert!(matches!(doc.blocks()[0].kind, BlockKind::Code { .. }));
assert_eq!(doc.blocks()[0].container, item);
assert_ne!(doc.blocks()[1].container, item);
assert_eq!(doc.container_count(), 3); // one list, two items
# Ok::<(), Box<dyn std::error::Error>>(())
```

仍按换行、EOF 或既有 1024 字节前缀上限提交；上限按字节计，不按展开列计。截断的空白/标记保持原有提交和失败规则，不因新字节到来回扫历史前缀。匹配器只增加常数大小的列号和空白末端摘要；相邻列表层复用同一空白串扫描，每个消费字节至多再访问一次。首次失配只做一次有界 lazy 判定，不扫描余下祖先；长物理行的后续追加不调用前缀匹配。累计解析 O(n log n)、空间 O(n) 和共享预算契约保持，无单次 append 的硬时延承诺。

`crates/mdstream-core/tests/container_tabs.rs` 覆盖 3072 组独立列宽/字节边界模型、嵌套与失败前缀、既有块/表格/脚注/数学规则、映射预算、稳定身份和合并通知；全局分块快照覆盖 UTF-8 全切点/逐前缀及分类边界。250 层混合缩进、千层/百万字增长的计数测试验证没有祖先或正文回扫。`cargo run -p mdstream-core -r --example container_tabs` 演示 tab 前缀去除、有预算数学读取及折叠意愿保留。正文、数学续行及所选新叶块的部分 tab 见下文；叶块内部 tab 缩进和分类上限扩展继续保留 TODO。

### 围栏代码续行的部分 tab

已开启的容器内反引号/波浪线围栏允许部分消费前缀 tab。按物理行四列对齐：每层 Item 先消费所需列数，同一个 tab 的剩余列可继续供子 Item 或引用标记前的缩进使用；`>` 后可选分隔只消费一列。整条已有路径匹配成功后才提交，最末 tab 留下的 1..3 列先作为虚拟空格参与[围栏正文去缩进](#围栏正文的有界空格去缩进)，剩余列才显示。例如 `- ```\n\tpayload` 的 tab 用两列续接 Item，另外两列显示在 payload 前；`- - ```\n\tpayload` 的四列全部用于两层 Item，不产生显示空格。对于保留为正文的行，路径之后的 tab 仅在围栏正文去缩进仍有额度时按列消费，其余 tab 保留字面，不自动展开整个代码正文；closer 的列宽判断见[围栏 closer 的 tab 缩进](#围栏-closer-的-tab-缩进)。

余下虚拟列参与关闭围栏的至多三列缩进判定：两列 Item 之后的 tab 还剩两列，再加一个 ASCII 空格和合法关闭标记可关闭；再加两个空格则共有四列，仍为字面代码行。定界长度、尾随空白、CRLF/EOF、1 KiB 分类窗口与超长候选规则沿用原方言。空行同样先按 opener 缩进去掉至多 N 列，再保留余下空格。新围栏和缩进代码的部分 tab 分类见下述独立子项；一般正文和数学续行也见下文。任一容器层失配时整个探测不生效，仍由原匹配器处理兄弟和作用域边界；例如 `- > ```\n\tpayload` 不会把未匹配的引用保留下来，也不发布替代空格。

`source()`、`runs()` 和块范围保留原字节；`display_runs[_in]` / `display_cursor` 把最末部分 tab 的一个源码字节映射到静态空格。`DisplayRun.source` 和 `replacement` 都是该 tab 的完整单字节区间，只有包含该字节的 tile/范围发出替代文本，其样式仍为 CODE。隐藏的前缀与关闭行不会变成显示空格。显示搜索和有界摘要复用这个映射：两个空格命中可指向同一个 tab，摘要截断在某个空格后仍保留完整 tab 映射。共享 Budget 在读取前收费，耗尽时不移动游标；一个替换最多输出三个字节，已有六字节、一 query、一 object 的基本预算足以推进。原始 `RenderRun` 排版消费者需要主动使用显示接口取得展开结果。

```rust
use mdstream::{Budget, DisplayStep, Document, Span, Style, WorkLimit};
let mut doc = Document::new();
doc.append("- ```\n\t中🙂\n\t```\n")?;
doc.finish();
assert_eq!(doc.blocks().len(), 1);
let tab = doc.source().find('\t').unwrap();
let mut cursor = doc.display_cursor(Span::new(tab, tab + 1)).unwrap();
let mut budget = Budget::new(WorkLimit {
    bytes: 3, queries: 1, objects: 1, ..Default::default()
});
let DisplayStep::Run(run) = cursor.next(&doc, &mut budget)? else { panic!() };
assert_eq!(run.text, "  ");
assert_eq!(run.replacement, Some(Span::new(tab, tab + 1)));
assert_eq!(doc.text(run.source), Some("\t"));
assert!(run.style.contains(Style::CODE));
# Ok::<(), Box<dyn std::error::Error>>(())
```

探测只保存当前字节、逻辑列和至多三列余量，复用空白末端缓存；每个保留的层消耗正列数，一个 tab 最多承担四列，不扫描无输入支撑的全部祖先。失败最多重试一次原有有界前缀；成功每个含余量的正文行只追加一条固定大小显示记录，无正文复制、无按列分配。已分类长行后续追加不再调用匹配器。累计 O(n log n)、空间 O(n) 保持，不承诺单次解析硬时延。

`crates/mdstream-core/tests/partial_tabs.rs` 和分块快照覆盖 400 组嵌套范围模型、UTF-8 全切点/逐前缀、关闭失败/路径回退、预算与搜索映射、百万字折叠和 4096 个历史替换；`crates/mdstream-core/tests/container_tabs.rs` 的 3072 组独立列宽模型已包含部分 tab。`cargo run -p mdstream-core -r --example partial_tabs` 演示小预算读取、源码映射和流式折叠意愿保持。

### 一般正文续行的部分 tab

同一确切容器中已有未结束的行内正文时，可以用部分 tab 显式续接整条路径。路径使用与围栏代码相同的物理行四列对齐和原子匹配：多层 Item 可共享一个 tab，引用分隔只消费一列。仅当实际发生部分消费、剩余水平缩进（含虚拟列）不超过三列、余下非空正文按当前有界分类器属于普通文字、且没有新容器标记时生效。保留物理叶块和 ContainerId，不建立新容器；例如 `- first\n\tsecond` 显示 `first\n  second`，而 `- - first\n\tsecond` 的 tab 全部用于两层路径，second 前没有显示空格。

此项只扩展已有正文续行；下述新叶块子项另处理 ATX/围栏及虚拟列加 ASCII 空格的四列代码缩进。空白行、定义候选、未匹配完整路径和待确认表头后的普通正文，继续原分类/lazy 方言；Setext 候选见[后续子项](#setext-下划线的-tab-缩进)。部分 tab 正文中的管道保持普通行内内容，不作为新表头；后续使用完整前缀的合法 Setext 可把整段迟确认为标题。新叶块的剩余类型和更完整缩进语义仍保留 TODO；数学内容扩展见下节。分类依然在 LF、EOF 或 1 KiB 前缀处提交，长行追加不重新分类；尚未提交的短行暂时显示源码。

余下 1..3 列在行内状态机中作为空格消费：参与强调 flanking、软/硬换行确认、代码 span 空白规范化和链接/图片标签。标签匹配使用去前缀、包含虚拟空格的有界缓冲，原始标签范围和对象 ID 保持源码语义。已开启代码 span 晚闭合时，空格映射与代码规范化区间组合；如果需要裁掉末端一个空格，只减少一列，tab 的其他列仍显示。未闭合代码保持已有失败方言与已经确认的前缀映射，不回放正文。同一行中新开的数学对象位于前缀之后，沿用原映射；跨行数学前缀也支持部分 tab，读取语义见下节。

```rust
use mdstream::{Document, Span, Style};
let mut doc = Document::new();
doc.append("- ` a\n\t`\n")?;
doc.finish();
let text: String = doc.display_runs_in(Span::new(0, doc.source().len())).unwrap()
    .filter(|run| !run.style.contains(Style::HIDDEN))
    .map(|run| run.text).collect();
assert_eq!(text, "a  \n"); // newline becomes one space; the tab retains one after trimming
let tab = doc.source().find('\t').unwrap();
let run = doc.display_runs_in(Span::new(tab, tab + 1)).unwrap().next().unwrap();
assert_eq!(run.text, " ");
assert_eq!(run.replacement, Some(Span::new(tab, tab + 1)));
# Ok::<(), Box<dyn std::error::Error>>(())
```

显示索引为部分 tab 单独保存有序的单字节区间/列数，可以被之后闭合的代码区间包含。代码确认只追加一个区间并检查末端，不改写历史 tab；窗口读取分别定位两类记录，合并当前交集，不回扫正文。替代输出最多三个字节，源码/tile/逐标量读取、预算游标、显示搜索与有界摘要共用同一映射；多个空格命中可以指向同一个 tab，裁成零列的 tab 也照常计费。保守 revision 失效、范围通知、合并 Update、折叠/预览和对象身份沿用现有接口。

正文探测先检查实际前缀中是否含 tab，不含则不重复走活动栈；含 tab 的原子探测失败时最多再执行一次原匹配器。成功的 250 层两列 Item 路径每行访问 250 帧；完整四列 Item 的 tab 路径当前先探测再走原分类，每行访问两遍已提供的帧。缺失深层前缀时在失配处停止，不遍历余下祖先；百万字长行后续追加只消费新增正文。每行最多一个空格记录，闭合代码不重写旧记录；累计 O(n log n)/空间 O(n) 保持。`Work.inline_bytes` 包含每个部分 tab 至多三次虚拟空格访问，解析不承诺单次硬预算。

`crates/mdstream-core/tests/prose_tabs.rs`、显示索引内部测试及分块快照覆盖 400 组显式空格模型、UTF-8 全切点/逐前缀、作用域/失败/分类上限、千层/百万字、4096 个历史替换、旧任务失效和预算搜索/折叠。`cargo run -p mdstream-core -r --example prose_tabs` 演示代码晚闭合、源码映射、预算读取与独立折叠意愿。

### 数学内容续行的部分 tab

同作用域的行内 `$…$` / `$$…$$` 候选现在接受上述部分 tab 正文续行：完整路径原子匹配，余下总缩进至多三列，非空普通文字才能延续。空白行、过量缩进、新块/表头和不完整路径仍按原分类/lazy 方言处理，不因数学候选而屏蔽块边界。虚拟空格送入原定界/转义状态机，原始源码、MathId 和物理叶块身份保持。未闭合或块中断不回放内容，已提交的前缀显示映射保留；后续 Setext 仍可提升整段。

已开启的独立数学块使用与代码围栏相同的完整路径探测，内容允许空白和任意缩进。最终余量 1..3 列成为数学内容空格；路径之外的 tab 保留字面。关闭 `$$` 前的虚拟列与 space/tab 列宽合计至多三列（见[定界行 tab 缩进](#独立数学块定界行的-tab-缩进)）：`- $$\n\t $$` 可关闭，`- $$\n\t  $$` 的四列则是内容。关闭行整体省略，不产生 tab 替换记录；错定界、长分类候选、EOF 未闭合及作用域退出沿用独立数学块规则。新数学块起始行的部分 tab 分类见下节。

`math_content` / `math_block_content` 返回原文。`math_payload` / `math_block_payload` 逐片合并前缀间隙与已有显示 tab 记录，省略完整前缀、展开保留的部分 tab，其余 CRLF、反斜线、实体和正文 tab 原样返回。`MathChunk.source` 连续覆盖原内容，替代空格对应原 tab 的一个字节；同一 tab 不拆成多个源码片段。每步计实际源码字节及一 query/一 object，一个字节的正预算可原子输出最多三个空格，预算不足不推进也不收费。该接口按源码计费，调用方若另有限制输出字节需计入至多三倍的替换量；不解析 TeX 或应用代码/实体显示规范化。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("- $a\n\t中$\n  $$\n\t🙂\n\t$$\n")?;
doc.finish();
assert_eq!(doc.text(doc.math(MathId(0)).unwrap().content), Some("a\n\t中"));
let mut inline = doc.math_payload(MathId(0)).unwrap();
let mut block = doc.math_block_payload(BlockId(2)).unwrap();
let mut output = [String::new(), String::new()];
loop {
    let mut budget = Budget::new(WorkLimit {
        bytes: 4, queries: 1, objects: 1, ..Default::default()
    });
    let Some(chunk) = inline.advance(&mut budget).unwrap() else { break };
    output[0].push_str(chunk.text);
}
let mut budget = Budget::new(WorkLimit::unlimited());
while let Some(chunk) = block.advance(&mut budget).unwrap() { output[1].push_str(chunk.text); }
assert_eq!(output, ["a\n  中", "  🙂\n"]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

游标 O(1) 构造、固定十个机器字以内；首次付费读取以 O(log G + log T) 定位间隙和 tab，后续片段 O(1)。显示映射每行最多一条，数学读取直接借用，不维护第二份 tab 数组。数学候选只更新最后消费位置，省略前缀时追加间隙；晚闭合不改写旧映射或回扫正文。百万字同一物理行的后续追加不重扫前缀/祖先、不新增映射。数学块继续做 O(log n) 范围失效，逐字追加时注解树可按新端点线性增长；累计 O(n log n)/空间 O(n) 保持，不承诺单次解析硬预算。

`crates/mdstream-core/tests/math_tabs.rs`、内部存储及分块测试覆盖行内/块数学各 400 组嵌套列宽模型、UTF-8 全切点/逐前缀、定界失败与作用域、CRLF/EOF/分类上限、千层/百万字和 4096 条历史映射。预算读取、搜索/摘要的单字节映射、旧任务/测量拒绝、合并通知和折叠/预览/导航均有回归；折叠正文零字节读取。`cargo run -p mdstream-core -r --example math_tabs` 演示共享预算读取和流式闭合后的独立折叠意愿。

### 容器所选新叶块的部分 tab

已有容器路径可以用部分 tab 开始 ATX 标题、反引号/波浪线代码围栏、独占行 `$$` 数学块，或缩进代码。整条旧路径先按物理行四列 tab 原子匹配，保留确切 ContainerId；只有匹配成功且分类器确认上述类型时才提交。此前是正文、标题、代码、定义或表格都可进入这个子项；失败不隐藏半条路径或发布虚拟空格，仍走原方言。新建容器的首行不在此扩展内。

分类优先级与源码规则：

- 未消费的虚拟列加后续 ASCII 空格共 0..3 列时，按既有 ATX/围栏规则识别。例如 `- old\n\t# title` 在同一条目内建立标题，`- old\n\t```rs` 开始代码块。旧行内候选在块边界结束；标题正文的第一个 UTF-8 标量仍参与正常行内解析，隐藏的 tab 不送入新解析器。
- 虚拟列加 ASCII 空格达到四列时优先识别缩进代码，移除四列。例如 `- old\n\t  # literal` 显示字面 `# literal`；多余的真实空格保留。四列包含至多三个虚拟列，其余可用原始字节偏移准确定位，不需要合成正文。相邻同作用域代码行沿用既有合并规则，显式空行封口。
- 上述新叶块中断待确认表头或已确认表格；四列缩进的 `-|-` 是代码，不确认表格分隔行，也不启动长分隔行探测。代码/数学围栏开启后仍先按既有字面内容和关闭规则处理，其内部 `#`、管道或定义不再创建叶块。
- 定义、表头、普通新段落和空白行的部分 tab 扩展仍未交付；Setext 下划线另见[后续子项](#setext-下划线的-tab-缩进)；完整路径上的新引用另见[新引用子项](#部分-tab-路径新引用)，部分 tab 路径的新列表另见[独立子项](#部分-tab-路径新嵌套列表)。ATX 标题和代码围栏的额外 tab 现按物理列宽计量，详见文末；例如 `- old\n\t\t# literal` 超过三列，继续原方言。标题后若要开始普通段落，应使用完整前缀（如两个空格），不能假定已开启的正文续行规则适用于新段落。

所有 Block/Heading/代码 info/数学 content 仍指向原始 UTF-8 源码。标题标记和代码缩进消耗的 tab 整字节隐藏，不建立替代记录；围栏内容中的部分 tab 先按 opener 缩进扣除虚拟列，余量仍复用前述单字节到 1..3 个空格的映射。预算显示读取、搜索、摘要、数学内容读取与折叠导航复用原接口，旧 revision 任务和测量仍被拒绝。

```rust
use mdstream::{BlockId, BlockKind, Document, Span, Style};
let mut doc = Document::new();
doc.append("- old\n\t# 中🙂\n\t  # raw\n\t```rs\n\tcode\n\t```\n")?;
doc.finish();
let heading = doc.headings().entries()[0];
assert_eq!(heading.block, BlockId(1));
assert_eq!(heading.container, doc.blocks()[0].container);
assert_eq!(doc.text(heading.content), Some("中🙂"));
assert_eq!(doc.blocks()[2].kind, BlockKind::Code { info: None });
let text: String = doc.display_runs_in(Span::new(0, doc.source().len())).unwrap()
    .filter(|r| !r.style.contains(Style::HIDDEN)).map(|r| r.text).collect();
assert_eq!(text, "old\n中🙂\n# raw\ncode\n");
# Ok::<(), Box<dyn std::error::Error>>(())
```

前缀探测只检查当前有界窗口，不含 tab 时不重走活动栈；这会为非正文行增加一次实际前缀字节扫描。含 tab 时至多一次原子路径探测和一次原分类回退，每个 tab 最多供四列，空白末端缓存避免按深度重复扫描。分类不分配展开字符串、不复制候选或回放旧正文；LF/EOF/含前缀的 1 KiB 提交时机不变。长行后续追加直接使用既有 StreamingLine，累计解析 O(n log n)、空间 O(n) 保持，无单次解析硬预算承诺。

`crates/mdstream-core/tests/leaf_tabs.rs` 与全局分块快照覆盖 576 组列阈值、400 组嵌套空格模型、UTF-8 全切点/逐前缀、CRLF/EOF、长失败候选、表格优先级和身份/失效。千层引用及 251 层列表的百万字长行再追加 1000 次，前缀/祖先/分类计数和容器容量保持不变；4096 个历史标题后仍只访问当前路径。共享预算显示、搜索、摘要及折叠/预览/导航通过，收起正文零字节读取。`cargo run -p mdstream-core -r --example leaf_tabs` 演示新叶块源码范围和预算消费。

## 列表 tight/loose

`Container.list_tightness` 在 List 节点上为 `Some(ListTightness::Tight | Loose)`，Quote/Item 为 `None`。新列表从 Tight 开始：Growing 时 Tight 是当前判断，可在后续内容到来时变成 Loose；Sealed 时 Tight 最终确定。Loose 单调成立，无论列表是否仍开放都不会撤回。所有条目共享父列表属性，叶块不复制该状态；O(1) 读取父列表，深层查找则使用已有的 `container_ancestors`，每步计 1 query / 1 object。

本阶段在既有显式容器方言上定义空行分隔，尚不声称完整 CommonMark tight/loose：

- 完整物理行去掉已匹配容器前缀后全为 ASCII 空白时，记作待确认分隔。完整前缀先记录确切作用域的尾部状态；省略列表缩进时保留活动列表，下一条已分类非空行决定恢复的作用域（见下节）。`- a\n\n- b\n` 是一个 Loose 列表，两个条目共享原 List 身份。
- 同一条目已有直接内容后，空行后再出现直接内容或新直接子容器，父列表变 Loose；同一列表的兄弟条目若跟在待定空行后，也确认 Loose，包括空条目。相邻正文物理行、lazy 正文、无空行的新子容器/兄弟本身不确认 Loose。编号变化不换列表，bullet/有序分隔符变化则结束旧列表。
- 内容前的空行不单独确认 Loose；只有列表/任务标记的首行不算分隔空行。末尾空行若随后 EOF、作用域退出或列表标记类型变化，保持 Tight。定义、标题、表格、代码和新嵌套 Quote/List 都属于直接内容；失败候选按既有正文规则处理。
- 嵌套列表中的空行若被内层后续内容消耗，不继续传播；若一直位于列表尾部，则随关闭交给实际恢复的祖先作用域，Quote 边界截断传播。例 `- a\n  - b\n    \n    c\n- d\n` 只有内层 Loose，去掉 `c` 行则只有外层 Loose。规则与计费见[嵌套末尾空行](#列表嵌套末尾空行)。
- 完整匹配前缀的围栏代码内空白是代码内容，不是分隔；有界空行可省略列表缩进并保留为代码正文，前提是未缺失引用标记（见[围栏空行](#列表内代码围栏跨省略缩进空行)）；缩进代码沿用空白行终止规则，随后新直接内容可确认 Loose。分类仍受 1024 字节窗口限制；没有在窗口内结束的全空白行按既有代码/正文分类提交，稍后的 LF 不重新判定。短尾部等 LF/EOF，长行扩展不重读旧内容。

```rust
use mdstream::{ContainerId, Document, ListTightness, SyntaxState};
let mut doc = Document::new();
doc.append("- first\n  \n")?;
let list = ContainerId(0);
assert_eq!(doc.container(list).unwrap().list_tightness, Some(ListTightness::Tight));
let update = doc.append("- second\n")?;
assert_eq!(update.loosened_lists, [list]);
assert_eq!(doc.container(list).unwrap().list_tightness, Some(ListTightness::Loose));
assert!(doc.finish().loosened_lists.is_empty());
assert_eq!(doc.container(list).unwrap().state, SyntaxState::Sealed);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Update.loosened_lists` 精确报告 Tight → Loose，每个列表整个生命周期仅一次，也包含同次追加中新建的列表。`new_containers`、`closed_containers` 和共享 `container_end` 继续负责身份、最终性及范围；重建容器快照时还须读取 `loosened_lists`。`merge` 移动新事件，`append_into/finish_into` 复用该 Vec；空追加、重复 finish 不产生事件。尚未分类的尾部不改变元数据。

确认 Loose 时对列表已有源码包络只做一次 O(log n) 的 `dirty` / `ChangeKind::Structure` 和语法版本区间标记，保留原样式与链接；旧 TileKey、DependencyKey、测量及检查点按既有协议失效，屏外高度保留估计，不枚举旧条目、叶块或 tile。后续新正文仅按普通追加失效，Loose 列表不会反复全范围失效。Growing/Sealed 仍是独立对象状态，不能凭 TileKey 缓存容器最终性。折叠意愿与预算消费保持；本单元不增加段落间距或终端绘制。

每个活动列表帧保存“已有直接内容/待定空行”两个布尔摘要，只检查当前直接条目或正在切换的兄弟。嵌套帧开闭成本按已有前缀/事件计费，不为短 lazy 行或长行追加遍历祖先。完整前缀的间距处理为 O(n)，每列表至多一次 O(log n) 区间失效；省略缩进空行另用下节的有序引用深度索引，全库仍为累计 O(n log n)、空间 O(n)。`crates/mdstream-core/tests/list_tightness.rs` 验证 UTF-8 全切点/逐前缀、400 组生成输入、千层引用、250 层列表、4096 个历史条目及百万字增长；`cargo run -p mdstream-core -r --example list_tightness` 演示晚确定通知及分帧元数据读取。

## 列表跨无前缀空行

完整空白行允许省略活动列表的部分或全部缩进，保留 List/Item 稳定身份；只保留到第一个缺失的 Quote 之前，缺失引用及其子树在空行开头封口。例如 `> - a\n>\n> - b` 保留同一列表；`> - a\n\n> - b` 的两个引用和列表互相独立；`- > a\n\n- b` 保留外层列表并关闭内层引用。新列表/任务 marker 自身的空正文仍不是分隔空行。

保留的最内层 Item 拥有空白叶块，全部仍开放容器的共享末端继续增长。空行使行内序列、待定表头/Setext、既有表格、缩进代码结束；若围栏/数学/脚注续行缺少其显式路径，也结束旧叶块。完整匹配路径的围栏/数学字面空行、脚注内部分段仍沿用各自规则。空行后的非空正文必须重新匹配路径，不能通过旧 lazy 状态跳过缩进。所有偏移继续指向原始源码，未提供的前缀不合成空格。

省略缩进的连续空行只保存一个待确认深度下界，覆盖最后显式匹配的 Item（若有）和其后保留的列表。下一条非空行只在实际恢复的直接 Item/同型兄弟处确认分隔：已有直接内容后继续正文或新子容器使父 List Loose；同型兄弟包括空条目也确认 Loose。恢复深层条目后不会把该空行继续传播给祖先。例如 `- outer\n  - inner\n\n    more\n- sibling` 只有内层 Loose；将 `more` 行去掉而直接恢复外层兄弟，则只有外层 Loose。完整缩进的尾部空行也可在子容器关闭时传递给实际恢复的祖先，见下节；仍不声明完整 CommonMark 兼容。

未分类短尾部不确认、关闭或重建任何容器；LF、EOF 或原有分类窗口提交时才决议。若空行后直接 EOF、退出作用域或更换列表标记，仍 Tight 的列表保持 Tight，并封闭在最后已归属空行之后；不把已发布范围向前收缩。空行无需在未来回填容器 ID，旧叶块与已发布行内对象保持身份。没有在 1024 字节窗口内结束的全空白行按既有代码/正文规则提交，不在稍后的 LF 重试。

```rust
use mdstream::{ContainerId, Document, ListTightness, SyntaxState};
let mut doc = Document::new();
doc.append("- first\n")?;
let blank = doc.append("\n\n")?;
assert!(blank.closed_containers.is_empty());
assert!(blank.loosened_lists.is_empty());
assert_eq!(doc.container(ContainerId(1)).unwrap().source.end, 10);
let sibling = doc.append("- second\n")?;
assert_eq!(sibling.new_containers, 2..3); // 只新建 Item
assert_eq!(sibling.loosened_lists, [ContainerId(0)]);
assert_eq!(doc.container(ContainerId(0)).unwrap().list_tightness, Some(ListTightness::Loose));
assert!(doc.finish().loosened_lists.is_empty());
assert_eq!(doc.container(ContainerId(0)).unwrap().state, SyntaxState::Sealed);
# Ok::<(), Box<dyn std::error::Error>>(())
```

空行只匹配实际提供的前缀，以 O(log(Q + 1)) 查询有序活动 Quote 深度索引来定位必须关闭的后缀；Q 是活动引用数，不枚举保留的祖先。被关闭的节点各处理一次；深列表上的无缩进空行每行仅一次活动帧检查，待确认区间更新为 O(1)。索引随引用开闭维护，容量计入 `container_arena_stats`，比较次数计入 `container_steps`；额外空间 O(depth)，累计解析仍 O(n log n)。长行提交后的继续追加不再查询前缀或索引。折叠和章节使用共享末端，无空行逐祖先索引写入。

`crates/mdstream-core/tests/list_blanks.rs`、全局分块快照与折叠模型覆盖 400 组间距/身份/范围、240 轮流式高度、UTF-8 全切点/逐前缀、CRLF/EOF、分类窗口、深层退出、旧缓存与测量拒绝、合并通知和共享预算。250 层列表/4096 空行、近千层引用、4096 历史条目及百万字验收无历史扫描；折叠正文零字节读取。`cargo run -p mdstream-core -r --example list_blanks` 演示内外层分别晚确定、身份保持和有预算祖先读取。

## 列表嵌套末尾空行

已分类的完整空白行可作为最后一个子容器的待定尾部分隔，沿已有 List/Item 关闭路径传给实际恢复的作用域，遇到 Quote 关闭则清除。只改变确认分隔的列表，不将路径上的所有列表设为 Loose：

- 恢复祖先 Item 的直接正文、新叶块或新子容器时，已有直接内容的父列表确认 Loose；同型兄弟（含空条目）也确认。`- - child\n    \n- sibling` 只有外层 Loose，`- - child\n    \n  - sibling` 只有内层 Loose。
- 引用内部末尾空行不传到引用外部，例如 `- > quote\n  >\n- sibling` 保持 Tight。缺失引用仍按既有规则关闭；已经退出的列表不会因另一个作用域的新内容变 Loose。末尾空行的已发布范围和叶块归属不收缩、不回填。
- 内层后续正文、同型兄弟或新子容器会消耗原分隔，之后退出不再传播；连续空行只保留一个尾部摘要。新 Item/Quote 的 marker-only 首行不算分隔，稍后的空行可以；根作用域退出清空摘要，不污染之后的新容器。
- 关闭本身不确认 Loose。尾部空行后直接 EOF 或退出整个列表，仍 Tight 的列表保持 Tight；列表类型变化结束旧列表，但在保留的父 Item 下打开另一子列表可确认父列表 Loose。短尾部在 LF/EOF 分类：若 EOF 先完成一条同型兄弟或直接正文，仍会按该行确认。
- 完整路径的围栏代码/数学内部空白是字面内容，不传递；关闭定界之后的普通空行可传递。脚注内部分段不传递，定义因下一行封口时可交出末尾空行。缩进代码空行、分类窗口及 tab 边界沿用既有规则；超过 1 KiB 窗口的空白不在晚来的 LF 重新分类。

```rust
use mdstream::{ContainerId, Document, ListTightness, SyntaxState};
let mut doc = Document::new();
doc.append("- outer\n  - child\n    \n")?;
assert_eq!(doc.container(ContainerId(0)).unwrap().list_tightness, Some(ListTightness::Tight));
let update = doc.append("- sibling\n")?;
assert_eq!(update.loosened_lists, [ContainerId(0)]);
let child = doc.container(ContainerId(2)).unwrap();
assert_eq!(child.state, SyntaxState::Sealed);
assert_eq!(child.list_tightness, Some(ListTightness::Tight));
assert!(doc.finish().loosened_lists.is_empty());
# Ok::<(), Box<dyn std::error::Error>>(())
```

解析器增加一个布尔尾部摘要；每次正文分类 O(1) 更新，长行后续追加无需处理。一次实际后缀关闭可传递分隔一次，关闭成本仍按实际关闭节点计入 `container_steps`，传递给保留 Item 计 1 个 `list_steps`；同型兄弟必须先关闭旧子树、再读取旧 Item 的分隔状态。没有逐行祖先写入、历史子块遍历或新增深度容器；既有每列表一次区间失效、通知、缓存与预算接口不变，累计 O(n log n)、空间 O(n)。64/128/250 层列表各 1000 条完整空行仅 1000 个间距步骤；恢复最外层兄弟均只需 4 个 `list_steps`，其余工作为实际关闭。千层引用、百万字追加和 4096 个历史条目验收同样无历史回扫。

`crates/mdstream-core/tests/list_tail_blanks.rs`、全局分块快照和折叠测试覆盖 400 组深度/恢复作用域与源码范围模型、240 轮折叠/预览/高度、UTF-8 全切点/逐前缀、CRLF/EOF、字面块与脚注、通知重建/合并、旧缓存/测量/检查点/任务失效以及共享预算。折叠百万字正文零读取。`cargo run -p mdstream-core -r --example list_tail_blanks` 演示内层保持 Tight、外层迟确认、稳定范围及分帧祖先读取。实际段落间距、完整缩进/块确认规则与终端 UI 继续保留 TODO。

## 容器内行内续行

同一确切 `ContainerId` 中连续的非空正文行共享一个行内状态机，支持跨物理行的 `*`/`_` 强调、`**`/`__` 加粗、`~~` 删除线、等长反引号代码 span、链接/图片显示标签及软/硬换行。可使用显式续行、等价前缀（如 `> >` 和 `>>`）或下节定义的 lazy 续行，判断依据是容器身份。输入只消费去前缀后的正文和原始 CRLF；前缀继续隐藏并保留源码映射，不复制或拼接一个虚拟段落字符串。

- 每个正文物理行仍有自己的 `BlockId`、范围和最内层容器；换行后叶块范围 Sealed 不代表行内样式已全部确定。稍后的 closer 可给多个旧叶块应用范围样式和 `dirty` / `Change` 失效，不改它们的身份或逐个生成块变更。`pending()` 仍只表示尚未分类的当前行前缀。
- 进入子容器、退出到父级/根、切换兄弟条目或列表类型、无前缀空行及显式空白正文终止旧序列；之后回到同一个外层容器也不恢复旧 opener。已识别 ATX、围栏/缩进代码、链接/脚注定义和确认的表格同样中断序列，代码 span 也不屏蔽块分类。已识别的容器内分隔线同样中断序列；超出分类窗口或失败的候选继续沿用正文方言。
- LF 在下一条同序列正文实际交给行内解析器时才成为 SOFT_BREAK 或 HARD_BREAK；未分类尾部、未决表头不提前确认。行尾两个以上空格或未转义反斜线产生硬换行，CRLF 只隐藏 CR。序列终止时保留末端原文，不把它标为行内换行；代码 span 内的换行由代码显示规则处理。
- 早管道表头候选保留前序行内状态但不提前消费候选正文；失败/EOF 时一次性消费，确认时丢弃前序状态并按独立单元格解析。新行内序列、只继承待确认换行、尚未闭合旧强调或可隔离的已闭合强调的晚管道表头复用已消费的首单元格前缀，只延迟首管道起的后缀；继承未闭合代码 span 或数学的前缀保持原候选状态；可隔离的代码闭合暂存端点，确认后在单元格扫描中解析正文。未进入根段落后缀延迟模式的无管道普通长行即时消费正文；`Deferred(key)` 候选行在结构归属判定后单次消费。其它继承状态的晚管道长行与非法定义继续普通正文。长分隔行采用下方单遍探测及一次回退规则，不在长行尾部重试块分类。
- **Setext 的晚确认保持单调语义**：同作用域连续正文整体成为标题，保留已确认的跨行样式、换行及对象 ID，下划线终止未闭合状态。例如 `> *first\n> last*\n> ---\n` 的标题显示为 `first\nlast`，已有强调继续有效。物理叶块保持身份，标题保存跨叶块范围，不重扫旧正文；需要撤销已确认语义的更完整块规则仍另行设计。
- 代码 span 的前缀不参与 all-spaces 判定。规范化照常把内容换行显示为空格，并在两端为空白、内容不全为空白时各去一个规范化空格；末端记录最后一个实际正文 byte/CRLF，所以 closer 前的隐藏前缀不会被错当正文。闭合仅检查端点并建立一个源码包络映射，不回扫内部行。
- 链接/图片显示标签支持同作用域的跨物理行正文，源码包络和去前缀消费见[容器跨行链接与图片标签](#容器跨行链接与图片标签)。数学候选也可跨同一作用域正文续行，去前缀内容与源码映射见[容器跨行数学内容](#容器跨行数学内容)；失败候选保持字面/opaque 行为，不重新解释内部标记。链接目标保持单物理行；直接 title 正文仅在实际根普通段落允许 LF/CRLF，见[根 title 正文](#根段落直接链接与图片的多行-title-正文)；[根段落后缀的组件间空白](#根段落行内链接后缀的组件换行)可跨行，容器普通段落的组件 gap 见[容器后缀组件换行](#容器普通段落后缀的组件换行)，full 引用后缀仍限单物理行。脚注、自动链接、实体及 EOF 确认沿用原有规则；更完整的块边界撤销仍待实现。

```rust
use mdstream::{Document, Span, Style};
let mut doc = Document::new();
doc.append("> - **one\n")?;
let first = doc.blocks()[0].clone();
doc.append(">   two**\n")?;
doc.finish();
assert_eq!(doc.blocks()[0], first); // Physical leaf identity/range is unchanged.
assert_eq!(doc.blocks().len(), 2);
let runs: Vec<_> = doc.display_runs_in(Span::new(0, doc.source().len())).unwrap().collect();
assert!(runs.iter().any(|r| r.text == "one" && r.style.contains(Style::STRONG)));
let visible: String = runs.iter().filter(|r| !r.style.contains(Style::HIDDEN)).map(|r| r.text).collect();
assert_eq!(visible, "one\ntwo\n");
# Ok::<(), Box<dyn std::error::Error>>(())
```

强调/代码续行只增加一个容器 ID 和代码末端区间；链接标签另使用下节的有界引用缓冲。同一序列复用原分隔符栈，每个 opener 仍只入栈/出栈一次，不保存逐行或逐祖先状态快照。块创建和作用域比较 O(1)，前缀匹配只消费本行前缀；样式闭合用 O(log n) 范围标记。百万字续行追加不增加分类或祖先操作，EOF 只丢弃尚存 opener；累计解析 O(n log n)、空间 O(n) 和零运行时依赖保持。解析与批量 EOF 是摊还保证，不承诺任意大 append 的硬预算。

显示游标、摘要、缓存与折叠继续共用既有 Budget 和 revision 契约；隐藏前缀及 CRLF lookaround 计入读取预算，晚闭合使旧测量/检查点失效但保留折叠意愿。`cargo run -p mdstream-core -r --example container_inline` 演示流式闭合、代码去前缀显示和分帧读取。`crates/mdstream-core/tests/container_inline.rs` 及分块测试覆盖边界、失败候选、UTF-8 全切点/逐前缀、千层/百万字、250 层列表、4096 个未闭合 opener、400 组独立显示/样式模型和共享预算。

## 容器跨行链接与图片标签

同一确切容器的连续正文可包含跨物理行 `[label](url)`、`![alt](url)`、shortcut `[label]`、collapsed `[label][]` 及 full `[label][key]`。显式、等价前缀和 lazy 续行共享候选；`!` 必须紧邻开括号，新的 `[` 替换旧候选，`<` 丢弃旧候选，代码/数学中的 `]` 不关闭标签。这是既有单候选方言的扩展，不新增嵌套链接解析。目标、title 和 full 引用的后缀 `[key]` 继续限单物理行。

`LinkInfo.label` 是原始源码包络，包含中间 LF/CRLF 和容器前缀；`Document::text(label)` 因此仍返回这些原始字节。显示/alt 使用 `display_runs_in(label)`，帧内消费则使用 `display_cursor(label)`，忽略 `Style::HIDDEN` 的片段。可见片段保留绝对源码区间、实体/代码替换映射及同一个 LinkId；前缀仍可按源码定位，整段 label 不复制成另一个字符串。显示搜索复用同一片段流，跨行命中的源码包络可包含隐藏前缀；源码搜索保持原始字节。

引用匹配使用去前缀的**原始正文**，不使用去标记/实体解码后的显示文本：例如 `[a&amp;\n> b][]` 匹配定义 `[a&amp; b]: url`。沿用 Unicode 小写和空白折叠，不新增转义解码。999 字节上限按实际喂给行内解析器的正文计算，含原始 CRLF、标记和转义，不含容器前缀；超长标签仍可成为任意长显示标签的直接链接或使用独立短 `[key]`。空/全空白标签不能作为引用，但直接图片仍可发布对象。

每个 Inline 只保留一个可复用、最多 1000 字节的候选缓冲（999 字节加潜在闭括号），溢出后只保留失败标记，继续消费正文而不保存长标签；Vec 容量按有界增长保留。缓冲只在新增字节到来时填充，候选被替换时清空，关闭时规范化至多 999 字节。collapsed 后缀复用已生成的 SymbolId，不重新读取源码包络。新对象仍通过 `new_links` 发布，暂定引用补全直接目标保持 LinkId 并通过 `changed_links` 通知；晚定义继续符号间接寻址，不遍历旧引用。累计解析 O(n log n)、核心空间 O(n)，不增加按行/祖先复制的映射数组。

块/容器切换和空行沿用上节的序列终止规则；未闭合标签保留已解析的显示语义，不回放正文。失败目标/引用后缀仍可留下未解析到目标的 shortcut 对象，保持原方言。表头确认隔离单元格并丢弃旧标签；失败/EOF 恢复候选且只消费一次后缀。晚管道前关闭的 shortcut 及普通字符显式后缀可隔离对象和引用副作用，其余后缀或新行内语法取消探测（见表格章节）；多物理行 Setext 晚确认保留已发布语义并结束未闭合候选。更完整的块撤销规则仍待设计。

```rust
use mdstream::{Budget, DisplayStep, Document, LinkId, Style, WorkLimit};
let mut doc = Document::new();
doc.append("> ![中 &amp;\r\n> 🙂][]\n>\n> [中 &amp; 🙂]: /image\n")?;
doc.finish();
let id = LinkId(0);
assert_eq!(doc.link_destination(id), Some("/image"));
let mut reader = doc.display_cursor(doc.link_info(id).unwrap().label).unwrap();
let mut text = String::new();
loop {
    // The caller can share this same frame budget with search/layout work.
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, bytes: 6, ..Default::default()
    });
    match reader.next(&doc, &mut budget)? {
        DisplayStep::Run(run) if !run.style.contains(Style::HIDDEN) => text.push_str(run.text),
        DisplayStep::Complete => break,
        _ => {}
    }
}
assert_eq!(text, "中 &\n🙂");
# Ok::<(), Box<dyn std::error::Error>>(())
```

显示游标保留原共享预算、原子暂停和 revision 校验；隐藏前缀及 CRLF lookaround 也计费，六字节/一 query/一 object 可推进一个 UTF-8 基本步骤。闭合只做区间标记与失效，不回扫全部叶块；旧布局/检查点和读取任务失效，折叠意愿保留，隐藏百万字标签的折叠布局零正文读取。`cargo run -p mdstream-core -r --example container_links` 演示稳定图片身份、晚定义及分帧读取；`crates/mdstream-core/tests/container_links.rs`、分块快照和缓冲容量测试覆盖映射、搜索/折叠、深嵌套、百万字与两万旧引用。

## 容器 lazy continuation

连续非空正文中允许省略引用的 `>` 或列表条目的续行缩进，包括只提供部分外层前缀。首次前缀失配后，若剩余内容属于下述正文方言，整条原活动路径保持开放，叶块继续绑定原最内层 `ContainerId`。例如 `> > first\n> second\nthird\n` 三行均属于内层引用；`- - first\n  second\n` 的第二行仍属于内层条目。已经匹配的前缀照常隐藏，剩余空格和 tab 保留为正文；不会补造标记、复制祖先链或拼接段落字符串。

- **资格**：仅在最内层有连续非空正文（包括未决表头候选）时启用。空条目/空任务、空白正文、标题、已确认表格、代码块和有效定义之后不推断 lazy 续行。兄弟条目或新子容器改变身份后不会恢复旧行内状态。代码 span 等未完成行内候选不屏蔽块中断。
- **中断**：在首次失配处，对剩余有界前缀使用既有块分类规则。空白、有效 ATX、合法长度围栏、完整分隔线、有效链接/脚注定义，以及最多三格缩进后的引用或列表标记均中断。沿用项目列表方言，任意合法编号和空条目标记都可中断，不限定编号为 1。只关闭未匹配的内层，再在保留的外层/根按已有规则解析当前行。
- **正文与失败候选**：不符合中断语法的 `#nospace`、失败围栏/定义/列表候选留在原容器，仍接受普通行内解析。缺失容器前缀时，剩余四格以上缩进也可作为正文；若路径全部显式匹配，继续使用既有缩进代码规则。纯空白分类前缀始终不作为 lazy 正文，即使长行后面还有非空内容。
- **表格与 Setext**：lazy 行只延续正文，不成为表头、表格正文行或分隔行；前一未决表头按普通正文消费一次。缺少前缀的 `===` 是正文，完整 `---` 作为分隔线中断，空 `-` 是列表中断。后续同作用域显式 Setext 下划线提升整个连续正文序列（包含 lazy 行），保留已经确认的行内语义。需要回撤旧语义的规则另行设计。
- **流式边界**：仍使用含前缀的 1024 字节分类窗口（UTF-8 最多额外 3 字节），短尾部等 LF/EOF 再判定；长行一旦分类不在后续追加或换行时重试。定义、分隔线和其它候选能否中断取决于原分类器的完整行条件。链接/图片标签可跨同作用域 lazy 正文；数学也可跨同作用域 lazy 正文，块边界与失败方言继续沿用[行内续行](#容器内行内续行)。

```rust
use mdstream::{Document, Span, Style};
let mut doc = Document::new();
doc.append("> - > **one\n")?;
let first = doc.blocks()[0].clone();
doc.append("> two**\nthree\n# Outside\n")?;
doc.finish();
assert_eq!(doc.blocks()[0], first);
assert_eq!(doc.blocks()[2].container, first.container);
assert_eq!(doc.blocks()[3].container, None);
let visible: String = doc.display_runs_in(Span::new(0, doc.source().len())).unwrap()
    .filter(|r| !r.style.contains(Style::HIDDEN)).map(|r| r.text).collect();
assert_eq!(visible, "one\ntwo\nthree\nOutside\n");
# Ok::<(), Box<dyn std::error::Error>>(())
```

每行只扫描实际提供的前缀和一个有界正文分类窗口；全部省略时只检查一次活动帧，不遍历余下深度。开放容器与章节沿用共享末端，普通 lazy 追加不产生祖先关闭/重开事件或折叠索引写入；叶块、范围失效、预算消费和 EOF 收尾沿用既有契约。`crates/mdstream-core/tests/container_lazy.rs` 与分块测试覆盖千层引用、250 层列表、4096 个短续行、百万字追加、1200 行独立范围/通知模型、共享预算和折叠回归。`cargo run -p mdstream-core -r --example container_lazy` 演示部分及全部省略前缀、晚闭合、折叠保持与分帧显示读取。此为有界项目方言；tight/loose 余项、部分 tab 的剩余方言及叶块缩进、分类上限扩展和更完整的块确认/撤销规则仍待实现。

## 容器内标题与章节作用域

显式引用和列表条目内识别 ATX h1–h6，以及连续一行或多行正文后、紧邻下一物理行的 Setext `=` / `-` 下划线。先消费显式容器前缀、列表续行缩进及任务标记，再按既有标题方言分类；相对正文最多三格缩进。标题 `source` 保留完整物理源码；ATX 的 `content` 去掉容器/标题前缀、CRLF 和受支持的 closing hashes，多行 Setext 的内容包络见下方；行内显示、实体和源码映射沿用原实现。条目即使变成标题，任务/编号仍可从 `ContainerKind::Item` 读取。

容器 Setext 确认同一 `ContainerId` 中自上次行内序列边界起的连续非空正文，支持显式、等价前缀及 lazy 续行；不跨空行、兄弟条目、实际容器切换或已识别块中断。仅缺少部分前缀的正文可能仍是最内层容器的 lazy 续行，不能据前缀长短推断作用域已退出。下划线必须显式匹配确切作用域；确认时新增一个 `HeadingId` 并隐藏整条下划线，旧正文及行内语义不重扫。`> text\n> -\n` 与 `- text\n  -\n` 中下划线优先于新建空子列表；`- text\n-\n` 的第二个 `-` 是兄弟条目。超长标题内容可以流式增长，但下划线仍须在包含容器前缀的 1 KiB 分类窗口内完整出现；ATX 的 hash run 必须在窗口内遇到空白分隔符或完整行末，否则作为正文；超长 ATX 的尾部 closing hashes 不回看。显式正文的强调、代码 span 与换行续接见[容器内行内续行](#容器内行内续行)；更完整容器规则及下划线长度上限扩展继续待办。

多行标题保留全部原有 `BlockId` 和源码有序、不重叠的叶块范围。`Heading.block` 指向首叶块，只有首块改为 `BlockKind::Heading`；最后一个叶块向后包含下划线，中间叶块完全不改写。`Heading.source` 从首物理行开始到下划线结束，可能大于首叶块范围；标题排版应依据这个范围，不能只凭各叶块的 `kind` 推断标题内容。`Heading.content` 从首正文 byte 到最后正文行的 CRLF 之前，再裁剪整段原文首尾的 space/tab（见[Setext 首尾空白](#setext-首尾空白)）；中间换行和容器前缀保留原始偏移，用显示游标去除隐藏前缀、CR 并保留软/硬换行或代码映射。

表头候选失败后仍属于同一正文序列，仅消费尚未解析的后缀；表格确认会终止序列。非法或超过分类窗口的下划线按原有正文/块规则处理，之后合法下划线可包含这些普通正文行。未完成下划线到换行或 `finish` 才确认；没有下划线则保持普通叶块。Setext 保留已发布的强调、链接、图片、数学与显示映射，结束剩余未闭合候选，不回放失败内容。

解析只保留一份固定大小的首叶块 ID 与内容区间；续行更新末端，确认修改首尾块、插入一个标题并发布一次整段范围失效。`changed_blocks` 保守扩大到首叶块（包含未修改的中间块），无逐块事件；`new_headings` 驱动折叠默认初始化、旧章节边界回退和导航。确认成本为有界前缀分类、O(log n) 范围更新及未闭合行内状态的一次性收尾，不按标题字节或物理行数回扫。收尾仍可能清理很多 opener，遵守累计 O(n log n)/空间 O(n)，不承诺单次确认硬时延。名称搜索、slug 和实际标题读取均在显式共享 Budget 中进行。

```rust
use mdstream::{BlockId, Document, HeadingId, Span, Style};
let mut doc = Document::new();
doc.append("> **first\n> last**\n")?;
let first = doc.blocks()[0].source;
let work = doc.work();
let update = doc.append("> ---\n")?;
let title = doc.headings().get(HeadingId(0)).unwrap();
assert_eq!(title.block, BlockId(0));
assert_eq!(title.source, Span::new(0, doc.source().len()));
assert_eq!(doc.blocks()[0].source, first); // 两个物理叶块仍各有身份
assert_eq!(update.changed_blocks, 0..2);
assert_eq!(doc.work().inline_bytes, work.inline_bytes);
let text: String = doc.display_runs_in(title.content).unwrap().filter(|r| !r.style.contains(Style::HIDDEN))
    .map(|r| r.text).collect();
assert_eq!(text, "first\nlast");
# Ok::<(), Box<dyn std::error::Error>>(())
```

每个 `Heading.container` 表示确切作用域：`None` 是文档根，否则是最内层 Quote/Item。`parent`、`first_child`、兄弟查询只比较同一作用域的标题级别，跳级仍取最近更浅标题，不凭容器嵌套创建标题父关系。章节包含嵌套容器；内层标题不会终止外层章节。`section(id, source_end)` 止于下一同作用域、同级或更高级标题的行首，否则止于该容器边界；根章节末尾仍使用调用方的 `source_end`。所以范围只会包含或不相交。`section_end` 只返回同作用域的后继标题 ID，返回 `None` 也可能表示已经在容器边界结束，不能单凭它判断章节是否开放。

`current(offset)` 返回当前位置最内层可用章节；当前容器尚无本地标题时使用进入容器时捕获的外层章节，退出后恢复外层。源码行前缀尚未分类时沿用最近已分类上下文，容器章节末端仍停在 `Update.container_end`；换行、lookahead 或 EOF 确认边界后更新。容器范围不包含 EOF，finish 后 EOF 位置回到根章节。全局 `entries`、`at_level`、前后/严格级别导航、名称搜索和 slug 仍按源码顺序包含全部标题；slug 在所有容器间共同消歧。

```rust
use mdstream::{Document, HeadingId};
let mut doc = Document::new();
doc.append("# root\n> ## quote\n> - # item\n> ## peer\noutside\n")?;
doc.finish();
let h = doc.headings();
assert_eq!(h.parent(HeadingId(2)), None); // Item 内章节根
assert_eq!(h.next_sibling(HeadingId(1)), Some(HeadingId(3)));
assert_eq!(h.section(HeadingId(2), doc.source().len()).unwrap().end,
           doc.source().find("> ## peer").unwrap());
assert_eq!(h.current(doc.source().find("outside").unwrap()), Some(HeadingId(0)));
# Ok::<(), Box<dyn std::error::Error>>(())
```

元数据只随新标题、作用域开闭和上下文切换增长：H 个标题、C 个容器、R 次上下文切换占 O(H + C + R) 空间，均为 O(n)。每个容器进入时从父作用域六个末尾 ID 捕获外层章节，开闭 O(1)，不复制路径；长行追加仅更新当前块/标题和共享末端。标题插入摊还 O(1)，结构查询最多六级二分，`current` 另外二分上下文边界；确认 Setext 的区间失效仍 O(log n)。容器关闭事件及 `new_headings` 改变派生章节范围，不因此给全部旧标题发送 `changed_headings` 事件；消费方应按这些索引/事件查询，不能只缓存旧范围。查询不读取标题文字，名称读取、搜索、slug 继续显式共享 Budget；解析本身仍是摊还保证，无单次调用硬预算。

顶层与容器内章节均提供独立 `FoldCandidate`，候选枚举、默认设置、批量、预览及导航覆盖全部已识别标题。内层 h1 不会截断外层章节；折叠父链保留章节与容器的交替嵌套，区别于 `HeadingIndex::parent` 的同作用域标题关系。流式尾部与事件消费见[容器内章节折叠](#容器内章节折叠)。

`cargo run -p mdstream-core -r --example container_setext` 展示多行标题迟确认与跳过百万字折叠正文的预算布局；`crates/mdstream-core/tests/container_setext.rs` 覆盖 400 组独立范围/显示模型、千层引用、250 层列表、4096 个连续叶块及历史标题。`cargo run -p mdstream-core -r --example container_headings` 展示晚确认、作用域切换及小预算 slug。测试覆盖 UTF-8 全切点/逐前缀、1200 行固定种子线性范围模型、千层嵌套、百万字 Setext 无回扫、500 个同时开放的章节作用域、共享预算搜索/slug/折叠导航。

## 容器内代码块

显式 Quote/Item 内识别围栏和缩进代码，复用 `BlockKind::Code { info }`、稳定 `BlockId` 与源码有序块索引；`Block.container` 是确切的最内层容器。块的 `source` 包含每行完整物理源码及容器前缀，显示时隐藏容器前缀，正文保留 CODE 样式、原始字符和 LF，CRLF 的 CR 隐藏。代码内的列表/引用标记、标题、表格、链接/脚注、数学、实体、强调和反斜线均为字面内容，不创建相应对象。

- 围栏 opener 在容器前缀后允许至多三列缩进（ASCII 空格加上述部分 tab 余量）、3..1023 个相同反引号或波浪线（定界串须在分类窗口内完整确认）；反引号 opener 的已分类 info 不得含反引号。`info` 是去掉两端 ASCII 空白的源码区间，不解析实体或复制正文；opener 整行隐藏。
- 围栏续行只匹配已经打开的引用/条目路径，不从正文新建子容器。例如 `> ``` ` 的后续 `> > - text` 正文是 `> - text`。同一确切容器内，完整行以至多三格缩进、至少 opener 长度的同字符组成的串及纯 ASCII 空白结尾才闭合，closer 整行隐藏。短串、另一字符、尾随非空白、四格缩进均为正文，再按下述有界空格规则去缩进；closer 判定使用去缩进前的列数。
- 每条引用行必须重复显式 `>`，列表非空行必须补足原 Item 的续行缩进。缺失某层非空前缀、兄弟条目或退出容器使旧代码止于新物理行开头，再按新作用域分类该行；外层的围栏可能因此新开一个独立块。完整匹配路径的显式空行仍属于围栏正文；有界空行省略列表缩进时仍保留代码正文与原列表间距，缺失引用仍关闭（见[围栏空行](#列表内代码围栏跨省略缩进空行)）。EOF 封闭未闭合围栏，保留代码语义，不回退/重扫正文。
- 缩进代码要求去掉容器前缀后至少四列，仅额外隐藏四格：通常为 ASCII 空格，上述新叶块子项可计入前缀部分 tab 的虚拟余量；前缀之外的 tab 不代替四格。相邻缩进代码行仅在相同 `ContainerId` 下合并，支持流式长行。沿用既有顶层方言：可中断普通正文，完整空白行立即终止代码（即使缩进足够），不延迟保存空行；下一行重新开块。新列表标记后含 tab 且至少五列时按下述[列表首行 tab 缩进代码](#列表首行-tab-缩进代码)处理；其余新列表/任务标记后的水平空白沿用原前缀规则。完整 CommonMark 缩进/空行规则仍待扩展。

结构识别沿用含容器前缀的 `BLOCK_LOOKAHEAD = 1024` 字节窗口（UTF-8 最多再取三个字节）。opener 在窗口内确认后，过长 info 后续部分整行隐藏，不因晚来的反引号撤销；未识别的超长/失败 opener 走普通行内方言。closer 必须连同行末在窗口内完整出现，超长 closer 行按代码正文流式消费，即使尾部只是空白也不回看。未分类尾部的块状态保持 Growing，`finish` 按 EOF 分类短尾行或封闭已流式消费的长行。

```rust
use mdstream::{BlockId, BlockKind, Document, FoldId, FoldView, SyntaxState};
let mut doc = Document::new();
doc.append("> - ```rust\n>   **raw** &amp;\n")?;
let id = FoldId::Code(BlockId(0));
let mut view = FoldView::new(&doc, 1)?;
view.set_collapsed(&doc, id, true)?;
let end = doc.source().len();
let update = doc.append("> - sibling\n")?; // sibling seals the old fence
view.apply(&doc, &update)?;
assert_eq!(doc.blocks()[0].source.end, end);
assert_eq!(doc.blocks()[0].state, SyntaxState::Sealed);
assert!(matches!(doc.blocks()[0].kind, BlockKind::Code { .. }));
assert!(view.is_collapsed(id));
assert!(!doc.fold_candidate(id).unwrap().open);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Update.new_blocks` 只在新代码块创建时增加；增长/关闭进入 `changed_blocks`，容器退出沿用关闭事件。`FoldId::Code` 隐藏整块（包括前缀和围栏），父节点是同作用域当前章节，否则为所属容器；`summary` 为 info，缩进代码没有 info。代码、表格、章节、容器的独立意愿、默认 `code`、批量/兄弟操作、临时预览、预算导航及旧测量拒绝直接组合。Growing 候选暂包含未分类尾部，分类确认退出时收回到精确边界；`DisplayExcerpt` 可按源码/输出/逻辑行及共享 Budget 读取正文预览，隐藏前缀同样计费，追加后显式重建任务。

围栏只增加一个常数大小的作用域检查，显式容器匹配每层消费正长度前缀；空行省略列表路径复用引用深度索引，不遍历保留的列表祖先；列表缩进串缓存末端。超长代码行的后续追加仅延长当前块和共享已解析末端，不做行内扫描、不更新祖先。闭合只访问当前行及实际关闭节点，不读取旧正文；累计解析 O(n log n)、空间 O(n) 不变。折叠仍至多刷新一个开放代码/数学/表格块，容器与内层章节共用尾部，不枚举历史代码块。解析及 `apply` 仍按输入/事件摊还，不承诺任意大输入的单帧时延。

`crates/mdstream-core/tests/container_code.rs`、`crates/mdstream-core/tests/container_sections.rs` 和分块/fuzz 测试覆盖作用域/失败候选、UTF-8 全切点及中间快照、长行/CRLF/EOF、预算与组合；400 个代码块的生成范围模型、240 次流式区间/高度模型、千层容器/百万字正文、250 层列表和 4096 个历史代码块验证增量成本。折叠百万字代码的布局字节计费和正文回调均为零。`cargo run -p mdstream-core -r --example container_code` 演示流式围栏、预算预览、兄弟关闭以及只展开目标路径。

### 围栏正文的有界空格去缩进

反引号/波浪线 opener 记录容器前缀之后的 N 列缩进（0–3，ASCII 空格加已有部分 tab 的虚拟余量）。每条代码正文行匹配同一容器路径后，先扣除至多 N 列虚拟空格，再消费剩余额度内的行首 ASCII 空格和 tab；tab 沿物理行每四列对齐，部分消费后的 1–3 列余量显示为空格并映射回原单字节。不足 N 列则只去掉实际存在的缩进；非 space/tab 字符终止消费，额度用尽后的 tab 和空白保留字面。零缩进 opener 保留全部正文缩进；空行、EOF 尾行、失败/过长 closer 的正文同样处理。数学块 `$$` 保持原有字面缩进，四空格缩进代码也沿用原规则。

短未分类行暂定显示原始源码，在 LF/CRLF、EOF 或分类窗口确认时隐藏缩进；源码、info、块/容器 ID、折叠意愿和字节映射保持，既有通知使旧缓存/游标失效。完整隐藏的前缀 tab 不建立替换记录；仍有余量时，只把保留列映射回原 tab 的一个源码字节。隐藏缩进同样消费显示预算，6 字节/1 query/1 object 可逐步读取 UTF-8。

活动围栏保存固定大小的缩进、本行剩余额度和物理列模四余数。需要消费原始正文缩进时，在当前分类窗口内读取一次已匹配前缀的字节以取得列位置，不遍历祖先节点。深容器前缀耗尽分类窗口时，最多三列剩余额度和列位置交给同一物理行的后续追加；新增缩进字节只检查一次，非 space/tab 终止后不再探测。实际字节检查计入 `Work.classified_bytes`，隐藏与部分 tab 映射复用既有接口，不读取旧正文、历史块或在续传时重读前缀。累计 O(n log n)、空间 O(n) 及共享预算契约保持。

`tests/fence_indent.rs` 的 7 项回归覆盖 840 组独立空格模型、根/引用/列表及嵌套作用域、部分 tab、UTF-8/CRLF/EOF、全切点/逐前缀、1 KiB 边界、源码/tile/预算映射、旧任务/测量拒绝和折叠身份。1015 层路径在分类窗口后逐字消费缩进不重访祖先；千层百万字后追加 1000 字节不增加分类/行内/容器/树访问；4096 历史代码块后新块只做局部工作。官方 131–133 修复，完整 tab/容器方言和其它章节仍由 TODO 跟踪。

`tests/fence_tabs.rs` 另以 5 项回归覆盖正文 tab：1008 组独立物理列宽模型，根/引用/列表、部分 tab 祖先、空白/失败 closer/数学边界、UTF-8 全切点与逐前缀及 EOF、缓存失效、tile 边界单字节替换/逐字符切片和六字节预算恢复。1015 层路径跨分类窗口后的 tab 只检查新增字节；百万字后千次追加不增加分类/行内/容器/树访问，4096 行保持一个代码块。关闭定界判断仍在去缩进前进行；容器内原始 tab 开头的 closer 后由[closer 列宽缩进](#围栏-closer-的-tab-缩进)扩展，完整 opener 缩进和其它容器规则继续待办。交付记录见 [围栏正文 tab 去缩进](DONE.md#2026-11-03-围栏正文-tab-列宽去缩进交付)。

复跑：`nix develop -c cargo nextest run --workspace --test fence_tabs --test fence_indent --test commonmark_fenced_code --no-fail-fast`。

## 容器内定义

显式 Quote/Item 内支持单行及跨物理行链接定义 `[label]: destination title` 和可延续的脚注定义 `[^label]: body`。定义起始行先匹配容器前缀、列表续行缩进及新条目的任务标记，再允许至多三列 space/tab 缩进（包括部分 tab 余量，按物理行四列制表位计量）；四格缩进和已有围栏优先，代码里的定义保持字面内容。`BlockKind::Definition` / `FootnoteDefinition` 保存原有符号，`Block.container` 记录最内层容器，`source` 覆盖完整定义物理行和前缀，链接定义可占多条有界物理行，脚注块可跨多行。目标/title 与脚注 content 均是借用源码的绝对 `Span` 区间。任务信息仍从 Item 读取。

定义的**符号作用域是整个文档**：根、嵌套、兄弟条目和已经关闭的容器共享链接符号表，也共享独立的脚注符号表；不创建局部命名空间或按祖先查找。每种符号的首个有效定义获胜，后续重复定义仍产生隐藏块但不覆盖、不改变符号版本。链接标签在归一化前按去容器前缀后的真实原始字节（含 LF/CRLF，不计部分 tab 虚拟列）限 999 字节，匹配键采用 Unicode 空白折叠及[完整 Unicode case folding](#引用标签的完整-unicode-case-folding2026-11-10)；脚注标签沿用区分大小写的字母数字/下划线/连字符规则。脚注编号仍按全局首次引用顺序分配，定义先到不占号。容器结束或折叠不删除定义，任意位置的引用链接、引用图片及脚注都能解析到它。

- 链接定义的每个物理行必须在含容器前缀的 1 KiB 分类窗口内完成（UTF-8 最多延长三个字节）。短行等 LF/EOF 才发布；超过窗口后按普通容器正文消费，不因晚来的行尾再确认。裸目标、尖括号目标及三种 title 沿用顶层方言。
- 脚注只要求合法 `[^label]:` 位于分类前缀内，正文可无限流式增长。冒号后前缀内已有的一个 space/tab 可被省略，其余原样保留；正文不解析 Markdown、链接、数学或嵌套脚注。未分类短行仍显示原文；确认后整个定义隐藏，保持 Growing 直到非续行边界或 EOF。同容器路径下的四列完整边界续行、空行分段和去前缀读取见[脚注核心](#脚注核心)。CRLF 只从当前正文包络末端去掉，内部换行原样保留。
- 链接定义支持[跨物理行标签](#引用定义跨物理行标签)，闭合 `]` 后须在同一行紧接 `:`；冒号后仅有水平空白时，可从紧接下一物理行读取完整目标及该行 title。目标行未带 title 时，可由紧接下一物理行提供[独立完整 title](#引用定义的独立下一物理行完整-title)，包括空 title；已有 title 不再吸收下一行。各续行须完整匹配原显式容器路径，目标/title 行在路径后可多于三列水平缩进。标签/目标候选失败只单次回流；独立 title 候选失败保留已发布目标，当前行走普通块流程。已有段落不能被定义候选中断；表头/Setext 资格和结构优先级保持。脚注续行仍须匹配原容器路径。表格单元格、标题正文和行内代码保持自己的上下文。
- 确认目标前的非法/超长标签、目标/title 失败或未完成候选沿用普通行内处理，可能创建未解析引用，但不创建有效定义。不回放长行已消费的前缀；无效的纯空白链接标签也保留容器前缀隐藏和独立叶块边界，不并入外层段落。既有完整显式 Quote/List/嵌套路径中，符合原普通文本资格的多行 title 正文及完整前缀匹配后的 1–3 虚拟列已支持，见[容器多行 title 正文](#引用定义容器多行-title-正文)；虚拟余量仅参与结构判定，不产生额外元数据文本，真实缩进保留。根段落行内后缀的组件换行（510）已支持，见[后缀组件换行](#根段落行内链接后缀的组件换行)；容器普通段落后缀组件 gap 已支持，见[容器后缀组件换行](#容器普通段落后缀的组件换行)；容器直接 title 正文换行、脚注正文 Markdown 排版和完整容器/字面回退仍待办；根作用域的 196 见[多行 title 正文](#引用定义根作用域的多行-title-正文)。

```rust
use mdstream::{Budget, Document, FootnoteReferenceId, FootnoteTarget, LinkId, WorkLimit};
let mut doc = Document::new();
doc.append("[guide][ref] [^note]\n")?;
assert_eq!(doc.link_destination(LinkId(0)), None);
doc.append("> - [ref]: <guide.md> 'Guide'\n>   [^note]: 原始 **正文**\n")?;
assert_eq!(doc.link_destination(LinkId(0)), Some("guide.md"));
let mut budget = Budget::new(WorkLimit { queries: 2, objects: 2, dependencies: 1, bytes: 32, ..Default::default() });
let resolved = doc.resolve_footnote(FootnoteReferenceId(0), &mut budget).unwrap().unwrap();
let FootnoteTarget::Defined(definition) = resolved.target else { panic!() };
assert!(doc.block(definition.block).unwrap().container.is_some());
let mut body = doc.footnote_content(resolved.symbol).unwrap();
assert_eq!(body.advance(&mut budget).unwrap().unwrap().text, "原始 **正文**");
# Ok::<(), Box<dyn std::error::Error>>(())
```

晚定义沿用 `LinkId -> SymbolId` 和 `FootnoteReferenceId -> FootnoteId` 间接寻址，只改当前符号及版本，发出 `references_changed` / `footnotes_changed` 和从 first_use 起的保守范围失效；不访问历史引用对象，不给旧链接逐个发 changed 事件。脚注增长/封口及新块使用原有 Update，支持批量合并；DependencyKey、旧布局 ticket 拒绝与独立折叠意愿保持不变。定义的语法隐藏独立于视图折叠，展开祖先也不使定义正文可见；脚注正文另用共享 Budget 的原文游标读取，实际脚注区排版仍待接入。

实现复用现有全局符号表和流式行状态机，不复制祖先；多行脚注额外记录去前缀所需的有序间隙，见[脚注核心](#脚注核心)。有界标签寻址 O(log S)，长脚注追加只更新当前块/符号和共享解析末端；容器匹配按实际前缀/开闭节点计费。累计解析 O(n log n)、空间 O(n) 不变，不承诺大 append 或 EOF 批量闭合的单帧硬时延。`crates/mdstream-core/tests/container_definitions.rs` 与分块测试覆盖两万处旧引用晚定义、千层容器百万字增长、250 层列表、4096 个历史符号、400 组首定义/物理范围模型、通知重建/合并和共享预算；隐藏百万字定义的布局正文回调和字节计费均为零。`cargo run -p mdstream-core -r --example container_definitions` 演示全局晚定义、容器折叠及逐帧脚注读取。

## 容器内表格

显式 Quote/Item 的普通正文行可以成为表头；紧接的完整物理行只有在**同一最内层 ContainerId** 中，且列数与分隔单元格有效时才确认表格。识别先匹配已有引用前缀/条目续行，再验证分隔行：`> a|b` 后的 `> - | -` 是表格，分隔行优先于新开子列表；`- a|b` 后的 `- -|-` 已换到兄弟 Item，不能确认旧表头。顶层的分隔行优先级保持。

```rust
use mdstream::{BlockId, Document, FoldId, TableId};
let mut doc = Document::new();
doc.append("> - a|b\n")?;
let item = doc.block(BlockId(0)).unwrap().container;
assert!(doc.tables().is_empty());
let update = doc.append(">   - | -\n>   x|y\n> - sibling\n")?;
assert_eq!(update.new_tables, 0..1);
let table = doc.table(TableId(0)).unwrap();
assert_eq!(table.block, BlockId(0));
assert_eq!(doc.block(table.block).unwrap().container, item);
assert_eq!(doc.text(table.cell(1, 0).unwrap().source), Some("x"));
assert_eq!(table.source.end, doc.source().find("> - sibling").unwrap());
assert!(!doc.fold_candidate(FoldId::Table(TableId(0))).unwrap().open);
# Ok::<(), Box<dyn std::error::Error>>(())
```

表头和分隔行沿用下节的管道语法：至少一个未转义管道、可选首尾管道、列数相同，分隔单元格为一个以上 `-` 和可选左右单个 `:`，去掉 ASCII 首尾空白；分隔行不另外限制前导空格。长表头须在含容器前缀的 1024 字节窗口内被分类为显式普通正文（UTF-8 边界最多额外 3 字节）。已在窗口内出现未转义管道的候选可继承前序状态；首管道较晚时支持新序列，以及同一容器中只继承待确认换行或尚未闭合旧强调的续行，见下节。候选余文、分隔行和正文行没有该长度上限；长分隔行只允许显式匹配表头的确切容器路径。容器前缀已隐藏，候选只延迟尚未提交的语义；分隔行失败或 EOF 无分隔行时仅消费未解析后缀，短失败行恢复普通容器叶块/Setext 语义，长失败行保持原有不完整前缀分类。尚未确认的容器表头块为 Growing，确认时保留 BlockId 并产生 `new_tables`。

正文只接纳同一容器中的普通文字行，允许无管道的稀疏行。空行、标题、代码/定义/分隔线形式的行，进入子容器、离开容器或换兄弟条目都会终止表格；回到外层后不恢复旧表。随后合法的围栏/缩进代码和链接/脚注定义进入独立块；有界完整分隔线生成独立 `ThematicBreak`。单元格行内状态按行/列隔离，前缀不进入单元格；任务信息留在 Item 上。普通正文的跨行行内语义见[容器内行内续行](#容器内行内续行)，不会跨确认后的表格单元格；lazy continuation 和完整 GFM 兼容仍未实现。

`Table.block` 指向保存确切容器 ID 的叶块；Table/source、delimiter 和 TableRow/source 包含完整物理行及容器前缀，TableCell/source 仅含去空白的单元格正文。`table_at` 覆盖前缀/分隔行，`row_at` 覆盖行前缀但不覆盖分隔行；按 ID/行列读取仍 O(1)，定位表/行仍 O(log B)/O(log R)。未分类尾部排除在结构索引外，已分类的超长行按追加增长；开放表格折叠暂覆盖 pending 尾部，边界确认后收回。列宽增长、行变化与表格创建沿用既有 Update，旧行不重扫。

同一时间仍只有一张开放表格，整表折叠、默认收起、临时展开、导航和 TablePreview 直接复用现有接口；折叠父节点为确切作用域的当前章节，若尚无本地标题则为该容器（根作用域无标题时无父节点）。摘要的源码总上限和共享 Budget **包含隐藏容器前缀**；千层前缀也不能免费越过。正文追加只消费新字节，不遍历祖先；长分隔行也只匹配一次有界容器前缀，后续按字节更新状态。空间不复制祖先链或创建空单元格矩阵，全库保持累计 O(n log n)、空间 O(n)。`container_bytes` 包含分隔行优先级检查，`table_candidate_bytes` 统计长分隔行探测字节。

`cargo run -p mdstream-core -r --example container_tables` 演示流式确认、兄弟边界、按源码位置查询最内层表格候选及其所属条目父节点、默认整表收起和小预算摘要。`crates/mdstream-core/tests/container_tables.rs` 与折叠测试覆盖 UTF-8 全切点/逐前缀、独立范围模型、千层/百万字追加、超宽稀疏表、隐藏前缀预算和单占位跳过正文。实际表格排版及行范围折叠仍见 TODO。

## 表格、任务列表与图片

表格和任务标记的基本写法参考 [GFM 表格](https://github.github.com/gfm/#tables-extension-) 和 [任务列表](https://github.github.com/gfm/#task-list-items-extension-)。这是项目方言的扩展，不声明完整 GFM 合规。

`BlockKind::Table { id }` 指向 `Table`，`Update.new_tables` 给出新创建的 ID 范围：`columns` 保存默认/左/中/右对齐和列宽提示，`rows()[0]` 是表头，`row(i)` / `cell(i, j)` 可直接访问。`row_at(byte_offset)` 二分定位所在行，分隔行没有数据行编号。单元格保存裁剪首尾空白后的原文区间，内部语法用 `runs_in()` 或与可见片段相交后查询。每个单元格使用独立行内状态，强调不会跨越单元格。

```rust
use mdstream::{Alignment, Document, TableId};
let mut doc = Document::new();
doc.append("| Name | Value |\n| :--- | ---: |\n| CPU | **42** |\n")?;
let table = doc.table(TableId(0)).unwrap();
assert_eq!(table.columns[1].alignment, Alignment::Right);
assert_eq!(doc.text(table.cell(1, 1).unwrap().source), Some("**42**"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

表头/分隔行允许可选首尾竖线，列数必须一致；正文缺列为隐式空单元格，多余列隐藏，不分配 rows × columns 的空矩阵。`\|` 在文本和代码 span 中均表示单元格内的竖线。表格在空行或其它块级结构处结束，可以直接跟在段落后。短表头沿用既有规则；长表头支持窗口内早管道，以及新行内序列、只继承待确认换行、尚未闭合旧强调/代码 span/数学/链接及图片标签，以及可隔离的旧强调/代码/数学闭合配对和旧链接/图片标签及普通字符显式后缀的晚管道。候选余文、分隔行、正文行和单元格没有此长度限制。解析器只保存一个候选的源码区间、已消费端点和固定大小摘要，延迟部分以原文预览；`pending()` 仍只报告未分类行的有界前缀。判定失败时表头只语义解析一次，不回扫历史段落。

**长表头的流式规则**：分类窗口内的前缀按既有不完整行规则属于普通正文时，窗口内已出现未转义 `|` 的候选延迟全部行内消费。若窗口内尚无管道，新行内序列和可隔离的继承续行参与晚管道识别：顶层新段落、新容器/空行/标题后的正文，以及前序状态只剩待确认软/硬换行、或还保留未闭合强调 opener / 代码 span 的顶层或同一 Quote/Item 显式续行，以及继承未闭合数学或链接/图片标签的顶层段落及同容器正文。旧强调、代码、数学和链接标签（含普通字符显式后缀）的可隔离闭合暂存效果（见下文）；其它无法隔离的旧候选在首管道前闭合，则取消当前行的表头探测，保留普通正文效果。它在首管道前保持即时普通行内解析，从首管道起延迟后缀。lazy 行、代码、标题、定义等已识别块和既有表格正文不参与；窗口后不再重试块/容器分类。

长候选按新增字节维护列数、反斜线奇偶及首尾管道/空白状态；不存单元格数组，不复制原文。早管道候选不发布行内语义；晚管道候选已发布的首单元格前缀语义保持，延迟后缀不发布样式、链接、数学或脚注对象。LF 后固定列数并等待下一行，CRLF 可跨分块；容器前缀与 CR 隐藏，候选内容仍可按片段显示。已经分配的叶块保持 Growing 和稳定 BlockId，块/容器/章节的末端按新字节增长；紧随顶层段落的早管道候选保持原段落端点；即时消费的晚管道续行先延伸原段落，确认时恢复原端点并新建表格块。

确认分隔行时，早管道候选丢弃前序行内状态，候选通过既有行解析器消费一次。可复用前缀的晚管道候选先丢弃暂存的跨行强调配对、仍未闭合的继承强调 opener 与旧标签候选，再把原有 Inline 移给首单元格；行解析器扫描源码以建立单元格、裁剪空白和隐藏代码内的转义管道反斜线，跳过已消费前缀的行内解析，并在首管道处结束该 Inline；后续单元格独立。分隔行无效、作用域改变或 EOF 无分隔行时，只把未消费后缀送入保留的普通状态，再按当前块规则继续或结束；既有Setext 回退保持。已发布的首单元格语义无需撤销，链接/数学/脚注 ID 不变且只发布一次。长分隔行晚失败沿用一次有界前缀回退，禁止同一行再次开启长表头候选。

**继承续行的隔离**：在消费当前分类前缀之前，O(1) 检查行内状态，仅有待确认换行、可选的强调 opener、代码 span、数学或链接/图片标签时可参与晚管道探测。下面先说明强调/换行状态；代码、数学和标签的规则见后文。把该换行的源码区间、待隐藏尾空白/反斜线和软硬类型移入候选；当前行保留原强调栈即时消费，并记录开始时的栈边界和是否曾闭合旧 opener。前序换行及其尾标记暂按原文显示，当前行的局部样式、实体和对象仍即时发布。确认表格时丢弃此换行；无管道行到 LF/EOF，或候选因分隔行失败、作用域改变、EOF 而回退时，提交一次原换行效果。此提交产生正常范围失效，旧缓存和测量不能漏掉前序行变化。换行及栈边界为固定大小状态；可隔离的已闭合强调另按实际配对保存端点（见下文）。不克隆栈、不撤销对象、不重读正文。短行及早管道候选保持原有行为。

**未闭合强调的探测**：支持既有 `*`、`**`、`_`、`__`、`~~` 方言。当前行内部的配对正常发布；只要它尚未闭合任何继承 opener，已发布效果便与独立首单元格一致。确认表格时一次移除旧栈前缀并重建当前行剩余 opener 的前驱下标，保留当前行代码/数学/链接等候选以及所有已发布对象 ID。首管道前仍待定的 delimiter run 在移除旧 opener 后按单元格末端结算。失败、EOF 或无管道行结束时只清除探测标记、恢复前序换行，原栈及尚未消费后缀继续普通解析。

若旧 opener 在首管道前以无法隔离的方式闭合，则一次切回普通流式行，当前行不再尝试表头；已确认的跨行样式和对象保持即时可见。同次追加中首管道后的后缀也只普通消费一次。记录实际闭合事件，不能仅比较当前栈长度：旧 opener 弹出后，新 opener 可能又把栈长恢复。确认时的删除/重建计入 `delimiter_steps`，每个 opener 最多在所属序列变为单元格时被处理一次；失败和普通续行不遍历历史栈。

```rust
use mdstream::{Document, LinkId, Style};
let mut doc = Document::new();
doc.append("> prior  \n")?;
doc.append(&format!("> [cell](url) {}", "x".repeat(2048)))?;
let original = *doc.link_info(LinkId(0)).unwrap();
assert!(doc.tables().is_empty());
doc.append("|b\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), original);
assert!(!doc.runs_in(mdstream::Span::new(9, 10)).unwrap()
    .any(|run| run.style.contains(Style::HARD_BREAK)));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`table_header_bytes` 对每个候选正文摘要字节只计一次（也计无管道的合格序列和短普通正文的候选检查）；`inline_bytes` 对可复用的前缀不重复消费；继承代码/数学的成功路径还计一次当前行的独立单元格解析，失败和 EOF 则保持原候选状态、不重复消费前缀。确认时 `table_bytes` 计一次单元格边界扫描。无管道长行继续即时消费并发布本行局部语义；隔离的前序换行及可分离跨行强调只在候选失败、无法隔离的旧候选闭合而取消探测或无管道行结束时提交，不重写整行注解。候选摘要和边界为 O(1)，暂存强调端点为 O(实际闭合旧 opener 数)，每对仅提交或丢弃一次；已有 Inline 只移动、不克隆；分隔行对齐值 O(列数)，确认后才分配真实列/单元格，稀疏行不补空矩阵。输入 String、tile 和正式语义索引仍按需增长，全库累计 O(n log n)、空间 O(n)。**长表头确认与失败均不承诺单次 append 的硬时延或字节预算**；有预算显示、摘要和折叠读取继续使用既有接口，确认通过范围失效拒绝旧测量。

```rust
use mdstream::{Document, TableId};
let mut doc = Document::new();
doc.append("> |**name**|")?;
doc.append(&"x".repeat(4096))?;
assert_eq!(doc.work().inline_bytes, 0); // Candidate stays raw.
doc.append("\n")?;
assert!(doc.tables().is_empty());
let update = doc.append("> |-|-\n")?;
assert_eq!(update.new_tables, 0..1);
assert_eq!(doc.table(TableId(0)).unwrap().columns.len(), 2);
# Ok::<(), Box<dyn std::error::Error>>(())
```

**仍保留的边界**：已有段落或同一容器正文续行若旧强调以无法隔离的方式闭合，或旧代码/数学闭合后带有本子项不支持的行内语法字符，或旧链接及图片标签闭合后带含行内语法字符的显式后缀或新行内语法，且首管道晚于窗口，仍按普通正文处理。例如 `*prior\nend*…|b` 可能在管道出现前已闭合跨行强调；现有单调样式与稳定对象 ID 无法直接撤销。此范围仍在 TODO，不能通过延迟所有普通长行来绕过即时消费要求。短表头及早管道长表头继续支持前序段落。尚未移除其它块/容器分类上限，也不承诺单次解析硬预算。

```rust
use mdstream::{Document, LinkId, TableId};
let mut doc = Document::new();
doc.append("> **name** [link](url) ")?;
doc.append(&"x".repeat(4096))?;
assert_eq!(doc.links().len(), 1); // First-cell semantics are already live.
let first = *doc.link_info(LinkId(0)).unwrap();
doc.append("|[other](target)\r\n")?;
assert_eq!(doc.links().len(), 1); // Suffix waits for confirmation.
doc.append("> -|-\n")?;
assert_eq!(doc.table(TableId(0)).unwrap().columns.len(), 2);
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), first);
assert_eq!(doc.links().len(), 2);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`crates/mdstream-core/examples/late_headers.rs` 演示首管道前百万字即时解析、已发布链接身份复用及共享 Budget 摘要；`crates/mdstream-core/tests/late_headers.rs` 覆盖首单元格语义对照、成功/失败、未闭合跨行状态限制、千层/百万字、4096 对象与稀疏列、400 组范围模型及布局/折叠预算。

`crates/mdstream-core/examples/open_headers.rs` 演示保留旧强调 opener、百万字即时消费、成功丢弃与失败闭合，以及稳定对象和共享预算摘要。`crates/mdstream-core/tests/open_headers.rs` 覆盖五种 delimiter、首管道前待定 run、闭合后栈重新增长、512 组局部配对模型、400 组表格范围模型、千层/百万字、4096 旧 opener/对象/列、缓存与合并通知；逐切点及中间快照在 `crates/mdstream-core/tests/chunking.rs` 验证。

```rust
use mdstream::{Document, LinkId, Span, Style};
let mut doc = Document::new();
doc.append("> *prior\n")?;
doc.append(&format!("> [cell](url) {}", "x".repeat(2048)))?;
let original = *doc.link_info(LinkId(0)).unwrap();
doc.append("|end*\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), original);
assert!(!doc.runs_in(Span::new(3, 8)).unwrap()
    .any(|run| run.style.contains(Style::EMPHASIS)));
# Ok::<(), Box<dyn std::error::Error>>(())
```

**可隔离的已闭合强调**：对 `*`、`**`、`_`、`__`、`~~`，若匹配的是继承 opener，当前栈没有本行尚未闭合的 opener，且该 run 按既有 ASCII flanking 规则只能闭合、不能开启（例如 `end* `），则这次配对在独立首单元格中只会成为字面标记，不改变其局部解析状态。解析器正常弹出旧栈，仅保存 open/close 源码区间和 style，不发布这对跨行样式及定界符隐藏；可连续保存多对，也可跨过其它类型的旧 opener。本行局部强调、实体、代码、链接、数学及脚注仍即时消费和发布，不复制正文、不克隆栈。

确认表格时丢弃这些配对，closer 保持字面可见，首单元格 Inline 与已发布对象直接复用。失败、无管道行到 LF/EOF、作用域退出或随后遇到无法隔离的闭合时，按原端点一次提交暂存样式/隐藏，再沿原状态继续；提交产生正常范围失效。探测期间可能暂见旧 opener 和本行 closer，跨行样式要到判定后才可见；未延迟本行其余语义。短行、早管道和原本不参与探测的输入保持原行为。

`end*next` 等可同时开启的 run，以及会弹出本行未闭合 opener 的跨行配对（如旧 `*old` 后接 `**local end* `），仍立即提交原效果并取消探测；不重试该行。代码/数学闭合按下文隔离或取消，旧标签闭合按后文隔离或取消。此子项通过隔离未发布的效果保持单调注解，没有新增通用撤销机制。

每个暂存配对必定消费一个旧 opener，保存/提交/丢弃各至多一次，后两者计入 `delimiter_steps`；空间 O(被消费的旧 opener 数)，累计 O(n log n)。无配对的长尾只消费新增字节，不遍历暂存数组、历史正文或祖先；失败恢复只做区间注解，不回放字节。`crates/mdstream-core/tests/closed_headers.rs` 对照全延迟早管道语义及独立 flanking/范围模型，验证失败、千层/百万字、4096 配对与历史对象、预算和失效；`crates/mdstream-core/examples/closed_headers.rs` 演示成功丢弃与失败提交。

```rust
use mdstream::{Document, LinkId, Span, Style};
let mut doc = Document::new();
doc.append("> *prior\n")?;
doc.append(&format!("> end* **local** [cell](url) {}", "x".repeat(2048)))?;
let link = *doc.link_info(LinkId(0)).unwrap();
assert!(!doc.runs_in(Span::new(3, 8)).unwrap()
    .any(|run| run.style.contains(Style::EMPHASIS))); // Cross-line pair is isolated.
doc.append("|b\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), link);
let cell = doc.tables()[0].cell(0, 0).unwrap().source;
assert!(doc.display_runs_in(cell).unwrap()
    .filter(|run| !run.style.contains(Style::HIDDEN))
    .map(|run| run.text).collect::<String>().starts_with("end* local cell"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

**未闭合代码 span 的探测**：继承代码候选可同时保留旧强调栈，以及顶层未闭合链接/图片标签。当前行继续即时更新原代码的反引号 run、全空白标志和最后一个内容区间；代码内的实体、链接、数学和脚注保持原文，不提前发布独立单元格对象。首管道起的后缀仍延迟。若旧代码在首管道前闭合，可隔离子项按下节暂存原跨行样式及空白规范化映射；闭合后出现不支持的行内语法字符则先提交该映射并取消该行探测，即使随后开启新代码候选也不恢复探测。

确认表格时，旧代码尚未闭合，所以当前行前缀没有已发布的行内效果。一次丢弃原 Inline（包含旧强调栈），**不结算首管道前待定的反引号 run**，再由已有 RowParser 扫描当前候选行并按独立单元格解析。确认前的旧对象 ID 保留，当前行对象只在这次解析中分配一次，源码映射按单元格规则生成；不撤销已发布的显示映射。失败、无管道、作用域退出或 EOF 只取消探测标志，继续原代码状态及未消费后缀，保留原空白摘要，代码闭合仍只读端点、不回扫正文。

代码路径的每个合格字节只做一次表头摘要和一次原代码状态消费；成功后在既有单元格扫描阶段再做一次当前行的行内消费。两次消费均如实计入 `inline_bytes`，全流满足 `inline_bytes <= source().len() + table_bytes`；额外消费仅属于互不重叠的已确认表头，不读前序代码正文、不重新分类、不在失败时重放，因此累计仍为 O(n log n)、空间 O(n)。候选只增加一个布尔标志，不复制正文或栈。这个子项保留单次确认的摊还契约，显示、摘要和折叠的共享预算不变。

```rust
use mdstream::{Document, LinkId};
let mut doc = Document::new();
doc.append("> [prior](u) ` old\n")?;
let prior = *doc.link_info(LinkId(0)).unwrap();
doc.append(&format!("> [cell](v) &amp; {}", "x".repeat(2048)))?;
assert_eq!(doc.links().len(), 1); // The inherited code remains opaque.
doc.append("|b\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(doc.links().len(), 2); // Cell syntax is consumed on confirmation.
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), prior);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`crates/mdstream-core/examples/code_headers.rs` 演示百万字代码候选的成功/失败、旧对象身份和共享预算摘要。`crates/mdstream-core/tests/code_headers.rs` 验证全延迟表头与普通正文对照、363 组反引号组合、400 组范围模型、映射的逐标量/tile/预算读取、千层/百万字、4096 旧 opener/对象/列、缓存与合并通知；`crates/mdstream-core/tests/chunking.rs` 覆盖全部 UTF-8 切点和逐前缀快照。首管道前已经闭合的可隔离代码子项见下节，其余跨行代码映射撤销仍保留 TODO。

**已闭合代码 span 的可隔离子项**：顶层或同一显式 Quote/Item 续行沿用上述晚管道入口。旧代码在首管道前由等长反引号串闭合时，只暂存原 opener、closer、最后内容区间和全空白标志。从闭合 run 后到首管道之间，允许普通文本、UTF-8、空白和不会启动行内语法的标点；遇到 `&`、`<`、反斜线、星号、下划线、波浪线、反引号、`$`、`!`、`[` 或 `]` 任一字节即取消，包含尚未成形或最终无效的语法候选。闭合 run 尚待后继字节判定且已到首管道时，继续按未闭合代码路径处理。早管道、短行、lazy/部分 tab 续行和已分类块保持各自既有规则。

取消必须在处理该字节之前提交原代码映射：显示替换索引按源码顺序追加，不能先发布后面的实体/代码，再插回前面的跨行映射。此时旧样式与空白规范化即时发布、继续正常解析，后续再开代码也不重启本行探测。合格普通长尾仍按新增字节即时消费，仅这一个跨行闭合效果等待；没有延迟所有普通长行或撤销已发布对象。无管道 LF/EOF、无效分隔行、Setext/作用域退出时提交一次原映射，再消费未解析后缀。旧源码范围正常失效，缓存、测量及 revision 绑定读取/搜索任务不能复用旧状态。

确认表格时丢弃暂存闭合与旧 Inline，在已有单元格扫描中独立解析当前表头。闭合之前处于旧代码内的实体/链接/数学/脚注到这时才按单元格语义发布；旧对象身份和编号不变。等长反引号在独立单元格里可能成为 opener，因此不能直接复用原代码解析状态。此前隐藏的容器前缀、CR 及历史部分 tab 源码映射保持原归属；不改写前序代码正文。额外状态 O(1)，提交/丢弃计一个 `delimiter_steps`；成功额外消费仅限当前表头，失败无正文回放，继续满足 `inline_bytes <= source().len() + table_bytes`、累计 O(n log n)/空间 O(n)。确认仍无单次硬时延保证，显示/摘要/折叠消费保持共享预算。

```rust
use mdstream::{Document, LinkId, Span, Style};
let mut doc = Document::new();
doc.append("> [prior](u) ` old\n")?;
let prior = *doc.link_info(LinkId(0)).unwrap();
doc.append(&format!("> [cell](v) &amp; end` {}", "x".repeat(2048)))?;
assert_eq!(doc.links().len(), 1);
assert!(!doc.runs_in(Span::new(0, doc.source().len())).unwrap()
    .any(|run| run.style.contains(Style::CODE))); // Only the cross-line effect waits.
doc.append("|b\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(doc.links().len(), 2);
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), prior);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`crates/mdstream-core/tests/closed_code_headers.rs` 覆盖独立早管道对照、闭合后语法取消、长分隔行及 400 组范围模型、源码映射/共享预算、缓存/通知与折叠；全切点/逐前缀快照在 `crates/mdstream-core/tests/chunking.rs`。千层/百万字追加不遍历旧正文/祖先，4096 个历史对象、旧 opener 和当前行对象/列保持增量消费；内部测试确认一个摘要在 200 轮成功/失败后释放。`crates/mdstream-core/examples/closed_code_headers.rs` 演示百万字确认/失败及有预算预览。闭合后带其它行内语法的表头、通用已发布映射撤销仍留 TODO。

**未闭合数学的探测**：顶层段落或同容器正文的 `$` / `$$` 候选可参与晚管道探测，可伴随旧强调栈和未闭合链接/图片标签。首管道前继续即时维护原美元串和转义状态；数学内容仍为原文，不提前发布单元格中的链接、数学或脚注。若旧数学在首管道前实际闭合，按下节暂存原定界端点；闭合后遇到其它行内语法起始字节则先发布稳定 MathId、原始内容包络和范围失效，再取消当前行探测。即使随后开启新数学也不恢复探测；已有对象不会被撤销或重编号。

若确认表格，旧数学从未闭合，因而首单元格前缀没有已发布的行内效果。复用代码候选的固定大小探测标志，丢弃旧 Inline 及仍待定的美元串（不结算并发布跨行对象），再在已有 RowParser 扫描中独立消费当前表头。当前行对象只在此时发布一次，既有 MathId/LinkId/脚注编号保持。失败、无管道、EOF 或块中断继续原候选与未消费后缀；失败分隔行按原块规则处理，旧数学可随后正常闭合。候选额外状态 O(1)，无正文/栈复制；成功只额外消费互不重叠的当前表头，失败不重放，实际计数满足 `inline_bytes <= source().len() + table_bytes`，累计 O(n log n)、空间 O(n)。确认/回退仍不承诺单次硬预算。

容器内数学现沿用同一隔离/取消规则，Quote/Item 中的跨行内容读取见[容器跨行数学内容](#容器跨行数学内容)。`crates/mdstream-core/examples/math_headers.rs` 演示百万字成功/失败、稳定 MathId 与预算读取；`crates/mdstream-core/tests/math_headers.rs` 覆盖 338 组定界/转义模型、400 组表格范围、待定美元串、4096 个旧 opener/对象/列、千层容器边界、缓存/合并通知及共享预算，UTF-8 全切点和逐前缀快照在 `crates/mdstream-core/tests/chunking.rs` 验证。

```rust
use mdstream::{Document, MathId};
let mut doc = Document::new();
doc.append("$prior$ $$ old\n")?;
let prior = *doc.math(MathId(0)).unwrap();
doc.append(&format!("$cell$ {}", "x".repeat(2048)))?;
assert_eq!(doc.maths().len(), 1); // The inherited math remains opaque.
doc.append("|$tail$\n-|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(doc.maths().len(), 3); // Cell objects publish on confirmation.
assert_eq!(*doc.math(MathId(0)).unwrap(), prior);
# Ok::<(), Box<dyn std::error::Error>>(())
```

**可隔离的已闭合数学**：顶层或同一显式 Quote/Item 续行中，继承 `$` / `$$` 在首管道前闭合后，可以继续探测只含普通文本/空白的长尾。闭合后遇到 `&`、`<`、反斜线、`*`、`_`、`~`、反引号、`$`、`!`、`[` 或 `]` 即取消；其它标点和 Unicode 文字允许。这是保守的语法起始字节边界，即使后来的语法最终失败也不恢复探测。lazy、部分 tab 表头、分类窗口与块优先级沿用原规则。

数学 arena 是按源码顺序追加的稳定对象集合，不能先分配跨行 MathId，再在表头成功时删除。因此匹配美元串时仅保存一对 open/close 端点，不发布 MATH 样式、定界符隐藏、MathId 或 `new_maths`。长尾继续逐字消费；原文预览保持。首管道前待定美元串仍不强制结算；普通短行和早管道路径不变。

确认表格时丢弃隔离端点及旧 Inline，在现有单元格扫描中独立解析当前行；新对象只发布一次，历史 MathId、LinkId、脚注编号和叶块身份保持。失败、无管道 LF/EOF、Setext 或作用域退出时只提交原公式一次，再消费未处理的后缀；语法字节取消必须在处理该字节前提交，以保证同一次 feed 中旧公式先于后来的对象发布。提交沿用正常范围失效，拒绝旧测量、显示游标和搜索任务。

额外状态为 O(1) 的两个源码区间；提交/丢弃各计一次 `math_steps`，美元串扫描仍照常计费。原容器前缀间隙、部分 tab 映射直接复用，闭合不扫描或改写历史映射。成功只额外消费当前表头，失败不回放，保持 `inline_bytes <= source().len() + table_bytes`、累计 O(n log n)/空间 O(n)；解析仍不承诺单次硬预算，数学读取和表格预览继续共享 Budget。

```rust
use mdstream::{Document, MathId};
let mut doc = Document::new();
doc.append("> $prior$ $$ old\n")?;
let original = *doc.math(MathId(0)).unwrap();
let update = doc.append(&format!("> $cell$ end$$ {}", "x".repeat(2048)))?;
assert!(update.new_maths.is_empty()); // No tentative cross-line MathId.
doc.append("|$tail$\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(doc.maths().len(), 3);
assert_eq!(*doc.math(MathId(0)).unwrap(), original);
assert_eq!(doc.text(doc.math(MathId(1)).unwrap().content), Some("cell"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`crates/mdstream-core/tests/closed_math_headers.rs` 覆盖独立单元格语义、400 组范围模型、语法后缀取消、失败/EOF/容器边界、共享预算映射与通知失效；UTF-8 全切点/逐前缀见 `crates/mdstream-core/tests/chunking.rs`。千层/百万字和 4096 历史对象/旧 opener/列验证增量工作，内部测试验证一个端点对在 200 轮判定后释放。`crates/mdstream-core/examples/closed_math_headers.rs` 演示成功丢弃/失败提交与预算预览。闭合后带其它行内语法、通用已发布对象/样式撤销仍留 TODO。

**未闭合链接/图片标签的探测**：顶层段落或同容器正文继承 `[...` 或 `![...` 时记录旧左括号的源码位置，继续即时消费原标签及本行语义。沿用单括号候选方言：新 `[` 替换旧标签，`<` 丢弃标签并尝试自动链接；这些操作不发布旧标签对象。确认时只清除仍与记录位置相同的旧标签，保留当前行新标签、待定后缀、强调/代码/数学以及已经发布的 LinkId/MathId/脚注编号；首单元格前缀直接复用。旧图片的 `!` 保持原文，不能作用到新行标签或后续单元格。

如果首管道前的未转义 `]` 关闭了旧标签，按下文隔离 shortcut 对象及引用副作用；普通字符显式后缀也可等待判定。其余后缀和新行内语法先提交原 shortcut 再取消探测，取消后不恢复探测。当前行新标签的闭合不触发取消。未闭合标签可伴随旧强调或代码/数学；旧强调、代码或数学闭合遵循上述隔离/取消规则；继承代码/数学的成功路径仍独立消费当前表头。

失败、无管道、EOF 或块中断沿用原候选和未消费后缀，前序换行只提交一次；不回扫标签、不重新发布对象、不重建符号索引。标签来源仅一个固定大小偏移，普通路径每个字节消费一次；仅叠加继承代码/数学时沿用其已证明的一次当前表头重读。累计 O(n log n)、空间 O(n)，确认/回退不承诺单次硬预算。容器跨行标签现沿用同一隔离/取消规则，去前缀表示见[容器跨行链接与图片标签](#容器跨行链接与图片标签)；闭合 shortcut 的独立子项见下文；通用跨行语义撤销及多行目标/title 继续保留 TODO。

```rust
use mdstream::{Document, LinkId};
let mut doc = Document::new();
doc.append("[prior](u) ![old\n")?;
let prior = *doc.link_info(LinkId(0)).unwrap();
doc.append(&format!("**cell** {}", "x".repeat(2048)))?;
doc.append("|end](image) [tail](w)\n-|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(doc.links().len(), 2); // The old image label cannot cross into a cell.
assert_eq!(doc.link_destination(LinkId(1)), Some("w"));
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), prior);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`crates/mdstream-core/examples/link_headers.rs` 演示百万字成功/失败、稳定 LinkId 与共享预算摘要；`crates/mdstream-core/tests/link_headers.rs` 覆盖 200 组替换/转义模型、400 组表格范围、代码/数学组合、千层容器边界、百万字、4096 个旧 opener/对象/列、缓存及合并通知。UTF-8 全切点和逐标量中间快照见 `crates/mdstream-core/tests/chunking.rs`。

**旧链接/图片 shortcut 可隔离闭合**：同一顶层段落或显式容器作用域继承的 `[...` / `![...` 在首管道前闭合时，先保存括号端点和图片起点，复用现有至多 `MAX_LABEL_BYTES + 1` 字节的去前缀标签缓冲。此时不分配 LinkId、不登记 SymbolId、不更新 `first_use`，也不发布跨行链接标记或隐藏括号；已经解析的标签内强调、代码、数学、实体等局部语义继续可见。成功确认表格时丢弃暂存 shortcut，并复用当前首单元格状态；失败、无管道到行末、EOF 或作用域退出时规范化有界标签并提交一次普通 shortcut，保留原始源码包络、去前缀显示及既有局部对象 ID。空白/超长标签仍不创建引用，但同样保留原方言的直接目标回退能力。

本子项允许闭合后到首管道间的普通文本、空白及不启动语法的标点。紧接 `]` 的 `(` 或 `[` 可进入下文的普通字符后缀子项。其它后缀取消隔离并恢复原 `Tail`，直接目标、full/collapsed 引用和非法/未完成后缀继续使用原对象 ID；已取消且后缀尚未完成时保留 Provisional 状态。其它 `&`、`<`、反斜线、星号、下划线、波浪线、反引号、美元、`!`、`[`、`]` 起始字节也先提交并取消，该行不重试。括号与标签内已发布语义无需撤销；新行内序列、短表头、早管道、lazy 行、部分 tab 的当前表头分类及块边界规则保持原方言。与继承代码/数学组合时，它们的闭合后语法限制仍适用。

```rust
use mdstream::{Document, LinkId};
let mut doc = Document::new();
doc.append("> [prior](u) ![old\n")?;
let prior = *doc.link_info(LinkId(0)).unwrap();
let symbols = doc.references().to_vec();
doc.append(&format!("> **cell** end] {}", "x".repeat(2048)))?;
assert_eq!(doc.links().len(), 1);
assert_eq!(doc.references(), symbols); // No tentative shortcut symbol/first_use.
doc.append("|[tail](v)\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(doc.links().len(), 2);
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), prior);
assert_eq!(doc.link_destination(LinkId(1)), Some("v"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

额外状态只有一个固定大小候选；标签缓冲在已有上限内，长尾不扫描/复制旧标签、局部样式或祖先。成功/失败各只处理该候选一次（计入 `delimiter_steps`），失败的规范化字节计入 `label_bytes`；引用仍按 BTreeMap 间接寻址，晚定义不遍历旧引用。当前行已消费前缀不重读，累计 O(n log n)、空间 O(n)，不承诺单次判定硬时延。`crates/mdstream-core/tests/closed_link_headers.rs` 验证早管道语义、400 组范围模型、边界/预算/缓存和引用副作用；全切点与逐前缀快照见 `crates/mdstream-core/tests/chunking.rs`。千层/百万字、两万旧引用和 4096 个历史对象/旧 opener/列验证计数；内部测试检查 200 轮判定不积累候选或标签存储。`crates/mdstream-core/examples/closed_link_headers.rs` 演示成功丢弃、失败晚定义与预算预览。普通字符显式后缀见下文；其它行内语法及通用已发布对象/引用副作用撤销继续保留 TODO。

**旧链接/图片的普通字符显式后缀**：在上述可隔离旧标签后，直接裸目标及可选 title、full `[key]` 和 collapsed `[]` 后缀也可参与晚管道识别。目标沿用已有单物理行、平衡括号与长度规则，例如 `(path?q=1#part '标题')`、`(a(b)c)`、`()`；full 引用沿用 999 字节上限和大小写/空白规范化。目标、title 和引用标签中只允许不启动独立单元格语法的字符；语法起始字节 `&`、`<`、反斜线、星号、下划线、波浪线、反引号、美元、`!` 和非定界方括号先取消探测。引用后缀内的 `^` 也取消；已完成引用后缀到首管道之间的 `(` 保守取消，以免形成另一种单元格链接。尖括号目标、转义目标和含这些字符的 title/引用仍正确作为普通正文解析，只是不作为本子项的晚管道表头。闭合后的其它新行内语法、lazy/部分 tab 当前表头和块优先级规则保持既有边界。

候选阶段复用原 `Destination`/`Tail` 的单遍状态，只额外保存一组完成后缀的源码端点，不复制目标、不分配 LinkId、不登记符号或更改 `first_use`。成功时丢弃旧跨行链接：`(url)` 与 `[]` 是单元格的字面正文，非空 `[key]` 按独立 shortcut 创建自己的 LinkId、源码映射和引用符号，空白标签仍不创建对象。这一局部引用在确认时、后续单元格对象之前发布；无需重读长首单元格。若首管道位于尚未完成的普通后缀内，单元格边界丢弃未完成候选，仍遵循表格管道优先规则。

失败、EOF、无管道行末、Setext 或作用域退出先提交原 shortcut，再应用已完成后缀，保持既有符号创建顺序和首引用副作用（直接目标仍可留下原 shortcut 符号）。full 引用独立登记目标标签；collapsed 复用已去前缀的原标签。非法/超限或不支持的后缀在拒绝字节前取消隔离，恢复原 parser 位置与同一 LinkId，后续完成只更新该对象。普通长尾继续即时消费；已解析的局部样式、代码/数学对象与早先链接保持身份。原始标签包络仍包含容器前缀/部分 tab，显示读取沿用已有映射。

```rust
use mdstream::{Document, LinkId, LinkKind};
let mut doc = Document::new();
doc.append("> [prior](u) ![old\n")?;
let prior = *doc.link_info(LinkId(0)).unwrap();
let symbols = doc.references().to_vec();
doc.append(&format!("> end][KEY] {}", "x".repeat(2048)))?;
assert_eq!(doc.links().len(), 1);
assert_eq!(doc.references(), symbols);
doc.append("|b\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(doc.link_info(LinkId(0)), Some(&prior));
let local = doc.link_info(LinkId(1)).unwrap();
assert_eq!(local.kind, LinkKind::Link);
assert_eq!(doc.text(local.label), Some("KEY"));
assert!(!doc.references().iter().any(|r| r.label == "old end"));
doc.append("\n[key]: /late\n")?;
assert_eq!(doc.link_destination(LinkId(1)), Some("/late"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

后缀每个字节只进入原 parser 一次；成功/失败至多规范化两个有上限的标签，计入 `label_bytes`，候选决议计入 `delimiter_steps`。额外状态固定大小，百万字普通长尾不增加标签容量、不扫描祖先或旧对象；累计 O(n log n)、空间 O(n)，不承诺单次判定硬预算。`crates/mdstream-core/tests/link_suffix_headers.rs` 覆盖独立早管道对照、400 组范围模型、长度/失败边界、千层/百万字、两万旧引用、4096 对象/旧 opener/列、预算及失效；`crates/mdstream-core/tests/chunking.rs` 验证 UTF-8 全切点和逐前缀快照，内部测试验证 200 轮决议释放状态。`crates/mdstream-core/examples/link_suffix_headers.rs` 演示 full 引用的两种解释、稳定对象和预算预览。含行内语法字符的后缀、闭合后其它语法、多行目标/title、通用对象/符号撤销继续保留 TODO。

`crates/mdstream-core/examples/inherited_headers.rs` 演示前序硬换行隔离、百万字即时消费、失败恢复与成功摘要；`crates/mdstream-core/tests/inherited_headers.rs` 对照全延迟表头语义，覆盖旧对象与块身份、软硬换行、失败/EOF/Setext、原候选不回归、千层/百万字、4096 对象/列、400 组范围模型和共享预算。

`crates/mdstream-core/examples/long_headers.rs` 演示百万字长表头的迟确认与共享 Budget 摘要。测试覆盖 117649 组摘要语法对照、全 UTF-8 切点/逐前缀、千层引用和 250 层列表、百万字成功/失败、4096 列稀疏表、400 组范围模型及布局/折叠组合。

**长分隔行的流式规则**：仅紧邻有效候选表头时启用（包含上述长表头）。到达分类上限先只读匹配已有容器路径，再把分隔行前缀喂给状态机；没有完整路径、兄弟条目、新嵌套前缀或 lazy 行不进入探测。单元格的连字符、首尾冒号及 ASCII 空白可以任意长，首个管道也可以在窗口之后；保留一个单元格状态和至多表头列数个对齐值，不复制候选源码。短行保持原有优先级。长行在 LF 或 EOF 才检查最终列数并发布表格；CRLF 跨分块有效。探测期间表头和分隔行原文可见，分隔行尚无块/表对象，容器与章节的已解析末端保持在表头；确认后一次更新范围并隐藏整条分隔行。

**失败与复杂度**：非法字节、空单元格、多余列或 EOF 列数不足只触发一次回退。块分类只重用原来最多 1 KiB 的前缀及不完整行规则，不把长尾重新当成完整 Setext、定义、容器标记或新表头；延迟的长尾交给普通流式路径一次，此后只消费新增字节。`table_candidate_bytes` 对每个探测字节最多计费一次；成功不重读长分隔行，失败的长尾只有一次普通消费。这里有界的是分类回退窗口和重试次数，**不是失败调用的硬字节/时延上限**：单次晚失败可能消费整个延迟尾部，累计仍 O(n log n)、空间 O(n)。无法隔离的旧强调/代码/数学闭合、旧链接/图片标签闭合后带含行内语法字符的显式后缀或新行内语法的晚管道表头，以及其它块分类上限继续留在 TODO。

```rust
use mdstream::{Document, TableId};
let mut doc = Document::new();
doc.append("> a|b\n> |")?;
doc.append(&"-".repeat(4096))?;
assert!(doc.tables().is_empty());
let update = doc.append("|-|\n")?;
assert_eq!(update.new_tables, 0..1);
assert!(doc.table(TableId(0)).unwrap().delimiter.len() > 4096);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`crates/mdstream-core/examples/long_delimiters.rs` 演示百万字分隔行以及共享 Budget 的表头/正文摘要；摘要按行索引直接跳过分隔行，旧布局、折叠与合并 Update 的失效语义保持。

`TableRow::complete` 区分完整行与正在增长的行。当前行的最后一个空单元格可能在确认尾部竖线后被移除；先前完整行保持稳定。`source_width_hint` 是已完成单元格的最大原文字节数，单调增长，**不是终端字符宽度**。渲染器需要结合窗口预算、字体/Unicode 宽度、padding 和内容换行决定最终列宽；列宽提示不要求读取旧单元格。提示变大时，`Update.tables_changed` 通知具体表格，同时 `dirty` 覆盖该表范围，旧高度/检查点按现有接口惰性失效。

进入表格视口时先定位行，再读取可见列的单元格；超长单元格仍按片段和检查点恢复。表格一行的显示高度由各单元格排版高度的最大值决定，渲染器应约定显示行归属于哪个源码片段，把总行高回填一次，不能把每列高度相加。标题行、边框、横向滚动、列宽冻结/修正策略属于后续渲染器工作，见 [TODO](TODO.md)。

任务列表通过 `BlockKind::ListItem { task: Some(TaskMarker { checked, source }), .. }` 提供状态；识别列表项开头的 `[ ]`、`[x]`、`[X]`，标记后需要空白或行结束。任务标记不会成为引用链接，不会污染符号表。渲染器据此绘制复选框；当前解析器不修改原文中的勾选状态。

图片复用链接目标解析，但 `link_info(LinkId)` 的 `LinkKind::Image` 与普通链接明确区分，保留 alt 区间和完整语法区间。支持直接目标、引用式图片、延迟定义。可以先把 alt 渲染成链接；空 alt 图片也会保留对象，用 `links_in(visible_span)` 在 O(log L + 结果数) 内查询，避免因没有可见 label run 而遗漏。图片解码和绘制未实现。

链接和图片支持裸目标、`<含空格的目标>`，以及双引号、单引号、括号包围的 title；目标和 title 之间需要分隔空白，根普通段落中可含一个 LF/CRLF，其它行内上下文仍只接受空格或 tab。`LinkInfo.title` 保留直接链接的 title 区间；`link_title(id)` 同时处理直接链接和延迟定义的引用 title。直接链接和引用定义共用单遍状态机，原始接口返回的字符串仍保留反斜线转义，不分配解码副本；title 的可选预算解码见[链接及图片 title 解码读取](#链接及图片-title-解码读取)，目标解码见[链接及图片目标解码读取](#链接及图片目标解码读取)。规则参考 [CommonMark 链接](https://spec.commonmark.org/0.31.2/#links)。目标正文保持单物理行；直接 title 正文仅在实际根普通段落允许 LF/CRLF，见[根 title 正文](#根段落直接链接与图片的多行-title-正文)；根段落的 `(` 后、完整目标后、完整 title 后三个组件空白各可含至多一个物理行尾，详见[后缀组件换行](#根段落行内链接后缀的组件换行)。容器、标题和表格单元格的后缀不启用该扩展。

`Style::SOFT_BREAK` / `Style::HARD_BREAK` 标记继续同一行内序列的 LF，包括同一容器的显式正文续行。两个以上行尾空格或一个未转义的反斜线产生硬换行，普通换行为软换行；CRLF 跨 append 也有效。为避免把块末换行误判为行内换行，直到后续同序列内容到达才确认，并通过 `dirty` 使旧片段失效；序列末和代码内部不加这两种标记。继续行前的尾部空格及硬换行反斜线隐藏，源码仍可寻址。

```rust
use mdstream::{Document, LinkId, Span, Style};
let mut doc = Document::new();
doc.append("[guide](<docs/my guide.md> 'Guide')  \nnext\n")?;
doc.finish();
assert_eq!(doc.link_title(LinkId(0)), Some("Guide"));
assert!(doc.runs_in(Span::new(0, doc.source().len())).unwrap()
    .any(|run| run.style.contains(Style::HARD_BREAK)));
# Ok::<(), Box<dyn std::error::Error>>(())
```

## ATX 空白与关闭尾缀

顶层与已支持显式容器内的 ATX 标题遵循 [CommonMark 0.31.2 的原始内容规则](https://spec.commonmark.org/0.31.2/#atx-headings)：开头 1–6 个井号之后必须是 space/tab 或行结束；解析行内语义前去除原始正文首尾的 space/tab。末尾连续井号仅在前面是 space/tab（或标题正文为空）、后面只剩 space/tab 时作为关闭序列，同时去除其前面的空白。`foo#`、转义井号、`foo ### b` 保留；`foo ### ###` 只去掉最后一组。Unicode 空白、form feed 不是这些定界空白；实体生成的空格、代码 span 内部空白保持其已有显示语义。

短行与超过 1 KiB 分类窗口的已识别标题共用状态机。只保留正文首尾、候选井号串末端和待确认 CR 的源码位置；每个新增字节计一次 `Work.atx_bytes`，不缓存整行、不回扫正文或容器祖先。候选失败时将尚未消费的区间送入行内解析一次；LF/EOF 成功确认时仅标记隐藏尾缀并收缩同一个 `Heading.content`，不读取该尾缀。百万字尾缀的失败可能在一次 append 内消费百万字；这是累计 O(n log n)/空间 O(n) 的摊还契约，不是单次调用硬预算。额外候选状态为 O(1)。

开放标题中的可能尾缀暂时按原始内容可见，相关行内结尾也可能尚未定型；`Heading.content` 在 LF/EOF 前包含该尾部，持续前导空白则随到达隐藏。确认后保留原始源码、BlockId/HeadingId 和既有行内对象，使用正常 Update 失效范围通知消费者；旧 revision 的读取、搜索和 slug 任务必须重建。CRLF 可以跨追加，物理 LF 仍可见；沿用库的 LF/CRLF 行方言，孤立 CR 仍属正文。首行块分类的有界前缀及其它容器规则没有扩展。

```rust
use mdstream::{Document, Style};
let mut doc = Document::new();
doc.append("# **title**")?;
doc.append(&" ".repeat(2048))?;
doc.append("###\r")?;
doc.append("\n")?;
doc.finish();
let heading = doc.headings().entries()[0];
assert_eq!(doc.text(heading.content), Some("**title**"));
let text: String = doc.display_runs_in(heading.content).unwrap()
    .filter(|run| !run.style.contains(Style::HIDDEN))
    .map(|run| run.text).collect();
assert_eq!(text, "title");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`crates/mdstream-core/tests/atx_whitespace.rs` 提供 1600 组独立原始区间模型、长前导/尾部空白与井号、千层/百万字及 4096 历史标题验收，并覆盖源码映射、共享预算、搜索/slug 和缓存失效。完整分块快照见 `crates/mdstream-core/tests/chunking.rs`；官方 18 例的独立评分见文末标准样例说明。

## Setext 首尾空白

顶层与已支持容器内的 Setext 标题按 [CommonMark 0.31.2 原始内容规则](https://spec.commonmark.org/0.31.2/#setext-headings)裁剪整段原文首尾的 space/tab。内部换行、空白、代码/数学/链接及实体保持既有行内语义；顶层物理行的前导 ASCII 空格已按[段落行首规则](#顶层段落行首缩进)隐藏，容器内部缩进仍保留。实体产生的空格不属于原始尾部空白。`Heading.content` 收缩到裁剪后的源码包络，仍包含内部物理行的容器前缀；显示读取隐藏这些前缀和两端空白。`Heading.source`、物理叶块及已发布对象身份保持，标题正文末行的物理 LF 仍可见。沿用 LF/CRLF 方言，孤立 CR、Unicode 空白和 form feed 不作为可裁剪 space/tab。

普通正文仍按既有规则流式发布；下划线在 LF/EOF 完整确认时才应用 Setext 裁剪。失败或过长下划线保留原文，空行/块/容器边界重置候选；不改变普通段落的空白规则。原文非空白端点与待定 CR 直接在既有行内扫描中维护（计入 `Work.inline_bytes`），只增加 O(1) 状态。确认只读端点、写至多两个隐藏区间，并使用原有整标题结构失效；不回扫正文、内部叶块、历史对象或祖先，保持原有摊还复杂度。末行换行语义尚未提交，裁剪不会撤销内部已确认软/硬换行。

```rust
use mdstream::{Document, Style};
let mut doc = Document::new();
doc.append(">   Foo *bar\r\n> baz*\t\r\n")?;
assert!(doc.headings().entries().is_empty());
let before = doc.work();
doc.append("> ===")?;
doc.finish();
assert_eq!(doc.work().inline_bytes, before.inline_bytes);
let heading = doc.headings().entries()[0];
assert_eq!(doc.text(heading.content), Some("Foo *bar\r\n> baz*"));
let text: String = doc.display_runs_in(heading.content).unwrap()
    .filter(|run| !run.style.contains(Style::HIDDEN))
    .map(|run| run.text).collect();
assert_eq!(text, "Foo bar\nbaz");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`crates/mdstream-core/tests/setext_whitespace.rs` 和 `crates/mdstream-core/tests/chunking.rs` 覆盖 1200 组独立原始区间模型、UTF-8 全切点/逐前缀、百万字正文/尾部空白及千层容器、4096 历史对象与物理叶块、表头失败/成功边界、源码映射/共享预算、搜索/slug、合并通知及旧测量/任务拒绝。Setext 下划线的分类窗口、完整缩进/空行与部分 tab 规则仍受既有方言限制；官方章节评分见文末。

## 标题索引与导航

`doc.headings()` 提供文档顺序的 `HeadingIndex`。每个 `Heading` 包含稳定 ID 对应的块、容器作用域、级别、整个标题的源码区间和不含标题标记的内容区间。标题文字不复制；超长 Setext 标题也只保存区间。`at_level(level)` 直接返回该级别的 ID 列表；`next(offset, max_level)` / `previous(...)` 查找严格位于指定源码位置之后/之前、级别不超过上限的标题。`Update.new_headings` 给出新加入的 ID 范围。长 ATX 标题的区间会随追加更新；全局索引包含上述方言支持的容器内标题，结构关系遵循确切容器作用域。

```rust
use mdstream::{Document, HeadingId, HeightIndex, NavAction, Navigator};
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

章节/代码块折叠、隐藏目标定位、源码锚点恢复、文内 slug 及跳转历史见下方接口。后续的目录及更多折叠对象见 [标题导航与折叠联动规划](TODO.md#标题导航与折叠联动规划)及[折叠支持规划](TODO.md#折叠支持规划)。

## 标题显示名称检索

`HeadingSearch::new(&doc, query, max_level)` 只搜索标题索引中各标题 `content` 的非 `HIDDEN` 显示文本，复用实体解码、代码 span 规范化和源码映射。`max_level` 与标题导航一致：选择级别不超过上限的标题，大于 6 按 6 处理，0 不选择标题。空查询返回 `InvalidRange`。匹配区分大小写，保留重叠命中，不做额外空白处理、Unicode 归一化或模糊匹配；标题之间独立匹配，不串接名称，包含已识别的容器内标题，不读取正文或代码块。

```rust
use mdstream::{Budget, Completion, Document, HeadingId, HeadingSearch, WorkLimit};
let mut doc = Document::new();
doc.append("# a**b**&amp;c\nbody-only\n## ` ab&c `\n### ab&c\n")?;
doc.finish();
let mut search = HeadingSearch::new(&doc, "ab&c", 2)?;
let mut matches = Vec::new();
loop {
    let mut budget = Budget::new(WorkLimit {
        bytes: 16, queries: 2, objects: 2, ..Default::default()
    });
    if search.advance(&doc, &mut budget, &mut matches)? == Completion::EndOfDocument {
        break;
    }
}
assert_eq!(matches.len(), 2);
assert_eq!(matches[0].heading, HeadingId(0));
assert_eq!(matches[0].display, 0..4);
assert_eq!(doc.text(matches[0].source), Some("a**b**&amp;c"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`HeadingMatch` 每项表示一次出现，按标题及匹配位置排序；重复名称由 `HeadingId` 区分，同一标题可返回多项。`display` 是该标题显示内容内的 UTF-8 字节范围，`source` 是绝对源码包络，端点来自实体等替代内容时包含完整原始区间。调用方可按 ID 生成选择器条目，用显示范围高亮，并通过 `NavAction::Heading(hit.heading)` 定位标题或 `Source(hit.source.start)` 定位命中。搜索包含被折叠隐藏的标题，不改变折叠状态；选中结果后可交给 `FoldNavigation::Reveal` 展开必要路径。

候选查找先消费 1 query / 1 object，通过最多六个级别索引的二分查询跳过被过滤的标题；没有后续候选的查询也计费。每个标题的读取、KMP 比较/回退及命中输出沿用 `DisplaySearch` 的字节/query/object 计费，整个过程共用调用方 Budget 和结果 Vec，每轮 6 bytes、1 query、1 object 可持续推进。`current_heading()` / `source_position()` 返回当前标题及其已读源码位置，读取可能领先于匹配；标题之间或完成后返回 `None`。完成后再次调用不重复输出。

任务与已返回命中绑定文档身份及 revision。追加、Setext 晚确认、长 ATX 继续增长和首次 `finish` 后，旧任务在消费预算/修改结果前返回 `StaleContinuation`；错误文档返回 `WrongDocument`。调用方丢弃旧命中并按需重建，库不自动重搜历史标题。空追加及重复 `finish` 不失效；仍在流入的文档也能搜索当前 revision。

查询预处理一次 O(m)，任务存储 O(m + TILE_BYTES)，m 为查询字节数；切换标题只重置位置，不重建查询表、清空映射环或复制标题。设 H 为全部标题数、h 为选中标题数、s 为其内容源码字节数、n 为文档总字节数，单 revision 最坏搜索成本为 O(m + (h + 1) log(H + 1) + s log(n + 1))，输出存储由调用方逐批管理。百万字 Setext 的确认和任务构造不读取标题，扫描在显式预算调用中推进；被过滤标题和正文的长度不增加读取字节。完整显示名称可用已有 `display_cursor(heading.content)` 限量读取，不先生成所有名称。文内 slug 和 `#fragment` 解析见下节。

`cargo run -p mdstream-core -r --example heading_search` 演示级别过滤、逐帧搜索、复用输出缓冲区及跳到折叠内命中。`crates/mdstream-core/tests/heading_search.rs` 覆盖重复/空标题、跨 run/tile 和长查询、各预算维度、身份/过期、流式前缀/切点、固定种子朴素模型、百万字正文/Setext 和级别过滤跳过测试。

## 文内 slug 与 fragment

`HeadingAnchors::new(&doc)` 在 O(1) 时间创建当前 revision 的空索引；`advance` 按共享 Budget 读取已索引标题的非隐藏显示内容，包括容器内标题，忽略正文和代码块。不会在 `append`、Setext 确认或构造任务时扫描旧标题。`indexed_headings()` 报告完成数量，`get(&doc, id)` 为已完成项返回借用的 `HeadingAnchor`，未处理的合法 ID 返回 `None`，非法 ID 返回 `InvalidRange`。

slug 方言明确采用以下规则，不宣称兼容某个平台的完整 slug 算法：Unicode 字母/数字用 Rust `char::is_alphanumeric` 选择并用 `to_lowercase` 小写化；保留 `_`、`-`；每个 Unicode 空白标量变成一个 `-`；删除其余字符。不额外裁剪或合并空白，不做 Unicode 归一化、音译或 emoji 保留；小写映射产生的组合字符保留。输入先经过既有显示语义，因此隐藏标记/链接目标不参与，实体和代码 span 使用显示值。空结果用 `section`。按源码顺序分配全局唯一名称，依次尝试 base、base-1、base-2 等，跳过已经占用的名称。例如 `A`、`A-1`、`A`、`A-1` 分别得到 `a`、`a-1`、`a-2`、`a-1-1`；空标题也参与同一消歧规则。

```rust
use mdstream::{Budget, Completion, Document, FragmentStep, HeadingAnchors, HeadingId, WorkLimit};
let mut doc = Document::new();
doc.append("# Guide\n## 中文\n## 中文\n")?;
doc.finish();
let mut anchors = HeadingAnchors::new(&doc);
loop {
    let mut budget = Budget::new(WorkLimit {
        bytes: 6, queries: 1, objects: 1, ..Default::default()
    });
    if anchors.advance(&doc, &mut budget)? == Completion::EndOfDocument { break; }
}
let anchor = anchors.get(&doc, HeadingId(2))?.unwrap();
assert_eq!(anchor.slug.base, "中文");
assert_eq!(anchor.slug.suffix, Some(1));
let mut lookup = anchors.fragment("#%E4%B8%AD%E6%96%87-1")?;
loop {
    let mut budget = Budget::new(WorkLimit {
        bytes: 6, queries: 1, objects: 1, ..Default::default()
    });
    match lookup.advance(&doc, &mut budget)? {
        FragmentStep::BudgetExhausted => continue,
        FragmentStep::Resolved(target) => {
            assert_eq!(target.heading, Some(HeadingId(2)));
            assert_eq!(target.source, anchor.target.source);
            assert!(!target.provisional);
            break;
        }
        other => panic!("unexpected resolution: {other:?}"),
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

`fragment` 只接收以 `#` 开头的 URI fragment，O(1) 借用输入和索引；完整 URL 的拆分由调用方完成；Markdown destination 的实体/反斜杠可先用 `decoded_link_destination` 按预算解码。解析百分号转义一次，校验解码后的 UTF-8，然后按字节精确匹配；`+` 保持字面值，大小写敏感，不再对 fragment 生成 slug。非法转义、非法 UTF-8 或缺少开头 `#` 返回 `InvalidRange`；即使已知名称不存在，也继续按预算校验剩余编码。`#` 表示源码 0，`heading: None`；空标题通过 `#section` 等名字寻址。未完成索引返回 `IndexIncomplete`，调用方释放 lookup、推进索引后重新查询；完整索引中的缺失目标在流式文档返回 `Unresolved`，在 finish 后返回 `NotFound`。完成的查询可免费重复读取。

标题 slug 和目标在首次 `finish` 前一律标为 `provisional`，包括已经局部封口的标题：长 ATX 继续增长、行内晚闭合、晚到引用定义和 Setext 转换都可能改变显示文本及消歧结果。索引、lookup 和输出绑定文档身份/revision，追加或首次 finish 后须丢弃并显式重建；错误文档/过期任务在扣费前返回 `WrongDocument` / `StaleContinuation`，空追加和重复 finish 不失效。finish 后结果固定。库不自动维护跨 revision 的 slug；反复显式重建的总成本由调用方调度，不能当作跨追加增量索引。稳定的 `HeadingId` 与源码目标可继续用于现有导航/书签，但旧 slug 到该目标的对应关系不能沿用。

标题初始化和消歧候选检查各消费 1 query / 1 object；显示读取沿用 `DisplayCursor` 计费（包含隐藏语法和重复前瞻），每个规范化标量另消费输入 UTF-8 字节数与 1 object，每个输出 trie 字节消费 1 byte / 1 object。fragment 每个原始字节或 `%HH` 分别消费 1 或 3 bytes、1 object，最终查询消费 1 query / 1 object；非法编码也计费。两类任务每帧 6 bytes、1 query、1 object 均可持续推进，长标题不会截断或退化成摘要哈希。目标包含精确标题起点，可交给 `FoldNavigation::Reveal` 展开必要路径；索引和查询自身不改变折叠、高度或布局。

索引以字节 trie 保存名字，每个节点至多 256 条有序边，避免长字符串比较和哈希碰撞；重复 base 的后缀计数单调推进，后缀候选从 base 节点开始，不重复遍历长前缀。设 S 为标题内容源码总字节、H 为标题数，单 revision 构建为 O((S + 1) log(n + 1) + H log(H + 1))，空间 O(S + H)。失败的数字候选已属于旧标题，且由唯一的 base 产生，因此总失败探测 O(H)；连续数字的 trie 前缀共享，后缀总节点 O(H)。`AnchorSlug` 只保存借用的 base 和整数后缀；显式 `Display`/`to_string()` 由调用方承担 O(slug 长度) 的格式化成本。借用结果 O(1)，长度 m 的 fragment 查询总计 O(m)、任务空间 O(1)。String/Vec/树节点的分配成本按摊还计，不承诺 allocator 的硬时延。

`cargo run -p mdstream-core -r --example anchors` 演示小预算索引、碰撞、晚到中文目标、百分号 fragment 和折叠导航。测试覆盖所有 UTF-8 单切点、长 ATX 逐前缀、显示语义、Unicode、错误身份/过期与各预算维度，包含固定种子模型、百万字 Setext/长 fragment 和 4 万标题消歧的工作量与存储检查。链接点击和目录 UI 继续留待接入阶段。

## 共享预算与增量缓存

`Budget` 同时限制查询次数、tile 次数、实际交付的源码字节、语义对象、恢复步骤和依赖检查。一次帧内所有工作共用同一个实例；新建 Budget 表示开始新的帧或显式后台任务。原有 `viewport` / `runs` 是底层无整帧约束的接口，接入方不能将它们与预算接口混用后宣称整帧有界。

`viewport_budgeted` / `resume_viewport` 复用输出 Vec，返回 `Covered`、`EndOfDocument` 或 `BudgetExhausted`。继续位置保存稳定 tile 锚点，并验证视图、文档 revision 和失效版本；高度校准后重新计算累计位置。`LayoutPass` 保留半个 tile 的字节/对象位置，下一帧不会重新扫描已完成前缀。`LayoutPass::from_checkpoint` 限制恢复源码距离，每个恢复步骤也计入预算。无附近上下文时，从局部起点排版并标记为 `Local`；未知前缀的绝对行号仍是估计。

一个不可暂停的外部回调不会因为传入 Budget 就自动获得时间上界。`LayoutSink` 每次收到的文本和对象有界；昂贵的图片解码、字体排版或用户析构需另外拆成可暂停任务。`crates/mdstream-core/examples/budget.rs` 展示 UTF-8 字节输入、Update 合并以及跨帧推进；其中按 Unicode 标量计列，仅验证消费协议，不是终端宽度或字素排版实现。

`HeightState` 区分 `Measured`、`Local`、`Estimated`、`Expired`。`expire` 用区间操作保留旧的非零高度，并将旧零高度替换为正估计，恢复可检索性；`invalidate` 保留原来统一重置估计的低层语义。默认 `apply` 从最早可能改变尺寸的位置过期后续布局，不扫描屏外内容。渲染器确认引用目标、样式或表格宽度提示不影响尺寸时，可用 `LayoutPolicy` 保留对应测量；更换策略需重置布局。

普通 ticket 保守依赖当前 tile 之前的整个输入布局。`ticket_from` / `measure_from` 将依赖缩小到指定有效检查点，并验证该检查点的身份与版本；替换输入状态后用 `insert_for_layout` 同时过期后续高度和检查点。单次高度回填只更新当前 tile，不把后继 tile 逐个重排。

`DependencyKey` 针对直接语义依赖检查符号版本，检查数量计入 Budget；它不能替代传入排版状态的 ticket。`Document::snapshot` 显式复制受上限和字节预算约束的源码，未复制全 AST。`CheckpointIndex::drain_before` 按对象/查询预算移出旧状态，返回状态所有权；失效本身不批量执行用户析构。

### 检查点历史树压缩

`drain_before` 保留树节点和 arena 以便回填。需要归还历史树容量时，显式调用 `CheckpointIndex::into_compaction`，将缓存所有权移入 `CheckpointCompaction<S>`。任务先以 DFS 跳过失效子树，将有效叶状态移入新树，再逐节点退役旧 arena；失效 payload 追加到调用方的 `out`，由调用方选择析构时机，不要求 `S: Clone`。有效检查点的 token、版本、文档身份和已消费 revision 保持，失效 token 不会复活；完成后旧有效测量 ticket 仍按原有依赖规则验证。

```rust
use mdstream::{Budget, CheckpointIndex, Completion, TileId, WorkLimit};
let mut cache = CheckpointIndex::new();
for i in 0..128 { cache.insert(TileId(i * 97), vec![i as u8; 256]).unwrap(); }
let token = cache.token(TileId(0)).unwrap();
cache.invalidate(1..usize::MAX).unwrap();
let old_capacity = cache.arena_stats().capacity_bytes;
let mut task = cache.into_compaction();
let mut garbage = Vec::new();
loop {
    let mut budget = Budget::new(WorkLimit {
        objects: 31, queries: 31, ..Default::default()
    });
    let status = task.advance(&mut budget, &mut garbage);
    assert!(task.validates(token));
    assert_eq!(task.previous(TileId(usize::MAX)).unwrap().0, TileId(0));
    garbage.clear(); // 此处显式运行用户析构，不属于核心 Budget 的时间保证
    if status == Completion::Covered { break; }
}
assert_eq!(task.stats().source.capacity_bytes, 0);
let mut cache = match task.finish() {
    Ok(cache) => cache,
    Err(_) => panic!("advance 尚未完成"),
};
assert!(cache.arena_stats().capacity_bytes < old_capacity);
assert!(cache.validates(token));
cache.insert(TileId(1), vec![1; 256]).unwrap();
```

暂停期间 `len/is_empty/get/previous/token/validates` 查询两个树的并集，前驱仍为 O(log T)；没有可写的缓存别名。`finish` 未完成时返回原任务，可以继续分帧推进。持有任务即保留进度；直接丢弃任务等同于丢弃整个缓存，会运行剩余 payload 的析构。以下写入被所有权规则拒绝：

```compile_fail
use mdstream::{CheckpointIndex, TileId};
let mut cache = CheckpointIndex::<usize>::new();
let task = cache.into_compaction();
cache.insert(TileId(0), 1).unwrap(); // cache 已由 task 接管
```

每次步骤原子消费 1 object + 1 query：处理一个 DFS 节点（有效叶还含两次 O(log T) 的树写入），或退役一个旧节点；零/非对称预算不推进，`Covered` 后重复调用不再计费。N 个历史节点、L 个有效状态的整轮成本为 O(N + L log T)，任务 scratch 为 O(log T)，原有查询、失效与插入复杂度不变。没有逐字读取文档、复制 payload 或结束时整树析构；常规 Vec 扩容仍有摊销搬移成本，Budget 不是分配器的硬实时上界。

压缩会暂存新旧两份 arena，旧 arena 在最后一个节点退役时释放；它不能作为严格内存上限。完成后的容量随保留状态分布和 Vec 容量台阶变化，不承诺每种输入都节省空间。旧 `drain_before` 游标是 arena 下标，压缩后须置零。任务期间可继续解析 Document，但必须暂存更新，取回缓存后按序或合并 `apply`；漏通知仍需 `rebuild`。压缩不改变 HeightIndex，返回的缓存继续供原有 recovery/测量接口使用。测量、独立模型和边界见 [存储实验](docs/storage.md#检查点历史树压缩实验)；并发可写压缩、硬内存上限及总堆剖析仍未实现。

## 更新事件与源码交互

Update 携带文档身份及 `base_revision -> revision`。空追加、重复 EOF 和重复应用均为无副作用操作；漏更新、乱序、错误文档或视图返回明确错误。已消费更新之后的新测量不会被重复通知清掉。`merge` 移入新事件，累计成本随事件数增长；合并后的失效可比逐步处理更保守，但不遗漏失效。`append_into` / `finish_into` 与 `viewport_into` 保留 Vec 容量用于复用。

`changes` 将文本、隐藏性、样式、块结构、引用和表格宽度提示分开。`changed_blocks`、`changed_headings`、`changed_links` 是有界增量范围，允许额外包含最后一个没有变化的旧对象；`rows_changed` 报告表格行范围。`SyntaxState` 区分暂定、增长中、局部封口，标题的状态读取其所属块；封口不代表引用定义不会晚到。Update 对象事件用于对象状态缓存，TileKey 用于显示语义缓存。

`HeadingIndex` 提供 `current`、`parent`、`first_child`、`next_sibling`、`previous_sibling`、`section` / `section_end` 和严格同级前后查询，对应动作已接入 `Navigator`。跳级标题按最近更浅级别确定父节点，兄弟不等于同级。章节末尾可随追加增长，查询不读取标题文本；父子/兄弟和范围遵循上文容器作用域，`current` 退出容器后恢复外层。

`SourceSearch` 是按字节预算推进的固定查询 KMP 任务，支持跨追加和重叠匹配；失败回退也计入预算。更换查询新建任务，历史搜索不会隐式同步完成。`Selection` 绑定文档身份及源码区间，`TaskState` 保存独立的复选框覆盖状态，二者都不修改追加源码。这些接口为后续 UI 提供基础；显示文本搜索见下文，折叠联动另行实现。

## 显示文本与源码映射

`runs` / `runs_in` 保留逐字对应源码的文本；需要实体解码和代码 span 空白规范化时使用 `display_runs` / `display_runs_in`。两种读取都返回隐藏标记，显示或复制可见文本时过滤 `Style::HIDDEN`。普通文字直接借用原文，实体只存至多 8 个 UTF-8 字节的替代值；代码 span 保存端点和流式空白摘要，不复制或在闭合时回扫正文。

```rust
use mdstream::{Document, Span, Style};
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

需要共享整帧预算时，创建 `display_cursor(请求区间)`，反复调用 `DisplayCursor::next(&doc, &mut budget)`，处理 `Run`、`BudgetExhausted` 和 `Complete`。游标在耗尽后保存源码位置；文档 revision 改变后返回 `StaleContinuation`，需按源码锚点重建。每步预先消费一次 query、一个 object 和至多 `TILE_BYTES + 2` 源码字节的检查窗口，包含最多两个相邻 CRLF 字节；返回片段较短时仍保守计入整个检查窗口。预算为 1 query、1 object、6 bytes 即可容纳任意一个 UTF-8 标量并保证前进，替代输出最多 8 字节。调用方的排版回调成本仍需另外约束。可运行流程见 `crates/mdstream-core/examples/display.rs`；现有 `LayoutSink::text(RenderRun)` 仍接收源码文本。

命名引用支持固定的 2125 个 WHATWG 分号结尾名称（最长 31 字节），表在 `crates/mdstream-core/src/entity_names.rs`，附来源和输入 SHA-256；更新时用 `scripts/generate_entities.py` 读取另行下载的 [WHATWG 数据](https://html.spec.whatwg.org/entities.json)，构建和测试不联网。数字引用及代码 span 规范化依据 [CommonMark 0.31.2 字符引用](https://spec.commonmark.org/0.31.2/#entity-and-numeric-character-references)和[代码 span](https://spec.commonmark.org/0.31.2/#code-spans)：十进制 1..7 位、十六进制 1..6 位，必需分号；零值、代理码点及越界值替换为 U+FFFD。代码换行变为空格，首尾均为空格且并非全空格时各去掉一个；其它空白和内部连续空格保留。顶层段落物理行首缩进在行内解析前已隐藏，见[段落行首规则](#顶层段落行首缩进)。

实体失败、超长或未完成时保留原文，只按正常语法处理拒绝候选的当前字节；不会回扫候选。反斜线转义、代码块/span、自动链接地址、链接目标/title、引用标签匹配及 fence info 不在行内解析时做实体解码；info、title 与目标的独立预算读取接口见文末。未闭合代码 span 只恢复下述反引号配对，不重放实体或其它行内语法。晚确认通过 Text 变化和区间 revision 使旧显示缓存及布局过期，即使宿主关闭了样式影响尺寸的策略。

### 查询栈复用

高频窗口可使用 `QueryScratch::default()` 配合 `runs_in_with_scratch` / `display_runs_in_with_scratch`；按 tile 读取分别用 `runs_with_scratch` / `display_runs_with_scratch`。返回的 `RunsWithScratch` / `DisplayRunsWithScratch` 独占借用 scratch，耗尽后仍须 drop 才能再次借用；正常退出、提前停止及 panic unwind 都会归还容量。返回文本只借用文档，可在释放迭代器后继续使用。原有不带 scratch 的接口仍可用。

```rust
use mdstream::{Document, QueryScratch, Span};
let mut doc = Document::new();
doc.append("a **b** &amp;")?;
doc.finish();
let span = Span::new(0, doc.source().len());
let mut scratch = QueryScratch::default();
let runs: Vec<_> = doc.runs_in_with_scratch(span, &mut scratch).unwrap().collect();
let warm = scratch.stats();
let again: Vec<_> = doc.runs_in_with_scratch(span, &mut scratch).unwrap().collect();
assert_eq!(runs, again); // 已返回文本不占用 scratch 的借用。
assert_eq!(scratch.stats(), warm); // 相同查询不再分配栈。
let display: Vec<_> = doc.display_runs_in_with_scratch(span, &mut scratch).unwrap().collect();
assert!(display.iter().any(|run| run.text == "&"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

scratch 无文档引用或可恢复语义，只保留 Vec 容量；每次从当前文档索引重新查询，允许在追加/迟确认之后或不同文档之间复用。查询期间文档保持不可变借用；无效范围不改 scratch，空范围不分配。遍历仍是 O(log n + 请求范围)，栈按树深度增长；预热工作集后不增长，切换到更深查询可能继续分配。容量保留到 drop 或赋值 `QueryScratch::default()`；没有全局或线程局部缓存，不需要 unsafe。

`QueryScratch::stats()` O(1) 返回保留容量及生命周期累计分配/扩容调用，查询之间 `nodes = 0`；迭代器的 `scratch_stats()` 返回待处理帧、当前容量及**本查询**新增分配调用。`DisplayCursor` 自动复用自己的栈，`scratch_stats()` 为该游标各步骤累计统计；每步只保留容量，仍按原规则重查有限窗口、预先扣费并检查文档/revision，预算耗尽和错误不会分配或改变继续位置。游标销毁释放容量；重建游标重新预热。其它高层消费者是否使用此接口取决于各自实现，未声称全部调用链零分配。

分配、访问数、预算及独立结果模型见 [查询 scratch 测量](docs/storage.md#查询-scratch-复用实验)。这些计数只覆盖标注遍历栈，不等于全局 allocator、总堆峰值或实际终端时延。

## 显示文本搜索

`DisplaySearch::new(&doc, query)` 创建绑定当前文档身份及 revision 的搜索任务。搜索对象是按源码顺序串接的非 `HIDDEN` 显示片段：实体已解码、闭合代码 span 空白已规范化，隐藏标记和链接目标不参与匹配。采用区分大小写的字面匹配，支持重叠命中和跨 run/tile 匹配；空查询返回 `InvalidRange`。不增加块/表格分隔符，不把普通软换行变为空格，不做 Unicode 归一化，也不包含终端绘制的边框、占位或将来的折叠状态。

```rust
use mdstream::{Budget, Completion, DisplaySearch, Document, Span, WorkLimit};
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

查询预处理时间及任务存储为 O(m)，m 为查询字节数；另有固定大小的片段缓冲，源码映射使用长度为 m 的环形缓冲。单 revision 的 KMP 比较总量 O(d)，d 为显示字节数；每次显示读取另有 O(log(n + 1) + 检查窗口) 的成本，窗口不超过 `TILE_BYTES + 2`，最坏累计搜索 O(n log(n + 1) + m)。输出空间由调用方管理，可逐批消费，不在任务中保存全文或全部命中。示例见 `crates/mdstream-core/examples/search.rs`。

## 基础折叠

`FoldView` 为每个视图保存独立的收起意愿、高度和检查点。当前支持顶层及容器内标题章节、围栏和缩进代码块、表格及显式 Quote/List/Item 容器：章节保留标题，正文及子章节变为一个占位；代码块、表格和容器整体变为占位，表格保留表头源码作为摘要。空章节不生成占位。父节点展开后恢复原来的子节点状态，源码、解析对象和 `Style::HIDDEN` 不变。

```rust
use mdstream::{Document, FoldView, FoldId, HeadingId, Budget, WorkLimit};

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

`Document::fold_candidate` 按稳定 `FoldId` 查询整段范围、隐藏正文、父节点、摘要源码及开放状态；`fold_candidate_at` 查询当前位置的最内层候选。`fold_candidates(span)` 二分定位范围，归并枚举开头块与范围相交的候选以及起点在范围内的容器；同起点先按 ContainerId 发出外到内容器，再发出叶块。其 `advance` 与视口共享 Budget，每检查一个块（包括非候选）或一个容器消耗一次 query 和 object，耗尽可续，完成返回 `Covered`。游标绑定文档和 revision；范围之前开始的祖先通过 `parent` 查询。摘要用 `summary_text` 限定 UTF-8 字节数并计费，不统计正文行数、任务数或复制长标题。`hidden_at` 按预算返回 `FoldLookup::{Visible, Hidden, BudgetExhausted}`，隐藏结果是最外层遮蔽者，适合占位命中和源码定位。

`LayoutPass::folded` / `advance_folded` 接收 `FoldLayoutSink`，复用已有有预算排版循环。每个可见占位只调用一次 `placeholder`，传入稳定 ID、隐藏源码、摘要区间及归属 tile；接入方将占位作为块边界绘制并在 `end_tile` 中计入其高度、重置必要的换行状态。回调只获得元数据，若要读取摘要，应另用共享 Budget。占位高度全部归属隐藏起点所在 tile；同一 tile 可有多个占位、标题和后继正文。隐藏内部的文字、图片和子占位不会交给回调。该接口仍使用 `RenderRun`，显示实体解码、终端宽度及真实软换行由接入方处理。完整示例见 `crates/mdstream-core/examples/folding.rs`。

高度树独立保存基础高度/有效性与区间遮蔽计数。`height`、`prefix_rows`、`locate` 和视口查询返回折叠后的高度，`base_height` 返回遮蔽前的储存值；完整隐藏的内部 tile 无须逐个清零或恢复。每个占位及末端边界 tile 保持可检索，再按预算精确测量。单节点操作为 O(log H + log C + log F + log T)，顶层标题祖先最多六级，H/C/F/T 分别是标题、容器、已收起节点和 tile 数。父子交替收放、失效、基础高度变化、树增长及 O(1) resize 保留嵌套遮蔽。重复操作不保存历史快照，新增视图状态 O(F + C + H)，容器与内层章节稀疏索引保留已访问位置，分别受 O(C) / O(H) 上界约束；高度树仍为 O(T)。

`set_collapsed` / `toggle` 返回源码变化范围和视图版本；从受影响边界起惰性过期后续高度与检查点，旧异步测量和布局继续任务会被拒绝。`view.measure` 验证本视图的测量及检查点依赖；`insert_checkpoint` 同时使下游布局失效。`reset` 保留折叠意愿与遮蔽，只重置基础估计和检查点。保留的高度仍可能是过期估计，展开后仅测量请求窗口；选择远处 tile 时默认局部近似，不能承诺精确绝对行号。`folded_from_checkpoint` 只接受本视图的有效起点及指定字节距离内的恢复，接入方恢复 sink 状态，恢复步骤也使用同一 Budget。

每次 append/finish 或连续 Update 批处理后调用 `view.apply`，然后才能查询。对已有折叠，更新检查最多六个开放顶层章节和一个开放代码/数学/表格块；开放容器与内层章节共享一次尾部遮蔽更新；新标题和容器关闭各检查相关作用域的至多六个末尾章节，另检查一个可能增长的旧标题。开启默认收起时还处理新增标题/块/表格/容器的元数据，不枚举历史折叠或开放容器祖先链。新 tile 在 apply 返回前继承遮蔽。开放容器/内层章节/代码/数学/表格的未分类行前缀暂属该范围，闭合/下一块确认后可以收回；Setext 晚确认也能缩短前一章节的范围。普通源码追加不会读取旧正文。重复应用不清掉新测量，错误文档或不连续更新在修改视图之前被拒绝。

折叠导航、锚点恢复、有预算批量操作、临时展开和摘要读取见下文。固定手工/宿主源码范围见下文；目录 UI、摘要占位绘制和其它范围策略继续见 [TODO](TODO.md#折叠支持规划)。

### 整表折叠

`FoldId::Table(TableId)` 把整张已识别表格（表头、分隔行、正文）替换为一个占位，`summary` 指向第 0 行的原始源码。宿主可以用 `summary_text` 读取有界原文前缀，或用 `DisplayExcerpt` 读取显示摘要；两者都需要自己的共享预算。列分隔符、对齐、边框和实际表头布局仍由渲染器处理，摘要接口不会自动拼接单元格或测量全表。仅含表头和分隔行的表也产生一个占位。

```rust
use mdstream::*;

let mut doc = Document::new();
let mut view = FoldView::new(&doc, 1)?;
view.set_defaults(FoldDefaults { table: true, ..Default::default() });
let update = doc.append("| Name | Value |\n| - | - |\n| first | 1 |\n")?;
view.apply(&doc, &update)?;
let id = FoldId::Table(TableId(0));
assert!(view.is_collapsed(id));
let update = doc.append("| second | 2 |\n# After table\n")?;
view.apply(&doc, &update)?;
let candidate = doc.fold_candidate(id).unwrap();
assert!(!candidate.open);
assert_eq!(candidate.hidden, doc.table(TableId(0)).unwrap().source);
let mut budget = Budget::new(WorkLimit {
    queries: 1, objects: 1, bytes: 32, ..Default::default()
});
assert_eq!(candidate.summary_text(&doc, 32, &mut budget), Some("| Name | Value |\n"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

表格的折叠父节点是确切作用域的当前章节；该作用域还没有标题时挂在所属容器下，根作用域没有标题时无父节点。父节点收放保留表格自身意愿。默认设置、批量命令、临时展开、源码锚点与书签复用现有契约，导航到隐藏单元格时逐帧展开必要的顶层/内层章节、容器和表格。`Update.new_tables` 处理表头晚确认，包括已有段落块转成表格的情况；显式展开后追加行不会再次套用默认值。开放表格的待分类行前缀暂属表格，下一块确认时收回，EOF 封闭范围。最多六个顶层章节和一个代码/数学/表格块仍单独刷新；任意多个开放内层章节与容器共用尾部，见下文。

候选查询、切换及更新均不枚举行/列/单元格；排版直接跳过隐藏正文，占位高度只归属表格起点 tile。`crates/mdstream-core/examples/folding.rs` 演示百万字单元格收起后追加新行、读取表头前缀及有预算排版。前几行有界摘要读取及连续行范围折叠见下文；表格绘制仍是后续工作。

### 容器折叠

`FoldId::Container(ContainerId)` 支持整个 Quote、List 和 Item，包括单行、空内容、任务条目及嵌套容器。隐藏范围覆盖完整物理行及前缀，替换成一个占位；`summary` 借用整个范围，调用方通过 `summary_text` / `DisplayExcerpt` 限量读取，不复制正文或统计条目数。原始 `Container::source` 保持已分类范围；开放折叠候选暂包含未分类尾部，兄弟/外层行确认或 finish 后收回到精确边界。稳定身份和子折叠意愿在增长、关闭和 resize 后保留。

```rust
use mdstream::*;

let mut doc = Document::new();
doc.append("> - first\n>   continuation\n")?;
let mut view = FoldView::new(&doc, 1)?;
let quote = FoldId::Container(ContainerId(0));
let item = FoldId::Container(ContainerId(2));
view.set_collapsed(&doc, item, true)?;
view.set_collapsed(&doc, quote, true)?;
let update = doc.append("> - second\noutside\n")?;
view.apply(&doc, &update)?;
view.set_collapsed(&doc, quote, false)?;
assert!(view.is_collapsed(item));
assert_eq!(doc.fold_candidate(item).unwrap().parent,
           Some(FoldId::Container(ContainerId(1))));
let mut budget = Budget::new(WorkLimit { queries: 1, ..Default::default() });
assert!(matches!(view.hidden_at(&doc, 4, &mut budget)?, FoldLookup::Hidden(c) if c.id == item));
# Ok::<(), Box<dyn std::error::Error>>(())
```

容器的折叠父节点是其结构父容器作用域中的当前章节；没有本地章节时回退到结构父容器，根容器同理查询根章节。表格挂在确切作用域的当前章节下，没有本地标题时回退到所属容器。内层标题的折叠父节点是同作用域父章节，否则为所属容器。顶层章节覆盖容器，最外层已折叠容器覆盖子容器与表格的占位；展开父节点恢复孩子的本地意愿。`All`、`Subtree`、`Siblings`、`Focus`、临时展开、导航与源码锚点均包含容器。新容器默认展开；`FoldDefaults::container_depth` 可按结构深度初始化新节点的收起意愿，具体语义见下文。按类型或任务完成状态自动折叠仍在 TODO。

视图以 ContainerId 的前序索引维护已折叠范围的最大终点，开放节点用开放终点标记，关闭时由 `Update.closed_containers` 写入固定终点。可见性查询先二分起点，再查询最外层遮蔽者，O(log C)，无需逐层读取祖先。全部开放容器位于同一条活动路径，完整 tile 的遮蔽并集只需一个共享范围；追加只更新这个并集，关闭的折叠容器转成固定遮蔽。未开启深度默认策略时，容器部分的 apply 成本为 O((E + 1)(log H + log C + log F + log T))，E 为实际关闭事件数，不随仍开放的深度增长；这仍是按输入/事件消费的更新接口，没有任意大 Update 的单帧时延承诺。单次索引写入/查询有对数节点访问量，Vec 扩容沿用摊还保证。

`container_index_visits` 计索引读写的节点访问量，`container_index_nodes` 计保留节点数；反复切换不新增历史快照，节点数不超过完整二叉索引的 O(C) 上界。`crates/mdstream-core/tests/container_folding.rs` 验证千层开放链、百万字正文、4096 个关闭兄弟、同起点父子、有预算深层导航、UTF-8 全切点/逐前缀和独立区间/高度模型。`cargo run -p mdstream-core -r --example container_folding` 展示流式追加、父预览、子意愿及按帧恢复单元格源码锚点。类型/任务策略见下方；实际摘要/容器排版和终端 UI 继续独立推进。

### 容器内章节折叠

`FoldId::Heading` 包含显式 Quote/Item 内的 ATX 和多物理行 Setext。保留标题全部物理行（含容器前缀），正文及子章节替换为一个占位；空正文没有占位。范围止于同一确切作用域的同级/更高级标题或容器边界；嵌套作用域的标题不截断外层。`HeadingIndex::section` 保持已分类范围，开放的折叠候选暂包含未分类尾部，边界确认时收回。Setext 晚确认可回退前一章节末端；长 ATX 标题增长会移动隐藏起点，标题文字不被自己的折叠遮住。

折叠父链按包含关系组合章节与容器：章节先找同作用域父标题，否则挂在所属容器；子容器先找其结构父作用域的当前标题，否则挂在结构父容器。表格同理。原始 `Container::parent` 和 `HeadingIndex::parent` 不变。`fold_candidate_at` 返回包含当前位置的最内层候选，标题仍可作为导航目标；Reveal 只展开遮住目标的祖先，不改变目标章节自身正文意愿。All/Subtree/Siblings/AtLevel/Focus、预览、源码锚点和默认 `heading_levels` 直接覆盖内层章节，默认只初始化新 ID。

```rust
use mdstream::{Document, FoldView, FoldId, HeadingId, ContainerId, TableId};
let mut doc = Document::new();
doc.append("> # Guide\n> - ## Item\n>   a|b\n>   -|-\n>   x|y\n")?;
let mut view = FoldView::new(&doc, 1)?;
let guide = FoldId::Heading(HeadingId(0));
let item = FoldId::Heading(HeadingId(1));
assert_eq!(doc.fold_candidate(FoldId::Container(ContainerId(1))).unwrap().parent, Some(guide));
assert_eq!(doc.fold_candidate(FoldId::Table(TableId(0))).unwrap().parent, Some(item));
view.set_collapsed(&doc, item, true)?;
view.set_collapsed(&doc, guide, true)?;
let update = doc.append("> - ## Next\n>   body\nout\n")?;
view.apply(&doc, &update)?;
view.set_collapsed(&doc, guide, false)?;
assert!(view.is_collapsed(item));
assert!(!doc.fold_candidate(item).unwrap().open);
# Ok::<(), Box<dyn std::error::Error>>(())
```

内层章节使用按 HeadingId 排列的稀疏最大终点索引，开放终点为标记，查询时读取最新范围。它与容器索引分别定位最外层遮蔽者，再按源码前序比较，查询 O(log H + log C + log F)，不扫描祖先链。所有开放内层章节和容器都延伸到流末端，完整 tile 遮蔽并集只保存一个范围。普通正文追加只刷新共享尾部，不逐个写入开放章节；标题本身增长时只更新那一个隐藏起点。新标题检查同作用域至多六个可能结束的章节；容器关闭检查该作用域每级最后一个标题，跳过已经封闭或未收起的节点。合并 Update 仍从新标题及关闭事件恢复所有边界，不依赖逐次 apply。

设 N 为新标题数、E 为关闭容器数，范围维护为 O((N + E + 1)(log H + log C + log F + log T))，默认初始化另按新对象计费。关闭大量节点仍按实际事件消费，无任意大更新的单帧时延承诺；解析累计复杂度及查询/排版共享 Budget 不变。新增索引空间 O(H)，反复切换不积累历史节点。`section_index_visits` / `section_index_nodes` 分别报告节点访问数和保留节点数；`range_visits` 包含事件触发的刷新。

`crates/mdstream-core/tests/container_sections.rs` 用独立区间、父链和高度模型验证嵌套边界、同起点、同 tile、流式逐前缀/UTF-8 全切点、合并更新、批量、预览与表格组合。500 个开放章节和百万字正文下逐字追加没有章节索引写入；150 层交替章节/容器的导航每帧展开一个节点；4096 个关闭章节验证对数查询和节点复用。`cargo run -p mdstream-core -r --example container_sections` 演示共享尾部、预览保留子章节及精确源码恢复。实际摘要/容器排版、终端 UI 及未实现容器方言仍保留 TODO。

## 折叠导航与源码锚点

`FoldNavigation::new` 从当前 `Navigator` 快照创建任务，接受全部 `NavAction`。目录选择用 `Heading(id)`，搜索命中或已解析链接用 `Source(offset)`；普通滚动后的起点由 `Navigator::reconcile_source` 提供。创建任务只检查身份及参数，实际索引查询在 `advance` 中计入共享 Budget。

```rust
use mdstream::*;
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

每个有界步骤消费 1 query、1 object：解析动作、检查可见性并展开/跳过一个遮蔽范围、或查询最终估计行。祖先可能包含任意深度的已识别容器；一次步骤通过索引只定位并展开一个最外层遮蔽者，含固定数量的索引操作，不读取正文或标题文本。只给上述最小预算也可推进。耗尽返回 `BudgetExhausted`，保存目标或已经跳过的区间；`Covered` 表示动作结束，`target: None` 表示无匹配目标。`change` 是**本次调用**已展开区间的包络和最新视图版本，已同步更新高度/检查点失效；取消或失效不会撤销已完成的展开。任务存储 O(1)，Reveal 按实际打开的祖先数量计费，可跨帧恢复；VisibleOnly 总工作按实际跳过的折叠区间数计费，每步 O(log H + log C + log F + log T)，不会逐个检查被隐藏的标题。

任务绑定文档身份/revision、视图身份及折叠版本，文档追加/首次 finish、外部折叠操作和 reset 后拒绝旧任务；错误返回前不消费预算或修改视图。自身展开更新其版本，期间的正常高度测量不使任务失效；最终定位使用最新高度。完成后重复调用返回原结果且不重复展开，返回行仍是当时的估计。任务不改写原 Navigator；调用方只应用当前有效任务的结果，新的用户导航应丢弃旧任务。用 `Navigator::apply_target` 应用结果，它在 source 为 None 的普通滚动后清除旧的精确源码目标。远处定位、同 tile 多标题和折叠占位仍由局部 `LayoutPass` 在共享 Budget 下细化，随后再 reconcile；此接口不排整篇前缀，也不承诺精确绝对行号。

`FoldAnchor::new(&doc, source)` 保存文档身份和精确源码位置，`resolve(&doc, &view, viewport_rows, &mut budget)` 返回当前投影。隐藏时 `hidden_by` 标明最外层折叠，`target.source` 指向其占位起点；可见时返回原位置。保留同一个 FoldAnchor，在收起/展开、append、Setext 晚确认或 resize 后重新 resolve 并应用目标即可恢复；不要用占位位置覆盖保存的原锚点。每次 resolve 消费 1 query、1 object，预算不足返回 None，不改变折叠。锚点允许 EOF，追加后原字节偏移保持不变；不绑定 revision，错误文档或未同步的 FoldView 会被拒绝。它保存源码而非精确屏幕行/列，选区仍使用原源码范围。示例 `crates/mdstream-core/examples/folding.rs` 演示占位投影和分帧恢复到 120 万字节正文内的目标。

## 批量折叠与新节点默认设置

`FoldBatch` 提供当前 revision 上的显式命令：`All` 展开/收起全部候选，`Subtree` 包括根及全部后代，`AtLevel` 只修改严格等于指定级别（1–6）的标题，`Focus` 展开目标及祖先并收起所有其他候选。聚焦会修改后代意愿；仅想露出目标并保留无关状态时使用 `FoldNavigationPolicy::Reveal`。

`Siblings { target, collapsed }` 展开/收起与目标具有相同 `FoldCandidate::parent` 的其他候选，保留目标、祖先和各节点后代的原有意愿。兄弟可以是不同级别的标题、代码块、数学块或表格；没有父节点的候选互为根层兄弟，包括首个标题之前的代码块、数学块和表格。目标已被祖先隐藏时仍可操作，但不会自动展开祖先；没有兄弟时完成且无变化。该命令只修改已有节点，不改变新节点默认值。

```rust
use mdstream::*;

let mut doc = Document::new();
let mut view = FoldView::new(&doc, 1)?;
view.set_defaults(FoldDefaults {
    heading_levels: [false, true, true, true, true, true],
    code: true,
    ..Default::default()
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

创建任务为 O(1)（行范围目标额外验证视图注册，O(log R)），范围定位在第一次 `advance` 中消费 1 query、1 object；之后每个块、容器 opener 或已注册行范围同样消费 1 query、1 object，包括非候选及不匹配级别/父节点的对象。子树范围通过索引定位；兄弟操作检查父节点隐藏范围内的块与容器 opener，包含后代元数据但不修改后代意愿，根层兄弟操作及 All/AtLevel/Focus 命令分帧扫描块与容器索引；Containers 只遍历容器 arena，计费见下节。同起点、同范围的 List/Item/Table 通过容器前序顺序区分祖先，Subtree 与 Focus 不会误改同范围祖先/后代。既不扫描源码正文，也不逐 tile 更新隐藏范围。每步为 O(log B + log H + log C + log R + log F + log T) 的固定次数索引操作（B 为块数，R 为已注册行范围数），完整任务按实际访问的块、容器与行范围数计费。`visited` / `changed` 是累计对象访问数/实际状态变化数，`change` 是本次调用的区间包络及最新版本，耗尽返回 `BudgetExhausted`。任务仅保存 O(1) 游标状态；不复制候选列表或折叠快照。

批量任务渐进生效；取消、文档更新、外部折叠操作、默认设置变化或 reset 后，已经完成的修改保留。旧任务会在消费预算或修改视图之前被拒绝；任务自身修改及正常高度测量允许继续，完成后重复调用不再产生修改。每次实际修改复用单节点的高度遮蔽、检查点及测量失效，调用方可以在两次 advance 之间正常绘制。需要原子切换整棵树或取消回滚的宿主，不能将该接口当成事务。

`FoldDefaults` 只规定**后续首次识别**的节点初始意愿：`heading_levels[0..6]` 对应 h1–h6，`code` 控制代码块，`math` 控制独立数学块，`table` 控制表格，`container_depth` 控制容器深度阈值，`containers` 控制容器类型/任务状态（见下节），默认全部展开。`set_defaults` 为 O(1)，不追溯已有候选，不覆盖显式展开/收起；已有文档创建视图后仍全部展开，需要调整时使用批量命令。默认设置变化增加视图版本，但不改变当前布局；相同设置无操作。批量命令不修改默认设置，因此“全部展开”后新到来的标题仍按当前默认初始化。单节点收放继续保留子节点意愿；resize 保留默认设置和所有节点意愿。

`view.apply` 通过 `Update.new_headings` / `new_blocks` / `new_tables` / `new_containers` 初始化新候选，Setext 和表格晚确认使用新 HeadingId/TableId，即使其块早已存在；重复应用不会重置意愿。`default_visits` 计量检查的新增标题/块/表格/容器元数据，`range_visits` 计量最多七个已有开放顶层章节/代码/表格、一次共享容器/内层章节尾部检查、事件触发的内层章节刷新（含一个可能增长的旧标题），以及实际关闭的已折叠容器。初始化成本计入新增对象事件，不宣称任意大 Update 的消费具有整帧硬预算；默认初始化在没有新候选时不回访历史节点或读取旧正文。合并 Update 在 apply 时统一使用当前默认设置；需要在流中切换默认值时，先应用切换前的更新。

可运行示例：`cargo run -p mdstream-core -r --example fold_batch`。

### 容器深度默认策略

`FoldDefaults::container_depth: Option<usize>` 是包含式阈值：新容器的 `Container::depth >= threshold` 时初始化为收起。`None` 关闭策略，`Some(0)` 与 `Some(1)` 都包括全部根容器，`Some(usize::MAX)` 也无需加减运算。深度按结构父链计数：根为 1，Quote、List、Item 各占一层；例如 `> - text` 的 Quote/List/Item 深度依次为 1/2/3，不按视觉缩进或列表标记个数计算。标题与表格不增加容器深度；该深度阈值自身不区分任务状态，与下节 `containers` 按任一命中组合。

```rust
use mdstream::*;

let mut doc = Document::new();
let mut view = FoldView::new(&doc, 1)?;
view.set_defaults(FoldDefaults { container_depth: Some(2), ..Default::default() });
let update = doc.append("- first\n")?;
view.apply(&doc, &update)?;
let first = FoldId::Container(ContainerId(1)); // List = 1, Item = 2.
assert!(!view.is_collapsed(FoldId::Container(ContainerId(0))));
assert!(view.is_collapsed(first));
view.set_collapsed(&doc, first, false)?;
let update = doc.append("- second\n")?;
view.apply(&doc, &update)?;
assert!(!view.is_collapsed(first));
assert!(view.is_collapsed(FoldId::Container(ContainerId(2))));
# Ok::<(), Box<dyn std::error::Error>>(())
```

策略只消费 `Update.new_containers`，包括同一合并更新内已打开又关闭的容器，以及物理行晚确认的节点。初始化与祖先是否收起或正在预览无关，每个节点保留自己的意愿；预览父节点期间新出现的深层子节点仍按阈值初始化。显式单节点、导航及批量操作修改已有选择；预览期间的显式操作更新预览结束时恢复的选择。更改/关闭默认策略、普通追加、关闭、finish、重复 apply 和 resize 都不会重新初始化已有 ID。`set_defaults` 不影响当前布局，变化会使旧视图任务失效，相同值无操作；合并更新统一使用 apply 当时的策略。

启用策略时每个新容器检查一次已存深度，计入 `default_visits`，无论是否达到阈值；禁用时不访问这些元数据。设 N 为新增容器数，K 为达到阈值的个数，初始化额外成本为 O(N + K(log H + log C + log F + log T))，只增加 O(1) 策略字段，折叠状态沿用 O(F + C + H) 空间（含内层章节索引）。没有新容器的普通追加不检查历史节点或开放祖先，仍只更新一次共享尾部。apply 按新增对象/开闭事件消费，不承诺任意大更新的单帧时延；查询、批量操作、预览和排版继续共用 Budget。测试覆盖千层/百万字逐字追加、4096 个历史兄弟、UTF-8 全切点/逐前缀及 240 次流式策略/显式操作的区间与高度模型。`cargo run -p mdstream-core -r --example container_defaults` 演示预览中的新子节点、显式展开优先及批量展开后新条目的默认收起。

### 容器类型与任务完成状态策略

`ContainerFoldFilter` 的五个布尔字段按并集筛选：`quotes` 选 Quote，`lists` 选所有有序/无序 List，`items` 选所有 Item（含任务），`completed_tasks` / `incomplete_tasks` 只选有效状态为已完成/未完成的任务 Item。全部为 false 时不选任何节点；普通条目不会被任务字段选中。List 与 Item 是独立节点，不根据子任务聚合列表完成状态，也不隐式修改后代意愿。

`FoldDefaults::containers` 与 `container_depth` 按“任一命中即默认收起”组合。字段 false 表示不增加选择，不是强制展开；例如 `items: true` 会选中所有任务，不受 `completed_tasks: false` 排除。默认规则只消费新增容器，任务状态取源码 `[ ]`、`[x]`、`[X]`，含晚识别、EOF 与合并更新。已有节点、显式展开、导航展开、预览保存的意愿均不重置；默认变化也不回溯已有任务。

`view.task_checked(&doc, id)` 查询该视图中的有效状态，非任务容器返回 `None`，无效 ID 返回 `InvalidRange`。`set_task_checked(&doc, id, bool)` 仅接受任务 Item；只保存与源码不同的覆盖，设回源码值立即移除。它不修改 Document、任务标记及样式、其它视图、折叠意愿或预览；渲染任务控件时由宿主读取有效状态，通用源码/显示读取仍表示原文。修改返回 marker 的 `FoldChange` 区间和新视图版本，并惰性失效该 tile 起的测量/检查点、旧批量/导航/排版任务；相同值为无操作。覆盖跨追加、finish、reset 保留。状态改变后要重新折叠，显式运行下面的批量命令。

```rust
use mdstream::*;
let mut doc = Document::new();
let mut view = FoldView::new(&doc, 1)?;
let completed = ContainerFoldFilter {
    completed_tasks: true, ..Default::default()
};
view.set_defaults(FoldDefaults { containers: completed, ..Default::default() });
let update = doc.append("- [x] done\n- [ ] review\n")?;
view.apply(&doc, &update)?;
assert!(view.is_collapsed(FoldId::Container(ContainerId(1))));
view.set_task_checked(&doc, ContainerId(2), true)?;
assert!(!view.is_collapsed(FoldId::Container(ContainerId(2))));
let mut task = FoldBatch::new(&doc, &view, FoldBatchAction::Containers {
    filter: completed, collapsed: true,
})?;
loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    if task.advance(&doc, &mut view, &mut budget)?.completion == Completion::Covered {
        break;
    }
}
assert!(view.is_collapsed(FoldId::Container(ContainerId(2))));
assert!(doc.source().contains("[ ] review"));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`FoldBatchAction::Containers { filter, collapsed }` 是显式修改已有意愿的命令，不改默认规则或任务状态。它包括收起祖先下面的容器，只修改命中节点；标题、代码、表格及注册行范围保持自己的意愿。预览节点保存的选择也会更新，预览结束后按当前范围恢复。任务使用当前视图的有效完成状态；外部修改该状态会先使旧任务失效，需重建任务。中间结果立即生效，取消或失效保留已完成部分。

创建批量任务 O(1)，逐个访问容器 arena；每检查一个容器扣 1 query / 1 object，不匹配和空筛选同样扣费，不收额外初始化费，不读取任何正文/叶块/表格行。预算不足时原子暂停；完成后的重复调用不扣费。每步固定次数元数据/索引操作，含状态查询 O(log U) 与命中后的既有折叠索引成本；任务只存 O(1) 游标。总访问 C 个容器，不依赖正文长度或叶块数。U 为覆盖数，存储 O(U)，不超过已识别任务数；单点覆盖 O(log U + log T)，不枚举祖先/后代或 tile。

默认规则只在 `new_containers` 中检查一次类型、深度、任务标记（计 `default_visits`），启用多个字段也不重复遍历；新增 N 个容器、命中 K 个时成本沿用 O(N + K(log H + log C + log F + log T))。普通正文追加不重查历史任务或祖先；不新增解析状态、不改变零运行时依赖及累计复杂度契约。默认 apply 仍按新增事件摊还，任意大更新的单帧硬预算不在该接口承诺内。

`crates/mdstream-core/tests/container_folding.rs` 与存储测试覆盖 32 种类型/任务筛选 × 4 种深度策略、UTF-8 全切点/逐前缀、CRLF/失败 marker/EOF、320 轮流式区间/高度模型、千层/百万字及 4096 历史条目；共享预算、独立视图、旧任务/测量拒绝、预览/导航和非容器折叠均有回归。`cargo run -p mdstream-core -r --example container_policies` 演示视图内完成任务、分帧筛选与预览/导航恢复。任务控件、列表完成率聚合和终端 UI 未由此接口实现。

## 手工及宿主源码范围折叠

`SourceFoldRegistration` 将一个宿主选区或语义对象的固定 `Span` 注册到 `FoldView`。成功后返回 `FoldId::Source(SourceFoldId)`，初始展开；ID 只属于该视图，进程内不复用。宿主可自行保存业务键与 ID 的对应关系，库不修改 Markdown 源码、结构父链或 `Style::HIDDEN`。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("# Root\nkeep secret tail\n\n")?;
let mut view = FoldView::new(&doc, 1)?;
let start = doc.source().find("secret").unwrap();
let selected = Span::new(start, start + 6);
let mut task = SourceFoldRegistration::new(&doc, &view, selected)?;
let id = loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    if let Some(id) = task.advance(&doc, &mut view, &mut budget)?.id {
        break id;
    }
};
view.set_collapsed(&doc, id, true)?;
let anchor = FoldAnchor::new(&doc, start + 1)?;
let update = doc.append("later\n")?;
view.apply(&doc, &update)?;
assert_eq!(view.fold_candidate(&doc, id)?.unwrap().hidden, selected);
let mut budget = Budget::new(WorkLimit::unlimited());
assert_eq!(anchor.resolve(&doc, &view, 10, &mut budget)?.unwrap().hidden_by, Some(id));
view.set_collapsed(&doc, id, false)?;
assert_eq!(anchor.resolve(&doc, &view, 10, &mut budget)?.unwrap().target.source,
           Some(start + 1));
# Ok::<(), Box<dyn std::error::Error>>(())
```

只接受非空、已存在且两端均为 UTF-8 边界的范围；两端是固定原始字节锚点，旧 EOF 之后追加的内容不会加入。范围须落在 `Document::source_fold_limit()` 返回的保守前缀内：当前叶块、待分类行、表头探测以及可能被 Setext 确认的整个容器正文序列暂不开放。局部关闭后可注册历史正文，无须结束整个输入；`finish` 后全部源码进入此前缀。这是结构边界保证，晚引用解析或行内语义仍可正常失效。当前开放代码、表格、数学或段落需先局部关闭。

同视图内手工范围必须不相交：重复、包含和交叉注册均返回 `InvalidRange`，展开/预览不解除约束；相邻范围各保留一个占位。手工范围与语法候选及已注册表格行范围可以互相包含，但其 `source` 包络和 `hidden` 区间都不能交叉。对仍开放的结构，还拒绝跨越其起点的选区，防止结构后续增长穿过固定终点；例如选择开放章节的标题末尾和当前全部正文，虽然当前 EOF 时不交叉，仍会被拒绝。只选标题或只选已定型正文可以独立成立。后注册表格行范围也检查同一规则，与注册顺序无关。

注册分步检查两端的语法祖先链及表格行前驱，不枚举中间所有候选。创建时只验证固定元数据和手工重叠索引；`advance` 每个端点查找、祖先检查、路径结束及最终提交各扣 1 query / 1 object，零正文字节。`visited` 只统计祖先。预算耗尽保留进度；成功前没有任何注册、折叠或高度修改。取消、交叉错误和任务失效不会留下半成品；追加、reset 或其它视图修改后需重建任务。成功后重复调用免费，外部状态变化仍按版本拒绝旧任务。

`FoldView::fold_candidate` 按 ID 查询，包括已注册源码/表格行范围；移除或其它视图的源码 ID 返回 `None`。`source_fold_at` 查找包含该字节的注册范围，包括展开的范围；`hidden_at` 查询实际遮蔽者。`SourceFoldCandidates` 按共享预算枚举与窗口相交的注册范围，包含起点更早的覆盖范围，且不发现语法候选；文档的 `fold_candidates` / `fold_candidate_at` 保持语法查询职责，`Document::fold_candidate` 对 Source ID 返回 `None`。`remove_source_fold` 释放注册、意愿和对应预览；重新选择边界会生成新 ID，旧 ID 不复用。`source_fold_count` 是当前注册数。

可见性取包含它的最外层已收起区间，完全相同的隐藏范围由语法/表格行范围优先。父级收起后再展开、reset 和追加都保留手工意愿；临时预览、`FoldAnchor`、`FoldNavigation`、摘要读取及预算布局复用现有接口。占位属于隐藏起点 tile，完整内部区间直接遮蔽；任意段内边界不自行补换行，真实占位/换行状态由排版 sink 处理。`summary` 指向选中源码，读取用有界 `summary_text` 或 `DisplayExcerpt`。

手工候选的 `parent` 是注册时最内层的包含其源码包络的语法节点；不将语法节点或表格行范围改挂到手工节点。批量 Subtree/Focus 仍按源码包含关系判断；完全相同的源码包络按容器、叶语法、手工范围顺序打破平局，避免操作手工子树时反向修改同范围的语法祖先。All/Subtree/Siblings/Focus 包含已注册手工范围，AtLevel/Containers 保持原筛选范围。Siblings 也能发现父标题文字中的手工范围，原有语法扫描及预算边界保持不变。默认策略不自动创建手工选区。

R 个非空不相交注册使用 O(R) 空间（不超过源码字节数），删除后不保留 ID 或范围历史；索引查询 O(log R)，单点切换/移除 O(log R + log F + log T)。注册额外按两端路径深度 D 付费，每步固定次数结构索引操作，游标 O(1)；手工注册的深度成本独立于输入解析契约。固定范围的普通追加不刷新任何历史手工范围，仅当一个收起范围恰好结束于旧 EOF 时，校正一次可能继续增长的 tile；可与代码/数学/表格行既有边界处理共存。解析器没有新增逐字或祖先工作。布局、导航、摘要和批量仍共用帧预算。

`cargo run -p mdstream-core -r --example source_folding` 演示百万字选区、后续追加、预算布局及源码锚点恢复。`crates/mdstream-core/tests/source_folding.rs` 覆盖 2400 组全量结构区间对照、1000 轮随机可见性/高度模型、240 次流式模型、千层注册暂停、4096 历史范围、百万字正文、UTF-8 分块/逐前缀、表格行双向交叉拒绝、批量/预览/导航和旧任务/测量拒绝。稳定根起点的自动增长尾范围见下节；其它手工嵌套、交叉拆分/优先级、重载后的业务身份持久化与终端选区 UI 仍待设计。

## 稳定根起点的手工尾范围

`SourceFoldRegistration::tail(document, view, start)` 从已定型 UTF-8 根切口登记到真实 `source.len()`，初始展开。文档必须未结束，且 `start < source_fold_limit()`；`advance` 付费确认 `Document::fold_candidate_at(start) == None`，标题、容器、代码、数学或表格整个包络内的起点均拒绝。其它手工/段落登记与尾包络相交也拒绝，展开/预览不解除约束。登记只有根验证和原子插入两步，各扣 1 query / 1 object，不读正文；零预算、错误或过时任务不发布 ID。

有效 `FoldView::apply` 用单个 live 槽更新真实 registry、summary、Mask 和高度覆盖到当前 EOF，包括尚未分类尾部。`source = hidden = summary`，`parent=None`、无段落 BlockId；内部新语法与表格行保留原语法父链和独立意愿，尾不会成为它们的语法父节点。预览只临时展开这一层；reset 保留登记和意愿。每次新增同步工作为固定次数 O(log R + log F + log T) 索引操作，无历史、叶块、正文或祖先回扫；现有无 Budget 的 apply 不新增伪计费。

`SourceFoldRegistration::seal_tail(document, view, source_id)` 对本视图 live ID 条件封口。若 EOF 超过稳定界限，构造立即 `InvalidRange`；即使 EOF 已稳定，仍按固定范围的两端路径逐预算验证，内部开放标题/容器等未来会交叉时拒绝。预算耗尽可续，失败保留已消费预算但不修改视图；成功的最终付费步骤复用同一 ID，关闭增长并失效布局依赖，保留预览和 saved choice。普通根尾成功需 5 query / 5 object，另加实际访问的祖先步。随后追加不会增长，旧 EOF 混合 tile 沿固定边界机制只刷新一次。

`finish` 自动同 ID 封口，保留尾内 pending 来源的最终分类，无须手动封口拓扑检查。移除释放槽、登记、意愿和预览，ID 不复用。ID/位置/窗口查询读取同步后的真实区间，EOF 保持半开；旧 EOF 锚在追加后可投影到同 ID 占位。增长、seal 和移除后旧布局 pass、ticket/checkpoint 及操作 continuation 按版本失效；语义 `DependencyKey` 可能仍 Valid，不能代替布局依赖。

`nix develop -c cargo run --release -p mdstream-core --example source_tail_folding` 演示增长和条件封口。测试复用 source folding 的朴素区间/高度/布局模型，覆盖八组 UTF-8 切点与前缀 EOF、真实语法父链和原始/显示映射、共享预算、预览/批量/导航、开放表格行双向登记、缓存失效、16/256/4096 历史登记及百万字隐藏尾。任意 pending 或语法宿主内起点、一般手工嵌套、交叉拆分/优先级、重载身份映射和 UI 仍保留待办，见 [交付记录](DONE.md#2026-11-12-稳定根起点的手工文档尾折叠范围自动增长)。

## 长段落源码前缀预览折叠

`SourceFoldRegistration::paragraph(document, view, block, preview_bytes)` 为已定型的顶层 `BlockKind::Paragraph` 注册前缀预览。它复用 `SourceFoldId`、预算注册和独立视图：`FoldCandidate::source` 是整个段落，`summary` 是保留的源码前缀，`hidden` 是余文。注册初始展开；成功后显式收起。`paragraph_fold_block` 查询注册绑定的稳定 `BlockId`，手工/已删除/其它视图 ID 返回 `None`。源码位置和窗口查询包含整个段落，实际隐藏查询只命中余文。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("visible prefix and a long remainder\n\n")?;
let mut view = FoldView::new(&doc, 1)?;
let mut task = SourceFoldRegistration::paragraph(&doc, &view, BlockId(0), 14)?;
let id = loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    if let Some(id) = task.advance(&doc, &mut view, &mut budget)?.id { break id; }
};
view.set_collapsed(&doc, id, true)?;
let candidate = view.fold_candidate(&doc, id)?.unwrap();
assert_eq!(doc.text(candidate.summary.unwrap()), Some("visible prefix"));
assert_eq!(candidate.hidden.start, 14);
let update = doc.append("following paragraph\n")?;
view.apply(&doc, &update)?;
assert_eq!(view.fold_candidate(&doc, id)?, Some(candidate));
# Ok::<(), Box<dyn std::error::Error>>(())
```

上限按原始源码 UTF-8 字节计，包含隐藏语法；向下对齐标量和 CRLF，允许 0，不接受覆盖整个段落的上限（没有可收起余文）。它不保证 N 个显示行或完整字素/单词/行内对象。截断保留已有样式与显示映射，不重新解释未显示的强调、代码、数学或链接尾部；额外的显示摘要用 `DisplayExcerpt` 限制源码、显示字节和 LF 行数。实体映射等仍沿用原始完整 replacement 范围；链接对象回调只交付元数据，宿主不得因此无预算读取整个隐藏标签。`LayoutPass` 保持原有 `RenderRun` 输入，实际显示规范化和终端列宽由接入方处理。

`paragraph` 只接受 `Sealed`、无容器、且完整位于 `source_fold_limit()` 内的段落。未完成段落、可能变为 Setext/表格的输入、容器物理叶块及其它块类型返回 `InvalidRange`。空行等局部闭合即可注册，不要求整个文档 finish；后续追加、引用晚定义、finish 和 reset 不改变注册身份、前缀或段尾。源码不可编辑，改变预览上限须移除后重新注册，生成新 ID。

整个段落包络占用一个不相交源码注册，包括仍可见的前缀；与手工范围重叠、重复注册均拒绝，与注册顺序和展开/预览状态无关。章节可以包含段落并隐藏其预览及占位，展开章节恢复段落原意愿；段落长度默认策略通过下文的 `ParagraphAutoFold` 按共享预算创建注册。All/Subtree/Siblings/Focus、临时展开预览、源码锚点和 Reveal 导航沿用 Source 身份及包络关系。表格、容器等不能出现在一个已定型顶层段落内部；继续使用端点路径验证保持结构范围不交叉，不改写语法父链。

段中边界的排版契约由两次回调表达：

- `FoldPlaceholder::paragraph == Some(block)` 表示收起的是该段落余文。占位接在当前可见前缀后，排版器在占位所属 `owner` tile 结束这一行并计入其高度，包括无换行 EOF；不要让隐藏尾部贡献高度。
- 跳过余文后，`FoldLayoutSink::paragraph_end(block)` 在下一段可见内容之前交付。它只复位换行列和段落局部状态，不再增加高度；默认无操作适用于无段落状态的 sink，有状态排版器应实现。即使从隐藏尾部中途开始、没有收到占位，也会收到这个结束回调；远处累计绝对高度仍按原契约保持估计。

占位和段尾恢复分别消耗 1 object，检查点重放期间另扣 recovery；两者之间预算耗尽保留待交付状态，不重复占位、不提前读取后文。跨 tile、同 tile 相邻段落和 EOF 使用同一流程。祖先折叠已隐藏整个段落时不产生额外的内层回调；展开或临时预览段落时消费原始内容。切换从隐藏起点惰性失效后续测量和检查点，旧任务不得继续写回。

创建仅做索引及固定次数 UTF-8/CRLF 端点探测；注册逐步检查至多两条顶层章节链，游标 O(1)，不读取前缀/余文。R 个注册占 O(R)，查询和切换沿用 O(log R + log F + log T)；不修改解析器，不为后续追加枚举旧段落。布局只读取有界前缀和后续可见窗口，完整隐藏内部按范围跳过，段尾上下文恢复不依赖余文长度。

`cargo run -p mdstream-core -r --example paragraph_folding` 演示百万字余文、12 列示例换行与追加后的独立段落。`crates/mdstream-core/tests/paragraph_folding.rs` 覆盖 640 组列宽/边界布局模型、1000 轮随机与 240 次流式可见性/高度模型、UTF-8 全切点/逐前缀、行内映射、百万字恢复、4096 个历史注册以及旧任务/测量拒绝。开放顶层段落增长、容器跨叶块身份及自动长度策略见下方；按逻辑/显示行数选择预览、媒体详情及完整终端排版仍待实现。

### 开放顶层段落的前缀折叠

`SourceFoldRegistration::open_paragraph(document, view, block, preview_bytes)` 显式注册当前尚未封口的顶层段落。它使用同样的稳定 `SourceFoldId`/`BlockId`、UTF-8/CRLF 前缀、两端路径共享预算、初始展开和整段不相交预留；上限须小于当前块已分类源码长度。短行尚未分类时没有可注册的段落，宿主可以等待换行或达到现有 `BLOCK_LOOKAHEAD` 窗口后重试；无需为了折叠调用 `finish()`。容器物理叶块、已封口段落和其它块类型不接受，已封口段落沿用 `paragraph`。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("visible prefix and body\n")?;
let mut view = FoldView::new(&doc, 1)?;
let mut task = SourceFoldRegistration::open_paragraph(&doc, &view, BlockId(0), 14)?;
let id = loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    if let Some(id) = task.advance(&doc, &mut view, &mut budget)?.id { break id; }
};
view.set_collapsed(&doc, id, true)?;
let update = doc.append("streamed continuation")?;
view.apply(&doc, &update)?;
let candidate = view.fold_candidate(&doc, id)?.unwrap();
assert!(candidate.open);
assert_eq!(candidate.hidden, Span::new(14, doc.source().len()));
let update = doc.append("\n===\nfollowing\n")?;
view.apply(&doc, &update)?;
// The same BlockId is now a heading; the paragraph choice has been removed.
assert!(view.fold_candidate(&doc, id)?.is_none());
assert!(!view.is_collapsed(id));
# Ok::<(), Box<dyn std::error::Error>>(())
```

活动时 `source.end`/`hidden.end` 暂定为当前源码 EOF，包含下一行尚未分类的输入；`summary` 和 `hidden.start` 固定。每次追加/finish 或合并通知后先 `apply`：普通增长保持 ID、折叠与临时预览意愿；空行、块中断或 EOF 封口后冻结为最终段落范围。若末行迟确认为表头，旧段落可能收缩；只有最终段尾仍大于固定截断点才保留注册。整块变成 Setext/表格或收缩后不再有隐藏余文时，原子移除注册、折叠意愿和临时预览；旧 ID 永不复用，不自动转交给新标题或表格。宿主通过 `fold_candidate == None` 或 `paragraph_fold_block == None` 观察失效；新语法对象继续按其自身默认策略初始化。

段尾回调依然表示这次布局已跳过的折叠尾部，即使文档尚未结束；追加使旧布局任务过期，宿主重建局部布局。收起范围增长、收缩或删除从原隐藏起点失效高度/检查点，过期测量和注册/枚举/导航任务均拒绝。展开及预览期间也维护活动范围，退出预览时使用最新边界；部分 tile 继续保留前缀/占位和后续可见内容，`paragraph_end` 的 object/recovery 预算及恢复规则保持不变。手工范围与整段预留不相交，章节祖先、批量和源码导航复用现有组合。

最多一个顶层段落活动，视图只保留一个更新槽；每次通知只查原 BlockId 和该注册，O(log R + log F + log T)，不扫描历史注册、正文或祖先。封口/移除后退出槽，旧 EOF tile 沿用一次性边界校正；每个视图状态独立。解析器和语法索引没有新增工作。15 项专项验收覆盖 384 组独立换行布局、240 轮流式区间/高度模型、UTF-8 全切点/逐前缀及合并通知、各种迟确认/失败、百万字增长与恢复、4096 历史注册、千层容器边界、预算暂停和旧任务拒绝。`paragraph_folding` release 示例现在先注册并收起开放段落，再追加百万字及后续段落。容器跨叶块身份、有预算候选发现及自动长度策略见下方；按显示行数截断、完整对象边界与终端排版仍在 TODO。


### 容器内多物理叶块段落的前缀折叠

`SourceFoldRegistration::container_paragraph(document, view, first, preview_bytes)` 接受已识别容器内的开放或已闭合逻辑段落；`first` 必须是首物理叶块的 `BlockId`。同一确切容器作用域内的显式/lazy 续行共用该身份，后续叶块、空容器标记、标题、代码、定义和表格不接受。叶块的 `Sealed` 只表示物理行结束，不能用它判断逻辑段尾；不合并叶块，不修改原有行内对象、样式或源码映射。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("> first line\n> second line\n").unwrap();
let mut view = FoldView::new(&doc, 1).unwrap();
let mut task = SourceFoldRegistration::container_paragraph(&doc, &view, BlockId(0), 7).unwrap();
let id = task.advance(&doc, &mut view, &mut Budget::new(WorkLimit::unlimited()))
    .unwrap().id.unwrap();
view.set_collapsed(&doc, id, true).unwrap();
let update = doc.append("> third line\n>\n> next paragraph\n").unwrap();
view.apply(&doc, &update).unwrap();
let candidate = view.fold_candidate(&doc, id).unwrap().unwrap();
assert!(!candidate.open);
assert_eq!(candidate.summary, Some(Span::new(0, 7)));
assert_eq!(candidate.source.end, doc.source().find(">\n").unwrap());
```

前缀按整个源码包络计字节，包含每行容器前缀、任务标记和隐藏语法；沿用 UTF-8/CRLF 向下对齐，上限必须小于已分类正文包络长度，不能只依靠未分类尾部留下余文。前缀可以跨多叶块，也可为零。开放尾部暂定到 EOF；空行、块中断、子容器/兄弟条目或作用域退出后冻结，返回外层的后续正文另起身份。EOF 也会封口。多行 Setext 改型使整个注册失效；晚表头从末叶块分离时，段尾收缩到表头物理起点，若余文消失则移除注册、意愿和预览。失败表头/分隔行仍留在原段落。宿主在逐次或合并 `apply` 后查询候选是否存在。

整段预留与手工/其它段落注册不相交，端点预算验证结构范围的包含关系；parent 是注册时的最内层章节或容器。祖先折叠、批量、临时预览和 Reveal 导航保留内层意愿。占位使用首叶块身份，`paragraph_end(first)` 与占位分开计费；结束回调只复位段落局部状态、不计高。后续可见叶块仍携带自身容器身份，宿主若需要完整祖先路径，须继续使用有预算的容器接口；本接口不恢复任意深度的终端缩进栈。

解析器只在真实逻辑段落闭合时写一个 `first -> end` 有序索引，`Work::paragraph_closures` 记录写入次数；P 个闭合段落占 O(P)，每次封口 O(log P)，不枚举物理叶块。活动首尾复用 `container_title/current`，普通增长不写此索引。注册构造 O(log P + log R)；随后两端祖先路径逐项共享 Budget，任务 O(1) 空间。每个视图仍仅有一个活动段落槽：普通增长 O(log R + log F + log T)，闭合时额外一次 O(log P) 查询；不扫描历史注册、正文或祖先。保留累计 O(n log n)、空间 O(n) 和摊还成本契约。

`cargo run -p mdstream-core -r --example container_paragraph_folding` 演示引用中的列表条目、跨物理叶块注册、百万字追加及兄弟条目恢复。15 项专项测试覆盖 288 组独立布局、240 轮/960 次切换的可见性与高度模型、UTF-8 全切点/逐前缀、CRLF/EOF、部分 tab/lazy 续行、长表头及长分隔行成功/失败、千层引用与 250 层列表两端路径、4096 历史段落/物理叶块、预算暂停、检查点恢复及旧任务/测量拒绝。有预算候选发现及自动长度策略见下方；显示行数与完整对象截断、媒体详情、完整容器语法及终端排版继续保留 TODO。

## 长段落有预算候选发现

`ParagraphFoldCandidates::new(document, window, min_bytes)` 在半开源码窗口中分帧发现可注册的顶层及容器逻辑段落。窗口从段中、任意物理续行或待分类尾部开始时，也返回更早的首叶块；结果按源码排序，同一段落只返回一次。空窗口无结果，窗口允许落在 UTF-8 标量内部，因为发现过程不读取字节。无效范围返回 `InvalidRange`。

`ParagraphFoldCandidate` 包含稳定首叶块 `block`、确切 `container`、整段 `source`、`classified_end` 与 `open`。`min_bytes` 是**整段已分类源码字节数的包含式下限**，包括语法、容器前缀、CRLF 等，不是窗口交集或显示长度；0 接受所有非空合格段落。开放段落的 `source` 暂定到当前 EOF，但待分类尾部不计入长度筛选，预览上限仍必须严格小于 `classified_end - source.start`。未形成首叶块的待分类输入、标题、代码、表格、空行、其它非段落叶块及未达到固定注册边界的闭合段落不返回。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("> first line\n> second line\n")?;
let mut task = ParagraphFoldCandidates::new(&doc, Span::new(16, 17), 20)?;
let mut out = Vec::new();
let mut budget = Budget::new(WorkLimit {
    queries: 1, objects: 1, ..Default::default()
});
assert_eq!(task.advance(&doc, &mut budget, &mut out)?, Completion::Covered);
let candidate = out[0];
assert_eq!(candidate.block, BlockId(0));
assert_eq!(candidate.source, Span::new(0, doc.source().len()));
assert!(candidate.open && candidate.container.is_some());
assert_eq!(budget.used().bytes, 0);
let mut view = FoldView::new(&doc, 1)?;
let mut registration = SourceFoldRegistration::container_paragraph(
    &doc, &view, candidate.block, 8
)?;
// 本帧发现已用完预算，注册必须保留进度，下一帧再处理祖先路径。
assert_eq!(registration.advance(&doc, &mut view, &mut budget)?.completion,
    Completion::BudgetExhausted);
# Ok::<(), Box<dyn std::error::Error>>(())
```

构造 O(1)，每次检查一个逻辑段落或非段落物理叶块，原子扣 1 query / 1 object；不匹配长度的段落和结束查找也计费。多叶块段落直接跳到逻辑段尾，长度不符也不逐叶块检查。零正文字节、tile、recovery 消费；预算不足返回 `BudgetExhausted` 并保留位置，`Covered` 后同 revision 的重复调用免费。每次成功进入 `advance` 清空并复用调用方 Vec；文档归属/版本错误不修改输出和预算。输出容量受当帧对象预算约束，Vec 扩容沿用摊还成本契约。

游标和结果属于当前文档 revision。非空 append/首次 finish 使旧游标返回 `StaleContinuation`，包括已完成或空窗口游标；另一文档返回 `WrongDocument`。普通增长后可以仅查询新增字节窗口，并重新发现包含它的原段落。Setext/整表迟确认移除候选，晚表头分离会收缩段尾，作用域退出使它封口；若宿主缓存了未注册候选，应重新查询受影响的原窗口，不能只靠新增字节窗口推断旧候选是否仍存在。已注册段落沿用 `FoldView::apply` 的收缩/移除/稳定身份规则。

发现不读取或改变视图：折叠、临时预览、注册、测量及 resize 不使游标失效，结果也包含已注册或已被祖先隐藏的段落。宿主选择候选后，按 `container` / `open` 使用 `container_paragraph` / `open_paragraph` / `paragraph`，继续共享当帧 Budget；重复、手工重叠和祖先交叉由已有注册验证拒绝。发现不会自动创建注册、替用户收起内容或选择预览前缀。默认长度/自动注册可使用下节的独立预算任务；显示行数及完整字素/对象截断仍保留 TODO。

每步只做固定次数 O(log B + log P) 索引查询（B 为物理块，P 为闭合容器段落），复用块的有序源码范围、已有 `first -> end` 索引和唯一活动段落端点；不新增解析索引、不遍历祖先或历史段落。任务 O(1) 空间。窗口内非段落叶块逐项计费，因此整窗完成成本取决于实际访问项；单帧不会无预算全量枚举。千层/百万字、4096 物理叶块及 4096 历史段落中的单段窗口均只需一个步骤，250 层列表的发现不提前支付其 1000 次注册祖先检查。

`crates/mdstream-core/tests/paragraph_discovery.rs` 的 14 项验收覆盖 400 个独立段落包络、1600 组随机窗口/长度筛选、UTF-8 全切点/逐前缀、CRLF/EOF/部分 tab/lazy、迟确认成功/失败、预算原子暂停、视图变更/独立注册、通知合并与旧任务/测量拒绝、240 轮流式范围/可见性模型及百万字后 1000 次逐字追加。`cargo run -p mdstream-core -r --example paragraph_discovery` 演示从百万字段中发现首叶块、共享预算注册及新 revision 的局部重查。

## 长段落自动注册与长度默认策略

`FoldDefaults::paragraphs: Option<ParagraphFoldPolicy>` 为每个视图启用源码字节长度策略；默认 `None`。`min_bytes` 是已分类原始字节数的包含式下限，`preview_bytes` 是保留的原始前缀上限；两者均包含语法、容器前缀及换行，前缀沿用 UTF-8/CRLF 向下对齐规则。长度必须同时达到下限且严格大于前缀上限，0 和 `usize::MAX` 不会溢出。待分类尾部不帮助跨阈值。

`ParagraphAutoFold::new(document, view, window)` 自动发现窗口内的顶层及容器逻辑段落，按两端祖先路径验证并原子注册、初始化为收起。`for_update(document, view, update)` 在 `view.apply` 后，根据首个 `changed_blocks` 与追加起点构造到 EOF 的窗口；新段落、旧段落跨阈值、无追加字节的 finish、合并通知和迟确认均被覆盖。不根据全文样式/引用失效区间扩展窗口，不逐帧枚举历史。启用/修改默认值自身不执行扫描，`apply` 只维护已有注册；宿主用共享预算驱动任务。需要对已有文档初始化时，显式提供相应窗口。

```rust
use mdstream::*;
let mut doc = Document::new();
let mut view = FoldView::new(&doc, 1)?;
view.set_defaults(FoldDefaults {
    paragraphs: Some(ParagraphFoldPolicy { min_bytes: 12, preview_bytes: 4 }),
    ..Default::default()
});
let update = doc.append("> first line\n> continuation\n")?;
view.apply(&doc, &update)?;
let mut task = ParagraphAutoFold::for_update(&doc, &view, &update)?;
loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    let progress = task.advance(&doc, &mut view, &mut budget)?;
    assert_eq!(budget.used().bytes, 0);
    if progress.completion == Completion::Covered {
        assert_eq!(progress.registered, 1);
        break;
    }
}
let candidate = view.source_fold_at(&doc, 0)?.unwrap();
assert!(view.is_collapsed(candidate.id));
view.set_collapsed(&doc, candidate.id, false)?;
let update = doc.append("> more\n")?;
view.apply(&doc, &update)?;
assert_eq!(view.source_fold_at(&doc, 0)?.unwrap().id, candidate.id);
assert!(!view.is_collapsed(candidate.id));
# Ok::<(), Box<dyn std::error::Error>>(())
```

阈值只决定首次准入：已注册段落的 ID、前缀及展开/收起意愿不会被新默认值或后续任务覆盖。普通增长与封口保持身份；迟确认收缩后只要还存在隐藏后缀便保留，即使长度已低于准入阈值。Setext/整表改型或后缀消失时，由 `apply` 移除注册及预览，不转交给新语法对象。因此，**自动策略的执行时机会影响是否曾准入**：在长表头回退前已注册的短残留段落可保持注册，直到回退后才处理的视图则可能不注册；不同通知/预算调度不承诺完全相同的视图选择。相同处理时机下，逐 UTF-8 前缀的更新窗口与全窗策略结果一致；语法分块契约不变。

显式意愿优先。已有手工或自动注册（包括展开及预览中）直接跳过，不改变前缀与选择；重叠和结构交叉也跳过并记入 `skipped`，任务可继续处理后续段落。`set_paragraph_auto_excluded(document, first_block, true)` 在跨阈值前也可排除段落；`false` 恢复自动准入。显式 `remove_source_fold` 移除段落时同时排除该首叶块，避免下一帧加回；结构迟确认移除不设置排除。排除不删除已有注册，不影响手工注册，随视图保存并跨 reset/默认值切换保持，空间为 O(E)，E 为显式排除的首叶块数，不积累操作快照。父节点、批量和临时预览仍使用既有折叠组合规则。

任务构造 O(1)，持有 O(1) 游标与至多一个未完成注册。每个发现步骤（包括非段落及长度不符）扣 1 query / 1 object；索引查询和排除/重叠检查均固定次数。两端路径、结束查找及最终插入继续按注册预算逐步计费；最终付费步骤同时初始化收起，没有零预算提交。正文、tile 和恢复预算均不消耗；只是前缀对齐最多探测一个 UTF-8 标量及 CRLF。任务输出累计 `inspected` / `registered` / `skipped` 和本次新增折叠区间包络 `change`。已完成注册立即可用；未完成注册不修改视图，取消任务不会撤销已完成部分。

append/finish、默认值、排除、其它折叠操作和 reset 使旧任务失效，归属/版本检查先于预算及修改。若旧任务未处理完，应保存 `remaining_range()`，与新 `for_update(...).remaining_range()` 取包络后，用 `new` 在新 revision 重建；已有注册直接跳过。仅消费最新更新窗口会遗漏此前积压工作，库不持有隐式后台队列。完成任务的空续调免费；`None` 策略为零步，也不会展开或移除历史注册。新的收起区间沿用测量票据、检查点和布局任务失效规则。

16 项专项验收覆盖 400 段/400 随机窗口长度模型、240 轮/960 次切换的流式区间与高度模型、72 组手工/自动及大小预算布局对照、UTF-8 全切点/逐前缀、CRLF/EOF、部分 tab/lazy、跨阈值、晚表头收缩与 Setext/整表移除、多视图/排除/预览/批量、未完成窗口合并和通知重放。千层引用首次注册总计 2006 个步骤，250 层列表为 1006；4096 物理叶块只发现一次，4096 历史注册后百万字段落逐字追加 1000 次，每次仅一个发现步骤与一次活动范围刷新，无正文或祖先回扫。`cargo run -p mdstream-core -r --example paragraph_auto_folding` 演示 8 帧自动注册、零正文读取和显式展开保留。按逻辑/显示行数选择前缀、完整字素/对象截断、媒体详情及终端 UI 继续保留 TODO。

## 表格连续行范围折叠

`FoldView::insert_table_rows(document, table, start..end)` 注册一个视图内的固定正文行范围，返回 `FoldId::TableRows(TableRowRange { table, start, end })`，初始展开。行号沿用 `Table::row`：第 0 行是表头；仅接受 `1 <= start < end <= rows.len()`，end 不含。表头、分隔行和范围外正文保留；范围内完整物理行（含容器前缀）合并成一个占位。摘要指向首个选中行，读取仍受 `summary_text` 或 `DisplayExcerpt` 的字节/帧预算限制。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("| h |\n| - |\n| first |\n| hidden |\n")?;
let mut view = FoldView::new(&doc, 1)?;
let rows = view.insert_table_rows(&doc, TableId(0), 2..3)?;
view.set_collapsed(&doc, rows, true)?;
let update = doc.append("| later |\n")?;
view.apply(&doc, &update)?;
let candidate = doc.fold_candidate(rows).unwrap();
assert_eq!(candidate.parent, Some(FoldId::Table(TableId(0))));
assert_eq!(candidate.hidden, doc.table(TableId(0)).unwrap().row(2).unwrap().source);
assert!(view.is_collapsed(rows)); // later is outside the fixed row interval
# Ok::<(), Box<dyn std::error::Error>>(())
```

范围身份由 TableId 和两个行号组成，与显示列宽、折行和字节增长无关，只在所属 Document 内有效。精确重复注册保留既有状态；同一视图内所有已注册范围必须不相交（包含、交叉均返回 `InvalidRange`），即使当前展开或临时预览也如此。相邻区间独立保留两个占位。不接受空范围、表头、未索引或未来行；调用方先通过 `Table::row` / `row_at` 选定范围。`remove_table_rows` 移除注册及局部状态，若正在预览则结束该预览；随后可重设边界。其它视图可注册不同范围，不相互约束。`table_row_range_count` 返回当前注册数。

已索引的未完成末行可以选入：该行追加时源码终点同步增长，`FoldCandidate::open` 表示末行尚未完成；换行或 finish 后固定。未分类 pending 尾部始终不在行范围内，新正文行也不自动加入。完整行恰好结束于当前 EOF 时，下次更新只刷新这一个范围的 tile 边界，避免旧 EOF 的未满 tile 吞掉新追加的可见行；无需刷新历史区间。修改范围须移除后重注册，不隐式合并或裁切。

行范围的直接父节点总是整表。整表、容器或章节遮蔽其占位，展开祖先后恢复行范围意愿。`set_collapsed`、`toggle`、`set_preview`、`FoldAnchor`、有预算 `FoldNavigation` 和 `LayoutPass::folded` 均支持已注册范围；直接使用未注册行范围修改视图返回 `InvalidRange`。`Document::fold_candidate` 可查询任意有效行范围的 O(1) 元数据，但文档候选枚举/`fold_candidate_at` 只发现语法节点，不生成任意行组合；被隐藏源码的实际行范围身份通过 `FoldView::hidden_at` 查询。

`FoldBatch` 包含当前视图已注册范围：All、Subtree、Siblings、Focus 沿用父子规则，AtLevel 只作用于标题，Containers 只作用于容器；每个行范围按 1 query / 1 object 分步处理，默认策略不自动创建行范围。批量先消费语法候选，再消费选中源码窗口内已注册行范围；中间结果立即可用。注册/移除及其它视图变更使旧批量、导航和布局任务失效，重置布局保留注册及意愿。`TablePreview` 的独立摘要读取不受行遮蔽影响，也不修改注册或意愿。

设 R 为已注册行范围数：有序起点索引通过相邻项验证重叠、前驱查询查找点覆盖，注册 O(log R)、单点折叠与移除 O(log R + log F + log T)，不枚举覆盖行、列、字节或 tile。非空且不相交保证注册存储 O(正文行数)，反复切换不保留历史。追加最多刷新一个未完成行范围或一次旧 EOF 边界；叠加原有至多六个根章节与一张整表/代码块，保持既定增量复杂度。布局、导航、摘要和批量使用共享 Budget；直接注册、移除和单点切换是有限次数索引操作，和已有 `set_collapsed` 一样不自行驱动排版。占位高度归属范围起点 tile，完整隐藏内部使用区间遮蔽，边界及新可见内容按帧预算消费。

`cargo run -p mdstream-core -r --example table_row_folding` 演示百万字末行、固定行范围、整表状态恢复及后续行可见性。真实表格边框、列宽、软换行、占位绘制，以及自动随新行延长的尾部策略仍由后续工作推进。

## 表格前几行有界预览

`TablePreview` 为整表折叠提供独立的摘要读取任务：`rows` 包含表头（第 0 行），跳过分隔行，最多读取前 N 个已索引行；`source_bytes` 与 `display_bytes` 是所有行共用的总上限，不逐行重置。源码上限包含行内隐藏标记，UTF-8 不截断标量；超长首个单元格也不能绕过上限。返回值保留行号、样式、链接和绝对源码映射，不合成列分隔、隐式空单元格、边框或省略号。`RowsEnd` 表示读完当前已索引行，`RowLimit` 表示仍有行未选入；字节上限返回 `SourceLimit` / `DisplayLimit`，不向后扫描确认余文可见。沿用 `DisplayExcerpt` 的停止语义：多个字节上限同时触及时，读取窗口可能影响报告哪个原因，但不影响输出前缀。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("| name |\n| - |\n| **世界** |\n| later |\n")?;
let mut view = FoldView::new(&doc, 1)?;
view.set_collapsed(&doc, FoldId::Table(TableId(0)), true)?;
let mut preview = TablePreview::new(&doc, TableId(0), TablePreviewLimits {
    rows: 2, source_bytes: 128, display_bytes: 64,
})?;
let mut budget = Budget::new(WorkLimit::unlimited());
loop {
    match preview.next(&doc, &mut budget)? {
        TablePreviewStep::Run { row, run } => println!("{row}: {}", run.text),
        TablePreviewStep::Complete(end) => {
            assert_eq!(end, TablePreviewEnd::RowLimit);
            break;
        }
        TablePreviewStep::BudgetExhausted => break, // retain task for next frame
    }
}
assert!(view.is_collapsed(FoldId::Table(TableId(0))));
# Ok::<(), Box<dyn std::error::Error>>(())
```

读取不改变整表占位、隐藏范围、高度、临时展开状态或父子折叠意愿；父章节收起时，是否显示该摘要由宿主决定。它提供行摘要数据，**不把前 N 行从整表遮蔽区间中释放**。实际表格布局、摘要占位高度及屏幕软换行上限仍由后续渲染器处理。

构造和存储 O(1)；每个选中行的初始化消费共享 Budget 的 1 query / 1 object，读取显示片段沿用 `DisplayExcerpt` 的计费，6 bytes、1 query、1 object 每帧可持续推进（行初始化可能独占一帧）。不枚举列、单元格或未选入行，R 个访问行、S 个检查源码字节的成本为 O(R + (S + 1) log(n + 1))。返回文本借用文档，输出存储由调用方管理。

任务和输出绑定文档身份与 revision；追加或首次 finish 后返回 `StaleContinuation`，调用方按需重建，不自动重读历史行。未分类的 pending 尾部不属于已索引行；已索引但未完成的行按当前源码范围读取，可由 `TableRow::complete` 判断。错误文档/过期任务在扣费前拒绝，完成可免费重复读取。`cargo run -p mdstream-core -r --example table_preview` 演示收起百万字表格、追加新行后按小预算读取前两行。

## 临时折叠预览

`FoldView::set_preview(&doc, Some(id), &mut budget)` 暂时展开一个已支持的章节、代码块、表格或容器；传入 `None` 结束预览，切换 ID 则先恢复旧节点。每个视图同时只有一个预览。祖先和后代的折叠意愿保持不变，因此被父章节隐藏的节点不会因预览而自动露出；展开父章节的预览仍保留子节点占位。这里提供暂时展开整节点的视图状态，实际只排版当前窗口；按逻辑行/字节截断的摘要读取见下文。

```rust
use mdstream::*;
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

开始、切换及结束各原子消费 1 query、1 object，支付最多两个节点的固定次数索引操作；O(log H + log C + log F + log T)，只额外保存一个 ID 和布尔意愿，O(1) 空间。不读取正文、不枚举覆盖的 tile、不保存历史快照。相同目标为无操作且不扣预算；预算不足返回 `BudgetExhausted`，保留当前预览和所有状态，可在后续帧重试。错误文档、未同步视图和非法候选在扣费及修改前被拒绝。预览变化返回旧/新隐藏范围的包络和最新版本，并使受影响的高度、检查点、测量和继续任务失效；即使节点原已展开或范围为空也增加版本。实际绘制继续使用共享 Budget 的剩余额度，或留待后续帧。

`cargo run -p mdstream-core -r --example folding` 演示父章节预览、子折叠保留及结束恢复。`crates/mdstream-core/tests/fold_preview.rs` 和折叠排版模型覆盖预算原子性、显式意愿、流式范围、独立视图和旧任务拒绝；百万字节点的切换不读正文，进入预览后的排版仍受窗口及字节预算限制。

## 跳转历史与书签

`Bookmark::new(&doc, source, viewport_offset)` 保存文档身份、精确 UTF-8 源码锚点、捕获时所在的章节/代码/表格 ID，以及目标行相对视口顶端的行偏移。源码应来自实际排版或精确跳转目标；普通滚动后先用 `Navigator::reconcile_source` 回填映射，不能把估计的 tile 起点当作精确返回位置。书签可单独保存，并通过 `Bookmark::navigation` 创建最小路径展开任务；可以用于同一文档的其他视图。追加、finish、Setext 晚确认和 resize 后仍以保存的源码位置为准，目标 ID 仅是捕获时的元数据。EOF 使用左亲和，新增内容不会移动旧 EOF 书签；重载后的新文档身份会被拒绝。

`NavigationHistory` 保存当前条目和线性的前进/后退分支。普通滚动用 `replace_current` 更新当前返回位置；新跳转成功后才 `push` 目标并截断前进分支。完全相同的书签是无操作，保留前进分支；相同源码、不同偏移或目标元数据则是不同条目。`navigation` 在历史边界返回 `None`，否则创建可暂停的 `HistoryNavigation`；完成后自动移动历史游标，宿主将返回的 `ScrollTarget` 应用到 Navigator。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("# Guide\n## Details\nbody\n# Next\ntail\n")?;
let mut view = FoldView::new(&doc, 1)?;
let mut nav = Navigator::new();
let body = doc.source().find("body").unwrap();
let saved = Bookmark::new(&doc, body, 2)?;
let mut history = NavigationHistory::new(saved);
let destination = doc.headings().get(HeadingId(2)).unwrap().source.start;
let target = nav.navigate(&doc, view.heights(), NavAction::Source(destination), 4).unwrap();
history.push(Bookmark::new(&doc, target.source.unwrap(), 0)?)?;
view.set_collapsed(&doc, FoldId::Heading(HeadingId(0)), true)?;

let mut back = history.navigation(&doc, &view, HistoryDirection::Back, 4)?.unwrap();
loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, ..Default::default()
    });
    let progress = back.advance(&doc, &mut view, &mut history, &mut budget)?;
    if let Some(target) = progress.target {
        nav.apply_target(target);
        assert_eq!(target.source, Some(body));
        assert_eq!(history.current(), saved);
        break;
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

书签恢复和历史任务复用 `FoldNavigation` 的共享 Budget，每步消费 1 query / 1 object，不扫描正文、不复制折叠树；最终定位步骤同时支付 O(1) 的历史游标更新。预算耗尽、取消和错误都不提前移动游标；取消时已展开的节点保持展开。历史条目变化或另一任务完成会使旧任务失效，错误历史实例返回 `WrongHistory`；文档/视图更新沿用原导航的过期检查。任务完成后重复调用不再移动游标。高度回填可在任务中途进行，最终定位使用最新高度；append 或 resize 后应从仍有效的书签/历史重新创建任务。

返回的 `top` 是估计位置。实际布局按预算定位到 `source_row` 后，用 `bookmark.viewport_top(source_row, viewport_rows)` 计算视口顶端，再调用 `Navigator::reconcile_source`，传入保存的源码目标。该计算在视口缩小时限制偏移，并对文档开头饱和减法；文档末尾的顶端限制由 Navigator 完成。历史完成表示已获得源码目标和估计位置，实际终端行定位仍由排版接入完成。

捕获书签只做 O(log B + log H) 的索引查询，每个书签/任务占 O(1) 空间。历史共 O(J) 空间，J 为保留条目数；Vec 容量保留历史峰值，追加摊还 O(1)，扩容可能复制现有条目，不承诺追加历史的硬时延。`cargo run -p mdstream-core -r --example history` 演示返回折叠代码、追加与 resize 后恢复、前进和历史边界。`crates/mdstream-core/tests/history.rs` 包含固定种子 2000 次操作模型、同 tile 重复标题、普通滚动、分支截断、身份/过期/取消、流式语义变化和百万字隐藏路径验证。

## 定界数学核心

数学使用本库明确限定的美元符号方言，不声明与某个数学 Markdown 扩展兼容：完整且未转义的 `$` 串长度为 1 时打开 inline 候选，为 2 时打开 display 候选；遇到相同长度的串才闭合。长度大于 2 的串在外部为普通文字，在候选内为原始内容；不同长度不嵌套、不切分匹配。没有空白、数字或货币启发式，例如 `$5 and $10` 会识别 `5 and `。`$$` 在行内上下文单独出现是未闭合 display opener；独占物理行时按下方独立数学块处理。连续 `$$$$` 是文字，无法表示空行内公式；空白内容允许。

代码 span/代码块、链接目标及 title、引用定义、正在消费的自动链接等既有上下文先于数学。数学内容内不解析强调、实体、链接、代码或另一个数学对象；原始反斜线保留，一个反斜线使紧随的一个字节不参与定界，连续反斜线按奇偶处理。外部 `\$` 仍是既有标点转义，内部 `$a\$b$` 内容为 `a\$b`。外层强调/链接 label 可以包含数学，显示 run 可同时带这些样式/链接。

两种候选均可跨同一顶层段落的普通换行，但不跨块或表格单元格边界。行内 `$$…$$` 只是对象的展示类型提示；独占行定界另按[独立数学块](#独立数学块)处理。在行内上下文，空行、标题、Setext、表格、围栏等仍先按块规则分类，管道仍先按表格规则分列；ATX 标题限单物理行，Setext 可迟确认多物理行正文；容器正文支持同一确切作用域的显式/lazy 续行，原始 content 包含前缀，另提供去前缀读取（见下节）。表格中的 `\|` 不分列，数学内容区间仍包含原始反斜线。`MathContent` 返回原始 CRLF；通用显示读取仍采用既有 CR 隐藏规则。块分类的 1 KiB 前缀缓冲、表头候选的延迟解析仍适用；顶层及同容器未闭合数学与晚管道长表头的确认/失败规则见[表格](#表格任务列表与图片)。

只有闭合确认才分配稳定 `MathId` 和 `Math { kind, source, content }`，source 包含两端定界符、content 排除定界符；容器跨行对象的两者都是包含前缀的原始包络。确认时内容标 `Style::MATH`，定界符标 HIDDEN，并发布范围失效及 Structure 变更，即使调用方关闭样式尺寸失效，旧测量仍被拒绝。尚未确认时既有显示通道呈现原始内容；块结束/EOF 未闭合时保留原文、丢弃候选，不重放期间跳过的行内语法。美元串需等下一非美元字节或行内作用域结束才确定长度；流式尾部 `$x$` 在追加更多美元符号前不能提前发布对象。

`Update.new_maths` 只报告新确认 ID 范围，支持 merge、append_into/finish_into、空追加和重复 finish。确认后的区间不会因后续追加、Setext 转换或容器关闭而变更；`maths()` / `math(id)` 借用元数据。`maths_in(range)` 创建 O(1) 游标，第一次 advance 在扣费后用 O(log M) 索引定位重叠对象，后续逐项 O(1)，完整对象按 source 与窗口相交判断；空范围不输出。每次检查（含终止查找）计 1 query / 1 object，不读正文，预算不足时不推进，完成后重复读取免费。

`math_content(id)` 创建 O(1) 原始内容游标，每次 advance 最多读取 `TILE_BYTES` 个 UTF-8 字节，计实际返回字节和 1 query / 1 object。下一标量放不下时原子暂停，4 bytes 足以推进任意标量；与其它消费者共用 Budget。没有复制完整公式，也不执行 TeX 校验、布局或字体塑形。两个游标均借用文档，Rust 借用保证消费期间不能追加；释放后可用稳定 ID 建新游标。通用 `LayoutPass` 仍将数学作为带 MATH 样式的文字回退，实际数学对象布局由后续消费方在预算下接入；收起的章节/容器继续跳过其数学正文。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("value $中🙂 + x_1$ and $$y^2$$\n")?;
doc.finish();
assert_eq!(doc.maths().len(), 2);
let mut objects = doc.maths_in(Span::new(0, doc.source().len())).unwrap();
let mut budget = Budget::new(WorkLimit {
    queries: 2, objects: 2, bytes: 4, ..Default::default()
});
let (id, math) = objects.advance(&mut budget).unwrap().unwrap();
assert_eq!(math.kind, MathKind::Inline);
let mut content = doc.math_content(id).unwrap();
assert_eq!(content.advance(&mut budget).unwrap().unwrap().text, "中");
assert_eq!(content.advance(&mut budget), Err(Completion::BudgetExhausted));
// 用下一帧的新预算继续；也可以丢弃游标，取消读取。
# Ok::<(), Box<dyn std::error::Error>>(())
```

解析只增加一个 O(1) 候选；每个美元串只决议一次（`Work::math_steps`），所有正文只走既有单遍 `inline_bytes` 消费。闭合做常数次 O(log n) 区间标记，不查正文；额外对象存储 O(M)，跨行去前缀索引另计 O(G)（见下节），全库保持累计 O(n log n)、空间 O(n)。Vec 扩容及批量解析仍是摊还保证，不承诺单次 append 的硬时延。`crates/mdstream-core/tests/math.rs` 和现有分块/fuzz 快照验证方言、预算、失败输入、百万字晚闭合及索引模型；`cargo run -p mdstream-core -r --example math` 演示流式确认与逐帧原始内容读取。

独立数学块见[独立数学块](#独立数学块)；后续仍保留 `\(...\)` / `\[...\]` 等额外定界方言、TeX 解析/排版和实际对象回调接入；脚注的独立核心接口见[脚注核心](#脚注核心)。

## 独立数学块

顶层或显式 Quote/Item 内，完整物理行去除容器前缀后，至多三列前导 space/tab（含前缀部分 tab 余量，按物理行四列制表位计量）、恰好 `$$`、可选 space/tab 和 LF/CRLF（或 EOF）构成定界行。第一行开启数学块，相同规则的下一行关闭；不接收 info、额外美元符号、转义定界或四空格缩进定界。`$$x$$` 继续按行内 display 数学解析。空数学块允许，空行也是正文。围栏/缩进代码中的 `$$` 保持代码字面内容；数学块能中断段落、lazy 正文和表格，正文中的 Markdown、代码围栏、表格、定义、美元串及反斜线全部保持字面，不产生行内对象。

定界行沿用含容器前缀的 `BLOCK_LOOKAHEAD` 分类窗口：短行等 LF/EOF，长行在窗口到达时一次分类；超长开闭候选保持普通正文或数学正文，不在长尾重试。已打开的数学块正文可无限追加。容器内只匹配已有的确切路径，不把正文标记解释为新容器；必须显式续接，缺失前缀、兄弟条目或作用域退出封存旧块，当前行再按外部规则处理。不存在数学块的 lazy 续行。EOF 未闭合也封存为数学块，不恢复成普通段落、不回放正文。

稳定身份是 `BlockId`，`BlockKind::Math { content, closed }` 保存当前内容包络与是否见到合法关闭行，`Block::state` 区分 Growing/Sealed。`source` 包括完整开闭物理行和容器前缀，`content` 排除整条定界行，保留正文前缀和原始 CRLF；空块的 content 为 opener 末端的空区间。未分类短尾还不属于已提交 content；长正文分类后只随新增字节增长。`new_blocks` / `changed_blocks` 报告新增、增长和封存，merge、空追加和重复 finish 沿用既有契约。它们不进入只含稳定闭合行内对象的 `maths()` / `new_maths`。

`math_block_content(BlockId)` / `math_block_payload(BlockId)` 均 O(1) 构造 `MathContent`，后者去除匹配的容器前缀并展开保留的部分 tab。逐步返回的 `MathChunk.source` 连续覆盖 content，正文借用原文、省略的前缀返回空 text，部分 tab 单字节返回 1..3 个静态空格；其它空白、CRLF、反斜线和实体拼写不变。每次最多 `TILE_BYTES` 个 UTF-8 源码字节，计实际字节与 1 query/1 object，前缀也收费；预算不足原子暂停。首次有效读取付费后 O(log G + log T) 定位间隙和部分 tab，之后 O(1)，不分配或复制整段公式。通过 `block_at` 定位数学块，或在窗口内用有预算 `fold_candidates` 枚举结构；消费期间文档借用阻止追加。

```rust
use mdstream::*;
let mut doc = Document::new();
let first = doc.append("> $$\r\n").unwrap();
assert_eq!(first.new_blocks, 0..1);
doc.append("> 中 + \\alpha\r\n> $$\n").unwrap();
let id = BlockId(0);
assert!(matches!(doc.block(id).unwrap().kind, BlockKind::Math { closed: true, .. }));
assert_eq!(doc.block(id).unwrap().state, SyntaxState::Sealed);
let mut cursor = doc.math_block_payload(id).unwrap();
let mut payload = String::new();
loop {
    let mut budget = Budget::new(WorkLimit {
        bytes: 4, queries: 1, objects: 1, ..Default::default()
    });
    let Some(chunk) = cursor.advance(&mut budget).unwrap() else { break };
    payload.push_str(chunk.text);
}
assert_eq!(payload, "中 + \\alpha\r\n");
assert!(doc.maths().is_empty());
```

通用显示/布局以 MATH 样式字面回退，隐藏定界行/容器前缀和 CRLF 的 CR；实际 TeX 排版及对象回调仍留后续。正文增长和封存以一次块范围失效拒绝旧缓存/测量，不遍历旧 tile。`FoldId::Math(BlockId)` 隐藏整块，以所在局部章节或容器为父；`FoldDefaults::math` 仅初始化新节点，显式意愿、独立视图、批量、预览和源码导航遵循现有规则。折叠范围暂含未分类尾部，作用域退出后收回；当前 EOF 关闭的块在后续追加时刷新末 tile 的覆盖，避免隐藏同 tile 的新文字。默认占位符不读取正文，内容预览可显式使用上述预算游标。

实现复用围栏路径和叶块索引，不增加数学块 arena；每块固定大小元数据，每个省略前缀一个有序间隙。分类只读有界窗口，正文不走行内解析，同物理行百万字增长不增加间隙或祖先更新；闭合/增长的范围标记是 O(log n)，历史块不参与追加刷新。累计解析 O(n log n)、空间 O(n)，不承诺单次 append/finish 的硬时延。`crates/mdstream-core/tests/math_blocks.rs` 及分块快照覆盖 400 组源码/内容模型、240 次流式高度模型、千层引用、250 层列表、百万字、4096 历史块、预算与折叠；`cargo run -p mdstream-core -r --example math_blocks` 演示去前缀内容与导航分帧消费。

## 容器跨行数学内容

同一确切 Quote/Item 内的连续正文保留 `$` / `$$` 候选，支持显式、等价前缀及 lazy 续行。定界仍按完整美元串长度匹配，保留反斜线逐字节转义和 opaque 内容；前缀不参与定界，LF/CRLF 仍参与原始数学内容。空行、兄弟条目、内外容器切换、标题、代码、定义和表格单元格边界结束候选；失败或 EOF 未闭合不回放正文。多物理行 Setext 晚提升保留已确认对象并结束剩余候选。数学在首管道前闭合按[长表头规则](#表格任务列表与图片)隔离或取消探测；未闭合或可隔离闭合时，成功丢弃旧候选并独立消费当前单元格，失败沿原状态提交或继续。

`math_content(id)` 继续读取完整原文；`math_payload(id)` 返回同类型 `MathContent` 游标，去除未送入数学解析器的容器前缀及顶层段落续行缩进，并把保留的部分 tab 映射为 1..3 个空格。它保留其它空白、CRLF、实体拼写和反斜线，包括表格中显示层会隐藏的 `\|` 反斜线；不等同于过滤通用 HIDDEN 样式。`MathChunk.source` 连续覆盖原 content 包络，普通正文 text 精确借用该区间，省略的前缀片段 text 为空，部分 tab 的 text 为静态空格且 source 为完整单字节区间。拼接 text 得到数学正文，保留 chunk 即可映射回原始源码。

两种读取都受共享 Budget 约束：每步最多消费 `TILE_BYTES` 源码字节，计 1 query / 1 object 和实际消费的源码字节数，空文本前缀也计费；四字节预算足以推进任意标量，预算不足时位置和预算均不变。完成后读取免费。构造 O(1)，第一次成功推进用已计费的 query 二分定位前缀间隙及部分 tab，后续逐片 O(1)；游标借用文档、状态固定大小，无正文副本。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("> - $$中 + \\alpha\r\n")?;
assert!(doc.maths().is_empty());
let update = doc.append(">   🙂$$\n")?;
assert_eq!(update.new_maths, 0..1);
let math = *doc.math(MathId(0)).unwrap();
assert_eq!(doc.text(math.content), Some("中 + \\alpha\r\n>   🙂"));
let mut cursor = doc.math_payload(MathId(0)).unwrap();
let mut output = String::new();
loop {
    let mut frame = Budget::new(WorkLimit {
        queries: 1, objects: 1, bytes: 4, ..Default::default()
    });
    let Some(chunk) = cursor.advance(&mut frame).unwrap() else { break };
    assert_eq!(frame.used().bytes, chunk.source.len()); // Includes omitted prefixes.
    if !chunk.text.is_empty() {
        assert_eq!(doc.text(chunk.source), Some(chunk.text));
    }
    output.push_str(chunk.text);
}
assert_eq!(output, "中 + \\alpha\r\n🙂");
# Ok::<(), Box<dyn std::error::Error>>(())
```

实现仅在活动数学候选中保存上次消费末端；下次跨物理行的不连续 feed 到来时追加一个有序源码间隙，不检查正文或遍历祖先。总间隙数 G 不超过省略前缀的行数；失败或被表头丢弃的候选可留下无对象引用的间隙，但不会与后续对象重叠。记录总成本 O(G)，空间 O(G)，百万字同物理行增长不增加此索引；闭合仍只做常数次范围标记、不回扫历史行或间隙。全库累计 O(n log n)、空间 O(n)，单次解析及存储扩容仍为摊还契约。查询构造不读取/复制完整公式，也不分配新输出。

物理叶块和已确认 MathId/区间保持稳定，Update 报告新对象并使受影响的缓存、测量、检查点与旧搜索任务失效，折叠意愿保留。`crates/mdstream-core/tests/container_math.rs`、分块快照和内部存储测试覆盖 400 组源码映射模型、UTF-8 全切点/逐前缀、千层引用、250 层列表、百万字、4096 条 lazy 续行及历史对象/失败候选。收起容器的布局不读取数学正文；`cargo run -p mdstream-core -r --example container_math` 演示分帧映射读取。独立数学块见[独立数学块](#独立数学块)；TeX 排版、更多定界方言、已发布跨行语义撤销和终端 UI 继续保留后续。

## 脚注核心

支持 `[^label]` 与顶层或显式容器内的多行、多段 `[^label]: body` 定义。label 为 1..999 个 UTF-8 字节，由 `char::is_alphanumeric` 接受的 Unicode 标量、ASCII `_`、`-` 组成；区分大小写、不做 Unicode 规范化，不支持空白、转义、实体或其它标点。脚注与普通引用链接使用独立命名空间；`[^` 定义前缀保留给脚注，非法定义不会成为普通链接定义。这里只定义本库方言，不承诺其它 Markdown 脚注扩展的全部行为。

定义起始行允许至多三个前导空格，冒号后可紧接正文，也可省略恰好一个 space/tab；其余空白原样保留。空正文有效。正文是不解析 Markdown、数学或嵌套脚注的原始源码包络；排除最后一个物理行的 LF/CRLF，保留内部换行、裸 CR、续行缩进和容器前缀。第一个合法定义生效，后续同标签定义连同续行一起隐藏，但不覆盖正文或改变该符号版本。顶层和容器内共用文档全局脚注符号；定义起始处围栏和缩进代码优先，定义可结束段落或表格。容器前缀和晚定义规则见[容器内定义](#容器内定义)。

续行先匹配定义的确切容器路径，不允许 lazy 省略或打开新容器；列表需先消费该 Item 原有的续行缩进。剩余正文的 space/tab 按物理行四列制表位计算，足够四列时延续定义，并仅去掉这四列，与起始行的缩进无关。容器路径及四列终点均可拆分 tab；终点余下的 1..3 列由 payload 映射为空格，原始内容仍保留 tab。已完成的纯 space/tab 空白行也可延续，不要求额外四格，但仍必须匹配容器路径；足够四列时去掉四列并映射末尾部分 tab，其余空白原样保留。空行直接归入定义，允许多段和连续空行，末尾空行也不撤销。正文中的 `#`、`>`、列表、表格、围栏和另一个 `[^label]:` 均为字面内容，不创建嵌套结构。遇到非续行、路径失配或兄弟条目时封口，然后只分类当前有界前缀；之前正文不回放。脚注内部空行不影响外层列表间距，末尾空行后出现同 Item 的其它子块或兄弟条目则确认 Loose；若同时关闭嵌套容器，尾部分隔沿用[嵌套末尾空行](#列表嵌套末尾空行)规则传递给实际恢复的祖先。

沿用 1 KiB 块分类前缀：短行在 LF/EOF 分类，长行到达前缀上限时确认定义或续行，后续正文单遍追加，无正文长度上限。定义标记、完整容器路径及续行必需的四格须在该窗口内；尚未完整的超长纯 tab 行不视为空行分隔符，但若已满足四列缩进则按正文续行流式消费。`FootnoteDefinition { block, content, state }` 与其唯一稳定叶块在 LF 后仍为 Growing，遇非续行边界或 EOF 才变为 Sealed。未分类的短行不加入正文包络；未到 LF 的 CR 暂属正文，后续 LF 到来时只调整尾端。引用在行内解析器读到 `]` 时确认，不等待定义、不转换成后续直接链接；未分类行仍暂时显示原文。`[^` 标签候选本身不跨物理行、块或单元格，失败、超限、EOF 未闭合时不回放已消费前缀，不分配符号或编号；拒绝字节继续按普通行内规则处理。

行内代码、数学、链接目标/title、引用链接后缀和自动链接等正在消费的上下文先于脚注。`\[^x]` 不创建引用，`![^x]` 沿用普通引用图片，`[text][^x]` 沿用普通引用链接；不支持嵌套链接 label 的特殊脚注语义，遵守已有单括号候选规则。脚注 label 自身不解析行内标记，外层强调仍可覆盖整个标记。标题、显式容器正文及表格单元格均可包含脚注引用。

`FootnoteId` 是按首次遇见引用或定义分配的文档内稳定符号，`FootnoteReferenceId` 是按源码顺序分配的独立引用对象 ID。`Footnote.number` 只在该符号首次确认引用时分配，从 1 开始；定义先到不占号，重复引用共享编号，未定义符号也占号。编号一经分配不变，晚定义与重复定义不会重排。引用只保存符号及完整源码区间；`resolve_footnote` 经符号 O(1) 解析到编号和定义快照，区分流式 Unresolved、EOF Missing、Defined。定义的 Growing/Sealed 与整个文档 EOF 独立。对象及快照只属于原文档，跨 revision 使用已复制快照时由调用方检查语义版本和 EOF 状态。

`Update.new_footnotes` / `new_footnote_references` 报告新增范围，`footnotes_changed` 报告编号分配、定义确认、增长和封口，可有重复 ID，支持 merge 和复用缓冲。每符号 revision 按新增定义源码字节和编号/开闭事件递增，与 append 分块无关；全局 `TileKey.footnote_revision` 保守失效。`DependencyKey` 按预算捕获并校验当前 tile 的脚注符号版本，未定义目标另检查 EOF，无关定义可保留直接语义缓存。晚定义/增长只用符号的 first_use 发出 Structure 范围变化，旧高度及后续检查点按既有区间协议失效，不枚举历史引用；关闭尺寸策略也不能忽略这类对象语义变更。首次 finish 对存在脚注引用的文档发布保守范围失效并改变全局脚注版本，保证 Unresolved → Missing 不漏通知；这一转换不逐符号发事件，消费者也需观察 `is_finished()`。

`footnote_references_in(range)` 构造 O(1) 借用游标，首个 advance 在扣费后 O(log R) 定位重叠标记，其后每项 O(1)。每个结果和终止查询各计 1 query / 1 object，不读取 label/body；空范围与完成后的重复读取免费。`resolve_footnote` 每次计 1 query / 1 object / 1 dependency，无效 ID 返回 None。`footnote_content(id)` 构造 O(1) 原文游标，advance 每次最多 TILE_BYTES 个 UTF-8 字节，计实际返回字节及 1 query / 1 object。所有接口共用调用方 Budget；不足时不推进且不扣费，4 bytes 足以读取任意标量。游标借用文档，消费期间不能追加；释放后可按稳定 ID 重建。

`footnote_payload(id)` 返回同类型的去前缀游标，省略已匹配的容器前缀和续行四格，保留原始 LF/CRLF、实体拼写、反斜线及其它空白。每个 `FootnoteChunk.source` 是连续消费的原始区间；`text` 借用该区间、在省略前缀时为空，或为单个部分 tab 字节对应的 1..3 个静态空格。每片段计 `max(source.len(), text.len())` bytes、1 query、1 object，部分 tab 原子返回，预算不足不推进也不扣费；不能免费跳过任意多层。构造 O(1)，首个成功 advance 付费后 O(log G) 定位全局有序间隙索引，其后每步 O(1)，没有正文复制或祖先遍历。原文和去前缀接口均仅读取已确认范围，不自动排版段落。

```rust
use mdstream::{Budget, Document, FootnoteId, SyntaxState, WorkLimit};
let mut doc = Document::new();
doc.append("> - [^n]: first\r\n>       中🙂\r\n>   \r\n>       last\r\n")?;
assert_eq!(doc.footnote(FootnoteId(0)).unwrap().definition.unwrap().state, SyntaxState::Growing);
doc.finish();
let mut cursor = doc.footnote_payload(FootnoteId(0)).unwrap();
let mut text = String::new();
loop {
    let mut frame = Budget::new(WorkLimit { queries: 1, objects: 1, bytes: 4, ..Default::default() });
    let Some(chunk) = cursor.advance(&mut frame).unwrap() else { break };
    assert_eq!(frame.used().bytes, chunk.source.len());
    text.push_str(chunk.text);
}
assert_eq!(text, "first\r\n中🙂\r\n\r\nlast");
# Ok::<(), mdstream::Finished>(())
```

```rust
use mdstream::{Budget, Document, FootnoteReferenceId, FootnoteTarget, WorkLimit};
let mut doc = Document::new();
doc.append("See [^note] twice [^note].\n").unwrap();
let mut budget = Budget::new(WorkLimit::unlimited());
let first = doc.resolve_footnote(FootnoteReferenceId(0), &mut budget).unwrap().unwrap();
assert_eq!(first.number, 1);
assert_eq!(first.target, FootnoteTarget::Unresolved);
doc.append("\n[^note]: 原始 **正文**\n").unwrap();
doc.finish();
let second = doc.resolve_footnote(FootnoteReferenceId(1), &mut budget).unwrap().unwrap();
assert_eq!((first.symbol, first.number), (second.symbol, second.number));
let mut body = doc.footnote_content(second.symbol).unwrap();
assert_eq!(body.advance(&mut budget).unwrap().unwrap().text, "原始 **正文**");
```

解析器只保留一个 O(1) 行内候选及当前定义的末尾空行标记；有界 label 使用 BTreeMap 确定性 O(log S) 符号寻址，编号/定义/增长均只更新当前符号。每个成功定义的带前缀续行至多记录一个间隙，重复定义不留间隙，额外存储 O(S + R + G)；全库维持累计 O(n log n)、空间 O(n)，Vec 扩容仍为摊还保证。`Work.footnote_steps` 统计符号操作与定义增长/封口，`footnote_label_bytes` 统计实际 intern 的有界标签，正文与失败候选检查计入已有 input/classified/inline 字节计数。两万次旧引用后首行定义只做两次符号步骤，后续每次增长或封口各一次；千层容器中百万字续行后的逐字追加不回扫正文、祖先或历史引用。读取/存储、400 组范围模型、240 次流式折叠高度模型、UTF-8 分块、预算及缓存验收见 `crates/mdstream-core/tests/footnote_blocks.rs`、`crates/mdstream-core/tests/footnotes.rs`、`crates/mdstream-core/tests/chunking.rs`；`cargo run -p mdstream-core -r --example footnote_blocks` 演示多段映射读取与折叠保持，原 `footnotes` 示例继续演示晚定义。解析不承诺任意大 append/finish 的单次硬时延。

通用显示与 LayoutPass 仍原样显示 `[^label]` 标记，并隐藏顶层及容器内定义；脚注编号上标、文末脚注区、定义正文 Markdown 排版、双向跳转/返回及对象回调仍待接入，读取核心不自动展开或排版脚注区。

## 明确支持的 Markdown 方言

- 段落与空行；ATX 标题；Setext 标题；分隔线。
- 顶层及显式容器内缩进代码、反引号/波浪线围栏代码；opening fence 长度 3..1023（须在分类窗口内确认定界串结束），代码 info 保留源码区间，作用域与边界见[容器内代码块](#容器内代码块)。
- 显式引用/列表容器 ID、父关系及流式开闭，支持有缩进的条目续行、混合嵌套、内层 ATX/Setext 及章节作用域；具体前缀、空行、深度和未实现规则见[容器方言](#显式列表与引用容器)。
- 管道表格、列对齐、单元格行内语法；任务列表复选框状态。
- `*em*`、`_em_`、`**strong**`、`__strong__`、`~~strike~~`，以及精确长度匹配的反引号代码 span。支持跨同一段落的多行输入。
- ASCII 标点反斜线转义；内联链接 `[text](destination)`；shortcut/full/collapsed 引用链接；顶层及容器内单行引用定义；裸目标和尖括号目标，可选 title。
- 尖括号 URI/email 自动链接；`link_destination` 借用原始地址，`link_url` 为 email 添加 `mailto:`。
- 图片 `![alt](destination)` 及引用式图片，保留独立类型和 alt 信息，可由渲染器降级为链接。
- CRLF 的 CR 隐藏，LF 保留。裸 CR 按原文处理。
- 同块行内软换行、双空格/反斜线硬换行的结构化标记。
- 显示语义读取支持实体/数字字符引用及已闭合代码 span 空白规范化，映射和上下文限制见上一节。
- `$…$` / `$$…$$` 数学内容区间和预算读取，具体规则及未实现边界见[定界数学核心](#定界数学核心)。
- `[^label]` 引用、顶层及容器内多行多段脚注定义、稳定编号及有预算原文/去前缀读取，方言与布局边界见[脚注核心](#脚注核心)。

取舍：

- 不是完整 CommonMark/GFM 实现。行内 `***` 等超过支持长度的强调 run 按原文处理；flanking 使用 ASCII 空白规则，单词内下划线不匹配。交叉闭合采用最近同类 opener，并丢弃中间未闭合 opener，不做回溯。
- 未闭合代码 span 在序列结束时尝试[代码配对回退](#未闭合代码候选的代码配对回退)，异长反引号串的数量没有上限；期间跳过的其它语法不会重新解析。嵌套链接不支持：新 `[` 替换尚未闭合的外层 `[`。
- 已识别的引用立即显示 label，并隐藏引用语法；尚未定义时没有可跳转目标。定义到来后只增加目标。第一个有效定义生效，后续重复定义不覆盖。引用匹配的原始正文标签最多 999 字节（不含容器前缀），折叠 Unicode 空白并转小写，不做完整 Unicode case folding。
- URL 和 title 各最多 4096 字节，左括号后的链接后缀候选最多 8256 字节（包括空白和闭合右括号）。支持平衡括号和转义定界，但返回的 destination/title 是原文；不提供多行目标/title、URL 解码或安全过滤。后缀失败时保留原文及已有 shortcut 引用，不回扫后缀中的其它语法。
- 引用定义、Setext、分隔线、闭合 fence 必须在块分类前缀内完成。超长候选行按普通内容/代码处理；超长 opening fence 的 info 只保留已分类前缀，其余该行隐藏。ATX 标题的前后 space/tab 和关闭井号采用流式端点摘要，超过分类窗口仍可确认，见[ATX 空白与关闭尾缀](#atx-空白与关闭尾缀)。
- 自动链接候选最多 4096 字节，URI scheme 为 2..32 个 ASCII 字符；email 按 CommonMark 0.31.2 的 ASCII 地址规则。失败或 EOF 未完成的候选不重放内部行内语法；新自动链接中止未完成的外层方括号候选，保持链接源码有序且不重叠。返回原始 URI，不做 percent encoding。
- 不处理 HTML、GFM 裸 URL 自动链接等额外语义。它们可能包含上述方言仍能识别的普通行内标记。数学仅提供上述定界核心，不解析 TeX。HTML 不在当前需求范围。
- 支持只追加，不支持任意位置编辑、撤回已有源码或移除定义。文档销毁会释放 O(n) 状态，这是一次性清理成本。

## 有界显示摘要

`DisplayExcerpt` 从指定源码范围读取非隐藏显示文本前缀，可用于代码块、章节正文及标题摘要。`ExcerptLimits` 同时限制源码前缀字节、输出 UTF-8 字节和 LF 分隔的逻辑行数；最后一行的 LF 包含在输出内，不添加省略号，空输入不产生空行。源码上限包含隐藏语法，防止巨型围栏 info 或隐藏目标拖延摘要；输出上限防止超长首行绕过行数限制。UTF-8 截断保留完整标量，行内实体/代码空白使用现有显示语义，样式、链接及完整替代源码映射保留。

```rust
use mdstream::*;
let mut doc = Document::new();
doc.append("```rs\nfirst\nsecond\nthird\n```\n")?;
doc.finish();
let body = doc.fold_candidate(FoldId::Code(BlockId(0))).unwrap().hidden;
let mut excerpt = DisplayExcerpt::new(&doc, body, ExcerptLimits {
    source_bytes: 512, display_bytes: 128, lines: 2,
})?;
let mut text = String::new();
loop {
    let mut budget = Budget::new(WorkLimit {
        queries: 1, objects: 1, bytes: 16, ..Default::default()
    });
    match excerpt.next(&doc, &mut budget)? {
        ExcerptStep::Run(run) => text.push_str(run.text),
        ExcerptStep::BudgetExhausted => continue,
        ExcerptStep::Complete(end) => {
            assert_eq!(end, ExcerptEnd::LineLimit);
            break;
        }
    }
}
assert_eq!(text, "first\nsecond\n");
# Ok::<(), Box<dyn std::error::Error>>(())
```

构造不扫描正文，任务 O(1) 空间，输出借用文档，由调用方决定是否收集。每次读取沿用 `DisplayCursor` 的 query/object/byte 计费，隐藏内容也计费；每个检查窗口至多 `TILE_BYTES + 2` 字节，额外两字节用于代码 CRLF 邻接检查，重复前瞻重复扣费。每帧 6 bytes、1 query、1 object 可推进；消费的源码前缀有总长度限制，实际检查量仍由共享 Budget 约束。截断仅扫描已支付的有界 run 或至多 8 字节的替代文本；累计成本 O((S + 1) log(n + 1))，S 为读取的源码前缀字节数。即使整块有百万字，也不会为了摘要统计全文行数。

`Complete` 区分范围结束、源码上限、输出上限和行数上限；达到上限时不会向后寻找可见内容，因此不承诺余文一定非空。替代文本仍映射完整语法区间，该映射可以超过读取前缀；`source_position` 表示已检查 run 的末尾，可能超出最后输出的字符，不能拿它拼接被截断的余文。任务和输出绑定 revision，追加或首次 finish 后须显式丢弃并重建；空追加、重复 finish 不失效，错误文档及旧任务在扣费前被拒绝。

读取不改变折叠意愿或高度，也不应用视图中的子折叠。宿主显式选取 `FoldCandidate.hidden` 预览正文、选取标题 `content` 读取名称；开放范围是当前 revision 的快照。逻辑行不等于终端软换行后的屏幕行，摘要占位绘制仍需由排版方限制实际显示行数、测量并归属高度。`cargo run -p mdstream-core -r --example excerpt` 展示百万字折叠代码的两行摘要；终端宽度、占位高度及 UI 接入继续见 TODO。

## 源码入口

下表路径相对 `crates/mdstream-core/`。

| 文件 | 职责 |
| --- | --- |
| `src/lib.rs` | 流式提交、稳定块/片段、按范围惰性查询、缓存 key |
| `src/block.rs` | 有界块分类 |
| `src/containers.rs` | 显式容器 arena、活动栈、共享开放末端与有预算祖先查询 |
| `src/inline.rs` | 单遍分隔符、代码和链接状态机 |
| `src/math.rs` | 行内/独立块数学的共享预算原文/去前缀读取、行内元数据与窗口索引 |
| `src/footnotes.rs` | 脚注符号/引用、稳定编号、晚定义与预算查询/原文读取 |
| `src/destination.rs` | 链接/定义共享的有界目标及 title 状态机 |
| `src/display.rs` / `src/entity.rs` | 显示片段、替代文本源码映射、预算游标及字符引用解码 |
| `src/excerpt.rs` | 限制源码/输出字节和逻辑行数的有预算显示摘要 |
| `src/search.rs` / `src/display_search.rs` | 源码增量搜索、revision 绑定的显示文本搜索与命中映射、选区及任务状态 |
| `src/annotations.rs` | 区间样式、隐藏聚合、最新 revision |
| `src/references.rs` | 有长度上限的标签规范化和 BTreeMap 符号表 |
| `src/layout.rs` | lazy 高度与遮蔽树、viewport、锚点和过期测量拒绝 |
| `src/folding.rs` / `src/fold_ranges.rs` / `src/frame.rs` | 章节/代码块/表格/容器候选、独立折叠视图、容器/内层章节区间索引、预算排版和占位 |
| `src/checkpoints.rs` | 有效检查点的范围失效和前驱查询 |
| `src/table_preview.rs` | 共享行数/字节上限与 Budget 的表格前几行摘要 |
| `src/table.rs` | 表格行/单元格、对齐、流式单元格解析和列宽提示 |
| `src/source_folds.rs` | 手工范围/段落前缀预览身份、两端路径预算验证及窗口枚举 |
| `src/paragraph_folds.rs` | 顶层/容器逻辑段落窗口发现、长度筛选和共享预算游标 |
| `src/headings.rs` | 标题 arena、全局/作用域六级索引及源码上下文边界 |
| `src/heading_search.rs` | 有预算的标题显示名称检索、级别过滤和命中源码映射 |
| `src/anchors.rs` | revision 绑定的有预算标题 slug、全局消歧和 fragment 解析 |
| `src/navigation.rs` | 逐行/翻页/首尾/标题导航及源码目标 |
| `src/fold_navigation.rs` | 有预算折叠导航、可见标题跳过及源码锚点投影/恢复 |
| `src/fold_batch.rs` | 分帧全部/子树/兄弟/级别/聚焦命令及进度 |
| `src/history.rs` | 文档内书签、视口偏移及有预算前进/后退历史 |
| `tests/` | 语义样例、随机模型对照、分块一致性、复杂度回归 |

## 验证与开发

Nix flake 锁定工具链和 nextest；构建产物按 workspace 路径隔离。已有 Rust 工具链也可以直接执行对应的 cargo 命令。

```sh
nix develop -c cargo check --workspace --all-targets
nix develop -c cargo clippy --workspace --all-targets -- -D warnings
nix develop -c cargo nextest run --workspace --release --no-fail-fast
nix develop -c cargo test --doc --workspace
nix develop -c cargo fmt --all -- --check
nix develop -c cargo run -p mdstream-core -r --example stream
nix develop -c cargo run -p mdstream-core -r --example structure
nix develop -c cargo run -p mdstream-core -r --example containers
nix develop -c cargo run -p mdstream-core -r --example container_headings
nix develop -c cargo run -p mdstream-core -r --example container_setext
nix develop -c cargo run -p mdstream-core -r --example partial_tabs
nix develop -c cargo run -p mdstream-core -r --example prose_tabs
nix develop -c cargo run -p mdstream-core -r --example math_tabs
nix develop -c cargo run -p mdstream-core -r --example leaf_tabs
nix develop -c cargo run -p mdstream-core -r --example container_sections
nix develop -c cargo run -p mdstream-core -r --example container_tables
nix develop -c cargo run -p mdstream-core -r --example scaling
nix develop -c cargo run -p mdstream-core -r --example budget
nix develop -c cargo run -p mdstream-core -r --example display
nix develop -c cargo run -p mdstream-core -r --example excerpt
nix develop -c cargo run -p mdstream-core -r --example table_preview
nix develop -c cargo run -p mdstream-core -r --example table_row_folding
nix develop -c cargo run -p mdstream-core -r --example source_folding
nix develop -c cargo run -p mdstream-core -r --example paragraph_folding
nix develop -c cargo run -p mdstream-core -r --example container_paragraph_folding
nix develop -c cargo run -p mdstream-core -r --example paragraph_discovery
nix develop -c cargo run -p mdstream-core -r --example search
nix develop -c cargo run -p mdstream-core -r --example heading_search
nix develop -c cargo run -p mdstream-core -r --example anchors
nix develop -c cargo run -p mdstream-core -r --example folding
nix develop -c cargo run -p mdstream-core -r --example fold_batch
nix develop -c cargo run -p mdstream-core -r --example history
```

测试包含任意单切点和随机 UTF-8 分块、与朴素数组/有序映射对照的区间/高度/检查点随机测试、深层 opener、百万字 Setext 转换、百万字隐藏区间、十亿片段的稀疏高度索引、逐字符超长段落、延迟定义成本和 resize 节点复用。没有把 wall-clock 阈值当作正确性条件。`scaling` 示例已扩展为 17 场景核心性能矩阵，记录实际操作/容量、吞吐及有预算消费帧的 p50/p95/p99；复跑脚本、测量边界和三轮原始 CSV 见 [性能矩阵](docs/performance.md)。

行内分块测试会特意跨过 `BLOCK_LOOKAHEAD`，避免短样例被整行缓冲后才解析。另有中间前缀一致性、TileKey 未变则语义不变、标题/链接索引与线性查询对照、目标/title 长度边界及百万字尾空格确认成本测试。自比较验证分块一致性，预期语义样例和朴素数据结构模型提供另外的判定依据，不能将这些测试视为完整规范合规率。

`crates/mdstream-core/tests/display_search.rs` 使用完整显示文本上的朴素搜索及独立语义样例作对照，覆盖实体部分命中、CRLF 映射、跨隐藏标记/tile、长查询和重叠匹配、真实流式逐前缀/逐切点、固定种子生成输入、极小共享预算、旧 revision 拒绝及失败回退计数。百万字隐藏区间验证有限预算不会触发整篇读取。

本 checkout 已安装 Rust skill 提供的本地 pre-commit：运行 `cargo fmt --all` 后只重新 stage 原本暂存的 Rust 文件。格式化可能改变提交中的行号；其它未暂存文件可能被格式化，但不会自动暂存。钩子不随 Git 分发，紧急情况下可用 `git commit --no-verify` 跳过。

## 可复现语法生成与失败缩减

`crates/mdstream-core/tests/syntax_fuzz.rs` 的 v1 语料固定四个持久化种子（0、1、`0x12345678`、`0xdeadbeefcafebabe`），每种 36 例，共 144 例；源码、精确分块位置及种子一起用 FNV-1a 指纹锁定。生成器使用显式 u64 wrapping 运算和转换前取模，不依赖系统随机数或时间。十二类块产生式组合递归行内语法、引用/列表前缀、晚定义、失败/未闭合候选、中文/emoji、LF/CRLF，以及分类窗口附近的长行。逐字符、随机和语法标记边界三种分块计划共检查 **19,536 个开放前缀**，再检查 EOF 和重复 finish；实际语料到达标题、表格、容器、链接、数学、脚注和代码对象，防止生成器退化为普通文本。

每个分块前缀都和同一前缀的一次 append 对照，复用既有 `chunking` 完整快照：块/容器/标题/表格、稳定对象 ID 与符号、数学/脚注内容、样式和显示映射。不比较文档身份或 append 修订次数。同时独立检查 UTF-8 区间、块/tile/run/display 源码覆盖、容器父顺序/深度/范围、对象引用和单元格/内容映射顺序，以及 pending 上限、finish 幂等与 EOF 后 append 拒绝。这验证增量一致性和数据不变量，不产生 CommonMark/GFM 的标准期望；官方样例评分保持独立。

命令行与 nextest 使用同一驱动，零新增依赖。CLI 默认 seed 0、48 例；`--seed` 使用十进制 u64。正常结束返回 0；发现失败先保存原始 case，再确定性缩减，返回 1；参数、文件或格式错误返回 2。

```bash
nix develop -c cargo nextest run --workspace --test syntax_fuzz --no-fail-fast --success-output immediate
nix develop -c cargo run -p mdstream-core -r --example syntax_fuzz -- --seed 305419896 --cases 256
# 注入演示预期返回 1，生成原始文件及同路径的 .min 文件：
nix develop -c cargo run -p mdstream-core -r --example syntax_fuzz -- --inject split-crlf --output /tmp/fuzz.case
nix develop -c cargo run -p mdstream-core -r --example syntax_fuzz -- --replay /tmp/fuzz.case.min --output /tmp/fuzz-replay.case
```

case 文件首行为 `mdstream-syntax-fuzz-v1`，后续固定字段为 `seed`、`case`、`fault`、`cuts`，空行之后是原始 UTF-8 输入，保留 CRLF、NUL、空追加和无换行 EOF。cuts 为非递减绝对字节端点，最后隐式追加剩余后缀；重放直接使用文件内容，不重新调用生成器，越界或非 UTF-8 边界会拒绝。nextest 失败也输出完整原始/缩减 case；CLI 在开始缩减前写入原始文件。修改生成算法需明确更新版本和语料指纹，不能以改种子或跳过失败样本掩盖缺陷。

缩减先按固定顺序由粗到细删除 Unicode 字符区间，再合并分块，反复执行到固定点；删除时同步映射切点，始终保持合法 UTF-8。只接受保持同一阶段/属性签名的失败，panic unwind 也会被记录；不以耗时或随机次数停止。结果对单字符删除和单切点移除不可再减，不保证全局最短或最简单语法。此测试工具会反复解析/复制全文，适用于有界语料；不承担解析核心的预算或复杂度保证，也不处理进程 abort、OOM 或挂死的恢复。

三个故障只存在于测试驱动：观察快照丢失 strong 样式、块范围越界，以及分块适配器在 CR/LF 分离时漏传 LF。验收确认每种故障被检测、缩减结果可重放且确定，关闭注入后通过；前两者分别缩减为 6/1 字节，第三者为 `\r\n` 并保留中间切点。既有随机字符测试、标准样例与百万字/深嵌套计数测试继续运行；32 位索引验收见下节，更广的性能矩阵仍见 TODO。

### 32 位索引与算术边界

在 `x86_64-linux` 上，项目提供单独的 `index32` devShell：沿用 flake.lock 的 Rust/nixpkgs，增加 `i686-unknown-linux-gnu` 标准库并指定同版本 Nix i686 C 链接器及 glibc。默认 devShell 和 macOS 环境不需要下载这些组件。从仓库根目录执行：

```bash
nix develop .#index32 -c bash scripts/check-index32.sh
```

脚本执行目标的全目标 check/clippy、debug/release 全套 nextest、文档测试及格式检查。需要宿主内核能执行 IA32 程序；缺少目标库、链接器或执行能力会直接失败，不把仅编译成功当作测试通过。快速边界回归可用 `nix develop .#index32 -c cargo nextest run --workspace --target i686-unknown-linux-gnu --lib -E 'test(index_boundaries)'`。产物路径仍由 devShell 按 workspace 设置；如需查询，使用 `cargo metadata --no-deps --format-version 1` 的 `target_directory`。

本轮审计覆盖以下算术及其调用方约束，没有更改公开类型或索引算法：

| 范围 | 算术约束与验证 |
| --- | --- |
| `HeightIndex` / `CheckpointIndex` | 长度最多 `isize::MAX`，可写 tile 必须小于该长度上限；二次幂容量最多为 `isize::MAX + 1`，中点用 `lo + (hi - lo) / 2`。稀疏测试跨越各二次幂，实际到达 i686 的 2,147,483,647 长度和最后有效 tile。 |
| `Annotations` / `FoldRanges` | 写入源码端点来自 String 或已验证的高度范围；折叠树键来自非零大小对象 arena，均受 `isize::MAX` 约束。`usize::MAX` 只作为开放范围终点参与比较，不能作为树键。测试以写入日志/有序映射核对覆盖、最新版本、可见位置和关闭事件。 |
| 高度与定位转换 | 单片高度 `u64`，宽度乘积/总高度 `u128`；先拓宽再乘。定位的除法商小于子树宽度，余数小于单片高度，因此转回 `usize` / `u64` 不截断。独立宽整数区间模型覆盖 `u64::MAX` 高度、超过 64 位的总高度、嵌套遮蔽和重置。 |
| 预算与窗口 | 先比较剩余额度再累加，六项扣费原子失败；源码/视口扩窗用饱和加法后裁剪。验证 `u32::MAX` / `usize::MAX` 预算、`u128::MAX` 行窗口、最后三片分帧恢复与有预算检查点回收。 |
| 源码、ID 与 arena | 源码游标受 String 长度限制，对象 ID 使用 Vec 下标；Vec 节点容量乘非零节点大小不超过其分配上限。无效公开 ID/跨度通过查询入口拒绝；`Span::len` 本身要求 `start <= end`，不替调用者校验任意手工跨度。修复 `ticket` 在拒绝最大 ID 前急切计算 `+1` 的 debug 溢出。 |
| 有界语法整数 | 有序列表号最多九位，累加为 `u32`；容器列计数只遍历有界分类前缀，部分 tab 余列最多 3 后再转 `u8`；标题等级限定 1–6。源码长度计入 `u64` 工作计数在当前 32/64 位目标上不缩窄。 |

`crates/mdstream-core/src/index_boundaries.rs` 的 11 项测试使用稀疏线性区间、写入日志和 BTreeMap 作为独立模型。64 位运行同时覆盖 32 位边界与本机 `isize::MAX`；i686 运行直接执行真实 32 位 `usize` 路径。四类树的修改和折叠候选查询按字长限制访问数，标注遍历按写入数×字长限制访问数；高度节点分配仅允许两条边界路径，避免用巨型数组模拟高位索引。既有标准样例、固定种子 fuzz 和百万字/深嵌套测试也在两种字长执行。

这些是索引算术与回归验收，不保证能实际分配接近 2 GiB 的文档、密集 arena 或进程地址空间。尚未实测内存耗尽/分配失败恢复、耗尽进程级身份、`u64` revision/序列/计数器绕回，以及其它 32 位 ABI/无 64 位原子的平台。源码存储仍为 String，公开 API 未新增可恢复的分配错误；稀疏索引上限不代表可加载文档大小。更广的性能矩阵和终端 UI 继续保留 TODO。

## 存储实测与标准样例范围

`annotation_arena_stats`、高度树/检查点的 `arena_stats`、高度依赖树的 `dependency_arena_stats` 返回 Vec 容量字节数、节点数与实际容量增长分配次数。统计不包含 allocator 元数据、碎片、其它 String/Vec 或用户状态的间接堆分配。`scaling` 示例同时记录操作计数、查询分位数与容量；`one_byte_reused` 对照复用通知缓冲的逐字符输入。

2026-10-30 在 ark 上的 release 样本：524,286 字节密集标记产生 854,391 个语义节点；子索引改用 `Option<NonZeroUsize>` 后，节点大小由 88 降至 72 字节，对应 arena 容量由 92,274,688 降至 75,497,472 字节（约 18.2%）。写入访问数 8,010,100、固定视口查询访问数 13,022 均未改变，分配次数同为 20。缩小了节点常数，没有增加全树压缩过程。峰值 arena 容量等于当前容量，因为该 arena 不缩容；这不是整个进程峰值内存。语义/修订存储拆分尚未实施；查询 scratch 复用已交付，见上方接口。后续 [存储与回收实验](docs/storage.md) 提供三规模可复跑容量/分配测量、查询栈 `Runs::scratch_stats()`、检查点 clear/drain/refill 对照及优化决策；已将检查点子索引同样压缩，Vec 状态节点在本机由 88 降至 72 字节。回收只释放移出后由调用方丢弃的外部 payload，保留树 arena；不声称总堆统计或硬内存上限。

标准样例固定为 [CommonMark 0.31.2 autolinks](https://spec.commonmark.org/0.31.2/#autolinks)，离线夹具保存在 `crates/mdstream-core/tests/fixtures/commonmark_autolinks.rs`，保留上游 CC BY-SA 4.0 归属信息。19 个样例中，18 个在源码文本/目标语义上符合；606 为刻意差异：失败候选内部的反斜线不重新解析。URI percent encoding 属输出层，因此对照时归一化目标编码。其它已评分子集见下文，其余 CommonMark/GFM 范围未评分；现有容器展平、强调/代码回退和语法长度限制仍按上文方言约定，不以自比较分块测试充当完整规范合规率。

显示语义另固定了 24 个 CommonMark 0.31.2 文本样例（25–30、35–37、39–40、328–340），离线夹具 `crates/mdstream-core/tests/fixtures/commonmark_display.rs` 同样保留归属信息，只对照移除 HTML 标签后的文字。HTML、属性解码、列表结构及完整代码回退未计入这组评分；不将局部样例通过表述为整个章节合规。`crates/mdstream-core/tests/display.rs` 另外验证源码映射、跨 tile/逐标量查询、逐前缀/逐切点一致性、百万字代码晚闭合、错误候选线性计数、预算推进和旧游标拒绝。

[CommonMark 0.31.2 ATX headings](https://spec.commonmark.org/0.31.2/#atx-headings) 的完整章节（62–79，共 18 例）另由 `crates/mdstream-core/tests/commonmark_atx.rs` 独立评分。`crates/mdstream-core/tests/fixtures/commonmark_atx.rs` 保存官方 Markdown 与 HTML 原值，附 John MacFarlane / CC BY-SA 4.0 归属、版本、规范行号及完整 `spec.json` 的 SHA-256。生成器只读取另行下载的本地文件，先校验摘要与完整章节成员；构建和测试不联网，也不调用本库生成预期。

| 分类 | 样例 | 当前判定 |
| --- | --- | --- |
| 支持（17） | 62–69、71–79 | 块类型/顺序、标题级别、显示文字及强调与官方预期相同 |
| 刻意差异（1） | 70 | 既有缩进代码方言允许中断正文，得到段落后代码块；规范要求同一个段落。方言见上方容器代码规则，完整缩进规则仍待扩展 |
| 未支持（0） | 无 | 67、71–73 的原始空白缺口已在解析核心修复；官方输入和 HTML 保持原值 |

测试投影只处理这些样例中的段落、标题、缩进代码、分隔线和行内强调，检查标题索引与块一致；跳过结构空行、去掉段落/标题的一个物理末尾 LF，保留代码 LF、内部换行及所有空格。不执行 `trim`、重解析 Markdown 或按样例编号修正输出，不代表库提供了 HTML 渲染器或支持其它 HTML 语义。非支持项同时断言仍不同于官方预期、且精确等于单独记录的当前观察值；回归或实现改善均要求重新审查分类。全部样例另做所有切点和逐字符输入对照，适配器有空标题、空白、级别、强调及块边界敏感性检查。这些短样例不替代已有长输入/预算/复杂度测试。

独立运行并显示逐例分类与汇总：

```bash
nix develop -c cargo nextest run --workspace --test commonmark_atx --no-fail-fast --success-output immediate
```

用另行下载的官方 `spec.json` 复核夹具（只读取文件）：

```bash
python3 scripts/generate_commonmark_atx.py /path/to/spec.json --check
```

[CommonMark 0.31.2 Setext headings](https://spec.commonmark.org/0.31.2/#setext-headings) 的完整章节（80–106，共 27 例）由 `crates/mdstream-core/tests/commonmark_setext.rs` 独立评分。`crates/mdstream-core/tests/fixtures/commonmark_setext.rs` 同样逐字固定官方 Markdown/HTML、规范行号、作者、CC BY-SA 4.0 许可及完整来源 SHA-256；`scripts/generate_commonmark_setext.py` 从本地固定来源生成并复核，不使用本库输出作为标准期望。

| 分类 | 样例 | 当前判定 |
| --- | --- | --- |
| 支持（25） | 80–84、86、88–106 | 标题级别、文字/强调、块优先级、引用及列表边界符合这些样例的官方期望 |
| 刻意差异（2） | 85、87 | 85：现有缩进代码在空行处封口，产生两个代码块；87：缩进代码可中断段落。均是上文已记录的代码方言，完整规则仍待扩展 |
| 未支持（0） | — | 82、84、89 已由解析核心的首尾空白处理修复；官方夹具和投影保留空白的规则不变 |

Setext 投影只覆盖这些样例涉及的块、根引用、根无序 tight 列表及强调；根据公开容器父关系保持容器边界，只有相邻、同一容器且带已发布软/硬换行的正文物理叶块才合并。块类型、标题索引/级别/范围、内部换行、空格/tab 和代码末尾 LF 均保留；只省略结构空行及非代码块的一个物理末尾 LF。不会合并空行两侧的代码块、重新解析 Markdown、按样例编号修正投影或 `trim` 输出；这不是通用 HTML 渲染器，也不宣称其它容器形态已评分。样例 91 的 HTML 形状文本按普通文字处理，不能据此宣称支持 HTML。

非支持项同时检查官方差异和独立当前观察值，改善或回归均要求审查分类。全部 27 例另以 533 个单切点、506 个逐字符前缀验证块/标题/容器身份、源码映射、样式及待分类范围；EOF 后再与固定分类结果对照。投影敏感性覆盖空格/tab、强调、级别、容器边界、内部软/硬换行及代码空行分块。这些短样例不替代长输入、预算及复杂度验收；空白修复的独立长输入验收见[Setext 首尾空白](#setext-首尾空白)。

独立报告及夹具复核：

```bash
nix develop -c cargo nextest run --workspace --test commonmark_setext --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_setext.py /path/to/spec.json --check
```

[GFM 0.29-gfm（2019-04-06）Tables (extension)](https://github.github.com/gfm/#tables-extension-) 的完整章节（198–205，共 8 例）由 `crates/mdstream-core/tests/gfm_tables.rs` 独立评分。`crates/mdstream-core/tests/fixtures/gfm_tables.rs` 固定官方 Markdown/HTML 原值、HTML 源码行号和样例锚点、GitHub / John MacFarlane 归属及 CC BY-SA 4.0 许可。官方站点不提供 `spec.json`，生成器 `scripts/generate_gfm_tables.py` 读取另行下载的完整规范 HTML，以 SHA-256 `b153d814fdfc8624bb6da7449162c1cd707a637f7d1c1b636eb44b9cf63fa220` 固定来源，并校验完整章节成员；仅解码网页用于展示代码块的实体和空格 span，不改变样例内容。生成、复核、构建与测试不联网，不使用库输出生成期望。

| 分类 | 样例 | 当前判定 |
| --- | --- | --- |
| 支持（8） | 198–205 | 表头/正文、居中/右对齐、可选外侧管道、文本/代码/粗体中的转义管道、引用和空行终止、无管道稀疏行、列数不符回退、缺失/多余单元格及无正文表格与官方一致 |
| 刻意差异（0） | 无 | 这组样例没有观察到刻意差异 |
| 未支持（0） | 无 | 这组样例没有观察到未支持项；未覆盖的规则不计为通过 |

表格投影从公开 Table/Column/Row/Cell 元数据及显示片段生成用于比较的 HTML，保持块/表/行/列顺序、表头与正文区分、列对齐、代码/强调/粗体和引用边界；同时核对表 ID、所属块和源码范围。缺失单元格按库已声明的隐式空单元格语义输出，多余单元格必须已被解析器排除；不会在适配器中重新分割管道、去反斜线或裁剪空白。只跳过结构空块及段落的一个物理末尾 LF；原始内部空格/tab、实体生成的首尾空格及源码映射均保留。这个窄投影只接受本组涉及的块和根引用，不是通用 HTML 渲染器，未评分容器内表格或其它 GFM 规则。

8 例全部 361 个单切点和 353 个逐字符前缀另核对块/标题/容器/表格身份、列宽提示、行/单元格源码范围、完成状态、原始/显示样式和待分类范围；最终结果直接对照官方 HTML。适配器敏感性检查覆盖列/行顺序、默认与左/中/右对齐、空表与空行、稀疏/多余单元格、嵌套样式、空格/tab/实体、段落和引用边界。本章节全部是 ASCII、最长 56 字节；不据此声称已验证全部 Unicode、长输入复杂度、预算或终端表格排版，既有相关验收继续保留。

独立报告及夹具复核（第二条只读取下载好的官方 HTML，来源改动会拒绝）：

```bash
nix develop -c cargo nextest run --workspace --test gfm_tables --no-fail-fast --success-output immediate
python3 scripts/generate_gfm_tables.py /path/to/gfm.html --check
```

[GFM 0.29-gfm（2019-04-06）Task list items (extension)](https://github.github.com/gfm/#task-list-items-extension-) 的完整章节（279–280，共 2 例）由 `crates/mdstream-core/tests/gfm_tasks.rs` 独立评分。`crates/mdstream-core/tests/fixtures/gfm_tasks.rs` 逐字固定官方 Markdown/HTML、HTML 源码行号及锚点、GitHub / John MacFarlane 归属和 CC BY-SA 4.0 许可。`scripts/generate_gfm_tasks.py` 从本地规范 HTML 离线生成或复核，使用与表格章节相同的完整来源 SHA-256，检查章节成员并拒绝改动后的来源；只解码代码块展示标记，不调用本库产生期望。

| 分类 | 样例 | 当前判定 |
| --- | --- | --- |
| 支持（2） | 279–280 | 未勾选/已勾选任务、兄弟条目顺序、嵌套列表与各层独立状态符合官方期望 |
| 刻意差异（0） | 无 | 这组样例没有观察到刻意差异 |
| 未支持（0） | 无 | 这组样例没有观察到未支持项；章节文字中未被样例覆盖的规则不计为通过 |

任务列表投影使用公开 List/Item 父关系、tight 状态、块/容器一致的 `TaskMarker` 和显示片段，保持条目顺序、嵌套边界、勾选状态、正文空白及代码/强调/粗体。根据 `checked` 输出语义复选框，核对标记的源码范围和隐藏状态；本组两个输入在标记后均恰有一个 ASCII 空格，投影检查其原文及隐藏映射，再保留这个空格。不把任意 padding 归一化为单空格，不扫描原文重新发现任务标记，也不修改官方 HTML。只移除正文的一个物理末尾 LF；其它正文空格/tab、实体产生的空格仍保留。

投影仅接受每条目一个物理正文叶块的 tight 无序列表；引用、有序/loose 列表、多行条目、其它块和不同任务 padding 会报错，不能被静默丢弃后计为支持。该投影只是测试适配器，复选框使用官方样例中的 disabled HTML 形式，不是库的 HTML 渲染器或终端 UI。4 项测试覆盖独立评分、全部 66 个切点和 64 个逐字符前缀、状态/结构/文本/样式敏感性及投影边界拒绝。流式快照同时核对块/容器身份、父关系、范围/状态、任务标记范围与勾选状态、原始/显示映射和待分类范围；最终仍直接比较官方 HTML。

两个官方输入均为 ASCII，长度分别为 20 和 44 字节，嵌套仅两层；大写 X 及其它正文/结构变体只用于适配器验收，不混入官方评分。其它任务标记空白、空内容、多行/有序/loose 任务、深嵌套、Unicode、长输入及交互行为未由这两例评分，继续使用已有专项验收或保留待办，不能据此声称完整 GFM 兼容。

独立报告及夹具复核：

```bash
nix develop -c cargo nextest run --workspace --test gfm_tasks --no-fail-fast --success-output immediate
python3 scripts/generate_gfm_tasks.py /path/to/gfm.html --check
```

[CommonMark 0.31.2 Paragraphs](https://spec.commonmark.org/0.31.2/#paragraphs) 完整章节（219–226，共 8 例）由 `crates/mdstream-core/tests/commonmark_paragraphs.rs` 独立评分。离线夹具逐字保存官方 Markdown/HTML，包含规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整 `spec.json` SHA-256；生成器校验来源摘要和完整章节成员，不调用本库生成期望。

| 分类 | 样例 | 结果与范围 |
| --- | --- | --- |
| 支持（7） | 219–222、224–226 | 段落分隔、内部软/硬换行、顶层段落行首缩进与末尾空格、缩进代码后恢复段落符合官方期望 |
| 刻意差异（1） | 223 | 已记录的缩进代码可中断段落方言；规范要求继续同一段落 |
| 未支持（0） | 无 | 仅限本章节 8 个样例，不代表完整段落兼容 |

投影仅读取公开块与显示片段，检查块顺序/边界、可见空白、软/硬换行和代码 LF；只省略结构空块及段落末尾一个物理 LF，不 trim、不重解析、不按样例编号修正输出。未通过样例另存当前观察，既断言仍与官方不同，也断言行为未变；改善与回归都须复核分类。容器、标题、行内代码/链接/数学等超出投影范围的语义显式拒绝，该适配器不是库 HTML 渲染器。

4 项评分测试覆盖独立分类、160 个单切点与 152 个逐字符前缀、空白/样式/块边界敏感性和投影边界拒绝；流式快照核对块身份/范围、显示映射/样式与 pending，EOF 后仍对照官方期望或固定差异。8 例均为 ASCII，最长 64 字节，不能据此推断 Unicode、长输入、容器段落或其它章节兼容。222/224/226 由解析核心修复，官方夹具及评分投影规则保持；额外边界/预算/复杂度验收见[顶层段落行首缩进](#顶层段落行首缩进)和[末尾空白](#顶层段落末尾空白)，剩余容器/缩进缺口和标准覆盖保留 TODO。

```bash
nix develop -c cargo nextest run --workspace --test commonmark_paragraphs --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_paragraphs.py /path/to/spec.json --check
```

[CommonMark 0.31.2 Hard line breaks](https://spec.commonmark.org/0.31.2/#hard-line-breaks) 完整章节（633–647，共 15 例）由 `crates/mdstream-core/tests/commonmark_hard_breaks.rs` 独立评分。离线夹具逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属与完整来源 SHA-256；`scripts/generate_commonmark_hard_breaks.py` 校验本地来源摘要和章节成员后生成或复核，构建及测试不联网。

| 分类 | 样例 | 结果与范围 |
| --- | --- | --- |
| 支持（11） | 633–635、638–641、644–647 | 空格/反斜线硬换行、强调内换行、代码 span 内换行归一化，以及段落/ATX 标题末尾不产生硬换行 |
| 刻意差异（2） | 636–637 | 已记录的缩进代码中断段落方言：五空格续行另开代码块，前一段不产生硬换行，637 的段末反斜线保留 |
| 未支持（2） | 642–643 | 原始 HTML 不在当前需求范围；标签作为普通文本显示，属性内部换行仍按普通行内规则处理 |

投影仅读取公开块、标题索引和显示片段；保留空格/tab、代码 LF、强调和代码 span、软/硬换行以及块顺序/标题级别。只省略结构空块和正文/标题的一个末尾物理 LF，且断言该 LF 没有行内换行标记；不 trim、不重解析 Markdown、不按样例编号调整投影。容器、链接/数学、其它块和未评分样式（含强调与代码嵌套）显式拒绝，HTML 字面文本正常转义。非支持项同时断言与官方不同和当前观察不变，改善或回归均须审查分类。

4 项测试覆盖独立评分、全部 199 个单切点和 184 个逐字符前缀、投影敏感性及边界拒绝；快照对照块/标题身份与范围、显示源码映射/样式及 pending，EOF 后仍对照独立评分。15 例均为 ASCII，最长 21 字节；这些短样例不代表完整 Unicode、HTML、容器、长输入复杂度或预算验证。本次只增加评分，不改变解析方言或运行时依赖，标准覆盖总项继续保留 TODO。

```bash
nix develop -c cargo nextest run --workspace --test commonmark_hard_breaks --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_hard_breaks.py /path/to/spec.json --check
```

[CommonMark 0.31.2 Soft line breaks](https://spec.commonmark.org/0.31.2/#soft-line-breaks) 完整章节（648–649，共 2 例）由 `crates/mdstream-core/tests/commonmark_soft_breaks.rs` 独立评分：**2 支持、0 刻意差异、0 未支持**，覆盖普通软换行及换行两侧空格。固定夹具逐字保存官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属和完整来源 SHA-256；生成器先校验来源摘要与章节成员，构建及测试不联网。

4 项测试验证官方期望、全部 **20 个单切点 / 18 个逐字符前缀**、投影敏感性及边界拒绝。流式快照保留块范围、显示源码映射/样式和 pending；另直接断言每例内部 LF 的 `SOFT_BREAK`、原始源码区间及无替代内容，避免只比较 HTML 而漏掉语义标记。投影仅处理段落/结构空块，保留内部空白和软/硬换行区别，只省略无行内换行标记的段末物理 LF；不 trim、不重解析 Markdown、不按编号调整输出，显式拒绝未评分结构及行内样式。

这两例均为 ASCII（8、10 字节），评分不覆盖容器、tab/混合缩进、CR/CRLF、Unicode、长输入复杂度或预算边界，也不代表其它章节全部支持。只扩充评分，解析核心及方言不变，标准覆盖总项继续待办。

```bash
nix develop -c cargo nextest run --workspace --test commonmark_soft_breaks --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_soft_breaks.py /path/to/spec.json --check
```

[CommonMark 0.31.2 Textual content](https://spec.commonmark.org/0.31.2/#textual-content) 完整章节（650–652，共 3 例）由 `crates/mdstream-core/tests/commonmark_textual_content.rs` 独立评分：**3 支持、0 刻意差异、0 未支持**，覆盖普通标点、希腊文与连续空格。离线夹具逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整来源 SHA-256；生成器校验来源摘要与章节成员，以原始 Unicode 生成 Rust 字符串，构建和测试不联网。

5 项测试验证官方期望、**48 个合法 UTF-8 切点 / 45 个逐字符前缀**，以及 `Utf8Document` 的 **53 个字节切点 / 50 个逐字节前缀**。快照核对块身份/范围、显示映射/样式与 pending；直接核对普通文本借用的原始字节映射，部分 UTF-8 标量只留在解码缓冲区，完整输入不出现替换字符。投影保留空白与 Unicode 标量，只省略结构空块和无换行标记的段末物理 LF；额外验证组合字符、连续空格、实体及软硬换行敏感性，并拒绝未评分结构/样式，不 trim、不重解析 Markdown、不按编号调整输出。

这三例的章节评分不代表完整 Unicode、非法 UTF-8、容器、长输入或预算边界的标准兼容率；其它章节和完整语法规则继续待办。本轮只新增评分及流式验收，解析核心、复杂度和预算契约不变。

```bash
nix develop -c cargo nextest run --workspace --test commonmark_textual_content --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_textual_content.py /path/to/spec.json --check
```

后续修复需保留官方输入/预期，更新当前观察与分类、汇总和 TODO；禁止通过忽略样例、裁剪空白或替换官方期望提升评分。

[CommonMark 0.31.2 Blank lines](https://spec.commonmark.org/0.31.2/#blank-lines) 完整章节（227，共 1 例）由 `crates/mdstream-core/tests/commonmark_blank_lines.rs` 独立评分：**1 支持、0 刻意差异、0 未支持**。覆盖文首、段落与 ATX 标题之间、文末的空行及仅空格行。夹具逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属和完整来源 SHA-256；生成器读取本地固定来源并校验摘要及章节成员，构建/测试不联网。

投影只读取公开块、标题索引和显示语义：跳过结构空白块，只移除段落/标题末尾一个物理 LF，不 trim 正文、不重解析 Markdown、不按样例编号调整输出。4 项测试覆盖官方评分、全部 **23 个切点 / 22 个逐字符前缀**（包括块/标题身份与范围、显示映射、样式、pending 和源码）、投影敏感性及范围外结构/样式拒绝。正文连续空格、内部 tab、实体产生的空格、空标题、块顺序和软硬换行分别保持可区分。

此单例不覆盖列表松紧、容器空行、代码块内空行、tab/CRLF/Unicode 空白或超长输入的完整规则，也不代表这些范围已通过标准评分；相关核心与标准覆盖总项保持待办。未改解析核心、运行时依赖、追加复杂度或共享预算契约。

```sh
nix develop -c cargo nextest run --workspace --test commonmark_blank_lines --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_blank_lines.py /path/to/spec.json --check
```

[CommonMark 0.31.2 Thematic breaks](https://spec.commonmark.org/0.31.2/#thematic-breaks) 完整章节（43–61，共 19 例）由 `crates/mdstream-core/tests/commonmark_thematic_breaks.rs` 独立评分：**18 支持、1 刻意差异、0 未支持**。49 沿用缩进代码可中断段落的既有方言；61 已由[容器内分隔线](#容器内分隔线)修复。官方 Markdown/HTML 与当前差异观察分开保存；夹具保留规范行号、John MacFarlane / CC BY-SA 4.0 归属和来源 SHA-256，生成器校验本地固定来源及章节成员，构建/测试不联网。

4 项测试覆盖评分、全部 **290 个切点 / 271 个逐字符前缀**、投影敏感性和范围外拒绝。快照核对源码、块/标题/容器身份与范围、显示映射/样式及 pending，完成后另验证逐 run 原始借用映射。投影只读取公开块、容器路径、标题索引和显示语义，保留嵌套列表、强调、代码尾部空格/LF、正文空白与软硬换行；不 trim、不重解析 Markdown、不按编号调整输出。当前投影仅接纳 tight bullet 列表及每个条目的单个直接物理叶块，未覆盖的结构和样式明确拒绝。

本次评分不代表完整容器、缩进/空白、长候选或其它章节均符合标准。本次容器内分隔线子项保留零运行时依赖、禁止 unsafe、累计追加 O(n log n)、空间 O(n) 与共享预算契约；分隔线 tab/混合缩进见[后续子项](#分隔线的-tab-缩进)，完整容器和超长分隔线仍待办。

```sh
nix develop -c cargo nextest run --workspace --test commonmark_thematic_breaks --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_thematic_breaks.py /path/to/spec.json --check
```

[CommonMark 0.31.2 Indented code blocks](https://spec.commonmark.org/0.31.2/#indented-code-blocks) 完整章节（107–118，共 12 例）由 `crates/mdstream-core/tests/commonmark_indented_code.rs` 独立评分：**8 支持、4 刻意差异、0 未支持**。110–112 属于空行立即终止缩进代码的既有方言，113 属于缩进代码可中断段落的既有方言；官方输入/期望与当前差异观察分开保存，任一方向变化均要求复核分类。夹具逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属与完整来源 SHA-256；生成器校验本地固定来源和章节成员，构建及测试不联网。

4 项测试覆盖独立评分、全部 **309 个切点 / 297 个逐字符前缀**、投影敏感性和范围外拒绝。快照比较源码、块/标题/容器身份与范围、显示映射/样式及 pending，完成后另核对逐 run 原始借用映射。投影只读取公开块、容器路径及 tight/loose 元数据、标题索引和显示语义，保留代码的字面标记、缩进余量、尾空格、内部 tab 和 LF，以及列表嵌套、正文空白与软硬换行；不 trim、不重解析 Markdown、不按编号调整输出。仅接纳该章节所需的列表结构（有序条目编号限 1）；引用、任务、容器标题/代码/分隔线、带 info 的围栏代码和范围外行内样式明确拒绝。

本次仅扩充标准样例评分，不改变解析核心、零运行时依赖、禁止 unsafe、累计追加 O(n log n)、空间 O(n) 或共享预算契约。完整缩进代码的空行/段落中断规则、tab/混合缩进、长输入及其它标准章节仍待办，不能由这 12 个短样例推断全面兼容。

```sh
nix develop -c cargo nextest run --workspace --test commonmark_indented_code --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_indented_code.py /path/to/spec.json --check
```

[CommonMark 0.31.2 Code spans](https://spec.commonmark.org/0.31.2/#code-spans) 完整章节（328–349，共 22 例）由 `crates/mdstream-core/tests/commonmark_code_spans.rs` 独立评分：**21 支持、0 刻意差异、1 未支持**。349 已由有界代码回退修复；344 涉及需求范围外的原始 HTML。官方 Markdown/HTML 与当前差异观察分开保存，任一方向变化均要求复核分类；夹具保留规范行号、John MacFarlane / CC BY-SA 4.0 归属和完整来源 SHA-256，生成器校验本地固定来源及章节成员，构建和测试不联网。

4 项测试覆盖独立评分、全部 **299 个合法 UTF-8 切点 / 277 个逐字符前缀**、投影敏感性及范围外拒绝。公开快照比较源码、块/对象身份与范围、样式/显示映射及 pending，完成后逐 run 验证借用文本或代码空白替代的原始映射。投影仅读取公开段落、显示样式和 URI 自动链接，不 trim、不重解析 Markdown、不按编号改写输出；保留连续空格、NBSP、内部换行规范化、代码与字面文字的区别及链接目标。346 的 URI 反引号仅在测试输出层编码为 `%60`，并独立验证核心仍返回原始目标；不宣称通用 URL 编码。范围外块、强调/数学、图片/引用/普通链接等明确拒绝。

该章节最初只新增评分；后续有界代码回退修复 349，专项复杂度和预算验收见下节。零运行时依赖、禁止 unsafe、累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。代码配对的 32 串上限已移除；其它行内语法的完整失败重放及其它标准章节继续待办，也不扩展 HTML 需求。

```sh
nix develop -c cargo nextest run --workspace --test commonmark_code_spans --no-fail-fast --success-output immediate
python3 scripts/generate_commonmark_code_spans.py /path/to/spec.json --check
```


### 未闭合代码候选的代码配对回退

外层反引号候选开放时仍显示原文字面内容；同长度定界符到达时优先闭合外层。若行内序列因 EOF、段落/容器/标题/单元格边界而结束，外层仍未闭合，则从左到右为每个异长反引号串寻找后续第一个同长度串，找到后越过整对，未找到则保留该串为字面文本并继续。**不限制串数**，也不限制串长和正文长度。CommonMark 349 因此显示为字面 `` `foo `` 加代码 `bar`。

这只识别代码，不重放其它行内语法：内部的反斜线不转义反引号，强调、链接、实体和数学仍按原不透明路径处理，代码外的换行/空白不补做普通行内解析。已确认代码复用现有空白规范化和源码映射，容器前缀及顶层行首空格仍由既有块规则处理。完整 CommonMark 失败回退继续待办。

每个正文原始字节只在既有扫描中更新当前段的全空白/末字节摘要；每个异长串保留一份区间摘要，另存一份表头边界快照，不复制或回扫正文、不更新祖先链；代码规范化沿用已有常数个端点字节读取。收尾按（串长、源码次序）排序摘要索引，建立最近同长度后继，再按源码顺序贪心提交不相交代码对。r 份摘要只参与一次 O(r log(r + 1)) 排序和线性索引/配对；排序比较记入 `Work.delimiter_search_steps`，索引及贪心访问不超过 4r 次，记入 `Work.delimiter_steps`。所有候选累计解析 O(n log n)、空间 O(n)；成功外层直接丢弃后备摘要。追加和 finish 不承诺单次硬时延，显示消费继续受调用方共享 Budget 约束。

早表头在旧序列关闭时完成回退；晚表头保存前一行的摘要边界，成功只恢复旧段落中的配对并独立解析表头，失败继续原候选。跨越新表头边界的内层配对不能泄漏。专项 `tests/code_fallback.rs` 复用原 255 组模型并新增 256 组长串独立模型与外层闭合对照，覆盖 31/32/33/1000 串、全切点/逐前缀 EOF、容器/标题/表格、UTF-8/CRLF 映射、稳定身份、缓存失效、共享六字节预算、千层百万字追加及 64–4096 串工作界限；内部测试验证摘要空间随串数增长、正文增长不增配和跨候选复用。

复跑：`nix develop -c cargo nextest run --test code_fallback --test commonmark_code_spans --test code_headers --test closed_code_headers --no-fail-fast`；示例：`nix develop -c cargo run --release -p mdstream-core --example code_fallback`。[原有界交付](DONE.md#2026-11-03-未闭合代码候选有界回退交付)保留历史，当前验收见[无串数上限交付](DONE.md#2026-11-11-未闭合代码候选无串数上限回退交付)。

## 围栏代码块标准样例评分

[CommonMark 0.31.2 Fenced code blocks](https://spec.commonmark.org/0.31.2/#fenced-code-blocks) 完整章节（119–147，共 29 例）由 `crates/mdstream-core/tests/commonmark_fenced_code.rs` 独立评分：**29 支持、0 刻意差异、0 未支持**。131–133 已由[有界空格去缩进](#围栏正文的有界空格去缩进)修复；删除旧差异观察，官方 Markdown/HTML 和评分投影算法保持。离线夹具逐字保留规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整来源 SHA-256，生成器校验本地固定来源及章节成员，构建和测试不联网。

4 项测试覆盖独立评分、全部 **585 个切点 / 556 个逐字符前缀**、源码映射、投影敏感性和范围外拒绝。快照比较源码、块/标题/容器身份与范围、显示映射/样式及 pending。投影仅读取公开块、引用路径、标题索引、原始 info 和显示语义，保留代码字面标记、空行、缩进余量、尾空格、内部 tab 和 LF，以及段落软硬换行、行内代码和块顺序；不 trim、不重解析 Markdown、不按样例编号调整输出。HTML 的 language class 取原始 info 首词并转义属性，完整原始 info 另行核对；这只是测试输出约定，不增加核心渲染 API。列表、容器正文/标题、表格、范围外行内样式及需要实体/反斜线解码的 info 明确拒绝。

正文的 ASCII 空格、容器前缀虚拟空格及原始 tab 列宽去缩进另有专项验收；完整容器规则、超过 32 字符的 opener、超长分类窗口及 info 实体解码等边界不由这些短样例证明，宽泛标准覆盖继续待办。核心追加复杂度、零运行时依赖及共享预算契约保持。复跑：

```bash
python3 scripts/generate_commonmark_fenced_code.py /path/to/commonmark-0.31.2-spec.json --check
nix develop -c cargo nextest run --workspace --test commonmark_fenced_code --no-fail-fast --success-output immediate
```

## 反斜线转义标准样例评分

[CommonMark 0.31.2 Backslash escapes](https://spec.commonmark.org/0.31.2/#backslash-escapes) 完整章节（12–24，共 13 例）由 `crates/mdstream-core/tests/commonmark_backslash_escapes.rs` 独立评分：**11 支持、1 刻意差异、1 未支持**。离线夹具逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属和完整来源 SHA-256；生成器校验固定本地来源与章节成员，构建和测试不联网。

| 分类 | 样例 | 当前行为 |
| --- | --- | --- |
| 支持（11） | 12–13、15–20、22–24 | ASCII 标点转义、非标点保留、反斜线与强调/硬换行、代码字面内容、URI 自动链接及目标/title/info 解码 |
| 刻意差异（1） | 14 | 未闭合代码候选不重放后续转义 |
| 未支持（1） | 21 | 原始 HTML 在需求范围外，当前作为普通段落解析 |

5 项测试覆盖独立评分、全部 **448 个合法 UTF-8 切点 / 435 个逐字符前缀**、投影敏感性和范围外拒绝。快照比较源码、块/对象身份与范围、样式/显示映射及 pending；完成后另核对连续源码覆盖、原始借用文本和代码空白替代映射。投影仅读取公开块、显示语义、核心 `decoded_link_destination`、`decoded_link_title` 与 `code_info` 预算读取输出，不 trim、不重解析 Markdown、不按编号改写输出。只移除段落一个物理末尾 LF，保留连续空格、tab、代码字面语法、强调、软硬换行及块顺序；未评分容器/标题/表格、嵌套行内样式、图片、email 和未解析引用明确拒绝。

直接及引用目标/title 通过核心游标解码，自动链接保留字面内容；原始目标/title/info 和标签另行核验。两章共用的 `tests/support/link_metadata.rs` 仅做输出层 HTML 转义及 UTF-8/反斜线百分号编码，已有百分号序列不再次编码，不做 Markdown 解码、trim 或协议过滤；ASCII 空白、控制字符和其它未评分 URL 编码明确拒绝。六字节、一次 query/object 预算在目标与 title 之间共享，耗尽后保留游标并换帧，实体探测可跨帧；映射连续覆盖原始区间，分块/前缀读取另与无限预算比较。完整代码回退及宽泛标准覆盖仍待办，当前差异观察和官方期望继续分开保存。

初次交付仅新增评分与文档，例 24 随后接入核心 info 读取，22–23 已接入核心目标/title 读取；零运行时依赖、禁止 unsafe、累计追加 O(n log n)、空间 O(n) 及共享预算契约保持；短样例不代替长输入与预算性能验证。复跑：

```bash
python3 scripts/generate_commonmark_backslash_escapes.py /path/to/commonmark-0.31.2-spec.json --check
nix develop -c cargo nextest run --workspace --test commonmark_backslash_escapes --no-fail-fast --success-output immediate
```

## 实体与数字字符引用标准样例评分

[CommonMark 0.31.2 Entity and numeric character references](https://spec.commonmark.org/0.31.2/#entity-and-numeric-character-references) 完整章节（25–41，共 17 例）由 `crates/mdstream-core/tests/commonmark_entities.rs` 独立评分：**15 支持、1 刻意差异、1 未支持**。离线夹具逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整来源 SHA-256；生成器校验固定本地来源与章节成员，构建和测试不联网。

| 分类 | 样例 | 当前行为 |
| --- | --- | --- |
| 支持（15） | 25–30、32–40 | 命名/数字引用、无效候选、代码字面内容、目标/title/info 解码和实体不充当结构标记 |
| 刻意差异（1） | 41 | 失败链接后缀保留 shortcut 引用，隐藏方括号且没有跳转目标 |
| 未支持（1） | 31 | 原始 HTML 在需求范围外，按普通段落解析，正文中的实体仍解码 |

6 项测试覆盖独立评分、全部 **524 个合法 UTF-8 切点 / 507 个逐字符前缀**、投影敏感性、范围外拒绝和逐字符切片/6 字节共享预算读取。公开快照核对源码、块/容器/对象身份与范围、样式/显示映射及 pending；逐 run 用独立固定映射核验实体原始区间及解码值，包括双标量实体、替代字符、tab 和换行。切片及分帧读取核对不重复输出实体、连续源码覆盖和预算耗尽后恢复。

投影只读取公开块、顶层 tight bullet 列表、显示语义及核心目标/title/info 预算解码输出，不 trim、不重解析 Markdown、不按编号调整输出。只移除一个来自原文的物理末尾 LF，实体产生的 LF 即使在段末也保留；连续空白、代码字面内容、强调、软硬换行和块顺序保持可区分。未解析引用直接呈现公开显示内容，不补回隐藏方括号；只有已有目标的链接输出 anchor。目标/title/info 不在投影中解码；目标中的非 ASCII 字符按 UTF-8 字节做百分号编码，反斜线编码为 `%5C`，其余已评分 URI 字符保持，属性按 HTML 转义。完整原始目标/title/info 和失败链接对象另行核验。范围外引用/嵌套或 loose 列表、多叶块条目、标题/表格/数学、嵌套行内样式、图片/email 及额外 URL 编码明确拒绝。

当前差异观察与官方期望分开保存，改善或回退均须复核分类。32–33 通过核心目标/title 游标匹配官方期望；补充非递归实体、转义优先、中文/emoji、双标量、长命名实体、原有百分号序列和 title 空白保留验证，共享六字节预算与无限预算保持一致。完整失败链接回退、通用 URL 输出和宽泛标准覆盖继续待办；本项不改变解析核心、追加复杂度、运行时依赖或共享预算契约，这些短样例不代替长输入性能验证。

```bash
python3 scripts/generate_commonmark_entities.py /path/to/commonmark-0.31.2-spec.json --check
nix develop -c cargo nextest run --workspace --test commonmark_entities --no-fail-fast --success-output immediate
```


## Tab 标准样例评分

[CommonMark 0.31.2 Tabs](https://spec.commonmark.org/0.31.2/#tabs) 完整章节（1–11，共 11 例）由 `crates/mdstream-core/tests/commonmark_tabs.rs` 独立评分：**11 支持、0 刻意差异、0 未支持**。1–11 均匹配官方输出；1、2、8 已由[顶层 tab 缩进代码](#顶层-tab-缩进代码)修复，6 已由[引用内 tab 缩进代码](#引用内-tab-缩进代码)修复，5 已由[列表续行部分 tab 缩进代码](#列表续行部分-tab-缩进代码)修复，7 已由[列表首行 tab 缩进代码](#列表首行-tab-缩进代码)修复；9 已由[部分 tab 路径新嵌套列表](#部分-tab-路径新嵌套列表)修复。离线夹具逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整来源 SHA-256；生成器校验固定本地来源与章节成员，构建和测试不联网。

本章节已无差异观察，逐例直接对照官方期望。投影只消费公开块、容器、标题和显示片段，保留列表嵌套与 tight/loose、引用、原始 tab、连续空格、代码字面和软硬换行，不展开 tab、不 trim、不重解析 Markdown、不按编号改写输出。投影接纳引用和条目内无 info 代码块，直接读取核心输出的部分 tab 空格；范围外样式、链接、表格、带 info 代码、容器标题/分隔线等明确拒绝。

全部 **148 个合法 UTF-8 切点、137 个逐字符前缀**核对公开快照与 pending；逐 run 核对源码连续覆盖、借用原文和部分 tab 的单字节替换映射，逐字符切片与 6 字节共享预算恢复验证隐藏前缀、字面 tab 和剩余空格。评分包含 5 项测试；顶层及引用内 tab 修复各有 5 项回归，列表续行、首行及新嵌套列表修复各有 6 项回归，见下节。累计追加 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 契约保持。完整容器叶块 tab 规则及宽泛标准覆盖仍由 TODO 跟踪，这 11 个短样例不代表全面兼容或性能证明。

```sh
python3 scripts/generate_commonmark_tabs.py /path/to/spec.json --check
nix develop -c cargo nextest run --workspace --test commonmark_tabs --no-fail-fast --success-output immediate
```


## 顶层 tab 缩进代码

顶层一般叶块分类中，行首 0–3 个 ASCII 空格紧接一个 tab，按第 0 列起算的四列 tab stop 达到代码缩进阈值；隐藏这几个原始字节，生成或续接同作用域 `BlockKind::Code { info: None }`。首个 tab 后的空格、tab、Markdown 标记和实体均作为字面代码保留，不展开或重写源码。已有四空格代码可与这些行合并，CRLF 只隐藏 CR，EOF 不补 LF。

容器路径匹配优先决定归属：留在容器内的叶块和 lazy 行继续既有规则；路径关闭并返回根时才采用顶层结果。顶层子项曾使官方 Tabs 5 从根段落变为根代码；其条目内代码语义随后由下述列表续行子项修复。围栏及数学正文、脚注续行的优先级不变；待确认表头的合法分隔行仍先确认表格（例如 `a|b\n\t-|-\n`），失败分隔行和已确认表格后的代码行才进入一般块分类。空行终止缩进代码、缩进代码可中断段落仍是项目方言。

沿用 LF/EOF 或 1024 字节分类窗口：短未完成行暂定显示原文，确认后隐藏前缀并失效旧显示游标/缓存；窗口内完整纯空白行是 Blank，跨过窗口的未完成长空白行按既有代码分类规则提交，随后不重新分类。额外检查最多四个前缀字节，复用已有隐藏区间及字面代码路径，无新增持久状态、旧正文/历史块/祖先回扫；累计追加 O(n log n)、空间 O(n) 与共享查询预算保持。

`tests/root_tab_code.rs` 的 5 项回归覆盖 360 组四空格模型、代码合并/空行/容器/表格/围栏边界、UTF-8 全切点与逐前缀、分类窗口、源码借用与逐字符切片/tile、六字节预算恢复和缓存失效。百万字代码后千次逐字追加无分类/行内/容器/树回扫，4096 行继续合并同一块。该子项将官方 Tabs 评分提升到 7 支持、4 刻意差异；引用内扩展随后修复 6，当前评分见上节。官方夹具保持不变，5 后由列表续行子项修复，7 后由列表首行子项修复；9 随后由新嵌套列表子项修复，完整容器/空行规则仍待实现。

复跑：`nix develop -c cargo nextest run --workspace --test root_tab_code --test commonmark_tabs --no-fail-fast --success-output immediate`。


## 引用内 tab 缩进代码

显式引用标记后的可选一列空白及四列代码缩进允许部分消费 tab，按物理行每四列对齐。新引用首行和已有路径最后一层 Quote 均适用，外层路径仍须满足既有匹配规则。例如 `>\t\tfoo` 输出两格加 `foo` 的引用内代码；扣满四列后未消费的完整 tab 保留字面，被部分消费的最后一个 tab 用原单字节源码区间映射为 1–3 个空格，核心源码不变。代码可中断引用段落/表格，同作用域相邻代码行合并，原有围栏及数学正文优先。

本项只原子接受“显式 Quote + 含 tab 的代码缩进”。不足四列、完整纯空白行、新引用非代码叶块和缺引用标记的 lazy 行保持原方言；列表续行部分 tab 代码另见下节。短行在 LF/EOF 前暂定显示原文；超过 1024 字节分类窗口后不重试，超长空白按既有一次分类规则处理，过深且窗口内未出现的代码前缀亦不追认。空行终止代码和代码中断段落仍是既有方言。

探测复制一个固定大小的前缀游标，至多重复检查当前窗口的引用标记/空白，失败不提交位置或虚拟列；实际检查计入 `container_bytes`。不分配或复制祖先路径，不读取旧正文；只新增本行分类标记，映射复用既有显示记录，累计追加 O(n log n)、空间 O(n)、零运行时依赖及共享预算契约保持。

`tests/quote_tab_code.rs` 的 5 项回归覆盖 504 组独立列宽模型（含引用/列表外层、LF/CRLF/EOF、UTF-8 和字面代码）、其中 336 组已有路径代码续接、失败/lazy/围栏/表格边界、合法切点与逐字符前缀、分类窗口及缓存失效、tile 边界上的单字节替换、切片和六字节预算恢复。千层引用百万字后千次逐字追加不增加分类/行内/容器/树访问；4096 行保持一个代码块。官方样例 6 修复；5、7 随后由下述列表续行/首行子项修复，9 随后由新嵌套列表子项修复，完整容器 tab 规则继续待办。

复跑：`nix develop -c cargo nextest run --workspace --test quote_tab_code --test commonmark_tabs --no-fail-fast --success-output immediate`。


## 列表续行部分 tab 缩进代码

已有容器路径完整匹配、其中消费了部分 tab、最内层为 Item 时，在普通段落续行未启用的路径上尝试再消费四列代码缩进。空行后的列表正文和相邻代码续行适用；表头候选阻止普通续行时也可按代码分类。例如 `- foo\n\n\t\tbar\n` 保留原条目，生成两格加 `bar` 的代码，列表转为 Loose。嵌套路径沿物理行四列 tab stop 计算，最后被部分消费的 tab 通过原单字节区间输出 1–3 个空格，之后的完整 tab 保留字面；同作用域相邻代码合并。

只有成功扣满四列且完整行不是纯空白才提交；未完成分类窗口沿用既有规则。普通段落/lazy 续行、失败路径、围栏和数学正文保持原方言；新列表首行和新嵌套列表分别见下文；完整字节边界路径随后由[完整边界续行子项](#列表完整边界续行-tab-缩进代码)扩展。空行仍终止代码，短行在 LF/EOF 前暂定原文，1024 字节窗口后不追认新前缀。

实现复用 `partial_prefix` 的固定大小游标及缓存空白末端，失败不修改位置或余量，实际检查计入 Work；只消费当前有界前缀，不读取旧正文或无输入支撑的祖先，不新增持久状态。累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算契约保持。

`tests/item_tab_code.rs` 的 6 项回归覆盖 396 组独立列宽模型（其中 264 组另验续接）、嵌套/有序列表、UTF-8/CRLF/EOF、字面语法、列表松紧/身份、失败/lazy/围栏/数学/表格边界、全切点与逐字符前缀、分类窗口及缓存失效。源码连续覆盖、tile 边界单字节替换、逐字符切片、零预算停步及六字节恢复均有验收；998 层容器百万字后千次追加不增加分类/行内/容器/树访问，4096 行保持同一代码块。官方 Tabs 5 修复时评分为 9 支持、2 刻意差异；7、9 随后分别由列表首行及新嵌套列表子项修复，完整容器/空行规则继续待办。夹具不改，投影直接消费核心代码及映射。

复跑：`nix develop -c cargo nextest run --workspace --test item_tab_code --test commonmark_tabs --no-fail-fast --success-output immediate`。


## 列表首行 tab 缩进代码

新列表或兄弟条目标记后的水平空白若含 tab 且至少五列，消费一列条目分隔加四列代码缩进；条目续行宽度只包含前置缩进、标记和一列分隔。按整条物理行的四列 tab stop 计算；最后部分消费的 tab 剩余 1–3 列映射回该原始字节，后续完整 tab/空格保持字面。例如 `-\t\tfoo` 显示两格加 `foo`，后续六空格正文续接同一代码块。代码正文中的 `[x]`、围栏、标题、实体与新容器标记均为字面；同层兄弟代码关闭旧条目及其子层。

仅适用于既有分类窗口内可识别的显式标记和含 tab 的空白。纯空格首行随后由[纯空格子项](#列表首行纯空格缩进代码)扩展；少于五列、完整空白行仍沿用原规则；围栏/数学正文、失败/lazy 路径保持原方言；样例 9 的部分 tab 新嵌套列表随后由下节交付。短输入等待 LF/CRLF/EOF，未完成窗口到达上限后提交且不回扫重分类，空行仍终止缩进代码。

标记扫描复用刚读过的空白末端和固定大小 Prefix，额外推进最多五字节；没有新增持久状态、旧正文或无输入支撑的祖先回扫。`tests/list_start_tab_code.rs` 的 6 项回归覆盖 2268 组独立列宽模型（1512 组另验续接）、有序/嵌套/兄弟条目、全切点/前缀、UTF-8/CRLF/EOF、分类窗口、源码映射/切片及六字节预算恢复、缓存失效、1000 层容器百万字后千次追加和 4096 个兄弟代码条目。官方 Tabs 7 修复时为 10 支持、1 刻意差异、0 未支持；9 后由下节修复，当前评分见上文；官方夹具保持，评分投影只补充紧凑条目以代码块开头时的 HTML 换行。纯空格首行随后由下文子项交付；完整容器/空行及宽泛标准覆盖仍由 TODO 跟踪。


## 部分 tab 路径新嵌套列表

已有容器路径完整匹配、其中确实部分消费 tab 后，剩余缩进不超过三列且存在新列表标记时，允许在当前作用域下建立新 List/Item。按物理行四列 tab stop 计算，多层 Item 可共用一个 tab；尚余虚拟列作为新标记缩进消费，整个前缀隐藏且仍可按原始字节读取。例如 ` - foo\n   - bar\n\t - baz\n` 形成三层列表，保留原有祖先 ID，显示 `foo\nbar\nbaz\n`。bullet、有序、任务、连续嵌套标记和已有列表首行代码规则复用原实现；容器范围、松紧、变更通知和行内作用域随正常新列表更新。

先复用现有部分 tab 叶块探测筛选候选，再原子检查结构优先级。失败路径、分隔线、Setext、表头分隔行、围栏/数学正文保持既有行为；同类标记兄弟条目的部分 tab 匹配随后由下节交付；完整路径上的新引用随后由[新引用子项](#部分-tab-路径新引用)交付；异类标记或错位兄弟、不完整路径的完整规则仍未扩展；纯空格首行见下文子项。短输入在 LF/CRLF/EOF 前暂定原文；有界分类窗口内识别，窗口外标记不追认，不回扫长行。

每次探测仅检查当前有界行前缀，复用空白末端缓存；每个匹配帧消耗正列数，一个 tab 至多承担四列，不遍历未匹配的祖先后缀。普通行沿原路径处理；新列表候选至多增加固定次数的前缀扫描，无持久状态、旧正文或历史块回扫，累计 O(n log n)、空间 O(n) 与共享预算契约保持。

`tests/nested_tabs.rs` 的 6 项回归包含 1695 组独立列宽展开模型、UTF-8/CRLF/EOF、全切点与逐前缀、结构优先级/失败路径、分类窗口、祖先 ID 与读取游标失效、源码连续覆盖/逐字符切片/tile 及六字节预算恢复；千层容器百万字后千次追加不增加分类/容器访问，4096 组根兄弟与部分 tab 新子列表保持有界前缀工作。官方 Tabs 9 修复，完整章节 11 支持、0 刻意差异、0 未支持；官方夹具和评分投影算法保持，完整容器规则与宽泛标准覆盖继续待办。

复跑：`nix develop -c cargo nextest run --workspace --test nested_tabs --test commonmark_tabs --no-fail-fast --success-output immediate`。


## 部分 tab 路径兄弟条目

已识别列表的祖先前缀匹配成功且部分消费 tab 后，若剩余缩进至多三列且不足旧条目的正文缩进、标记类型相同，识别为兄弟 Item（不同缩进扩展见文末）。例如 `- parent\n\t- first\n\t- second\n` 保留父 Item 和内层 List，仅关闭旧 Item 及其后代、创建新 Item。bullet 字符或有序列表分隔符必须相同，有序数字可以不同；复用任务、标题、嵌套标记、首行代码、空行松紧与增量通知规则。标记前的虚拟列全部隐藏，代码首行另按既有规则记录余量，源码区间保持原始字节。

探测在候选兄弟层停止，不要求匹配旧 Item 的正文缩进或后代；结构分类确认后才提交祖先游标。分隔线仍优先，兄弟改变作用域，不能被旧条目的 Setext 或表头分隔行吞掉。失败、异类标记、超出兄弟范围的缩进、缺失引用、已开启围栏/数学及窗口外标记保留原方言；部分 tab 表格续行、一般不完整路径和新引用规则不在本项范围。短输入等待 LF/CRLF/EOF，超过分类窗口不重新扫描分类。

固定大小游标复用缓存空白末端，每个成功匹配的祖先消耗正列数，一个 tab 最多承担四列；候选只增加固定次数的有界前缀扫描，旧后代仅在关闭时各弹出一次。无新增持久状态、旧正文/历史块回扫或运行时依赖，累计 O(n log n)、空间 O(n) 与共享预算契约保持。

`tests/sibling_tabs.rs` 的 6 项回归包含 15,660 组独立列宽展开模型、祖先/List 身份与后代关闭、列表松紧/通知、失败优先级、UTF-8/CRLF/EOF、全切点/逐前缀和分类窗口；另验 tile 边界、连续源码覆盖、逐字符切片、零预算停步与六字节恢复、读取游标失效、千层容器百万字后千次追加及 4096 个部分 tab 兄弟条目。CommonMark Tabs 保持 11 支持、0 刻意差异、0 未支持，官方夹具与评分投影未改；完整容器规则和宽泛标准覆盖继续待办。

复跑：`nix develop -c cargo nextest run --workspace --test sibling_tabs --test commonmark_tabs --no-fail-fast --success-output immediate`。


## 列表首行纯空格缩进代码

已识别的新列表或兄弟条目标记后至少五个 ASCII 空格时，消费一格条目分隔和四格代码缩进，剩余空格及正文保持字面。例如 `-       *raw*` 显示两格加 `*raw*`，不会解析强调；任务标记、实体、围栏和容器标记也保留字面。条目续行宽度包含前置缩进、标记和一格分隔，六空格后的根列表正文可续接同一代码块。既有部分 tab 祖先匹配路径也复用此规则，兄弟条目保留 List ID、松紧和关闭通知。

只有在当前有界分类窗口内识别到标记及五列空白才应用；1–4 格、完整空白行、失败/lazy、围栏/数学正文及空行终止规则保持。短尾等待 LF/CRLF/EOF，超过分类窗口后按已有规则提交，不重新扫描长行。完整容器和空行规则仍未完成。

实现沿用首行 tab 代码路径，复用已扫描的空白末端，额外最多消费五字节；无新增持久状态、旧正文或祖先回扫，累计追加 O(n log n)、空间 O(n)、零运行时依赖和共享预算契约保持。`tests/list_start_space_code.rs` 六项回归覆盖 2268 组独立空格模型（1512 组续接）、有序/嵌套/兄弟条目、部分 tab 祖先、松紧/通知、全切点/前缀、UTF-8/CRLF/EOF、分类窗口、源码借用/切片/tile、六字节预算和缓存失效，以及千层百万字追加和 4096 个兄弟代码条目。

复跑：`nix develop -c cargo nextest run --workspace --test list_start_space_code --test list_start_tab_code --test commonmark_tabs --no-fail-fast --success-output immediate`。


## 部分 tab 路径新引用

已有容器路径完整匹配且其中确实部分消费 tab 后，剩余 0–3 列允许开启显式 `>` 引用。多层条目可共用一个 tab，虚拟列作为标记缩进隐藏，源码字节和祖先 ID 保持。例如 `- parent\n\t> child\n    > next\n- sibling\n` 在原 Item 下创建 Quote，显示 `parent\nchild\nnext\nsibling\n`。已有 Quote 路径也适用，例如 `> parent\n>\t> child\n`；新引用内的标题、列表/任务、表格、定义、围栏/数学及代码继续使用既有叶块规则，正常更新通知、松紧和行内作用域。

本项只扩展“完整已有路径后开启新引用”。新标记自身的 tab 分隔仍使用原引用规则；不完整路径、异类/错位兄弟、缺失引用、超额缩进、已开启围栏/数学正文均保持原方言。短尾等待 LF/CRLF/EOF；只识别当前有界分类窗口内的标记，超长行提交后不回扫追认。完整容器/空行、其余叶块部分 tab 和宽泛标准覆盖继续待办。

复用部分 tab 只读探测及结构提交路径，仅将 `>` 候选交给现有新容器流程；固定大小游标缓存空白末端，每个匹配帧消费正列数，额外工作局限于当前有界前缀。没有新增持久状态、旧正文/历史块或无输入支撑的祖先回扫；累计追加 O(n log n)、空间 O(n)、零运行时依赖和共享预算保持。

`tests/nested_quotes.rs` 六项回归覆盖 3051 组独立列宽模型、祖先身份/松紧/通知、叶块组合与失败边界、UTF-8/CRLF/EOF 全切点及逐字符前缀、窗口边界、源码连续覆盖/借用/逐字符切片/tile、代码 tab 余量映射、零预算停步及六字节恢复、读取游标失效。999 层容器百万字后千次追加不增加分类/容器访问，4096 组新引用保持有界前缀工作。官方 Tabs 仍为 11 支持、0 刻意差异、0 未支持，评分夹具与投影未改。

复跑：`nix develop -c cargo nextest run --workspace --test nested_quotes --test commonmark_tabs --no-fail-fast --success-output immediate`。


## 列表完整边界续行 tab 缩进代码

已有容器路径在完整字节边界匹配、最内层为 Item 且普通正文续接未启用时，也允许消费四列 tab/混合代码缩进。例如 `-   seed\n\n\t\tcode\n` 保留原 Item，产生 `code` 代码正文，列表变为 Loose；`-  seed\n\n   \t\tcode\n` 的代码缩进在第二个 tab 内结束，剩余一列显示空格并映射回原单字节。各层沿物理行四列 tab stop 计算，后续完整 tab 保留字面，同作用域相邻代码合并。

复用含 tab 前缀的既有探测，仅放宽“四列代码缩进前必须已部分消费 tab”的条件；失败探测、普通段落/lazy、完整空白行、围栏/数学正文及分类窗口规则保持。空行仍终止缩进代码，短尾等待 LF/EOF；不完整路径和完整容器规则继续待办。固定大小 Prefix 复用空白末端，额外消费至多四列，不回扫旧正文或无输入支撑的祖先，不新增持久状态；累计追加 O(n log n)、空间 O(n)、零运行时依赖及共享预算保持。

`tests/full_item_tab_code.rs` 五项回归覆盖 414 组独立列宽模型（276 组另验续接）、有序/嵌套/引用路径、作用域/松紧、字面代码、失败边界、UTF-8/CRLF/EOF 全切点与逐字符前缀、分类窗口、缓存失效、tile 边界替换/逐字符切片及六字节预算恢复。500 层容器百万字后千次追加不增加分类/行内/容器/树访问，4096 行合并并符合工作计数上界。复跑：`nix develop -c cargo nextest run --test full_item_tab_code --no-fail-fast --success-output immediate`。


## 分类窗口内的长围栏

反引号或波浪线 opener 的 `MAX_FENCE_LENGTH` 为 `BLOCK_LOOKAHEAD - 1`（1023），沿用 CommonMark 的同字符、至少三字符规则；容器前缀和缩进仍计入现有 1024 字节分类窗口。定界串后须已看到非同字符字节或完整行末，窗口末端仍在串内时按普通行内语法消费，不追认后续到来的串尾。顶层 1023 个标记加 LF 可识别；1024 个标记不可识别；`> ` 后 1021 个标记加 LF 可识别，1022 个标记填满窗口则不可识别。这也修正深容器前缀将不足 32 字符的串截在窗口末端时的提前确认。

短尾在 LF/CRLF/EOF 时分类，open fence 可延续到 EOF；较短 closer、异字符、非空白后缀、四格缩进及不能连同行末放入窗口的 closer 都保留代码正文。已确认 opener 的超长 info 仍沿用既有方言，只保存窗口内原始区间，其余整行隐藏，晚来的反引号不撤销；info 实体解码已由文末读取接口提供，窗口外定界识别和完整容器/tab 缩进规则仍待实现。

实现只调整常量并检查当前有界切片是否已有定界串终点，不增加持久状态或扫描窗口，不回读旧正文、历史块或祖先。累计追加 O(n log n)、空间 O(n)、零运行时依赖及共享预算契约保持。`tests/long_fences.rs` 的五项回归覆盖 504 组长度/缩进/容器模型、UTF-8 全切点与逐字符前缀、失败/EOF/窗口边界、源码映射和六字节预算。千层容器、百万字成功/失败正文后的逐字追加保持分类和容器计数不增长。

复跑：`nix develop -c cargo nextest run --workspace --test long_fences --no-fail-fast`。


## 围栏 closer 的 tab 缩进

已有反引号/波浪线围栏完整匹配容器路径后，closer 前的 space/tab 按物理行四列制表位计量，加上前缀部分 tab 剩余的虚拟列，总缩进不超过三列即可闭合。例如 `> ```\n> x\n> \t```\n` 的最后一个 tab 从第二列到第四列，只增加两列叶块缩进；顶层 tab 增加四列，仍保留为代码正文。定界字符必须相同且长度不少于 opener，后缀沿用既有 ASCII 空白规则，整行仍须在分类窗口内确认。

关闭判断先于正文去缩进。四列及以上、短串、异字符、非空白后缀、跨分类窗口和未完成的候选沿用原行为；短行在 LF/CRLF/EOF 时确认，超窗候选不在后续追加中追认。缺失容器路径/兄弟切换及数学块规则保持不变；opener 的 tab 缩进随后由下节扩展。info 实体解码已由文末读取接口提供；完整容器/其它叶块缩进和窗口外定界识别继续由 TODO 跟踪。

当前有界窗口内的已匹配前缀只额外读取一次以计算列模四，不遍历祖先节点；同一列值交给已有正文去缩进状态，并计入 `Work.classified_bytes`。closer 缩进探测超过三列立即失败，无新增持久状态或旧正文回扫；累计追加 O(n log n)、空间 O(n)、零运行时依赖和共享预算契约保持。

`tests/fence_closer_tabs.rs` 五项回归覆盖 1920 组独立列宽/长度/行尾模型，根/引用/列表及部分 tab，UTF-8 全切点/逐前缀/EOF、失败/数学/窗口边界、稳定容器身份和缓存失效、tile 边界源码映射/逐字符切片及六字节预算。千层容器百万字和超长失败 closer 后千次追加不增加分类/行内/容器/树访问；4096 次开闭验证工作计数上界。

复跑：`nix develop -c cargo nextest run --test fence_closer_tabs --no-fail-fast`。


## 围栏 opener 的 tab 缩进

已识别容器前缀之后，反引号/波浪线起始围栏前的 space/tab 按物理行四列制表位计量，加上前缀部分 tab 的虚拟余量，总缩进至多三列才开启。该列数交给既有正文去缩进，例如 `> \t```rs\n>   x\n> ```\n` 的 tab 从第二列到第四列，正文去掉两列后显示 `x\n`。已有列表/引用完整路径的部分 tab 探测使用同一列位置，保留容器身份；开启行隐藏，info 仍借用原始源码，不解码实体或转义。

此项从现有匹配器给出的前缀边界开始计算；新引用自身 tab 分隔紧接代码围栏的子项已由[后续交付](#新引用-tab-分隔后的代码围栏)扩展，其余新引用分隔规则仍待办。顶层 tab 是四列，不能作为围栏缩进；超过三列、短串、反引号 info 内含反引号、路径失败及其它叶块保持既有分类。数学块定界行的 tab 缩进见[独立子项](#独立数学块定界行的-tab-缩进)。短行等待 LF/CRLF/EOF；定界终点须在现有共享分类窗口内确认，超窗串不追认。已确认的长 info 仍只保留窗口内区间，晚来的反引号不撤销。

列模四只在原始 tab 缩进候选或活动代码围栏需要时从当前有界前缀计算一次，并计入 `Work.classified_bytes`；部分 tab 探测直接复用 Prefix 的列位置。无 tab 的普通叶块不增加前缀扫描，缩进探测最多检查四个字节，无新增持久状态，不回扫旧正文或祖先节点。累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 及共享预算契约保持。

`tests/fence_opener_tabs.rs` 五项回归覆盖 720 组独立列宽/定界/行尾模型、原始 info、UTF-8 全切点和逐字符前缀/EOF、数学/作用域/失败及窗口边界、晚确认身份与游标失效、tile 边界连续源码映射/逐字符切片和六字节预算。千层容器百万字正文后千次追加不增加分类/行内/容器/树访问；超窗失败 opener 不重新分类；4096 次围栏开闭验证工作计数上界。

复跑：`nix develop -c cargo nextest run --test fence_opener_tabs --no-fail-fast`。


## ATX 标题的 tab 缩进

已识别容器前缀之后，ATX 开头井号前的 space/tab 按物理行四列制表位计量，连同部分 tab 的虚拟余量至多三列时识别标题。例如 `> \t# title` 的 tab 占两列，`>\t# title` 经下方新引用分隔扩展后余下两列，两者都显示 `title`。已有列表/引用完整路径保留确切 ContainerId；标题级别、关闭井号、行内语义、章节范围、搜索、slug 与折叠复用既有实现。

顶层 tab 占四列，仍为缩进代码；超过三列、七个及以上井号、井号后缺少 space/tab/行结束、失败容器路径保持原分类。活动代码/数学块仍保留字面内容；定义、普通正文规则保持；新引用标记自身的 tab 分隔见[后续子项](#新引用-tab-分隔后的-atx-标题)；Setext 下划线见[后续子项](#setext-下划线的-tab-缩进)。短行等待 LF/CRLF/EOF；井号串及必要分隔须在原共享分类窗口内确认，截断串不在后续追加中追认。已确认长标题的正文/尾缀仍沿用单遍端点摘要。

实现复用代码围栏的至多四字节缩进探测和当前有界前缀列位置，不增加扫描、持久状态、旧正文或祖先节点回访；追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 和共享预算契约保持。`tests/atx_tabs.rs` 六项回归包含 1848 组列宽/级别/行尾模型、全 UTF-8 切点/逐前缀/EOF、失败/窗口边界、标题身份/章节/通知、搜索/slug/折叠、tile 边界源码映射/逐字符切片、零预算停步与六字节恢复，以及千层百万字标题后千次追加、超长失败串和 4096 个标题的工作计数。

复跑：`nix develop -c cargo nextest run --test atx_tabs --no-fail-fast`。完整容器/其它叶块 tab、窗口外识别和宽泛标准覆盖仍见 TODO。


## 分隔线的 tab 缩进

已识别容器前缀后，分隔线前的 space/tab 按物理行四列制表位计量，连同部分 tab 的虚拟余量至多三列时识别 `BlockKind::ThematicBreak`。至少三个相同的 `*`、`-` 或 `_`，中间和末尾允许 space/tab。例如 `> \t* * *\n` 中 tab 占两列；`- seed\n\t- - -\n` 在原条目内生成分隔线，保留 ContainerId，不建立嵌套列表。显示仍沿用既有分隔线语义：隐藏容器前缀，叶块内原始缩进、标记和空白保持字面；部分 tab 前缀余量不生成可见替换。

Setext 判定保持优先；[后续子项](#setext-下划线的-tab-缩进)已扩展其 tab 缩进，因此 `> seed\n> \t---\n` 确认为二级标题。顶层 tab 占四列，仍为代码；超额缩进、少于三个标记、异类标记/非空白后缀、缺失路径、围栏/数学正文保持既有分类。整行须在共享分类窗口内完成，短尾等待 LF/CRLF/EOF，窗口外候选不回扫追认；新引用自身 tab 分隔的分隔线见[后续子项](#新引用-tab-分隔后的分隔线)，完整容器、其它叶块和窗口外识别继续待办。

叶块复用已有围栏/ATX 缩进结果；部分 tab 原子匹配允许分隔线，结构优先级复用既有后缀摘要和缓存列宽。无新增持久状态，不回扫旧正文、历史块或无输入支撑的祖先；累计追加 O(n log n)、空间 O(n)、零运行时依赖及共享预算保持。`rule_tabs` 验证 2376 组独立列宽模型、UTF-8 切点/前缀/EOF、映射/切片/六字节预算、千层百万字追加和 4096 行。复跑：

```sh
nix develop -c cargo nextest run --test rule_tabs
```

## Setext 下划线的 tab 缩进

同作用域已有段落（含待确认表头）之后，完整匹配容器前缀的 Setext 下划线按物理行四列制表位计算 space/tab 缩进，连同部分 tab 的余量至多三列时确认。连续一个或多个 `=` / `-` 分别生成一级/二级标题，允许既有尾空白；标题优先于分隔线及空嵌套列表。`> seed\n> \t---\n`、`- seed\n\t=\n` 均提升原段落，保留 BlockId、ContainerId、跨行内容、标题索引和通知。整个下划线物理行隐藏，源码及字节映射不变。

没有同作用域段落、缺失前缀、空行隔断、兄弟条目、非连续或混合标记、非空白后缀、超过三列及已开启围栏/数学块保持原规则。根 tab 占四列，仍是缩进代码。短尾等待 LF/CRLF/EOF；下划线整行须在包含容器前缀的共享分类窗口内完成，窗口外候选按原普通文字规则处理，不追认。引用链接定义见[后续子项](#引用链接定义的-tab-缩进)；完整容器匹配、脚注定义/表头等其余叶块、窗口外语法与宽泛标准覆盖仍待办。

复用已有叶块列宽计算和标题确认路径，部分 tab 原子探测接收同作用域段落资格；无新增持久状态或旧正文/历史叶块回扫。累计追加 O(n log n)、空间 O(n)、零运行时依赖及共享预算保持。`setext_tabs` 验证 1728 组独立列宽/标记/行尾模型、UTF-8 全切点及前缀/EOF、身份/折叠通知、原始源码与切片/tile 映射、六字节显示/搜索/slug 预算，以及 998 层引用中的百万字标题晚确认和百万字失败候选追加。复跑：

```sh
nix develop -c cargo nextest run --test setext_tabs
```


## 引用链接定义的 tab 缩进

已识别容器前缀后，引用链接定义前的 space/tab 按物理行四列制表位计量，连同部分 tab 虚拟余量至多三列时识别。例如 `> \t[r]: /url` 和 `- seed\n\t[r]: /url` 都在原作用域建立 `BlockKind::Definition`，整行隐藏，原始源码保留。引用仍使用全局命名空间、首定义优先；晚定义只更新符号及受影响区间，不枚举旧链接。目标及 title 保持原始借用区间，实体和反斜线不在此子项中解码。

短尾等待 LF/CRLF/EOF，完整定义必须落在包含容器前缀的共享分类窗口内，超窗候选不追认。超过三列、非法标签/目标/title、缺失路径、代码/数学正文和脚注定义沿用既有规则。表头候选及已确认表格可被定义中断；普通段落中断也沿用既有定义方言，不代表完整 CommonMark 引用定义规则已实现。脚注起始行见[后续子项](#脚注定义起始行的-tab-缩进)；完整容器、脚注续行及其它叶块缩进、元数据解码和窗口外语法仍见 TODO。

核心复用已有至多四字节缩进探测及当前有界前缀列位置，部分 tab 原子路径允许引用定义；无新增持久状态、旧正文或历史链接回扫，累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 和共享预算保持。`definition_tabs` 的五项测试覆盖 864 组列宽/目标/行尾模型、全 UTF-8 切点及逐前缀/EOF、失败/窗口/表格边界、引用更新和首定义优先、连续源码及 tile/逐字符映射、零预算停步和六字节恢复，以及 998 层引用中的百万字追加、超窗百万字失败候选、两万旧引用与 4096 个定义。

复跑：`nix develop -c cargo nextest run --test definition_tabs --no-fail-fast`。


## 脚注定义起始行的 tab 缩进

已识别容器前缀后的 space/tab 按物理行四列制表位计量，连同部分 tab 虚拟余量至多三列时识别脚注起始标记。例如 `> \t[^n]: body`、`- seed\n\t[^n]: body` 在原作用域建立隐藏的 `FootnoteDefinition`，保留原始源码及全局首定义优先、首次引用稳定编号。冒号后前缀内已有的一个 space/tab 沿用既有省略规则，其余正文原样借用，不解析 Markdown 或解码实体。

短尾等待 LF/CRLF/EOF；合法 `[^label]:` 只需落在含容器前缀的共享分类窗口内，正文可跨窗口增长，不重新扫描旧正文。超过三列、非法或窗口外标记、缺失容器路径、代码/数学正文沿用既有规则。同容器空白分段保持；续行的四列完整 tab 边界扩展见下节。普通段落、待定表头和已确认表格可被定义中断。完整容器、新引用标记自身 tab 分隔、其余叶块、脚注正文解析与实际排版仍见 TODO。

`crates/mdstream-core/tests/footnote_tabs.rs` 覆盖 864 组独立列宽模型、作用域/原始正文、失败/窗口/四空格续行、UTF-8 全切点及逐前缀/EOF、稳定身份/晚定义/重复定义、源码映射和六字节预算恢复。998 层百万字正文后的千次追加不增加分类/行内/容器扫描；两万旧引用的晚定义不回访引用对象，4096 个后续定义保持线性分类计数。实现复用已有缩进扫描及部分 tab 原子匹配，无新增持久状态，累计 O(n log n)、空间 O(n) 与共享预算契约保持。

复跑：`nix develop -c cargo nextest run --test footnote_tabs --test footnotes --test footnote_blocks --no-fail-fast`。


## 独立数学块定界行的 tab 缩进

已识别容器前缀之后，独占行 `$$` 的 opener/closer 前 space/tab 按物理行四列制表位计量，含部分 tab 虚拟余量至多三列才识别。例如 `> \t$$\n> \tx\n> \t$$\n` 开启并闭合引用内数学块，payload 为 `\tx\n`；开栏缩进不从数学正文扣除。已有路径的部分 tab 仍只展开容器前缀的剩余列，正文中的完整 tab 保持字面。原始源码、BlockId、ContainerId、折叠意愿和增量通知保持。

定界须恰好两个美元符号，后缀仅 space/tab，整行在含容器前缀的 1024 字节分类窗口内完成；短尾等待 LF/CRLF/EOF。顶层 tab 占四列、超额缩进、额外美元符号、非空白后缀及孤立 CR 均不作为定界；超窗候选不在后续追加中追认。数学块内部失败候选保留原文，作用域退出或 EOF 未闭合仍封存原数学块。代码围栏优先；缺失路径、新引用标记自身分隔、完整容器规则和额外数学方言继续由 TODO 跟踪。

opener 复用已有叶块列宽扫描，closer 复用围栏列宽扫描；遇原始 tab 缩进时，从当前有界前缀计算列模四并计入 `Work.classified_bytes`。无 tab 的数学正文不增加前缀扫描，无新增持久状态或旧正文/祖先节点回扫，累计追加 O(n log n)、空间 O(n)、零运行时依赖及共享预算保持。

`tests/math_delimiter_tabs.rs` 六项回归覆盖 576 组独立 opener/closer 列宽模型，UTF-8 全切点/逐前缀/EOF、失败路径与分类窗口、稳定身份/折叠/失效、原始与去前缀 payload 映射及六字节预算。998 层百万字正文和超窗失败 closer 后各千次追加不增加分类、容器或行内扫描，4096 次开闭符合工作计数上界。

复跑：`nix develop -c cargo nextest run --test math_delimiter_tabs --no-fail-fast`。


### 脚注续行的完整 tab 边界

已有脚注定义先完整匹配确切容器路径，再按物理行四列制表位计量 space/tab；恰好四列落在字节边界才去除缩进。例如顶层 `\ttext`、` \ttext` 续行均去掉四列；引用内 `> \t  text` 去掉四列；跨越四列终点的 `> \t\ttext` 随后由[部分终点子项](#脚注续行的部分-tab-终点)扩展。多余空白与正文保留字面，原始内容接口保持全部源码。完整空白行仍可分段，能消费四列时也去掉对应字节；长空白行已满足缩进即可流式增长。

复用容器前缀游标的物理列和空白末端，匹配失败不改变活动路径；没有新增持久状态、旧正文回扫或历史引用遍历。仅检查当前分类窗口，并将缩进探测计入 `Work.container_bytes`。累计追加 O(n log n)、空间 O(n) 和共享 Budget 保持。部分 tab 容器路径和四列终点由后续两节扩展；lazy 续行、正文 Markdown 解析及脚注区排版仍待办。

`tests/footnote_continuation_tabs.rs` 的五项回归覆盖 1701 组列宽模型、UTF-8 全切点/逐前缀/EOF、空白/失败/重复定义、稳定身份、原始内容和六字节预算映射；991 层百万字续行后千次追加不增加分类、容器及行内扫描，两万旧引用与 4096 次续行验证通知和工作计数。复跑：`nix develop -c cargo nextest run --test footnote_continuation_tabs --test footnote_blocks --no-fail-fast`。


### 脚注续行的部分 tab 容器路径

已有脚注定义可让多层 Item 共用一个 tab，也可在已有 Quote 的可选分隔处只消费一列；完整匹配原容器路径之后，虚拟余量参与四列续行缩进。本子项的四列终点落在字节边界，跨越终点的 tab 由[下一子项](#脚注续行的部分-tab-终点)扩展；正文及多余空白仍按原始字节借用。例如 `- [^n]: first\n\t  second\n` 的 payload 为 `first\nsecond`；`> [^n]: first\n>\t  second\n` 同样续接原定义。源码、BlockId、容器身份、全局首定义优先和首次引用编号保持。

部分 tab 探测是原子的：路径或四列缩进失败时，回到原有完整边界/空白行方言。因此 `>\t text` 仍可按旧引用分隔方式续接，`>\t` 空白行仍保留原 tab；不把失败探测的虚拟余量写入正文。列表中的 `\t\ttext` 跨越四列终点的情况见下一子项；缺失引用、兄弟条目及窗口外路径沿用原分类。短尾等待 LF/CRLF/EOF；有合法缩进的正文/长空白可跨窗口增长。四列终点本身的部分 tab 映射见下一子项；lazy 续行、正文解析和实际排版继续待办。

复用固定大小前缀游标与空白末端；仅已消费部分 tab 的失败探测需要重试原路径，普通失败不增加路径遍历。最多进行两次路径匹配，仅检查当前有界前缀，扫描全部计入容器工作，不增加持久状态、不回扫旧正文或历史引用，累计追加 O(n log n)、空间 O(n) 和共享 Budget 保持。表格分隔行的前缀匹配保持原规则。

`tests/footnote_partial_paths.rs` 五项测试覆盖 1512 组独立列宽/行尾模型、UTF-8 全切点/逐前缀/EOF、空白/失败/窗口/重复定义、原文与六字节预算映射；982 层容器中的百万字续行后千次追加无分类/容器/行内回扫，512 条深层续行及两万旧引用后的 4096 次增长满足工作计数。复跑：`nix develop -c cargo nextest run --test footnote_partial_paths --test footnote_continuation_tabs --test footnote_blocks --test container_definitions --no-fail-fast`。


### 脚注续行的部分 tab 终点

已有脚注完整匹配原容器路径后，消费恰好四列续行缩进；终点可以落在 tab 内，余下 1..3 列映射为空格。例如 `- [^n]: first\n\t\ttext\n` 的 `footnote_payload` 为 `first\n  text`，两个空格共同映射到第二个 tab 的单字节源码区间。`footnote_content` 保留原始包络，后续完整 tab 与其它正文仍按原字节借用；不解析 Markdown 或展开所有 tab。完整空白行同样映射可消费四列后的余量，仍参与既有分段和列表间距规则。

不足四列、路径失配时保留原子回退及原空白方言；例如引用内 `>\t` 空白行继续保留原 tab。短尾等待 LF/CRLF/EOF，完整路径与四列缩进仍须在共享分类窗口内，有效正文可跨窗口增长。首定义优先、稳定 BlockId/容器身份/编号及范围通知保持。

每个获胜定义的续行前缀记录增加一个 `u8` 余量字段（结构体有平台对齐开销），没有另建逐字节映射或复制正文；重复定义不保存记录，同物理行追加不增加记录。消费复用原前缀游标和空白末端，不增加路径或正文扫描；记录数 O(续行数)，累计追加 O(n log n)、空间 O(n)。读取器先返回隐藏前缀，再原子返回单字节 tab 对应的静态空格；每次收取 `max(源码字节, 输出字节)`、1 query、1 object，四字节预算足以持续前进，耗尽不改变位置也不扣费。

`tests/footnote_endpoint_tabs.rs` 五项回归覆盖 1701 组独立列宽模型、UTF-8 全切点/逐前缀/EOF、1..3 列源码映射、零/不足/六字节预算、失败/空白/重复定义与原文读取。982 层容器百万字正文后千次追加无分类/祖先/行内回扫，512 条深层续行和两万旧引用后的 4096 次增长满足计数边界；内部测试另验证记录只按获胜续行增长。lazy/完整容器规则、正文解析和实际排版仍见 TODO。

复跑：`nix develop -c cargo nextest run --test footnote_endpoint_tabs --test footnote_partial_paths --test footnote_continuation_tabs --no-fail-fast`。


## 围栏 info 反斜线解码读取

`Document::code_info(BlockId)` 在 O(1) 时间创建 `CodeInfo` 借用游标；非代码块、无 info 或无效 ID 返回 `None`。`advance(&mut Budget)` 每次给出一个 `CodeInfoChunk { source, text }`：按 [CommonMark 0.31.2 Backslash escapes](https://spec.commonmark.org/0.31.2/#backslash-escapes) 解码反斜线后的 ASCII 标点，映射区间包含原始两个字节，文本借用标点本身。普通字符映射其原始 UTF-8 字节。非标点、Unicode 和单独尾部反斜线保持字面；实体解码见下节；不递归解码、不做语言词选择、额外 trim 或行内解析。`BlockKind::Code { info }` 的原始区间及 `Document::text` 保持原语义。

普通字符/转义每步消耗一次 query、一次 object 和实际消费的源码字节数（转义对为 2，UTF-8 最多 4）；这类步骤预算不足时位置和预算均保持。实体探测及分帧恢复遵守下节扩展契约。到达结尾后免费返回 `Ok(None)`，丢弃游标即可取消；借用期间不能修改文档。游标固定大小，零输出分配，全部读取 O(info 字节数)，不访问正文、历史块或容器祖先，也不增加解析时状态。

此接口只读取分类器已经提交的 info 区间：短 opener 等待 LF/CRLF/EOF，超长 opener 沿用现有分类窗口截断与晚反引号不撤销方言。区间末尾的反斜线不能吞掉窗口外字符。代码正文仍完全字面，info 实体、title 与链接目标读取由后续子项交付。官方例 24 的评分适配器只改为消费本核心接口，不在投影里补做解码；官方输入和期望未改。

`tests/code_info.rs` 六项回归覆盖 3024 组独立 ASCII/连续反斜线/容器模型、UTF-8 全切点和逐字符前缀/EOF、原始区间与借用指针、跨 tile 转义映射、共享六字节预算和零预算恢复。窗口边界、990 层百万字正文及 4096 历史代码块验证读取成本与追加工作计数；超长隐藏 opener 仍有原有注释树更新，未宣称整次调用为常量成本。

```bash
nix develop -c cargo nextest run --workspace --test code_info --test commonmark_backslash_escapes --no-fail-fast
```


## 围栏 info 实体解码读取

`code_info` 在反斜线解码之外支持分号终止的命名实体和数字引用，复用行内实体的固定表及数字规则：命名区分大小写，十进制 1–7 位、十六进制 1–6 位（`x`/`X`）；零、代理码点和超出 Unicode 范围的值输出 U+FFFD，其它合法控制字符保持对应标量。不完整、未知、超长和非法候选保持字面，并继续处理后续转义/实体。`\&amp;` 输出字面 `&amp;`，`&amp;amp;` 也只解码一层；实体产生的换行、tab、反斜线或标点不重新进入 Markdown 解析。代码正文和原始 info 区间保持原语义。

每个未转义 `&` 的探测额外消费一次 query（包含至多 31 字节候选的固定表查找/数字校验），扫描最多 33 个源码字节，逐字节收费并保存继续位置。成功实体输出再收一次 query、一次 object 和至多 6 个 UTF-8 输出字节；失败探测按普通字符费用输出 `&`，其后字符再正常读取。因此字节预算包含探测和输出，不等同于源码长度。每帧至少 6 bytes、1 query、1 object 能持续推进，包括最长实体；探测耗尽时可以尚未输出、`source_position()` 仍指向第一个未输出字节，但已消费预算，下一帧接续而不从头扫描或重复查表。输出原子化，映射包含整个原始实体，双标量实体也只返回一个 chunk。

游标仅增加固定探测状态和 8 字节缓冲，零堆分配，累计读取 O(info 字节数)。`CodeInfoChunk.text` 的借用现在受当前游标步骤约束：普通文字仍借用源码，实体借用游标缓冲；需要跨步骤保留文字时由调用方复制。原始文档借用接口不变。扫描止于已提交的 info 区间，分类窗口末尾缺少分号的候选不能吞掉窗口外后缀；无新增解析持久状态、旧正文或历史块回扫。

`tests/info_entities.rs` 六项回归覆盖独立有效/失败/非递归模型、2125 个命名表项与数字标量模型、UTF-8 全切点/前缀及 EOF、跨 tile 完整映射、零预算/共享预算和六字节恢复、分类窗口截断、980 层百万字正文及 4096 历史块后的追加计数。官方例 34 通过核心接口支持；目标/title 接入后实体章节为 15 支持/1 刻意差异/1 未支持，官方夹具和生成器保持。完整围栏/容器和宽泛标准覆盖继续由 TODO 跟踪；title 与目标读取见后续小节。

```bash
nix develop -c cargo nextest run --workspace --test info_entities --test code_info --test commonmark_entities --no-fail-fast
```


## 链接及图片 title 解码读取

`Document::decoded_link_title(LinkId)` 在 O(1) 时间创建 `LinkTitle` 借用游标；直接链接、图片和已解析引用均可读取。无效 ID、未定义引用和缺失 title 返回 `None`，显式空 title 返回已完成的游标。引用遵守全局首定义优先，源码映射指向获胜定义的 title，可能位于链接使用点之外；新定义到来后创建的游标可读到 title，借用期间文档不能追加。

`advance(&mut Budget)` 返回 `LinkTitleChunk { source, text }`，解码 ASCII 标点反斜线转义和分号终止的命名/数字实体，规则与 `code_info` 相同。完整实体或转义对映射到原始全部字节；普通字符借用源码，实体借用游标固定缓冲，文本只在下一步之前有效。反斜线优先，失败/不完整候选保留字面；不递归、不 trim、不解析行内 Markdown，实体产生的引号、括号、LF 和 tab 只作为输出内容。原始 `link_title` 和 `Reference.title` 借用真实源码包络，包含多行 title 内的容器前缀；解码读取只跳过解析时索引的结构前缀，保留语义缩进，将真实 CRLF 映射为一个 LF。`LinkInfo.title`、引用定义区间、`link_destination` 和 `link_url` 均保持原语义。

每个普通字符/转义消耗一次 query/object 和其源码字节数；实体探测消耗一次 query 并逐字节扣除扫描预算，成功输出另收一次 query/object 和输出字节。探测可能在没有输出、`source_position()` 不变时消耗预算，进度和已解码结果保存在游标内；输出本身原子消费。每帧六字节、一次 query/object 保证可继续，到达结尾免费返回 `Ok(None)`。固定大小零分配游标直接复用 info 解码器，读取总成本 O(title 字节数)，不访问标签、正文、历史使用点或祖先；容器 title 的前缀间隙在解析时按序建立，只保留当前候选或获胜定义的索引；累计追加 O(n log n)、空间 O(n) 保持。

只读取已提交的 title；实际根普通段落的直接 title 正文允许 LF/CRLF，引用定义多行正文见后续小节。4096 字节 title 上限保持；引用定义保留逐物理行分类窗口，行内 title 继续流式消费，不套用定义窗口。容器结构前缀不计入 title/suffix 语法字节。跳过间隙逐段收取 query 和源码字节预算，需有 object 余量；`source_position()` 可在没有输出时前进，0/1/6 字节预算均可恢复，CRLF 输出仍原子消费。目标解码见下节；既有完整显式容器路径内，title 正文的部分 tab 余量已按原普通文本资格支持，虚拟列仅作结构、不新增元数据文本，真实缩进保留，见[容器多行 title 正文](#引用定义容器多行-title-正文)。URL 编码/过滤及完整失败回退继续待办。官方 Backslash escapes / Entity 评分已接入核心目标/title 读取，22–23、32–33 匹配官方期望；原始借用接口保持，详见上方章节评分。

`tests/link_title.rs` 七项回归覆盖 1440 组独立实体/转义/容器/直接与引用/图片/行尾模型、753 组 ASCII 模型、UTF-8 全切点与逐前缀/EOF、晚定义与首定义优先、空/缺失 title、原始 URL、跨 tile 完整映射、零预算和共享六字节恢复，以及长度/分类窗口边界。980 层百万字正文后千次追加与两万旧引用的晚定义验收保持工作计数边界，读取成本只取决于 title。

```sh
nix develop -c cargo nextest run --workspace --test link_title --test code_info --test info_entities --no-fail-fast
```


### 链接及图片目标解码读取

`Document::decoded_link_destination(LinkId)` 返回 `LinkDestination`，按共享 `Budget` 读取已提交的直接/引用链接和图片目标；`LinkDestinationChunk` 将每个字符或完整实体映射到原始源码区间。ASCII 标点转义和分号终止的命名/数字实体沿用 `code_info` 的规则，反斜线优先、不递归、不 trim；未知、超长、未完成实体保留字面。URI/email 自动链接保持字面反斜线和实体文本，email 输出不添加隐含的 `mailto:`。原始 `link_destination`、`link_url`、title 和标签接口保持现有行为。

引用映射到全局首个获胜定义，未解析引用和无效 ID 返回 `None`，显式空目标返回已完成游标。游标借用文档，读取期间不能追加；释放后可重新取得晚定义的目标。保留单物理行、4096 字节目标、引用定义分类窗口及失败候选规则。此接口不执行 URL 百分号编码、解码或协议过滤；解码产生的空白、括号、反斜线、冒号等仅为输出，不重新进入语法，也不承诺是可直接打开的 URL。

构造 O(1)，固定大小零分配状态复用 info 解码器，自动链接通过已有直接目标/标签区间相等关系识别；不增加解析持久状态。普通字符/转义按一次 query/object 和源码字节计费，实体探测按一次 query 和逐字节扫描计费，成功输出再按一次 query/object 及输出字节计费；自动链接每个字面字符按一次 query/object 及源码字节计费。探测可消耗预算而不改变 `source_position()`，输出原子；每个新帧六字节、一次 query/object 可恢复，到结尾免费返回 `Ok(None)`。总读取 O(目标字节数)，不访问历史使用点、标签正文或容器祖先，累计追加 O(n log n)、空间 O(n) 保持。

`tests/link_destination.rs` 七项回归覆盖 1440 组实体/转义/容器/图片/引用/行尾模型、750 组 ASCII 模型、自动链接字面语义和借用指针、裸目标和尖括号目标、非递归和原始 URL、空/缺失目标、首定义优先与晚定义、UTF-8 全切点/逐前缀/EOF、跨 tile 映射、零/不足/共享六字节预算、长度与分类窗口边界。980 层百万字后千次追加和两万旧引用的晚定义计数证明读取及追加成本保持。宽泛标准覆盖和多行目标/title、完整失败回退仍待办；官方夹具与生成器保持原样，章节评分已接入目标/title 解码游标，详见上方评分小节。

```sh
nix develop -c cargo nextest run --workspace --test link_destination --test link_title --test code_info --test info_entities --no-fail-fast
```


## 新引用 tab 分隔后的代码围栏

新开启的显式 Quote 中，`>` 后直接相接的 tab 可先消费一列可选分隔，剩余列再计入代码围栏 opener 的缩进。只有反引号/波浪线围栏在现有共享分类窗口内确认成功、总缩进至多三列时采用此路径。例如 `>\t ~~~\n>     x\n> ~~~\n` 的 opener 缩进为三列，正文保留一格，显示 ` x\n`；`>\t~~~` 的 opener 缩进为两列。物理行四列制表位、原始 info、容器身份、源码偏移和既有 closer 规则保持。

围栏探测失败后继续其它分类；普通正文的 tab 映射见文末子项；ATX 标题已由[后续子项](#新引用-tab-分隔后的-atx-标题)扩展，数学定界见[后续子项](#新引用-tab-分隔后的独立数学块)。已有路径仍使用原先的部分 tab 匹配；短尾等待 LF/CRLF/EOF，超过分类窗口的定界串不在后续追加中追认。原始 `info` 和解码读取接口均保持。完整容器、新引用其它叶块及窗口外识别继续属于 TODO。

只复制固定大小前缀游标；确认前复用已有列宽缓存，只有候选正文以反引号/波浪线开头才调用有界叶块分类，并将扫描计入 `Work.container_bytes` / `Work.classified_bytes`。不增加持久状态、不访问历史块/正文或重走祖先，累计追加 O(n log n)、空间 O(n)、零运行时依赖与共享预算保持。

`tests/quote_fence_tabs.rs` 覆盖 576 组列宽/围栏/行尾模型、正文去缩进、失败/窗口/作用域、UTF-8 全切点和逐前缀、连续源码映射/切片/tile、零预算停步与六字节恢复，以及 989 层百万字正文后千次追加的工作计数。复跑：`nix develop -c cargo nextest run --test quote_fence_tabs --no-fail-fast`。


## 新引用 tab 分隔后的 ATX 标题

新 Quote 的 `>` 后直接相接的 tab 消费一列可选分隔，剩余列按物理行四列制表位计入标题的至多三列缩进。例如 `>\t # title` 确认为引用内一级标题，`>\t  # title` 仍超过缩进额度。复用既有 ATX 的 1–6 个井号、必要空白/行尾、尾部井号裁剪、行内显示、稳定身份及确切容器作用域。

仅在共享分类窗口内确认 ATX 成功后识别标题；七个及以上井号、缺分隔等普通正文的 tab 映射见文末子项，代码/数学块内的字面内容保持原方言；数学定界见[后续子项](#新引用-tab-分隔后的独立数学块)。短尾等待 LF/CRLF/EOF，截断井号串及窗口外必要分隔不在后续追加中追认；已确认的长标题正文和尾缀继续单遍消费。完整容器、新引用其它叶块与窗口外识别仍待办。

现有围栏原子探测扩展为围栏/ATX 共用路径，只对有效缩进后相应首字节分类当前有界后缀，计入容器与分类工作；无新增持久状态、旧正文或祖先回扫。累计追加 O(n log n)、空间 O(n)、零运行时依赖与禁止 unsafe 保持。

`tests/quote_atx_tabs.rs` 六项回归覆盖 1344 组独立列宽/级别/行尾模型，失败及分类窗口边界，UTF-8 全切点/逐前缀，连续源码映射/逐字符切片/tile 和六字节预算，搜索/slug/折叠，989 层百万字标题后千次追加及 4096 组新引用标题的有界计数。

```sh
nix develop -c cargo nextest run --workspace --test quote_atx_tabs --test atx_tabs --test quote_fence_tabs --no-fail-fast
```


## 新引用 tab 分隔后的分隔线

新 Quote 的 `>` 后直接 tab 消费一列可选分隔，余量按物理行四列制表位计入分隔线的至多三列缩进。例如 `>\t ***` 确认为引用内分隔线，`>\t  ***` 超出额度。至少三个同种 `*` / `-` / `_`，允许标记间及行尾 space/tab；分隔线优先于新列表。已有作用域的 Setext 优先级保持，新开的引用没有可提升的旧段落。

只有完整有界规则确认成功才识别分隔线；普通段落的 tab 映射见文末子项，围栏/数学正文与已有路径方言保持，数学定界见[后续子项](#新引用-tab-分隔后的独立数学块)。短尾等待 LF/CRLF/EOF，整行共享 1024 字节分类窗口；恰好到窗口边界但尚无 LF 时已按未完成候选消费，随后 EOF 不追认。完整容器、其它新引用叶块及窗口外识别继续待办。

分隔线保留原始正文显示；`>` 与作为容器分隔消费的 tab 隐藏，后面的实际空白和标记按原字节显示，虚拟余量不合成空格。例如 `>\t ***\n` 显示 ` ***\n`。稳定块/容器身份、连续源码映射、逐字符切片/tile 和共享预算消费保持。

原子探测复用已有单次反向分隔线后缀摘要和缓存列宽，只做常数判断，不增加后缀分类扫描、持久状态或旧正文/历史块/祖先回扫。累计追加 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持。`tests/quote_rule_tabs.rs` 五项回归覆盖 1728 组独立列宽/标记数/行尾模型、优先级/失败/窗口、UTF-8 全切点/逐前缀/EOF、源码映射和六字节预算、989 层百万字后千次追加及 4096 组新引用分隔线。

```sh
nix develop -c cargo nextest run --workspace --test quote_rule_tabs --test rule_tabs --test quote_atx_tabs --test quote_fence_tabs --no-fail-fast
```


## 新引用 tab 分隔后的独立数学块

新 Quote 的 `>` 后直接 tab 消费一列可选分隔，余量按物理四列制表位计入独立数学块 opener 的至多三列缩进。例如 `>\t $$\n> x\n>\t $$\n` 确认为一个已闭合数学块；`>\t  $$` 超出额度。定界行必须恰好两个 `$`，后面只允许 space/tab；数学正文保持字面，opener 缩进不触发正文去缩进。

只在完整有界定界行确认成功后采用部分 tab 前缀；短尾等待 LF/CRLF/EOF，失败候选原子回退。整行共享 1024 字节分类窗口，达到窗口边界但尚无 LF 时的失败候选不会因后续 EOF 追认。已有容器路径、closer、作用域退出和未闭合 EOF 语义复用现有规则；普通段落、引用链接定义与脚注起始行见下方子项，其余新引用叶块规则仍待办。

实现仅将 `$` 加入既有 `quote_leaf` 候选集合，复用固定大小探测游标与数学分类/消费路径。只有直接 tab 后有效缩进的数学候选才分类当前有界后缀，工作计入分类字节；不会对每层祖先重扫正文，无新增持久状态、正文复制或历史块回访。累计追加 O(n log n)、空间 O(n)、零运行时依赖与禁止 unsafe 保持，显示和数学读取继续服从共享 Budget。

`tests/quote_math_tabs.rs` 六项回归覆盖 384 组独立列宽/行尾/后缀模型，完整与部分 tab 祖先路径、失败/窗口，UTF-8 全切点/逐前缀/EOF，身份/折叠/失效通知，原始与正文源码映射、连续显示覆盖/tile 和六字节恢复；989 层百万字正文及超长失败 closer 后千次追加无分类/容器/行内回扫，4096 组数学块计数保持有界。完整容器、其它新引用叶块、窗口外识别及数学实际排版仍见 TODO。

复跑：`nix develop -c cargo nextest run --workspace --test quote_math_tabs --test math_delimiter_tabs --test quote_fence_tabs --test quote_atx_tabs --test quote_rule_tabs --no-fail-fast`。


## 新引用 tab 分隔后的引用链接定义

新 Quote 的 `>` 后直接 tab 消费一列可选分隔，剩余列按物理四列制表位计入定义的至多三列缩进。例如 `>\t [r]: /url 'title'` 隐藏整行并定义全局引用；`>\t  [r]: /url` 超出额度。沿用单物理行目标/title、标签 Unicode 空白归一化与小写、999 字节标签上限及首定义优先规则，保留原始元数据、稳定链接身份和引用范围通知。

只有完整有效定义才识别为定义；语法失败后若分类为普通文本，tab 映射见文末子项；被分类为定义但标签纯空白/过长的候选保留原回退行为；脚注起始行由下方子项扩展。短尾等待 LF/CRLF/EOF，整行共享 1024 字节分类窗口；无 LF 时达到窗口上限的候选不因后续 EOF 追认。已有容器路径、围栏/数学正文与其它叶块方言保持，完整容器、多行引用链接定义及窗口外识别继续待办；脚注起始行见下方子项。

复用 `quote_leaf` 固定大小原子探测，只有有效缩进且 `[` 起始时额外分类当前有界后缀；标签有效性检查不分配、不登记符号，扫描计入容器工作。没有新增持久状态、正文复制、旧引用/祖先回扫，累计追加 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持；显示与元数据读取沿用共享 Budget。

`tests/quote_definition_tabs.rs` 五项回归覆盖 576 组独立列宽/目标/行尾模型，失败/窗口与祖先路径，UTF-8 全切点/逐前缀/EOF，首定义优先与范围通知，连续源码映射/逐字符切片/tile、零预算停步及六字节恢复；990 层新引用定义之前的百万字正文、超长失败定义后千次追加、两万旧引用和 4096 个后续新引用定义验证工作量。

复跑：`nix develop -c cargo nextest run --workspace --test quote_definition_tabs --test definition_tabs --test quote_math_tabs --no-fail-fast`。


## 新引用 tab 分隔后的脚注定义起始行

新 Quote 的 `>` 后直接 tab 消费一列可选分隔，余量按物理四列制表位计入至多三列脚注起始缩进。例如 `>\t [^n]: body` 定义全局脚注并隐藏定义行；`>\t  [^n]: body` 超出额度。标签沿用大小写敏感的 Unicode 字母/数字、下划线和连字符规则，上限 999 字节；首定义（包括空正文）优先，首次引用编号、原始正文和源码映射保持。

仅在现有分类器确认合法 `[^label]:` 时采用部分 tab 前缀；失败候选原子回退，仍可按既有引用链接定义/普通文本规则处理。短尾等待 LF/CRLF/EOF，标签和冒号必须在共享 1024 字节分类窗口内，正文可越过窗口单遍增长。围栏/数学正文、已有路径和四列脚注续行沿用原方言；本子项不改变续行的判定优先级，不增加 lazy 续行或正文 Markdown 解析。

实现只让 `quote_leaf` 接受分类器已验证的 `FootnoteDefinition`，复用固定大小游标和既有脚注消费路径。当前有界后缀（含标签验证）计入分类工作，无新增持久状态、正文复制或旧引用/正文/祖先回扫，累计追加 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持；显示与脚注读取继续消费共享 Budget。

`tests/quote_footnote_tabs.rs` 五项回归覆盖 576 组独立列宽/正文/行尾模型，完整及部分 tab 祖先、失败/标签上限/分类窗口、四列续行，UTF-8 全切点/逐前缀/EOF，稳定身份/编号与首定义优先，原始与正文源码映射、零预算停步和六字节恢复；989 层百万字脚注后千次追加、两万旧引用及 4096 个后续新引用定义检查操作计数。完整容器、窗口外识别及脚注正文解析/实际排版继续见 TODO。

复跑：`nix develop -c cargo nextest run --workspace --test quote_footnote_tabs --test footnote_tabs --test quote_definition_tabs --no-fail-fast`。


## 新引用 tab 分隔后的嵌套引用

新 Quote 的 `>` 后直接 tab 消费一列可选分隔，余量按物理四列制表位计量；余量加后续 space/tab 至多三列，且下一个字节是 `>` 时，允许开启内层 Quote。例如 `>\t > text` 现在形成两层引用，显示 `text`；分隔 tab 和引用标记保持源码可读，在显示中隐藏。内层继续复用已有叶块、容器身份和显式续行规则。

只需下一个引用标记位于共享的 1024 字节分类窗口内，不要求正文完整落在窗口内；短输入仍等待行尾或 EOF 后分类。超过三列时优先沿用缩进代码规则；普通新引用正文及分类为普通文本的失败候选见下方子项；围栏/数学字面正文和窗口外识别规则保持。完整容器及其它首行规则仍见 TODO。

实现复用固定大小前缀游标及缓存空白终点，直接检查下一个字节，不逐层分类剩余后缀、不回扫旧正文或祖先。新增工作计入 `container_bytes`，无新增持久状态或运行时依赖，累计追加 O(n log n)、空间 O(n) 和共享 Budget 契约保持。

`tests/quote_nested_tabs.rs` 五项回归覆盖 1152 组列宽/行尾/叶块模型，失败/分类窗口、完整及部分 tab 祖先，UTF-8 全切点/逐前缀/EOF，稳定父子身份与通知、源码映射、零预算停步和六字节恢复；连续 tab 引用链及 990 层百万字正文后千次追加验证工作量。

## 新引用 tab 分隔后的嵌套列表

新 Quote 的 `>` 后直接 tab 消费一列可选分隔，余量加后续 space/tab 按物理四列制表位合计至多三列时，可开启有效的无序或有序列表。例如 `>\t - item` 形成 Quote → List → Item，列表缩进为三列；后续显式正文 `>      tail` 和兄弟条目 `>    - next` 保持该列表身份。任务、编号、列表首行代码和内层叶块复用已有消费路径；tab 与标记保持原始源码可读，显示中隐藏。

无序标记为 `-`/`+`/`*`，有序标记为至多九位数字加 `.`/`)`；标记后须有空白，或在完整行/EOF 处结束。标记确认仍限共享 1024 字节分类窗口，不完整窗口末端的孤立标记不被追认。失败探测原子回退；完整分隔线优先于列表，超过三列沿用缩进代码规则。普通新引用正文见下方子项；完整容器及窗口外识别仍见 TODO。

实现复用固定大小原子前缀游标与现有列表标记解析，探测仅扫描有界标记及紧随空白；成功后下一次容器循环消费同一标记，每段只增加常数次访问，扫描计入 `container_bytes`。无新增持久状态、旧正文/历史容器/祖先回扫，保持累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享 Budget。

`tests/quote_list_tabs.rs` 五项回归覆盖 4608 组列宽/标记/正文/行尾独立模型，失败/优先级/分类窗口，完整及部分 tab 祖先、UTF-8 全切点/逐前缀/EOF，任务/父子身份、续行/兄弟通知、源码映射、零预算停步与六字节恢复；连续 128 组 Quote/List/Item 以及 991 层百万字正文后千次追加验证有界工作量。

复跑：`nix develop -c cargo nextest run --workspace --test quote_list_tabs --test quote_nested_tabs --test nested_tabs --test sibling_tabs --no-fail-fast`。


## 新引用 tab 分隔后的普通正文

新 Quote 的 `>` 后直接 tab 按物理行四列制表位展开，先消费一列引用分隔；余量与后续空白合计至多三列、且窗口内存在分类为普通文本的正文时，余下列通过显示映射归属于原 tab。例如 `>\ttext` 显示两个空格加 `text`，`>\t text` 显示三个空格加 `text`。这是沿用本库容器正文保留前导空白的方言，不宣称完整 CommonMark 段落缩进兼容；源码读取仍返回原 tab。

分类为普通文本的失败 ATX/围栏/列表标记也走这一路径。空白引用行、被分类为定义但标签无效的候选，以及正文尚未进入共享 1024 字节窗口的输入保留原规则；四列以上仍优先进入已有缩进代码路径。短输入等待行尾/EOF；已分类长行即时消费后续正文，不因 EOF 追认窗口外语法。已有引用续行、围栏/数学正文、稳定父子身份及通知沿用原语义。

普通正文中出现管道时，表头候选保留原 tab 的映射和一个固定大小余量字段；确认后将其作为单元格前导空白隐藏，失败/EOF 时按普通正文消费一次。早管道和晚管道长表头均保持单遍摘要与既有回退机制，不复制正文、不回扫旧行或祖先。新引用探测复用有界分类器，嵌套标记先短路，扫描计入容器/分类工作；显示仍服从共享预算。

`tests/quote_prose_tabs.rs` 六项回归覆盖 1536 组输入（至多三列与空格模型对照，四列以上校验代码语义），UTF-8 全切点/逐前缀/EOF、表头确认/失败/Setext/空白/分类边界，48 组长表头全切点及流式检查点，原始/逐字符/tile 映射、零预算停步与六字节恢复；连续引用链和 989 层百万字正文后千次追加检查工作量。完整容器、其它新段落/首行规则及窗口外识别仍见 TODO。


## 完整已有部分 tab 路径的普通新段落

完整匹配已有容器路径时，space/tab 按物理行每四列对齐，允许多个条目共享一个 tab。若匹配过程消费了部分 tab，且剩余列与后续空白合计至多三列、分类窗口内有普通正文，则空行、标题、围栏或定义之后也可开启普通段落。例如 `- seed\n\n\t*text*\n` 的新段落保留原 Item，显示两个空格加斜体 `text`；此前会退出条目并成为不解析行内语法的代码。原始读取仍返回 tab，显示余量映射到该单字节，稳定身份、列表松紧及通知沿用既有接口。

此子项沿用容器正文保留前导空白的方言，不宣称完整 CommonMark 段落兼容。空白行、未完整匹配的路径和窗口外起始保持原行为，四列缩进继续优先识别代码。完整空白可能关闭缺失的引用，后续正文匹配剩余列表身份。短行等待 LF/CRLF/EOF；长行分类后单遍消费，EOF 不追认窗口外语法。与已有部分路径续行一样，这种新段落不作为表头候选，管道保持普通行内文本；后续有效 Setext 仍可提升段落。部分路径表头与其它容器扩展继续见 TODO。

实现仅放宽原子路径探测中已确认为普通正文的接受条件，复用已有列宽、显示映射及预算游标；无新增持久状态、额外扫描或旧正文/历史容器/祖先回扫。累计追加 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持。

`tests/new_paragraph_tabs.rs` 六项回归覆盖 1536 组独立列宽模型、空行及不同前置块、父子身份/列表松紧、原始/逐字符/tile 映射、零预算停步与六字节恢复、UTF-8 全切点/逐前缀/EOF、表头限制/Setext/失败路径/窗口边界；982 层百万字后千次追加无新增分类或容器扫描，6 组早/晚管道长正文全切点与预算消费通过。复跑：`nix develop -c cargo nextest run --test new_paragraph_tabs`。

## CommonMark Block quotes 章节评分

固定 CommonMark 0.31.2 的完整 Block quotes 章节（228–252，共 25 例），独立报告 **23 支持、2 刻意差异、0 未支持**。官方 Markdown/HTML 逐字保存在 `tests/fixtures/commonmark_block_quotes.rs`；生成器校验固定 `spec.json` 的 SHA-256 和完整章节编号，`--check` 可离线复核，不调用核心解析器生成期望。

- 238、252 属于既定容器正文前导空白保留方言：lazy 续行及显式引用正文的剩余缩进保留可见。
- 239–241、244、249 已支持：核心将有界空引用行发布为 `Blank`，沿用投影既有空白分支；不在输出层删除空正文叶块或 trim 空白。段落边界、作用域及映射见下方空白引用交付。
- 其余 18 例匹配官方输出，涵盖标题、显式/lazy 续行、嵌套、作用域退出及代码/列表/分隔线边界；不据此声称完整引用或容器兼容。

测试投影只消费公开块、容器父关系/列表松紧、标题元数据、显示文本及软硬换行标记；它不接收样例编号、不重新解析 Markdown，也不修正核心差异。未评分的结构和样式显式拒绝。当前差异观察与官方期望分开，行为变化时要求重新审查分类。

`tests/commonmark_block_quotes.rs` 六项回归覆盖 409 个切点、384 个逐字符前缀、原始源码及隐藏前缀的连续映射、逐字符切片、零预算停步与共享六字节预算恢复，并验证投影对层级、标题级别、空白、代码字面内容及换行的敏感性。评分基础设施不改变追加路径；下方空白引用修复保留累计 O(n log n)、空间 O(n)、零运行时依赖及共享预算契约。短样例不替代长输入/深容器的复杂度验收，宽泛标准覆盖继续待办。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_block_quotes --no-fail-fast --success-output immediate`。


## 有界空白引用叶块

完整分类窗口内，去掉显式容器前缀后被现有分类器确认为空白的引用叶块，现在发布为 `BlockKind::Blank`。空引用、仅含 space/tab 的引用和嵌套引用均适用；物理 `BlockId`、原始源码范围和 `ContainerId` 保留。空白行终止逻辑段落，阻止强调、代码 span、链接标签、数学及 Setext 跨行连接；下一条显式正文继续使用同一 Quote，缺失引用前缀的正文退出该作用域。

显示读取仍隐藏容器前缀，剩余空白和换行保留原字节与映射；`Blank` 并不意味着删除源码或 trim 输出。列表间距按作用域确认：引用内部的分段及末尾空白不使外层列表 Loose，引用关闭时截断内部空白传播；引用外部的真实空白分隔仍可确认 Loose。marker-only 列表条目、缺失/部分 tab 路径的匹配、围栏代码/数学正文及窗口外长空白规则保持；无 LF 时达到 1024 字节窗口后，后续 EOF 不追认空白。空白字符集合沿用既有分类器的 ASCII whitespace 方言。

实现只扩展既有空白叶块的类型选择，不增加扫描、持久状态、依赖或 unsafe，也不回访历史正文、引用或祖先。`tests/quote_blanks.rs` 六项回归覆盖 1152 组组合、UTF-8 全切点/逐前缀/EOF、段落/作用域/列表/代码/表头边界、原始与切片映射、零预算停步和六字节恢复；980 层引用中百万字正文后 100 个空白行的工作量按新增前缀计费，后续百万字长行及千次追加不重分类或回扫容器。完整容器规则与宽泛标准覆盖继续见 TODO。


## 有界空列表条目叶块

完整分类窗口内，无任务标记且去掉列表前缀后被现有分类器确认为空白的条目，现在发布为 `BlockKind::Blank`；例如 `-\n`、`009) \r\n` 及引用中的空列表。`BlockId`、物理源码和最内层 `ContainerId` 保留，列表标记和有序编号从该 Item 及父 List 读取。空白叶块不会产生空正文段落，原始读取和显示前缀隐藏沿用既有映射。

空首行本身不使列表 Loose；后续正文、空行及兄弟条目仍按原有规则迟确认间距。任务条目即使正文为空，也保留 `ListItem { task: Some(..), .. }`，供调用方读取任务标记。非空正文、代码/数学正文、部分或缺失路径匹配和窗口外长空白不扩展；达到分类窗口后，后续 LF/EOF 不重新追认空白。空白字符集合沿用分类器的 ASCII whitespace 方言，不宣称完整 CommonMark 列表兼容。

实现仅扩展既有空白分支的类型选择，无新增状态、扫描、依赖、unsafe 或历史正文/祖先回扫；累计解析 O(n log n)、空间 O(n) 及共享预算契约保持。`tests/empty_items.rs` 六项回归覆盖 1728 组标记/编号/缩进/作用域/行尾组合、UTF-8 全切点与逐前缀/EOF、段落/Setext/表头/任务/代码/窗口边界、原始和逐字符映射、零预算停步与六字节恢复，以及 980 层引用中的百万字正文和连续空条目计数。复跑：`nix develop -c cargo nextest run --test empty_items`。完整容器及宽泛标准覆盖继续见 TODO。


## CommonMark Lists 章节评分

固定 [CommonMark 0.31.2 Lists](https://spec.commonmark.org/0.31.2/#lists) 完整章节（301–326，共 26 例），引用尾部空白、围栏空行及兄弟标记缩进修复后独立报告 **22 支持、4 刻意差异、0 未支持**。官方 Markdown/HTML 逐字保存在 `tests/fixtures/commonmark_lists.rs`；离线生成器验证固定 SHA-256 和完整章节成员，不调用核心解析器生成期望。

304 沿用任意合法有序编号可中断段落的方言；308、309 的 HTML 注释按普通文字显示，HTML 不在需求范围内。310、311、313 的不同标记缩进已修复，312 的列表结构也已修复，仅保留 lazy 正文前导空白方言。318 的围栏空行和 320 的引用末尾空白传播已修复，均保留 Tight（见后文）。完整列表匹配和宽泛标准覆盖继续待办。

`tests/commonmark_lists.rs` 的窄投影仅消费公开容器身份、父子关系、编号、列表松紧、叶块类型和显示片段；不合并独立 List、不推导原始 Markdown、不 trim 正文、不修正代码内容。空条目保留 `<li></li>`，隐藏定义不输出正文；有序起始值来自首个 Item，普通段落包装来自确切父 List 的 Tight/Loose。独立观察值与官方期望分开；差异消失或改变时必须复核分类。投影只用于小型测试，不是产品 HTML 渲染器或预算化祖先查询示例。

六项回归覆盖全章评分、730 个合法切点、704 个逐字符前缀及逐前缀 EOF、连续源码和逐字符切片映射、零预算停步与六字节共享预算恢复，并独立检查编号/分组/嵌套松紧/空条目/定义/字面代码和投影拒绝边界。评分投影本身不改变核心追加路径与累计复杂度；完整容器和宽泛标准覆盖仍未完成。

```sh
python3 scripts/generate_commonmark_lists.py /path/to/commonmark-0.31.2.json --check
nix develop -c cargo nextest run --test commonmark_lists
```


## 引用末尾空白的列表间距边界

关闭 Quote 时丢弃该引用内部的尾部空白分隔状态，包括引用中嵌套列表的尾部空白；它不能分隔引用外部的条目或块。因此 `* a\n  > b\n  >\n* c\n` 保持一个 Tight 列表，与官方 Lists 320 一致。引用外部的空行仍按原有规则确认 Loose；未跨 Quote 的嵌套列表尾部空白仍可传给父列表。EOF 只关闭作用域，不凭尾部空白确认 Loose。

实现只在既有 Quote 出栈分支清除一个布尔值，无新增扫描、持久状态、运行时依赖或 unsafe；每个容器仍只关闭一次，累计解析 O(n log n)、空间 O(n) 及共享预算契约保持。源码、物理叶块、容器身份、编号、显示映射和折叠意愿保持；列表松紧变更仅通知实际变 Loose 的列表。

`tests/quote_tail.rs` 六项回归覆盖 640 组标记/引用层数/空白/行尾/后继组合，嵌套与外部空行反例，UTF-8 全切点/逐前缀/EOF，源码映射、零预算停步与六字节恢复，以及 980 层引用、百万字正文和千次长行追加计数。官方 Lists 完整章节继续运行 730 切点、704 前缀及预算验收，夹具和评分投影保持。复跑：`nix develop -c cargo nextest run --test quote_tail --test commonmark_lists`。318 围栏空行已由下节扩展；310–313 列表缩进规则见文末；完整容器继续待办。


## 列表内代码围栏跨省略缩进空行

已开启的反引号/波浪线代码围栏允许完整分类窗口内的空行省略全部或部分列表缩进，前提是所有已有 Quote 标记仍显式匹配。不足列表缩进的 space/tab 全部隐藏，LF/CRLF 继续作为代码换行；足够缩进及多余空白沿用既有列宽、部分 tab 映射和 opener 去缩进规则。保持同一 BlockId、Item、代码字面内容和 Tight/Loose 状态，不把代码空行记为列表分隔。官方 Lists 318 通过，该次修复后完整章节为 19 支持、3 刻意差异、4 未支持，当前评分见上节。

非空缺失路径、缺失引用、兄弟条目和 EOF 沿用原规则；独立数学块、缩进代码及围栏外的空白间距未扩展。只接受 space/tab 空白，不追认分类窗口外的空白结尾。复用既有空白判定、缓存空白终点和引用深度索引；无新增持久状态、依赖、unsafe、旧正文或保留祖先回扫，累计 O(n log n)、空间 O(n) 和共享预算保持。

`tests/fence_blanks.rs` 六项回归覆盖 1440 组标记/嵌套/引用/空白/围栏/行尾列宽模型、失败边界、UTF-8 全切点/逐前缀/EOF、源码映射、零预算停步与六字节恢复、折叠/通知，以及 480 层列表（960 个容器）、千次空行和百万字代码追加计数。复跑：`nix develop -c cargo nextest run --test fence_blanks --test commonmark_lists`。310–313 不同标记缩进见下节；完整容器和宽泛标准覆盖仍见 TODO。


## 同型兄弟列表标记的不同缩进

匹配已有列表的祖先之后，新标记前的缩进按物理四列制表位计算；至多三列且不足旧条目的正文缩进时，同种 bullet / 有序分隔符开启兄弟 Item，保留 List ID 和 Tight/Loose。达到正文缩进时继续匹配原 Item 并按嵌套规则处理，不把内层列表提升为兄弟。每个新 Item 更新自己的正文缩进和 `BlockKind::ListItem.indent`，有序数字可变化；部分 tab 祖先路径遵守同一条件。例如 `- a\n - b\n  - c\n` 保留一个列表，而 `- a\n  - b\n` 开启子列表。

兄弟关闭旧 Item 及其后代，沿用尾部空白传播、一次 Loose 通知、任务和首行代码规则；不同标记、分隔线/Setext、缺失引用和分类窗口继续按原有优先级处理。Lists 310、311、313 通过官方 HTML，312 仅保留 lazy 前导空白的刻意差异。官方夹具、生成器和评分投影保持，完整容器仍未完成。

实现仅替换三个匹配路径中的常数条件，复用已缓存列宽与旧条目正文缩进，并移除不再需要的活动列表标记缩进字段；无新增状态、扫描、依赖、unsafe 或历史正文/祖先回扫。累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。`tests/marker_indent.rs` 六项回归覆盖 1920 组缩进/标记/引用/嵌套/空白/行尾模型，部分 tab、嵌套与四列代码边界，UTF-8 分块/逐前缀/EOF，源码映射、零预算停步和六字节恢复，关闭/松紧通知，以及 480 层列表、千个兄弟和百万字追加计数。复跑：`nix develop -c cargo nextest run --test marker_indent --test commonmark_lists`。


## CommonMark List items 标准样例评分

`tests/commonmark_list_items.rs` 固定 CommonMark 0.31.2 完整 List items 章节（253–300，共 48 例），与 Lists 章节单独评分。官方 Markdown/HTML 来自本地固定 SHA-256 的 `spec.json`，由 `scripts/generate_commonmark_list_items.py` 离线逐字提取；`--check` 校验夹具，来源摘要或章节成员变化会拒绝生成。

| 分类 | 数量 | 范围与原因 |
| --- | --- | --- |
| 支持 | 45 | 253–263、265–288、290、292–300，逐字匹配官方 HTML；278/279 续行缩进、280 空条目空行及 285 空标记中断由下方子项修复 |
| 刻意差异 | 3 | 264/289 沿用空行终止缩进代码；291 的 lazy 正文保留前导空白 |
| 未支持 | 0 | 本章已识别缺口已修复，完整容器和其它章节仍待实现 |

测试投影只读取公开容器父子/编号/松紧、叶块、标题索引和显示片段；支持本章的条目内 ATX/Setext 标题，核对标题级别、BlockId、源码及作用域。投影不接收样例编号、不重解析 Markdown、不 trim、不合并独立列表或补代码空行。官方期望与当前差异观察分别保存，差异变化或转为匹配都会要求复核。未评分的结构、任务、样式和对象显式拒绝；投影只供短样例测试，不是终端排版器，也不承诺窗口级成本。

六项回归覆盖全部 1541 个切点、1493 个逐字符前缀及各前缀 EOF、连续原始/显示/逐字符切片映射、零预算停步与共享六字节预算恢复；独立反例检查编号、标记分组、松紧、嵌套、空条目、隐藏定义、标题级别与代码空白。全章均为 ASCII 短输入，不据此声称验证了完整 Unicode、长输入或深容器复杂度。本单元不改核心解析、追加路径或预算实现，累计 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 的约束保持；完整容器与宽泛标准覆盖仍见 TODO。

复跑：`nix develop -c cargo nextest run --test commonmark_list_items --no-fail-fast --success-output immediate`。离线夹具校验：`python3 scripts/generate_commonmark_list_items.py /path/to/spec.json --check`。


## 有界空列表标记的段落中断规则

在完整分类窗口内，合法列表标记之后只有 space/tab 或直接行尾时，不能中断同一作用域的已有段落。例如 `foo\n*\n` 和 `foo\n1.\n` 继续作为普通段落；显式引用/列表正文、部分 tab 路径及省略前缀的 lazy 续行遵守相同规则。原始源码与容器身份保持，前导空白、CRLF、段落尾空白和行内样式沿用已有显示规则。

兄弟 Item 在匹配已有 List 时先处理，仍可为空；空行/标题等结束段落后，或新 Quote 内仍可创建空列表。Setext 与分隔线优先，`text\n-\n` 仍为标题；任务语法 `[ ]`/`[x]` 是非空候选，任意合法有序编号的非空条目仍可中断段落。只在 LF/EOF 于分类窗口内确认整行后判空，待定输入可继续追加正文；1024 字节窗口先提交时，后续 LF/EOF 不追认空标记。

实现复用标记探测已得到的空白终点，只增加当前根段落的临时选项和常数判断；既有 scoped 段落以稳定身份判断，新 Quote 与兄弟 Item 不误用旧段落状态。lazy 探测仅拒绝非空或未完成窗口内的列表候选，无额外扫描、持久状态、依赖、unsafe 或旧正文/祖先回扫，累计 O(n log n)、空间 O(n) 和共享预算保持。

`tests/empty_interrupt.rs` 六项回归覆盖 1440 组标记/缩进/作用域/尾空白/行尾模型、lazy 与部分 tab、兄弟/新作用域/任务/Setext/表头/窗口反例、UTF-8 全切点/逐前缀/EOF、源码与切片映射、零预算及六字节恢复、身份与关闭/Loose 通知，以及 980 层引用、百万字正文和千次追加计数。CommonMark List items 285 通过原始官方 HTML，该子项交付时全章为 **42 支持、3 刻意差异、3 未支持**，夹具/生成器/投影保持。278/279 的空条目续行缩进由下节修复；280 的跨空行规则由下方子项修复，完整容器与宽泛标准覆盖继续见 TODO。

复跑：`nix develop -c cargo nextest run --test empty_interrupt --test container_lazy --test commonmark_list_items --no-fail-fast`。


## 有界空列表标记的续行缩进归一

完整分类窗口内，合法列表标记之后只有 space/tab 或行尾时，后续正文要求的缩进统一为“标记前缩进 + 标记宽度 + 一列分隔”。例如 `-\n  foo\n` 与 `-   \n  foo\n` 都在同一条目显示 `foo`；有序数字和分隔符都计入标记宽度，tab 按物理四列制表位处理祖先路径。没有实际分隔字节也补一列要求，尾随空白则全部隐藏而不抬高续行门槛。首行仍发布 Blank 并保留完整源码、容器身份、编号；兄弟条目和部分 tab 祖先路径使用相同规则。

非空正文/任务的标记间距、空标记不能中断同作用域段落、Setext/分隔线优先级、列表松紧及关闭通知保持。只有完整有界行才能确认空标记；窗口外尾部不追认，未完成输入沿用既有等待/有界分类行为，EOF 在窗口内确认。空条目后的额外空行终止规则由下节单独交付（CommonMark 280）。

实现仅修正标记解析返回的续行列宽，复用已有空白终点；空行结束后没有剩余正文需要重新计算列宽，无新增持久状态、扫描、依赖或 unsafe，不回访旧正文或祖先。累计追加 O(n log n)、空间 O(n) 及共享预算契约保持。`tests/empty_indent.rs` 六项回归覆盖 1536 组标记/缩进/作用域/尾空白/行尾模型及不足一列反例、代码与部分 tab 映射、任务/通知/窗口边界、UTF-8 全切点/逐前缀/EOF、零预算与六字节恢复，以及 980 层引用、百万字正文和千次追加计数。官方 List items 278/279 通过，该子项交付时全章为 **44 支持、3 刻意差异、1 未支持**；官方夹具、生成器和投影不改，280 的独立观察仅去掉原先错误多留的一列空格。完整容器及宽泛标准覆盖继续见 TODO。

复跑：`nix develop -c cargo nextest run --test empty_indent --test commonmark_list_items --no-fail-fast`。


## 有界空列表条目的空行终止

完整分类窗口内确认的无任务空条目，首行后遇到一个完整有界空行，就不能再吸收第一段正文。例如 `-\n\n  foo\n` 的 `foo` 属于根段落；无序/有序标记、引用和嵌套列表遵守同一规则。空行可以省略列表缩进，但引用标记仍须匹配。空行暂时保持旧条目身份和范围，下一行若按既有缩进规则构成同型兄弟，则保留 List 并一次确认 Loose；否则关闭旧 Item/List，按父作用域重新分类正文、标题或代码，EOF 关闭而不主动确认 Loose。

任务条目即使没有任务正文，也不属于此处的空条目；已有正文、定义或直接子容器的条目仍沿用原空行规则。完整边界内的 LF/CRLF/EOF 与连续空行均可确认；标记首行或空行超出分类窗口时不在后续 LF/EOF 追认。部分 tab 祖先路径原子保留，退出空条目后剩余列用于父作用域叶块分类和源码映射；缺失引用及其它失败路径沿用既有方言。

实现复用活动条目的内容/空白摘要、延迟空行深度和任务属性，三个已有路径探测在消费条目缩进之前检查常数条件；没有新增持久状态、扫描、依赖、unsafe 或旧正文/保留祖先回扫。部分 tab 的结构探测同时处理空条目结束，保持一次提交；原有每层前缀消费和一次出栈计费仍成立，累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。

`tests/empty_blank.rs` 六项回归覆盖 1536 组作用域/标记/缩进/空白组合、父作用域重新分类、同型兄弟和关闭/Loose 通知、任务/子容器/窗口/EOF、部分 tab、UTF-8 全切点/逐前缀/逐前缀 EOF、源码映射、零预算与六字节恢复，以及 980 层引用、千次空行、百万字历史正文和长行追加计数。CommonMark List items 280 匹配原始 HTML，全章为 **45 支持、3 刻意差异、0 未支持**；官方夹具、生成器和评分投影保持。完整容器和宽泛标准覆盖仍见 TODO。

复跑：`nix develop -c cargo nextest run --test empty_blank --test empty_indent --test commonmark_list_items --test list_blanks --no-fail-fast`。


## CommonMark Link reference definitions 标准样例评分

`tests/commonmark_reference_definitions.rs` 固定 [CommonMark 0.31.2 Link reference definitions](https://spec.commonmark.org/0.31.2/#link-reference-definitions) 完整章节 192–218，共 27 例。根作用域多行 title 正文交付后，当前为 **20 支持、7 刻意差异、0 未支持**；这是本章评分，不代表完整引用定义或 CommonMark 兼容性。

| 分类 | 样例 | 当前范围 |
| --- | --- | --- |
| 支持 | 192–196、198、200、202–208、210、214–218 | 根作用域多行 title、跨物理行标签、单行定义、紧接下一行目标及空目标、独立下一行完整 title、标签转义右括号、目标/title 转义解码、晚定义与首定义优先、Unicode 大小写、标题/引用作用域及全局符号 |
| 刻意差异 | 197、199、201、209、211、212 | 无效定义或代码中的定义不生效，但后续未定义引用仍按既有方言隐藏方括号；投影不补回括号 |
| 刻意差异 | 213 | 定义已不再中断段落；未定义引用仍隐藏方括号 |
| 未支持 | 无 | 本章无未支持样例；更完整引用失败回退/容器语法仍待办 |

官方 Markdown/HTML 逐字保存在独立夹具中，保留 CC BY-SA 4.0 来源、版本及 SHA-256；离线生成器拒绝非固定来源及章节成员变化。当前差异观察独立保存，核心输出改变时必须审查分类，不能自动改写官方期望。

评分投影只消费公开块、标题索引、根引用容器、显示片段与链接对象，不接收样例编号、不重解析 Markdown、不 trim、不修复失败定义或未定义引用。非代码块仅省略一个物理末尾 LF（Setext 正文末行的 LF 亦为结构终止符），代码空白及内部换行保持；实体产生的 LF 不省略。目标/title 使用已有核心解码游标和六字节共享预算，再做 HTML/URI 输出编码（decoded 目标中的 U+0020 通用编码为 `%20`）；编码不是核心解析能力，其余不支持字符断言保持。投影只覆盖本章所用结构，其余行内样式、列表、嵌套容器、图片、数学、脚注等由断言拒绝，不作为通用渲染器。

章节回归覆盖 833 个 UTF-8 切点、806 个逐字符前缀及每个前缀的 EOF、原始/显示/切片映射、零预算停步与六字节恢复；元数据同时对照六字节和无限预算，验证晚定义后 LinkId/标签范围保持与首定义不被覆盖。投影敏感性覆盖空白、实体 LF、ATX/Setext、代码字面内容、未定义引用及隐藏定义生成的空引用容器。原评分单元未改变核心解析；194 的定义标签转义修复见下项。紧接下一行目标和 full 引用后缀内转义由独立子项交付；独立下一行完整 title 已交付；跨行标签与根作用域多行 title 见后续交付；容器内多行 title 及更完整引用回退继续见 TODO。

复跑：`nix develop -c cargo nextest run --test commonmark_reference_definitions --no-fail-fast`。夹具校验：`python3 scripts/generate_commonmark_reference_definitions.py /path/to/spec.json --check`。


## 有界引用定义标签内的转义右括号

单物理行且完整分类窗口内的链接定义，现在按反斜线奇偶寻找首个未转义 `]`。例如 `[r\]]: /url` 定义原始标签 `r\]`，匹配 `[r\]]`、`[r\]][]` 及 `![r\]]`；两个反斜线后的 `]` 则关闭标签。匹配键保留原始转义拼写，只沿用既有大小写和空白归一，不将显示相同的标签合并。首定义优先、全局符号、稳定 LinkId、原始目标/title 和源码映射保持。

扫描只替换既有有界查找，加一个局部布尔值；不新增持久状态、解码、历史引用或祖先回扫。未完成行仍等待换行/EOF，失败目标/title、超长标签与分类窗口沿用现有回退。脚注标签和跨物理行定义不在本项范围；full 引用第二个标签的转义由下方独立子项交付。

`tests/escaped_definition_labels.rs` 六项回归覆盖 1536 组转义奇偶/容器/行尾模型、失败与窗口边界、全切点/逐前缀及 EOF、首定义与晚定义通知、源码/切片/六字节预算、980 层百万字与两万旧引用。官方 194 使用原始夹具和原评分投影通过，该子项交付时本章为 14/7/6，根作用域多行 title 交付后当前为 20/7/0；完整引用语法和宽泛兼容性仍未完成。

复跑：`nix develop -c cargo nextest run --test escaped_definition_labels --test commonmark_reference_definitions --test definition_tabs --test quote_definition_tabs --no-fail-fast`。


## 有界 full 引用第二标签的转义括号

`[text][r\]]` 和 `![alt][r\[]` 现在分别匹配原始标签 `r\]` 与 `r\[`。第二标签按反斜线奇偶识别方括号：奇数个反斜线转义随后的括号，偶数个使 `]` 关闭标签、`[` 拒绝候选。规则对应 [CommonMark link label](https://spec.commonmark.org/0.31.2/#link-label) 的转义括号；本项目继续限制单物理行和 999 个源码字节，匹配只做原有大小写/空白归一，不解码标签。

后缀候选仅增加一个布尔值，每个新字节消费一次，闭合时沿用有界符号归一；不重扫旧正文或历史引用。未完成/失败后缀仍保留既有 shortcut 和不透明前缀回退，完整字面回退尚未完成。跨旧行标签的后缀出现反斜线仍取消晚管道表头探测；普通新表头内的引用可正常解析。LinkId、源码映射、首定义优先、晚定义范围通知及共享预算保持。

`tests/full_reference_escapes.rs` 八项回归覆盖 768 组转义奇偶/根与容器/链接与图片/行尾模型，UTF-8 全切点、逐前缀及 EOF，分类窗口外逐字追加的真实候选状态，长度/失败边界，晚定义身份与通知，原始/显示/切片和六字节预算，32 组长表头全切点及普通后缀控制组，以及 980 层百万字正文和两万旧引用工作量。该子项交付时 Link reference definitions 为 14 支持/7 刻意差异/6 未支持，跨物理行标签交付后当前为 19/7/1；本项不是完整 Links/Images 章节评分。

复跑：`nix develop -c cargo nextest run --test full_reference_escapes --test escaped_definition_labels --test commonmark_reference_definitions --no-fail-fast`。


## CommonMark Images 标准样例评分

`tests/commonmark_images.rs` 固定 [CommonMark 0.31.2 Images](https://spec.commonmark.org/0.31.2/#images) 完整章节 572–593，共 22 例：**19 支持、1 刻意差异、2 未支持**。`scripts/generate_commonmark_images.py` 从固定 SHA-256 的官方 `spec.json` 离线逐字提取 Markdown/HTML，`--check` 校验夹具；独立差异观察不参与投影。

投影只读取公开 `LinkInfo.kind/label/source`、显示片段和目标/title 解码游标，按源码包络输出图片或链接，因此空 alt 也保留图片对象。图片 alt 使用已解码的可见标签文字，不输出强调/代码样式标签；保留内部空白和换行，外部普通文字按已有样式输出。对象标签之外的语法必须由核心标为隐藏，投影不重新解析 Markdown、不补括号、不合成外层对象。输出只覆盖本章 ASCII URI，主动拒绝未评分的 URI 编码、块结构及样式组合；这不是通用 HTML 渲染器。

574/575 的嵌套图片/链接会替换外层图片候选，尚不能生成规范要求的外层图片；590 保留未定义引用隐藏方括号的既有方言。其它样例覆盖 inline/full/collapsed/shortcut 图片、大小写归一、强调 alt、空 alt、title、转义感叹号/方括号和跨行分离后缀。上述缺口、完整图片/引用失败回退及宽泛标准覆盖继续见 TODO。

六项回归验证 723 个切点、701 个逐字符前缀及逐前缀 EOF、完整公开快照、源码/切片映射、零预算停步及六字节显示/元数据恢复与无限预算对照；另验证 UTF-8/实体/转义、首定义优先、晚定义身份、空与缺失 title、投影属性敏感性及拒绝范围。本轮只增加评分设施和文档，核心解析、运行时依赖、追加复杂度及预算契约保持。

## CommonMark Inlines 标准样例评分

`tests/commonmark_inlines.rs` 固定 [CommonMark 0.31.2 Inlines](https://spec.commonmark.org/0.31.2/#inlines) 本节唯一的样例 327：**1 支持、0 刻意差异、0 未支持**。这里的 Inlines 不包含其 Code spans、Emphasis、Links 等子章节，不能据此宣称完整行内兼容。`scripts/generate_commonmark_inlines.py` 从固定 SHA-256 的官方 `spec.json` 离线逐字提取 Markdown/HTML，`--check` 验证夹具。

样例验证闭合代码 span 后的普通文字和末尾未配对反引号；只读投影使用公开段落类型和显示样式，不重解析源码、不补标记、不裁剪内部空白，只省略借用源码中的物理段末 LF。其它块、对象和未评分样式主动拒绝。五项测试覆盖官方输入的 9 个切点、8 个逐字符前缀及逐前缀 EOF、完整公开快照、原始/显示/切片映射、零预算停步和六字节恢复；另有 UTF-8、后续闭合、段落边界、实体换行、代码空白及投影敏感性/拒绝边界回归。完整代码失败回退与宽泛标准覆盖继续见 TODO；核心解析、依赖、累计复杂度及预算契约保持。

复跑：`nix develop -c cargo nextest run --test commonmark_inlines --no-fail-fast`。


## CommonMark Autolinks 官方 HTML 评分

新增 `tests/commonmark_autolinks_official.rs` 固定 [CommonMark 0.31.2 Autolinks](https://spec.commonmark.org/0.31.2/#autolinks) 完整章节 594–612 共 19 例，独立报告 **18 支持、1 刻意差异、0 未支持**。旧 `commonmark_autolinks.rs` 夹具的源码/归一化目标回归保留；本次新增的 `fixtures/commonmark_autolinks_official.rs` 逐字保存官方 Markdown 和 HTML，不归一化期望。离线生成器 `scripts/generate_commonmark_autolinks_official.py` 校验来源 SHA-256 与章节成员，支持 `--check`。

投影只读取公开段落、显示片段、`LinkInfo` 和 `LinkTarget`，要求 URI/email 自动链接的目标与标签共享源码区间；email 的 `mailto:` 由 `link_url` 提供。输出层仅做 HTML 转义及本章所需的反斜线/方括号百分号编码，并验证 UTF-8 编码与已有百分号序列不重复编码；不重解析候选、解码地址里的实体或反斜线，不补尖括号。606 仍保留失败候选不重放内部转义的方言，独立观察与官方期望分开；通用 URI 输出、完整失败回退及宽泛标准覆盖继续待办。

六项回归覆盖官方输入的 **421 个切点、402 个逐字符前缀及逐前缀 EOF**、公开结构/显示/链接快照与 pending、连续源码和切片映射、零预算停步、六字节显示消费及显示/目标游标共享预算。URI/email 原始目标与标签逐字符同源，目标读取保持字面，补充 UTF-8、实体字面、空 URI 目标、失败/未完成候选、相邻自动链接、投影变化敏感性和未评分结构拒绝。只新增评分设施，不改变核心解析、依赖或追加复杂度契约。


## CommonMark 行内链接目标与 title 子集评分（2026-11-04）

固定 [CommonMark 0.31.2 Links](https://spec.commonmark.org/0.31.2/#links) 中目标、title 和分隔规则的 **482–511 共 30 例**，现报告 **22 支持、6 刻意差异、2 未支持**（初次评分 21/6/3，510 由根段落后缀组件换行转为支持）。这是 Links 的一个连续子集；512–571 另有三份独立评分，不能据此视为完整 Links 章节通过。官方 Markdown/HTML 原文固定，支持空标签/空目标、尖括号目标中的空格、平衡与转义括号、原始百分号、实体/UTF-8、三种 title 定界及 NBSP 不作为分隔符的规则。

488/490/493/497/508/511 保留既有失败候选/未定义 shortcut 隐藏方括号的方言，493 还保留失败后不重放转义；491/494 含需求范围外的原始 HTML，并伴随回退差异；510 的根段落目标/title 组件分隔换行直接匹配原 HTML，见[后缀组件换行](#根段落行内链接后缀的组件换行)；容器后缀及容器内直接 title 正文换行、完整字面失败回退继续待办。公开语义投影按对象范围输出空链接，读取核心解码元数据，只在输出层做 HTML 与本子集所需的 URI 编码，不重解析、补括号或隐藏差异。源码中的 URL/title 保持原样，不新增通用 URI 输出接口。

新增六项回归覆盖 722 切点、692 逐字符前缀及逐前缀 EOF、源码/逐字符切片映射、显示和目标/title 共用六字节预算、空值与失败/未完成边界、追加后旧对象身份及投影敏感性/拒绝边界。来源、验收与剩余范围见[交付记录](DONE.md#2026-11-04-commonmark-行内链接目标与-title-子集评分交付)。核心解析、运行时依赖、累计 O(n log n)/空间 O(n) 和预算契约保持。


## CommonMark 引用标签匹配子集评分（2026-11-04）

离线评分覆盖 [CommonMark 0.31.2 Links 样例 539–552](https://spec.commonmark.org/0.31.2/#example-539)，经定义标签修复、完整 case folding 及跨行定义标签后现为 **14 例：8 支持、6 刻意差异、0 未支持**（初次评分为 6/6/2）。本项是引用标签的匹配、分隔与定义有效性子集；结合此前 482–511 的目标/title 评分，512–538、553–571 的独立评分也已交付，不能据此宣称完整 Links 兼容。

539–541/544/549–552 匹配官方 HTML，540 由下方完整 Unicode case folding 交付修复；542/543/545/547/548 保留未定义引用隐藏方括号、原始标签拼写与单候选嵌套方言。541 的跨行定义标签已支持；546 的定义标签未转义 `[` 已由下方子项拒绝，无效定义保留正文；失败引用仍隐藏部分括号，归为刻意差异。完整失败回退与多行 title 仍单独推进。

官方 Markdown/HTML 原样固定在 `tests/fixtures/commonmark_reference_labels.rs`，生成器 `scripts/generate_commonmark_reference_labels.py` 校验 SHA-256 和子集成员，支持离线 `--check`。独立观察与官方期望分离；投影只读公开块、对象、显示片段和解码游标，不重解析、补方括号或隐藏未识别定义，范围外结构主动拒绝。核心没有新增状态、扫描、依赖或 unsafe，追加与预算契约保持。

七项测试覆盖 **396 个合法 UTF-8 切点、382 个逐字符前缀及逐前缀 EOF**、源码/切片映射、六字节显示/目标/title 共享预算、未定义到晚定义的稳定对象与归一符号、首定义胜出和目标映射到定义源码。属性变化与失败/拒绝边界另有断言；这些短样例评分不替代既有深层或百万字复杂度验收。运行 `nix develop -c cargo nextest run --test commonmark_reference_labels`；交付与剩余范围见 [DONE](DONE.md#2026-11-04-commonmark-引用标签匹配子集评分交付)。


## 有界引用定义标签的未转义左括号（2026-11-04）

完整分类窗口内的引用定义标签，从初始 `[` 后寻找首个未转义括号；遇到 `[` 拒绝定义并沿用普通正文分类，遇到 `]` 才继续检查冒号及目标/title。反斜线按奇偶转义，`\[`、`\]` 仍合法，保留原始标签匹配、999 字节上限、首定义优先、容器作用域与源码映射。目标/title 中的 `[` 不受此标签检查影响；脚注、代码、数学、多行及窗口外规则保持。

复用已有有界扫描和一个局部布尔值，无新增持久状态、扫描或旧正文/引用/祖先回访；累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。本轮七项回归覆盖 768 组奇偶/作用域/UTF-8/行尾组合、全部切点与逐前缀 EOF、失败回退、稳定对象/晚定义、源码切片/六字节预算、980 层/百万字及两万旧引用。运行 `nix develop -c cargo nextest run --test definition_openers --test commonmark_reference_labels`；完整回退与跨行定义继续待办，见 [交付记录](DONE.md#2026-11-04-有界引用定义标签的未转义左括号交付)。


## CommonMark collapsed 与 shortcut 引用子集评分（2026-11-04）

新增 [CommonMark 0.31.2 Links 样例 553–571](https://spec.commonmark.org/0.31.2/#example-553) 的离线评分，共 **19 例：17 支持、2 刻意差异、0 未支持**。覆盖 collapsed/shortcut 引用、标签内样式、转义 opener、相邻括号和直接/full/collapsed 后缀优先级。553–571 与其它三个子集共同覆盖全部官方 Links 样例评分；完整 Links 兼容性继续保留待办。

553–568、570 匹配官方 HTML；564 已由下方[标签左边界隔离](#已发布标签左边界的强调隔离)修复。569/571 仍选择 full 后缀目标且不将已消费后缀重新解析为下一标签；本次合格完整引用现保留未解析的真实括号与后缀，但仍不匹配官方目标选择，分类保持两例刻意差异。

官方输入/HTML 原样固定在 `tests/fixtures/commonmark_reference_forms.rs`，离线生成器 `scripts/generate_commonmark_reference_forms.py` 固定 SHA-256 与章节成员；独立观察不参与输出。投影只读取公开块、对象范围、显示样式和核心目标/title 解码游标，不补括号、不重新解析 Markdown；输出层只做 HTML/URI 编码，未评分结构和叠加行内样式明确拒绝。

八项测试覆盖 **610 个切点、591 个逐字符前缀及逐前缀 EOF**、结构/显示/对象快照和 pending、源码与切片映射、零预算停步、显示/目标/title 三游标共享六字节预算、晚定义与首定义优先、稳定 LinkId/LinkInfo、输出敏感性与拒绝边界。运行 `nix develop -c cargo nextest run --test commonmark_reference_forms`；[交付记录](DONE.md#2026-11-04-commonmark-collapsed-与-shortcut-引用子集评分交付) 记录来源和全套验收。初次交付仅新增评分设施；后续左边界修复见下方，零运行时依赖、累计追加 O(n log n)、空间 O(n) 和预算契约保持；短样例不替代已有长输入复杂度验收。


## CommonMark 链接标签与嵌套组合子集评分（2026-11-04）

新增 [CommonMark 0.31.2 Links 样例 512–538](https://spec.commonmark.org/0.31.2/#example-512) 的离线评分，共 **27 例：18 支持、4 刻意差异、5 未支持**（标签左右边界修复后）。覆盖标签括号/转义、标签内强调与代码、链接/图片嵌套、强调边界、代码/自动链接优先级及 full 引用组合。结合此前 482–511、539–552、553–571，Links 482–571 的官方样例现已分四个子集评分；这不表示完整 Links 兼容性已经实现。

512/528 的未转义嵌套括号、517/531 的图片内嵌于链接、520 的外层图片均暴露单候选覆盖外层对象的缺口；535 的标签内未配对 opener 越过右边界已由[标签终点交付](#已发布标签的未配对分隔符终点)修复；521/534 的外入内跨界已由下方[标签左边界隔离](#已发布标签左边界的强调隔离)修复。513/523 保留未定义引用隐藏括号的既有方言，523 的外部与标签内部星号现在均保持字面；524/536 保留原始 HTML 字面处理、不提供 HTML 语法屏蔽的方言。多行后缀/定义、完整失败回退及嵌套/强调确认撤销模型继续待办。

官方输入/HTML 固定于 `tests/fixtures/commonmark_link_labels.rs`，离线生成器 `scripts/generate_commonmark_link_labels.py` 校验固定 SHA-256 和连续章节成员。投影只读取公开块、对象范围、显示样式及核心目标/title 解码游标；按覆盖整个对象包络的样式保留外层强调，支持本子集的 em 外包 strong/code，拒绝未评分的歧义/反向样式嵌套。投影不重解析 Markdown、不补括号或创建缺失外层对象；URI 百分号编码仅属于输出层，不改变原始目标。

八项回归覆盖 **814 个合法 UTF-8 切点、787 个逐字符前缀及逐前缀 EOF**、完整快照/pending、源码与切片映射、显示/目标/title 三游标共享六字节预算、晚定义与首定义优先、稳定对象身份，以及输出敏感性/拒绝边界。运行 `nix develop -c cargo nextest run --test commonmark_link_labels`；[交付记录](DONE.md#2026-11-04-commonmark-链接标签与嵌套组合子集评分交付) 记录来源和验收。初次交付只增加评分设施；后续标签左右边界修复见下方。依赖、累计追加 O(n log n)、空间 O(n) 及共享预算契约保持，短样例不替代既有长输入复杂度验收。


## CommonMark 强调与加粗评分

离线固定 [CommonMark 0.31.2 官方数据](https://spec.commonmark.org/0.31.2/spec.json) 的 Emphasis and strong emphasis 全章 350–481，共 132 例。当前八份评分投影合计 **106 支持、0 刻意差异、26 未支持**；支持表示既有投影能按公开语义复现官方 HTML。官方 Markdown/HTML 和投影算法保持，初次评分与后来修复的历史记录留在 DONE。

| 范围 | 支持 | 刻意差异 | 未支持 | 专项测试 |
| --- | ---: | ---: | ---: | --- |
| 350–373 | 22 | 0 | 2 | `commonmark_emphasis_single` |
| 374–389 | 15 | 0 | 1 | `commonmark_emphasis_strong` |
| 390–405 | 16 | 0 | 0 | `commonmark_emphasis_strong_close` |
| 406–416 | 6 | 0 | 5 | `commonmark_emphasis_nesting` |
| 417–430 | 7 | 0 | 7 | `commonmark_emphasis_runs` |
| 431–444 | 13 | 0 | 1 | `commonmark_emphasis_partial` |
| 445–459 | 15 | 0 | 0 | `commonmark_emphasis_unequal` |
| 460–481 | 12 | 0 | 10 | `commonmark_emphasis_final` |

旧投影只读取样式位，不消费公开的同类深度，也不猜测同时开启样式的顺序。408/409、417/426/427、464–466 的部分消耗与深度已经实现，仍因该投影限制计入未支持；413/416/430/467/468 的同时开启顺序被明确拒绝，并另行锁定显示文本及公开深度。独立的单类深度消费者直接匹配 369/373/389/407–409/417/425–427/461/463–466 的原始 HTML，未据此改写八份投影。473/474 的标签左边界已由下方交付修复；419 的链接优先级仍有核心缺口；475–477 的原始 HTML 在需求范围外。完整顺序/身份、一般确认撤销和链接优先级继续按 TODO 推进。

评分回归保留全切点/逐前缀及 EOF、完整与切片源码映射、六字节共享预算、长正文晚闭合身份和投影敏感性/拒绝边界。复跑 `nix develop -c cargo nextest run --test commonmark_emphasis_unequal --test emphasis_runs --test emphasis_depth --no-fail-fast`；对应 `scripts/generate_commonmark_emphasis_*.py /path/to/spec.json --check` 离线核验固定夹具。当前解析模型与验收见下方[部分消耗接口](#强调分隔符部分消耗与三倍数规则)。

## GFM 删除线章节评分

固定 [GFM 0.29-gfm Strikethrough (extension)](https://github.github.com/gfm/#strikethrough-extension-) 完整章节 **491–493 共 3 例**，当前评分为 **3 支持、0 刻意差异、0 未支持**。492 的空行中断和 493 的三连波浪号匹配官方；491 已由下方精确单波浪号交付修复。这不代表完整 GFM 删除线兼容性。

官方 Markdown/HTML 原样固定，`scripts/generate_gfm_strikethrough.py` 从本地规范 HTML 离线生成或核验，使用与表格/任务列表相同的来源 SHA-256。投影仅读取公开段落、显示文本和 `Style::STRIKE`；不扫描波浪号、补标记或重建嵌套深度，其它结构与样式主动拒绝。

六项回归覆盖 **91 个切点、88 个逐字符前缀及逐前缀 EOF**、源码/切片/样式映射、六字节共享预算、转义波浪号、段落边界及超过分类窗口的 Unicode 正文晚闭合块身份。运行 `nix develop -c cargo nextest run --test gfm_strikethrough`；来源核验用 `python3 scripts/generate_gfm_strikethrough.py /path/to/gfm.html --check`。核心及累计追加 O(n log n)、空间 O(n)、零运行时依赖与预算契约保持；详见[交付记录](DONE.md#2026-11-04-gfm-删除线章节评分交付)。

## GFM 裸自动链接扩展章节评分

固定 [GFM 0.29-gfm Autolinks (extension)](https://github.github.com/gfm/#autolinks-extension-) 完整章节 **622–635 共 14 例**，评分为 **0 支持、14 刻意差异、0 未支持**。这里的「刻意差异」对应上文明确的裸 URL 不识别方言：`www.`、裸 HTTP(S)、email、`mailto:` 和 `xmpp:` 保留普通行内文本，不产生链接对象；该分类不表示支持 GFM 裸链接。本章与已评分的尖括号自动链接章节分别报告。

官方 Markdown/HTML 逐字固定，离线生成器验证完整来源 SHA-256 和章节编号。独立观察另存于测试中；投影只消费公开段落与显示文本，执行 HTML 输出转义，只省略借用源码的段落末尾 LF，不扫描地址、合成链接、trim 或改写官方预期。实际链接、容器及其它未评分样式主动拒绝；实体和转义仍由核心显示接口处理。

六项测试覆盖 **945 个切点、931 个逐字符前缀及逐前缀 EOF**、快照/pending、源码/切片/样式映射、六字节共享预算、长 Unicode 裸地址追加的稳定块身份、尖括号链接对照及投影敏感性/拒绝边界。运行 `nix develop -c cargo nextest run --test gfm_autolinks_extension`；离线来源核验用 `python3 scripts/generate_gfm_autolinks_extension.py /path/to/gfm.html --check`。本项交付评分设施，解析核心和复杂度/预算契约保持；完整裸链接规则与宽泛标准兼容性未交付，见[交付记录](DONE.md#2026-11-04-gfm-裸自动链接扩展章节评分交付)。

## 部分 tab 路径的异类标记兄弟列表

祖先路径已完整匹配且消费了部分 tab 时，候选标记缩进至多三列并低于旧 Item 的正文缩进，允许有正文的异类标记开启同父新列表。例如 `- parent\n\t- first\n\t+ second\n` 保留外层 List/Item，关闭内层 `-` List/Item，创建同父的 `+` List/Item；bullet 字符变化、bullet/ordered 切换和 ordered 分隔符变化均走既有结构分类逻辑。同类标记继续复用原 List，达到正文缩进的标记仍按嵌套规则处理。

本节扩展此前“部分 tab 路径兄弟条目”及相关小节中保留的异类标记范围。完整短行上的异类空标记仍保持既有回退；分隔线、缺失引用/不完整路径、围栏或数学正文、Setext/表格优先级和分类窗口方言保持。短输入等待 LF/CRLF/EOF；超过窗口只根据已有前缀分类，不回扫后续长行。原始 tab 字节与标记前缀隐藏，正文和任务标记仍映射到原始源码；祖先、旧块及链接 ID 不变，旧列表及实际后代分别发出关闭通知，新列表不继承旧列表的松紧状态。

实现只扩展两处只读探测的标记准入条件，复用已缓存空白末端、现有关闭/开启路径及预算读取。没有新增持久状态、扫描或对历史正文/保留祖先的回访，每次匹配仍消费正列数或显式前缀，实际关闭按节点数计费，累计追加 O(n log n)、空间 O(n) 保持。`crates/mdstream-core/tests/mixed_sibling_tabs.rs` 六项回归覆盖 15,660 组独立展开列宽模型、空行/后代/优先级、UTF-8 全切点/逐前缀/EOF、稳定身份与通知、tile/切片映射和共享六字节预算，以及千层百万字追加和 4096 次异类列表切换。复跑：`nix develop -c cargo nextest run --test mixed_sibling_tabs --test sibling_tabs`。完整容器、异类空标记及宽泛标准兼容性仍按 TODO 推进。

## 部分 tab 路径的异类空标记

完整祖先路径已经消费部分 tab、候选缩进至多三列且低于旧 Item 正文缩进时，完整短行上的异类空标记也交给既有结构分类器。`- parent\n    - first\n\t+\n` 保留旧 Item 的 lazy 段落，末行显示两个空格和 `+`；中间插入空行后，旧 List/Item 关闭，创建同父的空 `+` List/Item。bullet 字符、bullet/ordered 类型及 ordered 分隔符变化采用同一规则，空标记仍遵守既有段落中断规则。

lazy 回退保留已消费 tab 的剩余列并映射到原始单字节 tab，同时保留显式匹配的引用块类型；未消费的正文 tab 仍保持原文。空标记须由 LF/CRLF/EOF 在分类窗口内确认，长空白后缀继续采用既有窗口方言。缺失引用、不完整祖先路径、分隔线、围栏/数学正文及其它优先级保持。

两处只读探测去掉非空限制，复用局部列宽和引用深度；无新增扫描、持久状态、历史正文或保留祖先回访，累计追加 O(n log n)、空间 O(n) 和共享预算契约保持。`nix develop -c cargo nextest run --test empty_mixed_sibling_tabs --test mixed_sibling_tabs --test sibling_tabs` 覆盖 25,056 组列宽模型、分块/EOF、稳定身份与关闭通知、源码切片/tile 和六字节共享预算，以及千层百万字和 4096 次空列表切换。完整容器与宽泛标准兼容性仍待办，详见[交付记录](DONE.md#2026-11-04-部分-tab-路径异类空标记交付)。

## 完整部分 tab 路径的表头与表格正文行

已有容器路径完整匹配后，部分消费的 tab 可像对应空格路径一样开启表头候选，或续接同一作用域的表格正文行。例如 `- seed\n\n\ta|b\n  -|-\n\tc|d\n` 在原 Item 中生成一张两行表格；确认后单元格裁掉前导空白，失败或 EOF 则沿用普通正文回退，tab 剩余列仍映射到原始单字节。早管道和可隔离的晚管道长表头复用既有候选、行内状态及对象身份处理。

这里只放开已成功分类为正文的完整路径。分隔行自身的部分 tab 准入、待定表头下一行的探测、缺失祖先/lazy、四列代码缩进及分类窗口边界保持原规则；例如 `- a|b\n\t-|-\n` 仍不成表。失败分隔行若自身成为另一个待定表头，下一条部分 tab 行仍可能按原始 tab 回退。先前文档中“部分 tab 表头一律排除”的限制由此收窄，完整容器及更广泛表格兼容性仍见 TODO。

实现只去掉正文入口的一处排除条件，无新增持久状态或扫描；原有每行摘要、一次单元格消费/失败恢复和源映射承担后续工作，不回访历史正文、行或祖先，保持累计追加 O(n log n)、空间 O(n) 与共享预算。七项新回归覆盖 1,536 组列宽模型、短/长表头的分块及 EOF、继承数学/链接/代码/强调候选、源码切片/tile 与六字节共享预算、980 层引用加列表的百万字追加及 4,096 条历史表格行。复跑：`nix develop -c cargo nextest run --test partial_table_tabs --test new_paragraph_tabs`。交付记录见 [DONE](DONE.md#2026-11-04-完整部分-tab-路径表头与表格正文行交付)。


完整部分 tab 容器路径现也允许待定表头后的非空普通文本：例如 `- a|b\n\t-|-\n` 可确认同作用域表格；列数不符或非分隔文本按原虚拟空格映射回退，含管道的失败行仍可成为下一表头。仅扩展已分类为普通文本的路径；四列代码、缺失祖先、空行、新容器标记分隔行（如 `- | -`）及超出分类窗口的分隔行探测保持原有边界。方言和验证见 [交付记录](DONE.md#2026-11-04-待定表头后部分-tab-普通文本交付)。


完整部分 tab 路径的短分隔行现也包含列表标记形态：`- a|b\n\t- | -\n` 在原 Item 中确认表格，分隔行优先于开启嵌套列表，缩进和分隔语法隐藏。此前“新容器标记分隔行保留”的限制由本项收窄；仅在表头确切作用域、完整分类窗口且剩余缩进至多三列时生效，列数不符/非法内容仍按现有结构规则回退，兄弟条目、缺失祖先、四列代码及长分隔行边界保持。六项回归包含 1,536 组列宽模型、分块/EOF、身份/映射/共享六字节预算及深层百万字追加；见[交付记录](DONE.md#2026-11-04-部分-tab-路径标记形态短表格分隔行交付)。


### 完整部分 tab 路径的长表格分隔行

已有表头下一行超出分类窗口时，原有字节边界路径探测失败后可尝试完整部分 tab 路径：必须匹配全部既有祖先及表头的确切容器，剩余缩进至多三列。`- a|b\n\t` 后接很长的 `---…|-` 或 `- | ---…` 可确认同一表格；新增路径的四列缩进、缺失引用、兄弟条目及新嵌套容器不予确认。原来已接受的字节边界长分隔行空白方言保持，不能据此推定短行或其它叶块规则。

复用现有长分隔行候选，不复制正文或增加持久状态；一次额外的只读路径匹配只消费当前有界物理前缀，长尾逐字节计入 `table_candidate_bytes`。成功隐藏完整物理分隔行并保留表头/容器身份；迟失败或不完整 EOF 只重新分类原有界前缀，延迟尾部消费一次、不重新启动同一行候选，恢复原 tab 剩余列映射。累计追加 O(n log n)、空间 O(n) 与共享显示预算不变，不承诺单次确认/失败/EOF 硬时延。

`tests/long_delimiter_tabs.rs` 六项回归覆盖 960 组列宽模型、UTF-8 全切点/逐前缀及窗口附近与尾部 EOF、成功/迟失败/列数不符、源码切片/tile/六字节共享预算，以及 980 层引用加列表的百万字候选增长。复跑：`nix develop -c cargo nextest run --workspace --test long_delimiter_tabs --test long_delimiters --no-fail-fast`。完整容器/表格兼容性、缺失祖先/lazy、一般空白与其它分类窗口限制继续见 TODO。


### 完整部分 tab 路径的短空白行

完整已有容器路径消费部分 tab 后，分类窗口内的完整空白行现在沿用同列空格路径的空行语义。例如 `- > a\n\t> \n\t> b\n` 中间行为原引用作用域的 `Blank`，不再作为 lazy 普通正文。引用内空行不直接使外层列表变松；Item 直接空行在后续同作用域正文出现时按原规则使列表变松。待定表头遇到空行回退，后续表头可以独立确认；块、容器和链接身份保持。

仅放开既有只读完整路径探测中的 `Blank` 分类，无新增状态、扫描或历史回访，保留累计追加 O(n log n)、空间 O(n) 和共享显示预算。剩余 tab 列保留在原单字节映射中；没有完整祖先路径、尚未收尾的空白、超出分类窗口的空白及活动围栏仍走既有规则。本项不表示完整容器/空白兼容性已完成。七项回归覆盖 1,728 组列宽模型、分块/逐前缀/EOF、身份/通知/表格回退、切片/tile/六字节共享预算、970 层引用外加列表和内层引用的百万字追加及 4,096 轮空行历史。复跑：`nix develop -c cargo nextest run --test partial_blank_tabs`；见 [交付记录](DONE.md#2026-11-04-完整部分-tab-路径短空白行交付)。


### 新引用 tab 分隔后的短空白行

首次创建引用时，紧邻 `>` 的 tab 现在也可用于分类窗口内完整空白行：按物理四列 tab stop 消费一列分隔，剩余列映射到同一个原始 tab 字节。例如 `>\t\n` 显示两个空格和换行，块仍为该 Quote 中的 `Blank`；额外尾部 space/tab 保留原文，不把空白分类当作空白删除。LF、CRLF 和 EOF 均可确认；部分 tab 余量也适用于嵌套新引用和列表内新引用。

这里只扩展新引用短空白准入；尚未收尾的短行仍留在 pending，分类窗口外长空白、缺失祖先、活动围栏与其它容器规则保持原方言。实现复用已缓存的空白末端和显示映射，不新增扫描或持久状态，累计追加 O(n log n)、空间 O(n) 及共享预算保持。`nix develop -c cargo nextest run --test new_quote_blank_tabs` 覆盖列宽模型、分块/EOF、身份/松紧/表头边界、切片/tile/共享六字节预算及深层百万字追加；详见[交付记录](DONE.md#2026-11-04-新引用-tab-分隔短空白行交付)。


### 有序列表的段落中断编号

同一段落作用域内，新有序列表现在只有数值为 1 的标记能中断正文（`1.`、`001)` 等）；`text\n14. body` 保留为同一段落，编号和分隔符按普通行内正文显示。规则适用于顶层、完整显式引用/条目路径和已有部分 tab 路径；沿用现有虚拟空格与源码映射。空标记仍不能中断段落。

同类兄弟条目、空行/其它块结束段落后、新引用或新条目作用域中的首个列表仍允许任意合法编号。省略祖先前缀的 lazy 路径仍保留原有方言，不把本项外推为完整 CommonMark 容器支持。只利用既有标记解析结果和确切作用域，不新增扫描、持久状态或历史回访；长行在有界前缀确认编号后即可流式消费。

本项取代此前“任意编号均可中断同作用域段落”的说明。[CommonMark 0.31.2 示例 304](https://spec.commonmark.org/0.31.2/#example-304) 已匹配官方期望，Lists 301–326 的当前评分更新为 **23 支持、3 刻意差异、0 未支持**；308/309 的原始 HTML 与 312 的 lazy 前导空白差异保留。复跑：`nix develop -c cargo nextest run --test ordered_interrupt --test commonmark_lists`。详见[交付记录](DONE.md#2026-11-04-有序列表段落中断编号交付)。


### 省略祖先前缀时的有序标记段落续行

有序列表的段落中断编号规则现扩展到 lazy 续行：例如 `> text\n14. body`、`> > text\n> 2) body` 和 `> - text\n14. body` 都保留原最内层段落及容器身份，非 1 编号作为普通正文参与行内解析。仅对已有非空段落、首次祖先匹配失败后原本允许 lazy 的路径生效；完整路径继续使用前一项规则。

数值 1 的非空标记、无序标记和其它块中断仍按原规则关闭未匹配路径；同类兄弟已在路径匹配阶段处理；异类兄弟候选仍切换列表，两者都可使用任意合法编号。例如 `- text\n14. body` 仍开启顶层有序列表。空行、标题、代码、定义或已确认表格之后不获得 lazy 资格。未完整的 tab 路径保留既有原始 tab 映射；空标记、前导零、最多九位编号及分类窗口规则保持。本项收窄前一节的 lazy 保留范围，不代表完整容器兼容性。

实现仅在 `Prefix::lazy_text` 既有标记结果上比较编号，无新增扫描、持久状态或历史回访；当前有界前缀判定后直接使用原 lazy 续行、通知与显示预算路径。累计追加 O(n log n)、空间 O(n) 保持，不承诺单次 append/finish 硬时延。复跑：`nix develop -c cargo nextest run --test lazy_ordered_interrupt --test ordered_interrupt --test container_lazy`；验收及剩余边界见[交付记录](DONE.md#2026-11-04-lazy-有序标记段落续行交付)。


### 省略引用前缀时的部分 tab 正文映射

已有非空段落的续行在匹配外层前缀时拆分了 tab，随后首次因缺失引用标记而失配，现在可保留已消费的前缀和剩余虚拟空格。例如 `> > first\n>\t14. body` 显示 `first\n  14. body`，`- > first\n\ttext` 显示 `first\n  text`；原最内层容器、块和行内对象身份保持，剩余空格映射到原始 tab 单字节。

准入复用已有 lazy 正文判定，短行等待换行或 EOF，长行在有界前缀后继续流式消费。缺失列表缩进、无活动段落、待定表头、空行和不能通过 lazy 判定的块候选继续沿用原规则；lazy 行不创建表头或新叶块。此前“不完整 tab 路径保留原始 tab”的范围由此收窄，完整容器/表格兼容性和分类窗口扩展仍在 TODO。

只读探测在首次缺失引用处停止，检查当前有界正文并复用已有显示映射，不遍历省略的祖先，不新增持久状态或回访旧正文；累计追加 O(n log n)、空间 O(n) 及共享预算保持。复跑：`nix develop -c cargo nextest run --test lazy_quote_tabs --test lazy_ordered_interrupt --test prose_tabs`；见[交付记录](DONE.md#2026-11-04-缺失引用前缀的部分-tab-正文映射交付)。


### 省略列表缩进时的部分 tab 正文映射

已有非空段落的续行在匹配祖先时拆分了 tab，随后因列表正文缩进不足而失配，现在也可保留已消费的前缀和剩余虚拟空格。例如 `-  - first\n\ttext` 显示 `first\n text`，`>  - first\n>\ttext` 显示 `first\n  text`，原最内层容器及行内对象身份保持。多个祖先恰好用完同一 tab 时，该字节只作为隐藏前缀；未消费的后续 tab 仍保留原文。剩余空格映射到原 tab 单字节，源码读取不变。

同类和异类兄弟标记继续优先进入结构分类器，包括非 1 有序编号。待定表头、无活动段落、空白及不能通过现有 lazy 判定的候选仍走原分类器；本项不赋予它们新的准入，也不改变原分类器可能保留 lazy 身份的方言。lazy 正文不启动表格。此前缺失列表缩进的映射限制由此收窄，完整容器/表格与分类窗口扩展继续待办。

只读探测在首次失配处复用当前有界正文的 lazy 判定，不遍历省略的祖先、不新增持久状态、不回访旧正文；累计追加 O(n log n)、空间 O(n) 和共享预算保持。复跑：`nix develop -c cargo nextest run --test lazy_item_tabs --test lazy_quote_tabs --test prose_tabs`；见[交付记录](DONE.md#2026-11-04-缺失列表缩进的部分-tab-正文映射交付)。


### 待定表头失败后的部分 tab lazy 映射

待定表头之后的续行若消费部分 tab 后缺失引用标记或列表正文缩进，且符合既有 lazy 正文条件，现在也保留已消费前缀和剩余虚拟空格。例如 `- > a|b\n\ttext` 显示 `a|b\n  text`，`-  - a|b\n\ttext` 显示 `a|b\n text`，剩余空格映射到原 tab 单字节；候选表头回退为正文，原容器、块及行内对象身份保持。

lazy 行不能确认旧表头，也不能成为新表头；同类/异类兄弟标记仍优先进入结构分类器。完整路径的表格确认、四列代码、空白、非段落及其它不符合 lazy 条件的候选保持既有规则。前述待定表头的部分 tab 排除范围由此收窄，完整容器/表格兼容性与分类窗口扩展继续待办。

实现复用已保存的同作用域段落/表头资格与有界 lazy 判定，无新增状态、扫描或历史回访，累计追加 O(n log n)、空间 O(n) 和共享预算保持。复跑：`nix develop -c cargo nextest run --test header_lazy_tabs --test pending_table_tabs --test lazy_item_tabs --test lazy_quote_tabs`；见[交付记录](DONE.md#2026-11-04-待定表头失败后的部分-tab-lazy-映射交付)。


## 引用链接定义不再中断同作用域段落

已有顶层段落或完整显式容器路径上的连续正文（含待定表头）之后，合法单行 `[label]: target` 不再直接建立定义，而是交给普通行内解析。例如 `Foo\n[bar]: /baz\n` 保持一个段落，不定义 `bar`；当前未定义引用仍隐藏方括号，因此显示为 `Foo\nbar: /baz\n`，完整字面回退仍待实现。完整部分 tab 路径同样适用，剩余虚拟空格继续映射到原 tab 字节。

首行、空行之后、标题/其它非段落块之后、新引用/列表/兄弟条目的首行仍可定义；连续定义和已确认表格的中断保持。缺失祖先前缀仍按既有 lazy/作用域分类，本项不扩展该路径。脚注定义保持独立的中断方言。短尾等待 LF/CRLF/EOF，分类窗口外候选不追认；失败定义继续使用既有行内回退，不回放旧正文。

实现复用 Setext 分类已有的同作用域段落资格，仅增加常量布尔判断，无新增扫描、持久状态、依赖、unsafe 或历史正文/引用/祖先回访。原段落、块、容器与链接身份保持，真正的晚定义仍按符号通知；段落折叠随这类正文续行增长，并在后续空行封闭。累计追加 O(n log n)、空间 O(n) 和共享预算契约保持。

`tests/definition_interrupt.rs` 六项回归覆盖 576 组作用域/空行/缩进/行尾模型、216 个 UTF-8 切点、206 个逐字符前缀及 EOF、待定表头回退、源码切片/tile/六字节共享预算、晚定义通知、970 层引用、百万字追加与 4096 行历史。旧定义测试补齐必要的段落边界，连续正文断言按新行为更新，原 metadata、映射与工作量约束保留。

CommonMark Link reference definitions 213 仍为刻意差异，剩余原因改为未定义引用隐藏括号；217 已由独立下一行完整 title 子项支持，后续 `baz` 定义也正常识别。独立观察已审查，官方 Markdown/HTML、生成器与评分投影保持；该子项交付时章节统计为 **14 支持、7 刻意差异、6 未支持**，当前为 **20/7/0**。本节收窄此前“链接定义可中断普通段落/待定表头”的描述，容器内多行 title、缺失前缀、完整字面回退及完整容器/标准兼容性继续见 TODO。

复跑：`nix develop -c cargo nextest run --test definition_interrupt --test commonmark_reference_definitions --test paragraph_folding --no-fail-fast`。


## 缺失祖先前缀的引用定义候选续接正文

已有非空容器段落省略部分或全部祖先前缀时，有界单行引用定义候选现在也遵守既有 lazy 续行规则。例如 `> seed\n[r]: /url\n` 保留原 Quote，第二行作为正文，不定义 `r`；未定义引用仍按当前方言隐藏方括号。待定表头回退为正文，lazy 行仍不确认或创建表格。部分 tab 路径沿用原字节的虚拟列映射，块、容器及行内对象身份保持。

首行、空行后、非段落块后和新作用域仍可建立定义；已确认表格的中断与脚注定义方言保持。实现只放行既有有界 lazy 判定中的 Definition 分类结果，不增加扫描或持久状态，不访问旧正文或缺失祖先；累计追加 O(n log n)、空间 O(n) 及共享预算保持。此前缺失前缀定义中断的保留范围由本项收窄，多行定义、完整字面回退、分类窗口和完整容器/标准兼容性继续待办。

复跑：`nix develop -c cargo nextest run --test definition_interrupt --test container_lazy --test header_lazy_tabs --test lazy_item_tabs --test lazy_quote_tabs --test commonmark_reference_definitions --no-fail-fast`；模型、映射、预算、深嵌套和历史工作计数见[交付记录](DONE.md#2026-11-10-缺失祖先lazy-的引用定义段落边界交付)。


## 引用标签的完整 Unicode case folding（2026-11-10）

链接、图片及引用定义的匹配键现在使用固定 **Unicode 17.0.0 默认非 Turkic 完整 case folding**，采用官方 `CaseFolding.txt` 的全部 C/F 映射，排除 S/T，未列出的标量保持原值。例如 `ẞ`、`ß`、`SS` 共用 `ss`；`ﬃ` 匹配 `ffi`，`ΐ` 展开为 `ι\u{308}\u{301}`，Cherokee 键可能是大写。`I` 匹配 `i`，`İ` 仅匹配 `i\u{307}`，`ı` 独立。仍按原有 `char::is_whitespace` 折叠 Unicode 空白、去掉标签两端空白；此次固定版本针对 case folding，不改变原空白分类契约。脚注符号表保持独立且区分大小写。

999 字节上限在 folding 前按实际原始标签检查，含原始 CRLF/标记/转义，不含已去除的容器前缀；展开后的键可超过 999 字节。`Reference.label` 是归一化匹配键，`LinkInfo` 的标签/对象区间及源码仍保留原始拼写、字节位置和显示映射。全局首定义优先、稳定 SymbolId/LinkId、晚定义的单符号与首次使用范围通知、显示/目标/title 共享预算保持。不解码标签转义或实体，不增加 NFKC、重音或其它规范化；例如 `É` 与 `e\u{301}`、全角 `Ａ` 与 ASCII `a` 仍不同。完整 folding 自带的连字展开不代表执行了 NFKC。

官方原始数据和完整 Unicode License V3 位于 [data/unicode](data/unicode/README.md)，来源、SHA-256、生成步骤及证明界限随仓库保存。`python3 scripts/generate_case_folding.py --check` 离线验证数据/许可摘要和静态表；生成器只读本地文件，构建和测试不会生成或下载数据，也不使用宿主 Unicode 大小写数据库。每标量最多输出 3 个标量、6 个 UTF-8 字节，归一键总字节数不超过原标签的 3 倍。固定表检索每个非空白标量至多 12 次比较，标签处理 O(label bytes)，有界键仍使用 BTreeMap；整体累计 O(n log n)、空间 O(n)，晚定义不访问历史引用。`Work.label_scalars`、`label_fold_steps`、`label_key_bytes` 分别记录实际标量访问、固定表比较和键输出字节，保留原 `label_bytes` 的原始字节语义；零运行时依赖、禁止 unsafe。

CommonMark 0.31.2 例 540 现匹配官方 HTML，539–552 子集更新为 **7 支持、6 刻意差异、1 未支持**；仅 540 改类，其余逐项观察保持，官方 Markdown/HTML、生成器及评分投影未改。两份评分中的 `[SS]`/`[ẞ]` 辅助期望及晚定义符号/目标断言同步更新。本节取代此前引用匹配“Unicode 小写/转小写、不做完整 case folding”及 540 差异/6–7–1 汇总的当前语义描述；旧交付记录保留历史。541 跨行定义标签见后续交付；542/543/545/546/547/548 字面回退/嵌套及完整容器等范围继续见 TODO。

新增 11 项回归复用现有 snapshot、元数据和共享预算设施：对照全部 1,112,064 个有效 Unicode 标量与官方独立模型；含全部 1585 映射的全文模型验证 6360 个引用对象及同键首定义；覆盖非 Turkic/不同规范化形式、原始 998/999/1000 字节、UTF-8 任意切点/逐字节及逐标量前缀/EOF、原始映射和六字节共享预算、稳定身份/晚定义通知，以及 980 层、百万字、两万历史引用的工作计数。复跑：`nix develop -c cargo nextest run --test reference_case_folding --test commonmark_reference_labels --test commonmark_reference_forms --lib --no-fail-fast`。


## 精确单波浪号删除线（2026-11-10）

当前行内方言支持 `~strike~` 与 `~~strike~~`，二者都发布 `Style::STRIKE`。run 在下一个不同字节或序列 EOF 才定长；只配对同长度的完整 run，单/双不混配，三连及更长 run 不成为删除线定界符，也不部分消费。未闭合 opener 在原作用域终结时丢弃，保留字面显示，不回放正文。转义与既有 ASCII flanking 保持：后字节非 ASCII 空白可开、前字节非 ASCII 空白可闭，能闭合时优先闭合且同一 run 不重新开启；不声称 Unicode flanking。

例如 `~a ~~b~~ c~` 和反向组合可配对，但公开位不区分删除线嵌套深度；`~a ~~b~ c~~` 在单 run 闭合时弹掉中间双 opener，余下 `~~` 字面。代码/数学候选内部继续不解析波浪号，未完成候选沿用原 opaque 失败规则；标签内部及外部已确认的配对保持；已发布标签的未配对 opener 现在在标签终点清退，见下方[标签终点](#已发布标签的未配对分隔符终点)，外入内跨界与目标/title 候选优先级沿用现有方言。连续正文仍以同一确切容器身份续接，不跨空行、实际块/容器切换或表格单元格。长表头继承仍只隔离 close-only 且不弹出本行 opener 的配对；成功丢弃跨行对、失败/EOF 提交一次，不可隔离的交叉继续取消探测。

核心仅增加一种固定 opener 索引，沿用原栈和区间标记。每字节单遍、每个 opener 至多入栈/出栈各一次，隔离 pair 数量由已消费的旧 opener 约束，表头确认只对当前 surviving opener 一次重建前驱索引；不回扫正文、不克隆或重放栈。累计解析 O(n log n)、空间 O(n)、公共查询共享预算、稳定对象与原始源码映射保持；零运行时依赖、禁止 unsafe。

GFM 0.29-gfm 例 491 现匹配官方 HTML，491–493 为 **3 支持、0 刻意差异、0 未支持**。官方 Markdown/HTML 和评分投影保持，离线固定来源与独立读取器核验通过。本节收窄前文仅列举 `~~` 的当前语义及旧 491 差异统计，历史交付记录保留；完整 Unicode flanking、同类嵌套深度、完整强调/部分 run 消耗、链接作用域及宽泛 GFM/容器兼容性继续见 TODO，不扩展渲染或 UI 阶段。


## 强调/加粗的 Unicode 侧界

星号/下划线 run 按相邻语义标量判断侧界：[CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/#emphasis-and-strong-emphasis) 的空白为 Zs + TAB/LF/FF/CR，标点为 Unicode P/S。左侧界要求后继非空白，后继为标点时前驱须为空白或标点；右侧界对称。星号按左右侧界开闭，下划线额外排除词内边界。因此 `a*"foo"*`、`*(*foo)`、`中_文_尾`、`a*£*b` 保持字面，`「_中_」` 显示为 `「中」`。实体不替换用于侧界的原始语义标量，部分 tab 虚拟空格按空白参与。

固定 Unicode 17.0.0 分类表有 357 段，`Work.flanking_steps` 计入每侧至多 12 次分类/比较；每个 run 只在后继或 EOF 到来时分类，无旧正文/祖先回扫。单/双波浪号保持独立的 ASCII 空白及精确长度方言。官方分类数据、摘要和 Unicode License V3 随仓库保存，`python3 scripts/generate_unicode_categories.py --check` 可离线核验。ASCII 与 Unicode 的历史交付分别见 [ASCII 记录](DONE.md#2026-11-10-强调加粗-ascii-侧界与词内下划线交付)和 [Unicode 记录](DONE.md#2026-11-10-强调加粗-unicode-侧界交付)。

## 已确认强调/加粗的同类嵌套深度（2026-11-10）

`RenderRun` 与 `DisplayRun` 新增 `emphasis_depth: usize`、`strong_depth: usize`，表示覆盖该源码片段的已确认强调/加粗配对次数。例如 `_foo _bar_ baz_` 的 `foo`、`bar`、`baz` 深度为 1、2、1；`**one **two** three**` 的 `two` 加粗深度为 2。未闭合的外层不贡献深度，已确认内层仍保留。原 `Style::EMPHASIS` / `STRONG` 样式位继续表示存在该样式；深度不同的 run 不合并。隐藏语法也携带原区间上的计数，实体、代码规范化及部分 tab 替换继承底层语义片段的计数。

普通区间、tile、可复用 scratch 及 `DisplayCursor` 读取同一标注；晚闭合通过原有区间失效更新缓存，稳定块/容器/链接/数学身份和源码区间保持。两个计数复用动态标注树的区间累加与既有遍历预算，无独立回扫或查询；每个现有 Mark/遍历帧增加两个 `usize`，保持累计解析 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe。示例：`nix develop -c cargo run --release -p mdstream-core --example emphasis_depth`；专项：`nix develop -c cargo nextest run --test emphasis_depth --no-fail-fast`。

此接口描述已确认配对，不提供完整嵌套树、相邻同深度配对的身份或同时开启多个样式的先后顺序。任意长度 run 的部分消耗见下节；通用确认/撤销、链接作用域与完整强调继续按 TODO 推进。八份旧评分仍只消费样式位，当前观察及统计见上方评分表；独立深度消费者只在单类顺序确定时输出对应 HTML。原接口验收见[交付记录](DONE.md#2026-11-10-已确认强调加粗的同类嵌套深度接口交付)，终端排版/UI 保留后续。

## 强调分隔符部分消耗与三倍数规则

星号和下划线支持任意长度的 run：就近寻找兼容 opener，双方剩余长度至少二字节时消耗两字节并增加 strong 深度，否则消耗一字节并增加 emphasis 深度；从 opener 末端与 closer 起端消费。`__foo_ bar_` 的 `foo`/`bar` 强调深度为 2/1，`***foo** bar*` 的 `foo` 同时有强调和加粗各一层。剩余字节保持字面，具备左侧界时也可继续作为 opener。若任一 run 可开又可闭，原始长度之和不得为三的倍数，除非两者原始长度都为三的倍数；部分消耗不改变原始分类。`*foo**bar*` 因此保留内侧双星号，`foo***bar***baz` 可配对。

run 等待后继标量或 EOF 再定长；未完成/失败外层不贡献深度，已确认内层保持，关闭块/容器/单元格时不回放候选正文。继承表头的 close-only 配对可暂存，成功丢弃、失败一次提交；双向旧配对及其它不可隔离情况仍取消探测。容器前缀、代码/数学/链接的现有优先级保持；本接口不扩展链接作用域或一般语义撤销。

每种标记用六个固定分类头表示原长度模三和原 can-close，单次匹配/失败最多探测六次，`Work.delimiter_search_steps` 逐次计数。每个原 opener 只入/出栈一次，剩余范围原地缩小；每次配对消耗至少一个新 closer 字节。加上既有标注树，累计解析 O(n log n)、空间 O(n)，核心继续零运行时依赖、禁止 unsafe，显示/查询/摘要共用原帧预算。

示例：`nix develop -c cargo run --release -p mdstream-core --example emphasis_runs`。独立模型、流式分块/EOF、稳定身份/源码映射/共享预算及长串/深容器验收见[交付记录](DONE.md#2026-11-11-强调分隔符部分消耗与三倍数规则交付)。


## 已发布标签的未配对分隔符终点

链接/图片标签发布为对象时，内部尚未配对的 `*`、`_`、`~`、`~~` opener 不再与标签外文本配对。例如 `[foo *bar][ref]*` 在定义 ref 后显示链接文字 `foo *bar` 和外部字面 `*`；`**前 [签 *中](u) 后**` 保留外层加粗及标签中的字面星号。标签内已确认的样式保持；未定义 shortcut 仍在 `]` 发布同一对象，原 `[]`/`!` 边界保持可见，后续定义不改变 LinkId 或源码范围。

未闭合、被新括号/自动链接起始替换的候选沿用原行为；不能发布 shortcut 的超长/空白标签只在显式后缀成功时结束作用域。失败/未完成后缀保持原不重放规则。旧标签在晚表头确认前继续隔离，失败才发布并清退，成功由原有单元格确认路径处理。此终点清退的互补左边界已由下节交付，521/534/564、473/474 现匹配官方 HTML；嵌套对象、通用失败回退和混合样式顺序继续待办。

终点清退按源码位置从原栈尾清退，恢复原分类头，不重扫标签/正文。每字节保持常数扫描，每个 opener 入/出栈各至多一次，新增清退计入 `Work.delimiter_steps`；累计解析 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 及整帧共享预算保持。六项回归覆盖 144 组边界模型、UTF-8 分块/前缀/EOF、稳定身份/原始源码映射、共享六字节预算、晚表头与 970 层/百万字/4096 旧对象/4096 opener 工作计数。官方 535 直接匹配原 HTML，仅此例改类；官方输入/HTML、生成器和投影未改。

复跑：`nix develop -c cargo nextest run --test label_delimiters --test commonmark_link_labels --test single_tilde --no-fail-fast`；`nix develop -c cargo run --release -p mdstream-core --example label_delimiters`。完整证据见[交付记录](DONE.md#2026-11-11-已发布标签的未配对分隔符终点交付)。

## 已发布标签左边界的强调隔离

在现有单候选模型内，direct/full/collapsed/shortcut 链接或图片标签内部的 `*`、`_` closer 只消费同标签内的 opener。`*[foo*](/uri)` 显示外部字面星号与链接文字 `foo*`；`*前 [**中***](u) 后*` 保留包围整个链接的强调、标签内部加粗及剩余字面星号。单/双、多字节 run 的部分消费与三倍数规则保持，右边界继续清退标签内部未配对 opener。

候选标签中的强调/加粗/删除线 run 保存原始区间和前后标量摘要，等待候选确认；不保存或重扫正文、不复制外部栈、不撤销已发布标注。合法有界 shortcut 在 `]` 即发布，未定义 shortcut 保留原 `[]`/`!` 边界，原符号定义后隐藏；随后成功的 direct/full/collapsed 后缀复用同一 LinkId。不能发布 shortcut 的超长/空白标签等待成功显式后缀；失败或未完成后缀在既有拒绝点/EOF 按普通正文处理摘要一次，不重放后缀。不闭合、被新 `[` 或自动链接起始替换的未发布候选也按普通正文处理；到确认或 EOF 前，其摘要尚不发布样式，因此普通正文的跨括号强调仍可在失败后成立。

晚表头成功时，旧标签前一行摘要作为原段落处理一次，当前行摘要用于独立首单元格；失败才按标签确认路径发布。若未发布的继承候选被新 `[` 或自动链接起始替换，释放摘要时先处理旧行，再以其剩余栈恢复当前行的继承边界；旧行内部配对立即保留，跨行配对继续等待表头决定，长短表头结果一致。此衔接不扩展不可隔离表头的一般确认/撤销。每个摘要只处理一次，每次 closer 查固定六个分类头并按源码位置排除外部 opener；局部配对后沿用终点清退。摘要占 O(run 数) 空间并复用容量，处理/移交计入 `Work.delimiter_steps`；无正文增长回扫，累计解析 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与整帧共享预算保持。

官方 521/534/564 及同一规则覆盖的 473/474 均经独立观察后匹配原 HTML；512–538 当前为 **19/3/5**，553–571 为 **17/2/0**，460–481 为 **12/0/10**（支持/刻意差异/未支持）。523 仍有强调隔离差异，当前独立观察为字面 `*foo [bar* baz]`；官方 Markdown/HTML、生成器和评分投影算法保持。419、嵌套链接/图片、多候选、跨行定义/后缀、完整失败回退、混合样式顺序与 UI 继续待办。

复跑：`nix develop -c cargo nextest run --test label_delimiters --test commonmark_link_labels --test commonmark_reference_forms --test commonmark_emphasis_final --test commonmark_emphasis_partial --test single_tilde --lib --no-fail-fast`。模型、UTF-8 分块/EOF、稳定身份/原始映射、共享预算与长输入计数见[交付记录](DONE.md#2026-11-11-已发布标签左边界的强调隔离交付)。


## 引用定义目标紧接下一物理行

块起始处，合法单行 `[label]:` 后仅 space/tab 时，紧接的下一物理行可提供完整目标及该行的可选 title，例如 `[r]:\n<my url> 'title'`。根和已识别的完整显式容器路径支持 LF/CRLF/EOF、Unicode 与转义括号标签、首定义优先及晚引用。两行各受既有分类窗口限制，标签仍限 999 个原始字节；独立下一行完整 title 见下方子项；title 正文本身跨行、跨行标签及行内后缀不在此范围。

待决首行只保存 Span/BlockId，不发布可撤销 LinkId，不进入固定手工折叠范围。确认后沿用原块和容器身份、目标/title 原始区间并使首行布局失效；失败首行只消费一次，保留原表头、Setext、标题、代码、lazy 和部分 tab 方言。目标确认按完整路径的实际剩余列数判断结构优先级：新引用/列表和至多三列缩进内的标题、Setext、分隔线、代码/数学围栏优先，部分 tab 不会把它们误算为四列；真正至少四列的目标缩进仍允许。首行同时是表头候选时，先走原表格确认路径：普通空格路径和至多三列部分 tab 保留表格优先级，实际代码叶块路径的目标可确认，不按分隔行字面形状拒绝。增长中的目标超窗后不再追认，不回扫旧正文；晚定义仅沿用符号范围通知，累计 O(n log n)、空间 O(n)。

`tests/next_line_definitions.rs` 覆盖原 1440 组目标模型、3465 组容器结构、3780 组叶块/列宽及 16380 组联合表头资格对照、UTF-8 全切点/逐前缀/EOF、复杂行内失败回退、稳定身份/映射、真实 FoldView 更新、共享六字节显示及 dest/title 预算、百万字失败候选和两万旧引用。本子项交付时官方 198 转支持、评分 15/7/5；独立下一行完整 title 子项随后使 193/195/217 转支持，当前 26/1/0（根作用域多行 title 的 196 已支持）。官方输入/HTML/生成器和 project 结构解析保持，测试输出 helper 仅新增通用 U+0020→`%20` 编码，单行与下一行定义同规则。见[交付记录](DONE.md#2026-11-11-引用定义目标紧接下一物理行交付)。


## 引用定义的独立下一物理行完整 title

块起始处已确认的引用定义，目标行未带 title 时，可由紧接下一条物理行提供完整单引号、双引号或括号 title，包含空 title。例如 `[r]: /url\n  "title"` 和 `[r]:\n/url\n(title)`；单物理行标签的定义至多三条物理行，跨行标签见后续交付。根和既有完整显式容器路径均支持，复用原路径及部分 tab 列宽规则；路径之后的水平缩进可超过三列。每行仍受既有分类窗口约束，标签仍限 999 个原始字节。

目标先发布，定义块保持 Growing；待决 title 只记录原 BlockId 及首定义所有权，不进入行内解析，不产生 Markdown 对象或段落/Setext 资格。确认后扩展同一块的原始 source 包络，title Span 排除定界符并保留转义、实体和 Unicode 原文；原 getter 与共享预算解码游标保持既有语义。首定义独占符号更新，重复定义可消费合法 title 而不覆盖目标/title；已有 title（包括空 title）不再吸收第二行。

空行、坏尾缀、缺失祖先、新兄弟、未闭合 title 或超窗都只结束候选，保留已发布目标，当前行按原普通块语义解析；根作用域及既有完整显式容器路径内符合原普通文本资格的正文续接（含 1–3 虚拟列）见下方独立子项。待决期间 `source_fold_limit` 停在定义起点，固定范围注册被拒绝；成功、失败或 EOF 后解除，真实 FoldView 更新与旧测量 ticket 校验保持。每条有界 title 行仅常数次扫描，新增工作计入分类/容器计数，超窗不重试；确认只更新当前符号及增量区间，不遍历历史引用。累计追加 O(n log n)、空间 O(n)、核心零运行时依赖与禁止 unsafe 保持。

`tests/next_line_titles.rs` 八项回归覆盖 2100 组成功模型、失败普通块对照、所有权/已有空 title、UTF-8 全切点与逐前缀 EOF、稳定对象/原始映射、固定折叠及 FoldView、共享六字节显示和目标/title 解码、窗口边界、970 层百万字失败尾部及两万旧引用。官方 193/195/217 直接匹配原 HTML，完整章节当前 26/1/0（根作用域多行 title 的 196 已支持）；208/541 的跨行标签已支持；196 的根作用域多行 title 已由下方子项交付；既有完整显式容器路径内符合原普通文本资格的多行 title 正文（含 1–3 虚拟列）已由[容器多行 title 正文](#引用定义容器多行-title-正文)子项交付；510 的根段落后缀组件换行现已支持；容器后缀及容器内直接 title 正文换行、完整容器/字面回退及窗口放宽继续待办。官方输入/HTML、生成器、评分投影和输出编码 helper 均未改。见[交付记录](DONE.md#2026-11-11-引用定义独立下一物理行完整-title-交付)。


## 引用定义跨物理行标签

块起始处的引用定义标签可跨多条有界物理行，例如 `[Foo\n  bar]: /url`。闭合 `]` 后须在同一物理行紧接 `:`；目标在闭合行或紧接下一行，未带 title 时可由再下一行提供完整 title。根与已有 Quote/List/嵌套/部分 tab 完整显式路径均支持。普通已有段落不能被定义中断；缺失前缀、新兄弟、新容器、真实叶块和表格边界沿用原普通流程。根作用域 title 正文跨行（196）见下方子项；既有完整显式容器路径内符合原普通文本资格的多行 title 正文及 1–3 虚拟列已支持，见[容器多行 title 正文](#引用定义容器多行-title-正文)；根段落行内后缀的组件换行（510）现见[后缀组件换行](#根段落行内链接后缀的组件换行)，容器后缀及容器内直接 title 正文换行继续待办。

匹配键连接真实去前缀标签内容，保留原始 LF/CRLF、转义和真实缩进，统一限 999 字节；部分 tab 剩余虚拟列仅参与结构/显示，不伪造标签源字节。首个未转义 `[` 拒绝候选，转义括号/反斜线奇偶、完整 Unicode case folding、空白归一化及首定义优先保持。目标/title 继续借用真实 Span。每条物理行仍受原 1024 字节分类窗口（可完成 UTF-8）的约束，不扩展完整容器或失败字面回退方言。

待定来源保留唯一消费权，首 BlockId/ContainerId 保持，后续未分类来源不发布普通叶块或 LinkId/HeadingId。确认时隐藏整个源码包络；失败先取走候选，再按来源顺序单次回流至真实普通管线。共享表头恢复其消费权，中间真表头在 delimiter 到来时先回流前缀，再由原表格流程确认；部分 tab 三列/四列与普通空格的既有优先级分别保持。待定范围排除固定 source fold；确认/回退通过真实 Update/FoldView 失效旧 ticket。

每个新行/字节只常数次分类、探测和扫描，失败来源至多一次普通回流，标签仅在确认时归一化；窗口/上限失败后恢复流式消费。工作计入既有 classified/container/table/inline/label 计数，不访问历史引用。累计 O(n log n)、空间 O(n)、核心零运行时依赖及禁止 unsafe 保持。`tests/multiline_definition_labels.rs` 与私有 deferred 普通流程对照覆盖 UTF-8 切点/逐前缀 EOF、CRLF、真实原始映射、共享微预算、折叠/通知、969 层大前缀及 999 字节、百万字失败尾部、两万历史引用和大量定义。

官方 208/541 直接匹配原 HTML，本标签子项交付时定义章 **19/7/1**（根作用域多行 title 后为 **20/7/0**）、标签子集 **8/6/0**，其余独立观察保持；官方输入/HTML、生成器、project 算法及输出编码不变。见[交付记录](DONE.md#2026-11-11-引用定义跨物理行标签交付)。


## 引用定义根作用域的多行 title 正文

根作用域的引用定义 title 可跨多条有界物理行，单引号、双引号和括号沿用原转义规则，括号内的括号仍须转义。支持单行/多行 label、目标位于闭合 label 行或紧接下一行，以及目标同行或独立下一行开启 title。例如 `[r]: /url 'first\nlast'` 与 `[r]: /url\n'first\nlast'`。每条物理行保留原分类窗口；正文续行复用真实普通叶块、缩进、空行、容器及表格边界探测，独立起始行保留既有水平缩进规则。

目标同行开启但未闭合的 title，在完整合法前不发布定义或可撤销 LinkId；独立 title 的目标已经发布，失败/EOF 保留目标，只将暂存 title 来源回普通流程。失败来源按唯一消费权至多迭代回流一次，触发行仅消费一次；不重新取得定义候选资格。重复定义不覆盖赢家，已有完整 title（含空 title）不吸收后续行。原始 title Span 借用真实源，保留 LF/CRLF、缩进、实体和转义；解码游标只将物理 CRLF 归一为 LF，并映射回两字节原源，沿用共享零/六字节预算。

原 999 字节 label、4096 字节 title/目标和 suffix 上限保持。暂存片段上限为 `MAX_LABEL_BYTES + MAX_LINK_SUFFIX_BYTES + 4`：未闭合 label/title 每个物理片段至少贡献一个受限的真实字节，加上闭合 label、目标及末行常数余量；合法长 title 不受旧 1002 片段界限误限。每字节仅常数次扫描、失败至多一次回流，晚定义/补 title 只通知当前符号与区间，不遍历旧用法。累计 O(n log n)、空间 O(n)、核心零运行时依赖与禁止 unsafe 保持。

容器中的未闭合 title 起始行继续沿用普通正文资格，包括缺失前缀的 lazy 延续、已有路径的 partial tab 和新 Quote 的 tab 余量；保留原 LinkId/BlockId/ContainerId 与 tab→显示空格的原始映射。本根作用域子项交付时，600 组原基线快照及 1836 组容器/UTF-8 分块/逐前缀 EOF 对照保持一致；后续容器多行 title 的窄扩展见下节。

`tests/multiline_definition_titles.rs`、私有真实普通流程对照及 Destination 界限测试覆盖入口/Unicode、UTF-8 全切点/逐前缀 EOF、空白/坏尾缀/结构和表格边界、原始/显示映射、共享预算、稳定身份、Growing/Sealed、真实 source fold/Update 合并及 ticket 失效、4096 字节与超过 1002 条来源、百万字失败尾部、两万旧引用和 4096 条历史定义。官方 196 直接匹配原 HTML，定义章 **20/7/0**；其余观察保持，官方输入/HTML、生成器、评分 project 与输出 helper 未改。证据见 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-018/implementation/` 和[交付记录](DONE.md#2026-11-11-引用定义根作用域多行-title-正文交付)。容器内无虚拟余量的多行 title 及去前缀元数据见下节；正文的部分 tab 余量见下节；510 的根段落后缀组件换行已支持；容器后缀及容器内直接 title 正文换行、完整字面回退/容器和窗口放宽仍待办。


## 引用定义容器多行 title 正文

既有 Quote/List/嵌套完整显式路径内，title 正文可跨有界物理行；复用单/多行 label、两种目标入口及目标同行/独立下一行开启 title。正文续行复用真实普通文本资格，既支持零余量，也支持完整前缀匹配后剩余 1–3 虚拟列；路径可原子或部分消费多个 tab，label、目标和 title 开启行保留原有能力。虚拟余量参与 Code/Heading/Setext/Fence/表格和新结构判定，不能用统一三列公式放行；例如两列余量加两个真实空格仍按 Code 中断。缺前缀 lazy、新兄弟/容器、真实叶块和表格边界仍由普通管线决定。

原始 `Reference.title` / `link_title` 是去定界符的真实源码包络，内部容器前缀也留在包络中，包括仅有闭合定界符的末行前缀。解析时追加有序间隙索引；`decoded_link_title` 在 O(1) 时间借用获胜定义的索引，按共享预算逐段跳过结构前缀，保留真实缩进、实体、转义和换行。实际 CRLF 原子归一为 LF，实体产生的 CR/LF 不再次归一。title4096 和 suffix 上限分别按去结构前缀后的真实语法字节计算，逐物理行窗口保持；长物理包络不会因 `Span.len()` 被误拒，虚拟列不计为字节。部分消费的 tab 字节属于已知结构前缀间隙，虚拟余量不产生额外 metadata 文本；如 `> [r]: /u 'a\n>\t b'` 的 decoded title 为 `a\n b`，保留真实空格。失败回流仍把原 tab 映射为对应显示空格。

同行开放 title 在完整合法前不发布定义或需撤销的 LinkId；独立 title 失败保留已确认目标，仅回流自己的来源。来源单次迭代回流、索引 O(1) 追加且只保留当前/获胜定义，重复定义不积累间隙；读取不访问祖先、旧正文或历史引用。首定义、稳定身份、版本和增量通知、Growing/Sealed、source fold 边界及真实 FoldView 更新保持。累计 O(n log n)、空间 O(n)、核心零运行时依赖与禁止 unsafe 保持。

`tests/container_multiline_titles.rs` 及私有普通流程/来源/长度回归覆盖 672 组成功模型、768 组普通回退对照、148 份真实基线快照、UTF-8 全切点/逐前缀 EOF、0/1/6 字节及 query/object 预算、原始映射、折叠/Update、百万字包络和失败尾部、两万旧引用与 4096 条定义。官方输入/HTML、生成器、评分投影保持，定义章 **20/7/0**、引用标签 **8/6/0**、collapsed/shortcut **17/2/0**。证据见 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-019/implementation/`。部分 tab 余量扩展复用同一来源/gap 游标，新增 4032 组独立零/部分余量模型、3840 组真实普通失败对照及 1760 份真实基线快照（仅 56 份新增成功，其余 1704 份结构/映射保持），并覆盖真实缩进、实体 CR/LF、closing-only、长度/窗口、共享微预算、稳定身份及折叠。最终命令/完整日志见 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-020/implementation/` 和[交付记录](DONE.md#2026-11-11-引用定义部分-tab-余量的多行-title-正文交付)。510 的根段落后缀组件换行已支持；容器后缀及容器内直接 title 正文换行、完整字面回退/容器和窗口放宽继续见 TODO。


## 根段落旧行内表头资格观察基础

实际普通根段落从首个被消费的 payload 建立私有 `RootEligibility`，通过同一 Inline 状态机和有限无发布 effects 观察原语法。它使用相同的 Unicode label 规范化与接受分支，但不持有公开存储、引用表或树的可变引用，不分配 LinkId/SymbolId，不发布标注、映射或 Update。定义暂存源到普通回流时才观察；容器普通段落由下方独立活动 key 的容器观察状态维护，ATX、表格单元格、代码、数学块和脚注不创建普通段落观察器。

观察随原缩进、eager 第一 pipe 截点和 isolate/cancel/confirm/finish 事件推进，游标保证 payload 不重复进入。额外最多一份当前根段落的行内摘要，总时间仍为累计 O(n log n)、空间 O(n)。`Work.root_eligibility_bytes` 单列额外 payload/等价缩进访问，`root_eligibility_steps` 累计该内核的词法、分隔符/查询、flanking、label 规范化和有界候选工作；原 `inline_bytes`、树访问及其它计数保持发布路径含义。普通长正文每个新字节多一次观察访问，历史树/引用不增加遍历。

该基础交付时只观察，表头资格仍由原 publisher 决定，未改变语法或公开结果。本次[根段落后缀组件换行](#根段落行内链接后缀的组件换行)已将根表头资格接入旧语法观察器，并接入 `Deferred(key)` 和官方 510；非根控制保留原路径，宽范围需求继续见 TODO。复跑：`nix develop -c cargo nextest run --lib --test root_eligibility --test code_fallback --test inherited_headers --no-fail-fast`。测试含逐事件状态差分、1552 份真实 HEAD 公开快照摘要、UTF-8 每字节/逐前缀 EOF 和百万字/两万历史引用计数，见[交付记录](DONE.md#2026-11-11-根段落旧行内表头资格观察基础交付)。

## 根段落行内链接后缀的组件换行

实际根作用域普通段落内，链接和图片后缀在 `(` 后、完整目标后、完整 title 后到 `)` 前的空白中，各允许至多一个 LF 或 CRLF，CRLF 计两个源字节、一次行尾。例如 `[link](   /uri\n  "title"  )`（`\n` 表示物理 LF）直接匹配官方 510。空目标、裸/尖括号目标、平衡括号、三种 title 定界及原始长度限制沿用原规则；目标正文仍为单物理行；直接 title 正文的根扩展见[下节](#根段落直接链接与图片的多行-title-正文)。实体产生的 LF 只是解码内容，不触发组件分隔。空白行、真实块与作用域中断保持原分类优先级；根模式不用于容器/lazy/partial-tab、ATX、表格单元格、代码、数学和脚注；真实普通容器的组件模式另见[容器后缀组件换行](#容器普通段落后缀的组件换行)。根段落后续被确认为 Setext 或表格时沿用原结构流程。

合法完成时，已发布的 shortcut LinkId 原子升级为直接目标，源包络包含后缀、title 保留真实借用 Span，原引用及 first_use 保持。未完、失败和 EOF 保留旧 ID、shortcut source/target 和空 title，完成 sealing；拒绝字节只按普通词法处理一次。原子闭合的失败直接 suffix 现恢复实体和 ASCII 标点转义（精确资格见末节），例如：

| 原始输入（转义行尾） | 旧单行语法的可见文本 | 当前根跨行语法的可见文本 |
| --- | --- | --- |
| `[a](u\n 'a&amp;'x)\n` | `a(u\n'a&'x)\n` | `a(u\n'a&'x)\n` |
| `[a](u\n 'a&amp;`（EOF） | `a(u\n'a&` | `a(u\n'a&` |

旧语法在首个 LF 拒绝，后面的实体成为普通正文；当前已消费 suffix 在完整资格验证后恢复闭合原子。含其它未转义行内语法、尾半实体/反斜线或转义物理换行的整个候选继续 opaque raw，不回放代码/强调。候选外的拒绝字节正常解析，例如合格的 `'a&amp;'&amp;)` 中两个实体均显示 `&`。LF/CRLF 的原 break 映射在完成前保留，成功后整个后缀隐藏。完整字面失败回退仍在 TODO；容器组件 gap、根/容器 title 正文见下方各窄项。

根表头资格从首个源 payload 起始终由旧单行 grammar 的 `RootEligibility` 决定，观察 effects 不发布、不合并到正文。实际 Inline 消费合法分隔 LF 后设置粘滞标志；仅继续当前根段落的合格候选显式使用 `HeaderMode::Deferred(key)`，与保留原块和观察器使用同一作用域条件，由旧观察器推进至首 pipe，唯一来源在取消/确认/EOF 时消费一次。新 Quote/List/嵌套及部分 tab 路径按原普通模式创建叶块，同时关闭旧根解析器和观察器；lazy、真实块及新根仍按原资格路由。非根及未分歧路径保留 ordinary/eager/opaque 复用。继承强调的否决、两个 gap 后的旧 opener、有效新 title 内的旧资格、code/label 屏蔽及晚管道表格均有原快照对照。无正文回扫、第二 publisher、ID 回滚或旧引用遍历，累计 O(n log n)、空间 O(n)，不承诺单次 append/EOF 硬时延。

`tests/root_link_suffixes.rs` 覆盖 3600 组组件模型、UTF-8 每字节/逐字/逐前缀 EOF、原始和显示切片、0/1/6 共享预算、稳定元数据与版本、真实折叠/Update、长度上限、百万字尾部和两万旧对象；历史 24 份非目标中 23 份保持完整快照，精确输入 `[a](u 't\nrest')\n` 由根 title 正文扩展独立断言新元数据/显示；48+24 份表格冻结快照保持。`tests/root_suffix_scope.rs` 另覆盖未完/成功/失败根前缀进入 Quote/List 的六个独立崩溃反例，以及 Link/Image、LF/CRLF、分类窗口附近、嵌套/partial-tab/lazy/真实块、新根、无 pipe/有 pipe 与分隔行确认/失败；1260 份冻结库对照保留容器结构、原始映射及身份，逐字节/逐字符/逐前缀 EOF、共享 0/1/6 预算和两万旧对象工作计数同时核验。成功的新根前缀按新语法独立断言升级，其容器部分仍与旧库对照。`root_eligibility` 的 foundation corpus 仅 `[a](u\n 't')` 及其带终止 LF 的两个完整新合法输入独立改预期，其余原 oracle 保持。官方 Links 482–511 现为 **22/6/2**；512–538 **18/4/5**、539–552 **8/6/0**、553–571 **17/2/0**、Images **19/1/2** 保持。原 Markdown/HTML、生成器、评分投影和输出 helper 不变。复跑：`nix develop -c cargo nextest run --lib --test root_link_suffixes --test root_suffix_scope --test root_eligibility --test commonmark_inline_links --test commonmark_images --test link_suffix_headers --test inherited_headers --test code_fallback --no-fail-fast`。完整证据与验证范围见[交付记录](DONE.md#2026-11-11-根段落行内链接后缀组件换行交付)。


## 根段落直接链接与图片的多行 title 正文

实际根 `Paragraph` 的直接链接/图片 title 正文支持多条 LF/CRLF 物理行，单引号、双引号、括号沿用原转义规则，括号 title 中未转义的 `(` 仍拒绝。例如 `[a](u 'first\n  last')` 的 raw title 为 `first\n  last`；闭合行只有缩进和定界符时，真实 1–3 个前导空格也属于 title Span。原目标正文仍限单物理行；正文行尾与三个组件空白各至多一个行尾的规则独立。空白行、四列代码、真实块、容器及表格保留原普通流程优先级；Setext 可在合法对象完成后提升原段落。容器、lazy/partial-tab、ATX、独立 cell、代码/数学及脚注不新增 title 正文资格。

raw title 直接借用连续真实源码，CRLF 计两个 title/suffix 字节；已有 `LinkTitle`/`CodeInfo` 游标把 CRLF 原子输出为映射到两个源字节的 LF，保留真实缩进，反斜线和实体沿用原一次解码。反斜线不跨物理行转义下行定界符；实体产生的 LF/CR 不参与结构。单独 CR 保留拒绝方言。label 999、目标/title 4096 和 suffix 8256 的原界限独立保持，不给 inline streaming 加定义的逐行分类窗口。

完整合法 suffix 才升级原 LinkId/Image；失败/EOF 保留原 shortcut/reference 来源、身份及解析状态。合格原子闭合前缀现恢复：`[a](u 'first\n&amp;\*` 在 EOF 显示 `[a](u 'first\n&*`；`[a](u 'first\n&amp;'&amp;)` 显示 `[a](u 'first\n&'&)`。含内部代码/强调/新链接标记的候选仍不重放，不因失败补发对象；这不是完整字面回退，精确失败域见末节，相关宽 TODO 保留。

新 title LF 复用粘滞分歧标志、旧单行 grammar `RootEligibility`、真实根续接限定的 `Deferred(key)` 及单一来源双游标；观察状态不发布或合并。每个新增字节仅常数次访问，树操作 O(log n)，累计 O(n log n)、空间 O(n)，核心零运行时依赖和禁止 unsafe 保持。没有旧正文回扫、历史引用扫描或单次 append 硬时延承诺。

`tests/root_title_bodies.rs` 覆盖 1440 组独立成功模型、每 UTF-8 byte/切点/逐前缀 EOF、LF/CRLF/转义/空 title 对照、原始映射、0/1/6 与 query/object 共享预算、稳定身份及首定义、Update/真实 FoldView/open/fixed/ticket/reader、原长度界限、2000 条正文行、百万字尾及两万旧对象。864 个新正文前缀转长容器的结构/身份/映射与真实 `47d95a8` 对照；另 120 个晚 pipe 输入中 88 个全快照保持，12 个新成功、20 个 opaque 失败差异逐条独立断言。原 48+24 表格、1260 scope 及资格 corpus 保持；旧 negative 测试另有 `[r](/url 'a\nb')`、对应图片及长段落后的 `[a](url "two\nlines")` 三个精确新合法输入，均保留原输入并单独断言 metadata/display；不泛化跳过 LF，历史 fixture 不改。官方 Links 四子集 **22/6/2、18/4/5、8/6/0、17/2/0** 与 Images **19/1/2** 的独立观察和分类保持；原官方 Markdown/HTML、生成器、project/helper 不变。容器内直接 title 正文、完整字面失败回退及其它宽需求继续保留；容器后缀组件 gap 见下方独立窄项。

复跑：`nix develop -c cargo nextest run --test root_title_bodies --test root_link_suffixes --test root_suffix_scope --test root_eligibility --no-fail-fast`。模型、真实基线、逐例显示差异与完整命令证据见 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-024/implementation/` 和[交付记录](DONE.md#2026-11-11-根段落直接链接与图片的多行-title-正文交付)。


## 容器普通段落的旧语法资格观察基础

容器普通逻辑段落共用私有 `Eligibility` 和现有无发布 Inline 内核，以首个物理 leaf 的 BlockId 与精确 ContainerId 记录唯一活动身份。同 scope 续行保持实例；blank、父 scope 返回、兄弟/嵌套、真实块、Setext、成功 definition、table 确认和 EOF 沿原生命周期退出。它不持有公开 ID、引用表或第二棵树，不发布或合并 effects。基础落地时容器语法及表头结构判断由原 publisher 驱动；下方 B 启用组件 gap 后，由该旧语法 observer 决定表头资格。

三个现场生产者统一镜像实际 raw payload 与整组 virtual spaces；结构 prefix 不进入观察，独立 raw cursor 和最后 virtual tab offset 避免 fallback 重复消费。definition/header 未释放的源不预扫；第一 pipe 截点、isolate/cancel/confirm/finish 与 first-cell reuse 同步，观察器不转入 RowParser。`Work.container_eligibility_bytes` 单列 raw 与 virtual 访问，`container_eligibility_steps` 单列局部词法/摘要/隔离工作；root 和旧发布计数保持原义。仅多当前普通段落的状态，累计 O(n log n) 时间、O(n) 空间，核心仍零运行时依赖、禁止 unsafe。

测试冻结真实 `7da6c27` 的 6592 份完整公开快照与全部旧 Work 字段，覆盖 scope、LF/CRLF、UTF-8 每字节/短输入每前缀 EOF、分类窗口/late pipe、partial tab 与 definition 回流；真实 producer 的 2700 组 header 路由逐事件核对全部活动语法状态和复用结果。另核验公开段落首 leaf/映射、virtual 1–3 列整组去重，以及深路径/百万字/两万旧引用后1000次追加精确新增1000次观察。既有共享预算、Update/Fold/sourceFold 和官方 fixture 验证保留。复跑：`nix develop -c cargo nextest run --lib --test container_eligibility --test root_eligibility --test root_suffix_scope --test root_title_bodies --no-fail-fast`。

本基础项未启用新语法；容器组件 gap 和 prefix 预算由下方组件项交付，直接 title 正文 LF/CRLF 由下方正文项交付。完整字面失败回退及其它宽需求继续保留 TODO。验证证据见[交付记录](DONE.md#2026-11-11-容器普通段落旧语法资格观察基础交付)。


## 容器普通段落后缀的组件换行

真实普通 Quote/List/嵌套容器逻辑段落内，链接和图片在 `(` 后、完整 target 后、完整 title 后三个 gap 各接受至多一个 LF/CRLF。target 正文仍限单物理行；lone CR、组件 gap 中的转义换行、未闭合尖括号或未平衡裸 target 的内部换行均拒绝。容器直接 title 正文见下节，root 已交付的 title 正文模式保持；cell、ATX、code、math、footnote、definition 不启用此模式。

续段只采用现有分类生产者给出的 raw/结构 prefix/virtual 事件。lazy 和部分 tab 由真实分类决定；例如 `>\t  't')` 的两虚拟列加两真实空格会成为 code，不能接续后缀。首 leaf 与精确 ContainerId 必须同时相同；blank、真实块、父返回、兄弟和嵌套边界不跨接。raw source 包络包含结构 prefix，target Span 只含单行正文，直接 title 的 raw/decoded 结构见下节。组件结构 prefix 从 suffix8256 限额独立扣除，不能减 title 长度；虚拟列为零 metadata 源字节，target/title4096 和 label999 原限制保持。

首次接纳新 gap LF 后的分歧标志持续到逻辑段落结束。旧单行 Eligibility 始终独立观察，决定首 pipe 前的表头资格；`Deferred(key)` 要求 candidate、observer 和实际 Inline 的首 leaf/scope 相同，先完成真实续段分类。eager deferred 不隔离或提前消费实际 Inline；否决时唯一 SourceOwner 释放 raw/virtual 各一次，确认 table 则封存原 shortcut、退役 observer 并使用独立 cell。旧 header 即使在新 containers 栈形成后回流，也按保存的旧 key 消费。Ordinary eager/opaque/inert 复用及 root 原跨容器保护保持。

完整 `)` 才原位升级原 LinkId/Image；引用 first_use/revision 不撤销重建。合格原子闭合失败前缀现恢复：`> [a](u\n> 'a&amp;'x)\n` 显示 `[a](u\n'a&'x)\n`，相应 EOF 显示 `[a](u\n'a&`，合格的 `\*` 显示 `*`。含未转义 `*b*` 等其它语法的整个候选继续保留 raw，不补发强调。拒绝字节起正常词法仅处理一次。多行 title 正文同此精确失败域；完整字面回退继续待办。

`container_component_gaps` 测试包含18,000组独立组件模型、真实角色/晚 pipe/作用域矩阵、UTF-8分块与每前缀 EOF、独立长度/结构 prefix 限额、身份/原始映射、共享0/1/6字节预算、Update apply/merge与Fold重建、open/fixed/ticket/reader及百万字/两万旧对象计数。历史静态 oracle 原字节保留：A corpus 仅66个完整输入有精确新期望，其中32个公开 Direct 成功、34个公开全快照保持而仅工作计数改变；旧 root non-target 仅4个精确容器输入升级，scope corpus 仅60个精确容器新成功逐条独立断言 typed metadata/display。无按换行或前缀泛化豁免，官方输入/HTML/评分/projection/helper 不改。

观察、发布、summary 固定次数消费新增 payload；prefix 为现场 O(1) 通知，无增长正文/旧引用/祖先回扫。独立 `Work.container_eligibility_*` 计费，累计 O(n log n) 时间、O(n) 空间，核心零运行时依赖且禁止 unsafe。复跑：`nix develop -c cargo nextest run --lib --test container_component_gaps --test container_eligibility --test root_link_suffixes --test root_suffix_scope --test root_title_bodies --no-fail-fast`。完整验证见[交付记录](DONE.md#2026-11-11-容器普通段落后缀组件换行交付)。


## 容器普通段落直接链接与图片的多行 title 正文

真实普通 Quote/List/嵌套/lazy/partial-tab 逻辑段落的直接 title 正文接受多条 LF/CRLF、三种定界和转义；blank、真实块及精确 scope 中断仍按原分类拒绝。target 正文、独立 ATX/cell/code/math/footnote 与 definition 规则保持。首行 opener 前及 closer 后的结构 prefix 只计组件；正文中的结构 prefix 由唯一 source 消费者现场记录，partial-tab 的虚拟列不进入元数据，真实空格/tab 则保留。例如 `> [a](u 'a\n> b')` 的 raw title 是 `a\n> b`，decoded title 是 `a\nb`，每个 chunk 映射原始源字节。

候选自持有有序 gaps，完整合法 `)` 才随 target/title/source 一次移动到原 LinkId 的私有平行槽；pending、失败、EOF 和 table 确认不改原 shortcut/引用 title，下一候选从空 gaps 开始。Reference reader 继续取首个 winning definition 的 gaps。原始借用 Span、CRLF/UTF-8 原子输出、实体 probe 恢复与 CodeInfo 共享 0/1/6、query/object 预算保持；跳结构 prefix 按真实源字节收费，末尾免费结束。失败实体/ASCII 转义仅在末节完整资格域内恢复，其余整个候选保留 opaque raw，拒绝字节起正常词法一次；完整字面失败回退、target 正文换行、嵌套图片和宽泛标准覆盖仍在 TODO。

正文4096计真实 indent 和 CRLF 两字节，结构 prefix 分别从 title/suffix8256 扣除，target4096及原分类窗口保持。`Work.link_title_gap_steps` 每次非空正文 prefix 入候选加一，不遍历旧 gaps；push 摊销 O(1)、发布 move O(1)，累计 O(n log n) 时间/O(n) 空间，核心零运行时依赖、禁止 unsafe。真实 depth1000 对照仍只有512个结构 Quote；其余488对 `> ` 是正文，decoded 保留。

测试含132组独立三定界/Link/Image/行尾/真实 indent 与逐 chunk 源映射模型、UTF-8 每切点/EOF、真实前驱6592份旧 corpus（仅5个精确新合法输入改变）、946份角色/EOF及100份窗口/late-pipe对照、稳定引用与原子升级、预算/真实 Fold/Update/stale、1000行 title 和两万旧定义加百万正文后的追加计数。旧 fixture 原字节保留，新成功与 opaque/Work 差异按完整输入精确列出。复跑：`nix develop -c cargo nextest run --test container_title_bodies --test container_eligibility --test container_component_gaps --test root_title_bodies --no-fail-fast`；示例：`nix develop -c cargo run --release -p mdstream-core --example container_title_bodies`。完整验证见[交付记录](DONE.md#2026-11-11-容器普通段落直接链接与图片的多行-title-正文交付)。


## 原子闭合的失败直接链接后缀恢复

所有实际发布 Inline（根/普通容器段落、ATX、独立 table cell）的 `Tail::Destination` 失败或 EOF，只对已消费 raw 数据执行窄恢复：完整实体按既有单层规则解码，ASCII 标点转义隐藏反斜线，未知完整实体保持 raw。开头 `(` 和当前拒绝字节不参与资格，拒绝字节仍正常处理一次；成功 grammar、旧 LinkId/Reference/first_use/title gaps 和 Sealed 流程保持。`\&amp;` 仅去掉反斜线，不二次解码；实体生成的 LF 只是映射文本。

必须先整体验证：未转义 `< [ ] ! * _ ~`、反引号、`$` 使整个已消费候选失格；结束时半实体、尾反斜线或反斜线紧邻物理 LF/CRLF 同样失格，前面的实体也不提前恢复。转义标点可合格；非法实体字符按原拒绝规则在原位继续，不跨拒绝点续接。原空白、缩进、CRLF 和 break 方言保持；不重读普通 lexer；未解析 shortcut 的边界由下节独立条件标记控制，不补发强调/code/math/autolink/嵌套对象。完整字面回退等宽需求仍在 TODO，官方输入/HTML/评分分类保持。

候选仅持有不可 Clone 的实际 raw Span RLE，无 Publishing 能力的 Eligibility 不建 trace；virtual 列和结构 prefix 不记录。失败先走原 header cancel/seal，资格通过才第二遍调用原 entity/hide 出口。`Work.link_fallback_bytes` 记录两遍实际字节读取，`link_fallback_steps` 记录 raw 录入/合并及每次 span 访问。若 A 为实际发布 raw visits，则恢复字节和 span 访问分别不超过 2A，录入不超过 A；每 trace 数据不超过 suffix8256，但物理包络可达40万字，不能据包络重扫。成功/失败/EOF 释放 trace，不累积历史；累计 O(n log n)/O(n)，核心零运行时依赖、禁止 unsafe。

`tests/atom_fallback.rs`、私有 trace/DisplayIndex 测试及既有共享 reader 测试覆盖11个真实前驱红目标、159个非目标完整快照、三定界/Link/Image/行尾、UTF-8 全切点和每前缀 EOF、replacement/source 切片、0/1/6/query/object、稳定引用/Fold/Update/stale、真实表头确认/取消/first-cell guard、8256边界/重叠链、40万字prefix与百万正文/两万旧定义计数。复跑：`nix develop -c cargo nextest run --test atom_fallback --no-fail-fast`；示例：`nix develop -c cargo run --release -p mdstream-core --example atom_fallback`。验证见[交付记录](DONE.md#2026-11-11-原子闭合的失败直接链接后缀恢复交付)。

## 未解析 shortcut 的字面边界与晚定义

已发布的 shortcut Link/Image 在原引用未定义时显示真实 `[]`/`!` 边界；例如 `[*a*]` 显示带强调的 `[a]`，失败后缀 `![a](u '&amp;'&amp;)` 显示 `![a](u '&'&)`。label 内部的样式、深度、LinkId 和源码映射保持。首个获胜定义隐藏原符号的边界，不替换对象或扫描历史使用；duplicate 和仅 title 续行不触发该可见性变化。候选 Image 仍有对象回调，destination 为 None 表示未解析。

私有条件 annotation 与永久 HIDDEN 分开；永久隐藏优先。条件 owner 现为稳定 LinkId，读取该对象当前目标，已解析 a 升级到未解析 b 时原 marker 可重新显示。Direct 与未先发布合法 shortcut 的完整 full 仍永久隐藏；已有 shortcut 的 full/collapsed 完整升级见下节，引用目标选择保持。Runs、scratch/display、raw LayoutPass、可见位置查找和基于 display 的搜索/摘要均读取有效样式；SourceSearch 和 raw summary 保留源码。晚定义另发 Visibility，因此 references_affect_size=false 也会使旧测量失效。

当前离线统计（支持/刻意差异/未支持）：定义章 **26/1/0**；Links 482–511 **27/1/2**、512–538 **19/3/5**、539–552 **13/1/0**、553–571 **17/2/0**；Images **20/0/2**；实体章 **16/0/1**。以上替代前文各历史交付时的计数。官方输入/HTML、生成器及评分投影不变；full/collapsed 回选、完整失败 lexer、HTML、嵌套图片与其余宽 TODO 仍未交付。

`tests/shortcut_boundaries.rs` 和私有 annotation 回归覆盖真实前驱快照、typed marker/ID/映射、UTF-8/EOF、hard LF/全空格 code/math、微预算、late winner/策略失效、FoldView、百万字追加及两万旧对象。纯 shortcut 的条件标记只占原 marker 的至多三个 ASCII 字节；下节合格完整升级还覆盖整个后缀。每个合法 shortcut label 保留 base-visible scalar，配合永久隐藏树汇总使 next_visible 保持 O(log n)。没有新 boundary/uses 索引或旧正文回扫，累计 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持。


## 已有 shortcut 对象的未解析完整引用边界（2026-11-12）

仅传入 Publishing.complete_link 时已有合法 shortcut LinkId 的 full/collapsed Reference 升级扩展字面边界：`[a][b]`、`[a][]`、`![*a*][b]` 未解析时分别显示 `[a][b]`、`[a][]`、带强调的 `![a][b]`；当前目标首个定义后隐藏原 `[]`/`!` 与整个后缀。标签内的样式、深度、ID、原始映射和 LinkInfo 源域保持。后缀继续 opaque raw，例如 `[a][*b*&amp;\!]` 的后缀不新增实体解码或语法对象。

资格在取得/复用 id 前保存，不从 label.link 推断。空/纯空白/超长/非法 shortcut 标签的完整 full（包括 `[][b]`、`![][b]`）保持原永久隐藏；空 collapsed、失败或未完成后缀、Direct、HTML、嵌套及 full 目标回选继续保持各自既有边界。569/571 不重新解析 `[bar]`，545 的 suffix 保留原始反斜线；本次三份相关官方子集统计没有改类，不能据此宣称完整标准回退已支持。

稳定 LinkId gate 只做当前目标的常数次读取；永久隐藏先短路，Direct/Email 不查旧符号。范围标注及 successor 沿用区间树 O(log n)，不缓存动态 resolved 汇总、不扫描历史 uses；合格标签的可见 witness 阻止跨任意多个条件对象。空 full 继续由永久隐藏汇总跳过。累计 O(n log n)、空间 O(n)、原共享预算与无额外渐进 query scratch 保持。

`tests/reference_boundaries.rs`、`tests/reference_consumers.rs` 及私有 annotation/cache 测试覆盖实际前驱 40 例、全 UTF-8 切点/前缀 EOF、同 ID 三类升级、定义先后/重复/title、原始映射、容器/真实表头、0/1/6 与零 query/object/dependency、marker-only/suffix-only tile、策略全 false 的失效，以及 999 字节后缀、两万不同 owner 与百万字工作计数。旧夹具（含 shortcut 大 baseline）保持原字节，只加精确 source/key/oldhash 增量；独立 scalar 比较覆盖 8,144 行旧 root/container 与 915 行 shortcut 快照，非目标和完整 resolved 的精确 run 保持。
