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

`Document::work()` 暴露输入扫描量、块分类输入量、行内扫描量、分隔符操作数、标签规范化字节数、区间树实际写入访问次数、表格行扫描字节数与自动链接候选检查字节数；`table_header_bytes` 计候选表头摘要扫描，`table_candidate_bytes` 计长分隔行探测；`container_bytes` 计容器前缀检查，`container_steps` 计活动帧匹配及单个容器开闭，`list_steps` 计条目直接内容/兄弟检查及一次性 Tight → Loose 转换；`heading_steps` 计作用域开闭、已分类行的上下文检查和标题插入，每步至多六个固定索引操作，不含查询和标题区间的 O(1) 延长。分类字节量是分类器接收的字节数，分类器内部对这些字节只做固定次数的线性扫描。它不是 CPU 指令计数。测试使用这些指标约束增长，不依赖机器跑分来“证明”复杂度。

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

不完整的行首也在片段中，立即以原文可见。`pending()` 最多 1024 + 3 字节；通常在换行、EOF 或达到上限时提交块分类。超过上限的行后续直接流式消费；符合条件的长表头与长分隔行延迟语义提交，按新增字节维护识别状态。长表头已完成块分类，`pending()` 为空；长分隔行的 `pending()` 只保留最初分类前缀，其后原文仍可由片段读取，见下方表格规则。不复制或反复解析增长中的尾部。缓存使用 `tile_key()`，不要只按片段 ID 永久缓存；`Update::dirty` 是源码区间通知，不是展开的片段 ID 列表。

## 显式列表与引用容器

`ContainerId` 指向独立 arena，`ContainerKind` 区分 Quote、List 和 Item；列表节点保存 bullet/有序分隔符，条目保存 number 和可选 TaskMarker。`Document::container(id)` O(1) 返回当前 revision 的 `Container` 快照：父 ID、深度（根为 1）、完整物理行源码包络、Growing/Sealed 状态。每个叶块只增加 `Block.container` 指向最内层容器，叶块索引仍按源码排列；`BlockKind::Quote/ListItem` 保留本行标记信息，不再代表整个容器范围。ID 只在所属 Document 内有效，关闭后不复用。

```rust
use mdtui3::{Budget, ContainerKind, Document, SyntaxState, WorkLimit};
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

- 支持显式 `>`、`>>`、`> >`，以及 `-`/`+`/`*` 和 1..9 位数字加 `.`/`)` 列表；可混合嵌套。每层标记前允许 0..3 个 ASCII 空格，`>` 后最多消费一个可选空格。相同父级、相同缩进、相同 bullet/有序分隔符延续 List；改变数字只创建新 Item，改变标记类型创建新 List。
- 列表标记后须为空格、tab 或完整行结束；消费标记后的全部水平空白。续行须用 ASCII 空格补足该 Item 从父前缀到正文的字节宽度（包含标记前缩进；空条目按标记末端宽度），再匹配子层。tab 只作为列表标记/任务后的分隔字节，每个算 1，不展开为 tab stop；不接受 tab 作为续行缩进。每个兄弟按自己的数字/空白宽度确定续行要求。
- 显式引用续行重复 `>`，列表续行补足缩进；连续非空正文也支持省略部分或全部前缀的 [lazy continuation](#容器-lazy-continuation)。无前缀空行关闭全部容器；显式 `>` 空行保留引用，缩进足够的空行保留对应条目。不满足 lazy 条件时，缺失某层前缀只保留已经匹配的外层，内层关闭。空条目和空任务保留条目身份；任务只识别最内层新条目的正文开头，任务标记之后继续识别标题或普通行内正文。
- 容器正文保留逐物理行叶块身份；同一确切容器的显式续行可衔接强调、代码 span 及软/硬换行，见[容器内行内续行](#容器内行内续行)。容器内 ATX 和多物理行 Setext 标题及章节作用域见下节，表格识别及索引见[容器内表格](#容器内表格)。容器内围栏/缩进代码见[容器内代码块](#容器内代码块)；链接与脚注定义见[容器内定义](#容器内定义)；独立分隔线仍作为正文，不生成对应块对象（仍按行内方言处理）。顶层表格分隔行确认、分隔线/Setext 和已打开围栏的优先级保持。列表/引用整段折叠见[容器折叠](#容器折叠)；容器内标题独立折叠见[容器内章节折叠](#容器内章节折叠)，tight/loose 元数据见[列表 tight/loose](#列表-tightloose)，终端布局仍待接入。
- 结构仅从既有 1024 字节分类前缀（UTF-8 边界最多额外 3 字节）识别。前缀中的完整 `>` 可以开层，列表/任务候选必须在前缀内有分隔符或完整行末；截断候选和前缀后的标记保留正文，不在后续追加中重新分类。深度受前缀字节限制；不声称支持无限深度或完整 CommonMark/GFM 容器规则。

容器新增解析工作累计 O(n + 开闭次数)，容器/活动栈空间 O(n)；匹配一层必定消费正长度前缀，首次失配后只判定一次 lazy 正文：成立则 O(1) 保留剩余活动路径，否则批量弹栈。列表续行共享空格扫描结果。整库仍累计 O(n log n)。`container_arena_stats` 统计节点和活动栈实际容量字节及 Vec 增长次数，不含标题索引/作用域、源码、分配器元数据或 Update。批量关闭和 EOF 仍可能一次处理整个活动栈；解析不承诺单次调用硬预算。有预算的查询不会枚举正文或子树。

`cargo run -r --example containers` 展示混合嵌套、续行、兄弟切换、未分类尾部及分帧祖先读取。`tests/containers.rs` 验证独立范围模型、增量快照重建、1000 层引用、深列表单次空格扫描、百万字长续行、空行批量关闭及分块一致性；现有分块/fuzz 快照也包含容器。

## 列表 tight/loose

`Container.list_tightness` 在 List 节点上为 `Some(ListTightness::Tight | Loose)`，Quote/Item 为 `None`。新列表从 Tight 开始：Growing 时 Tight 是当前判断，可在后续内容到来时变成 Loose；Sealed 时 Tight 最终确定。Loose 单调成立，无论列表是否仍开放都不会撤回。所有条目共享父列表属性，叶块不复制该状态；O(1) 读取父列表，深层查找则使用已有的 `container_ancestors`，每步计 1 query / 1 object。

本阶段在既有显式容器方言上定义空行分隔，尚不声称完整 CommonMark tight/loose：

- 只有完整物理行在去掉已匹配容器前缀后全为空白，且处于某个确切 Item 的直接作用域，才记作该条目的待定空行。列表续行需补足已有缩进；引用前缀的可选一个空格另行消费。无前缀空行仍关闭全部容器，`- a\n\n- b\n` 因此是两个 Tight 列表，跨这类空行保留身份仍是后续需求。
- 同一条目已有直接内容后，空行后再出现直接内容或新直接子容器，父列表变 Loose；同一列表的兄弟条目若跟在待定空行后，也确认 Loose，包括空条目。相邻正文物理行、lazy 正文、无空行的新子容器/兄弟本身不确认 Loose。编号变化不换列表，bullet/有序分隔符变化则结束旧列表。
- 内容前的空行不单独确认 Loose；只有列表/任务标记的首行不算分隔空行。末尾空行若随后 EOF、作用域退出或列表标记类型变化，保持 Tight。定义、标题、表格、代码和新嵌套 Quote/List 都属于直接内容；失败候选按既有正文规则处理。
- 嵌套列表/引用中的空行只作用于其确切作用域，不向祖先传播。例 `- a\n  - b\n    \n    c\n- d\n` 只有内层列表 Loose；若需要外层分隔，空行必须先退回外层 Item。嵌套末尾空行传播及更完整块边界规则仍保留后续。
- 围栏代码内的空白是代码内容，不是分隔；缩进代码沿用空白行终止规则，随后新直接内容可确认 Loose。分类仍受 1024 字节窗口限制；没有在窗口内结束的全空白行按既有代码/正文分类提交，稍后的 LF 不重新判定。短尾部等 LF/EOF，长行扩展不重读旧内容。

```rust
use mdtui3::{ContainerId, Document, ListTightness, SyntaxState};
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

每个活动列表帧仅增加“已有直接内容/待定空行”两个布尔摘要，只检查当前直接条目或正在切换的兄弟。嵌套帧开闭成本按已有前缀/事件计费，不为短 lazy 行或长行追加遍历祖先。累计额外状态 O(n)，新增语义处理 O(n)，每列表至多一次 O(log n) 区间失效，全库仍为累计 O(n log n)。`tests/list_tightness.rs` 验证 UTF-8 全切点/逐前缀、400 组生成输入、千层引用、250 层列表、4096 个历史条目及百万字增长；`cargo run -r --example list_tightness` 演示晚确定通知及分帧元数据读取。

## 容器内行内续行

同一确切 `ContainerId` 中连续的非空正文行共享一个行内状态机，支持跨物理行的 `*`/`_` 强调、`**`/`__` 加粗、`~~` 删除线、等长反引号代码 span、链接/图片显示标签及软/硬换行。可使用显式续行、等价前缀（如 `> >` 和 `>>`）或下节定义的 lazy 续行，判断依据是容器身份。输入只消费去前缀后的正文和原始 CRLF；前缀继续隐藏并保留源码映射，不复制或拼接一个虚拟段落字符串。

- 每个正文物理行仍有自己的 `BlockId`、范围和最内层容器；换行后叶块范围 Sealed 不代表行内样式已全部确定。稍后的 closer 可给多个旧叶块应用范围样式和 `dirty` / `Change` 失效，不改它们的身份或逐个生成块变更。`pending()` 仍只表示尚未分类的当前行前缀。
- 进入子容器、退出到父级/根、切换兄弟条目或列表类型、无前缀空行及显式空白正文终止旧序列；之后回到同一个外层容器也不恢复旧 opener。已识别 ATX、围栏/缩进代码、链接/脚注定义和确认的表格同样中断序列，代码 span 也不屏蔽块分类。容器中未识别为独立块的分隔线仍按既有正文方言消费。
- LF 在下一条同序列正文实际交给行内解析器时才成为 SOFT_BREAK 或 HARD_BREAK；未分类尾部、未决表头不提前确认。行尾两个以上空格或未转义反斜线产生硬换行，CRLF 只隐藏 CR。序列终止时保留末端原文，不把它标为行内换行；代码 span 内的换行由代码显示规则处理。
- 早管道表头候选保留前序行内状态但不提前消费候选正文；失败/EOF 时一次性消费，确认时丢弃前序状态并按独立单元格解析。新行内序列、只继承待确认换行或尚未闭合旧强调的晚管道表头复用已消费的首单元格前缀，只延迟首管道起的后缀；继承未闭合代码 span 或数学的前缀保持原候选状态，确认后在单元格扫描中解析正文。无管道普通长行仍即时消费正文。其它继承状态的晚管道长行与非法定义继续普通正文。长分隔行采用下方单遍探测及一次回退规则，不在长行尾部重试块分类。
- **Setext 的晚确认保持单调语义**：同作用域连续正文整体成为标题，保留已确认的跨行样式、换行及对象 ID，下划线终止未闭合状态。例如 `> *first\n> last*\n> ---\n` 的标题显示为 `first\nlast`，已有强调继续有效。物理叶块保持身份，标题保存跨叶块范围，不重扫旧正文；需要撤销已确认语义的更完整块规则仍另行设计。
- 代码 span 的前缀不参与 all-spaces 判定。规范化照常把内容换行显示为空格，并在两端为空白、内容不全为空白时各去一个规范化空格；末端记录最后一个实际正文 byte/CRLF，所以 closer 前的隐藏前缀不会被错当正文。闭合仅检查端点并建立一个源码包络映射，不回扫内部行。
- 链接/图片显示标签支持同作用域的跨物理行正文，源码包络和去前缀消费见[容器跨行链接与图片标签](#容器跨行链接与图片标签)。数学候选也可跨同一作用域正文续行，去前缀内容与源码映射见[容器跨行数学内容](#容器跨行数学内容)；失败候选保持字面/opaque 行为，不重新解释内部标记。单行链接目标/title、显式引用后缀、脚注、自动链接、实体及 EOF 确认沿用原有规则；更完整的块边界撤销仍待实现。

```rust
use mdtui3::{Document, Span, Style};
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

显示游标、摘要、缓存与折叠继续共用既有 Budget 和 revision 契约；隐藏前缀及 CRLF lookaround 计入读取预算，晚闭合使旧测量/检查点失效但保留折叠意愿。`cargo run -r --example container_inline` 演示流式闭合、代码去前缀显示和分帧读取。`tests/container_inline.rs` 及分块测试覆盖边界、失败候选、UTF-8 全切点/逐前缀、千层/百万字、250 层列表、4096 个未闭合 opener、400 组独立显示/样式模型和共享预算。

## 容器跨行链接与图片标签

同一确切容器的连续正文可包含跨物理行 `[label](url)`、`![alt](url)`、shortcut `[label]`、collapsed `[label][]` 及 full `[label][key]`。显式、等价前缀和 lazy 续行共享候选；`!` 必须紧邻开括号，新的 `[` 替换旧候选，`<` 丢弃旧候选，代码/数学中的 `]` 不关闭标签。这是既有单候选方言的扩展，不新增嵌套链接解析。目标、title 和 full 引用的后缀 `[key]` 继续限单物理行。

`LinkInfo.label` 是原始源码包络，包含中间 LF/CRLF 和容器前缀；`Document::text(label)` 因此仍返回这些原始字节。显示/alt 使用 `display_runs_in(label)`，帧内消费则使用 `display_cursor(label)`，忽略 `Style::HIDDEN` 的片段。可见片段保留绝对源码区间、实体/代码替换映射及同一个 LinkId；前缀仍可按源码定位，整段 label 不复制成另一个字符串。显示搜索复用同一片段流，跨行命中的源码包络可包含隐藏前缀；源码搜索保持原始字节。

引用匹配使用去前缀的**原始正文**，不使用去标记/实体解码后的显示文本：例如 `[a&amp;\n> b][]` 匹配定义 `[a&amp; b]: url`。沿用 Unicode 小写和空白折叠，不新增转义解码。999 字节上限按实际喂给行内解析器的正文计算，含原始 CRLF、标记和转义，不含容器前缀；超长标签仍可成为任意长显示标签的直接链接或使用独立短 `[key]`。空/全空白标签不能作为引用，但直接图片仍可发布对象。

每个 Inline 只保留一个可复用、最多 1000 字节的候选缓冲（999 字节加潜在闭括号），溢出后只保留失败标记，继续消费正文而不保存长标签；Vec 容量按有界增长保留。缓冲只在新增字节到来时填充，候选被替换时清空，关闭时规范化至多 999 字节。collapsed 后缀复用已生成的 SymbolId，不重新读取源码包络。新对象仍通过 `new_links` 发布，暂定引用补全直接目标保持 LinkId 并通过 `changed_links` 通知；晚定义继续符号间接寻址，不遍历旧引用。累计解析 O(n log n)、核心空间 O(n)，不增加按行/祖先复制的映射数组。

块/容器切换和空行沿用上节的序列终止规则；未闭合标签保留已解析的显示语义，不回放正文。失败目标/引用后缀仍可留下未解析到目标的 shortcut 对象，保持原方言。表头确认隔离单元格并丢弃旧标签；失败/EOF 恢复候选且只消费一次后缀。晚管道前已关闭旧标签则取消探测，不能撤销已发布对象；多物理行 Setext 晚确认保留已发布语义并结束未闭合候选。更完整的块撤销规则仍待设计。

```rust
use mdtui3::{Budget, DisplayStep, Document, LinkId, Style, WorkLimit};
let mut doc = Document::new();
doc.append("> ![中 &amp;\r\n> 🙂][]\n> [中 &amp; 🙂]: /image\n")?;
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

显示游标保留原共享预算、原子暂停和 revision 校验；隐藏前缀及 CRLF lookaround 也计费，六字节/一 query/一 object 可推进一个 UTF-8 基本步骤。闭合只做区间标记与失效，不回扫全部叶块；旧布局/检查点和读取任务失效，折叠意愿保留，隐藏百万字标签的折叠布局零正文读取。`cargo run -r --example container_links` 演示稳定图片身份、晚定义及分帧读取；`tests/container_links.rs`、分块快照和缓冲容量测试覆盖映射、搜索/折叠、深嵌套、百万字与两万旧引用。

## 容器 lazy continuation

连续非空正文中允许省略引用的 `>` 或列表条目的续行缩进，包括只提供部分外层前缀。首次前缀失配后，若剩余内容属于下述正文方言，整条原活动路径保持开放，叶块继续绑定原最内层 `ContainerId`。例如 `> > first\n> second\nthird\n` 三行均属于内层引用；`- - first\n  second\n` 的第二行仍属于内层条目。已经匹配的前缀照常隐藏，剩余空格和 tab 保留为正文；不会补造标记、复制祖先链或拼接段落字符串。

- **资格**：仅在最内层有连续非空正文（包括未决表头候选）时启用。空条目/空任务、空白正文、标题、已确认表格、代码块和有效定义之后不推断 lazy 续行。兄弟条目或新子容器改变身份后不会恢复旧行内状态。代码 span 等未完成行内候选不屏蔽块中断。
- **中断**：在首次失配处，对剩余有界前缀使用既有块分类规则。空白、有效 ATX、合法长度围栏、完整分隔线、有效链接/脚注定义，以及最多三格缩进后的引用或列表标记均中断。沿用项目列表方言，任意合法编号和空条目标记都可中断，不限定编号为 1。只关闭未匹配的内层，再在保留的外层/根按已有规则解析当前行。
- **正文与失败候选**：不符合中断语法的 `#nospace`、失败围栏/定义/列表候选留在原容器，仍接受普通行内解析。缺失容器前缀时，剩余四格以上缩进也可作为正文；若路径全部显式匹配，继续使用既有缩进代码规则。纯空白分类前缀始终不作为 lazy 正文，即使长行后面还有非空内容。
- **表格与 Setext**：lazy 行只延续正文，不成为表头、表格正文行或分隔行；前一未决表头按普通正文消费一次。缺少前缀的 `===` 是正文，完整 `---` 作为分隔线中断，空 `-` 是列表中断。后续同作用域显式 Setext 下划线提升整个连续正文序列（包含 lazy 行），保留已经确认的行内语义。需要回撤旧语义的规则另行设计。
- **流式边界**：仍使用含前缀的 1024 字节分类窗口（UTF-8 最多额外 3 字节），短尾部等 LF/EOF 再判定；长行一旦分类不在后续追加或换行时重试。定义、分隔线和其它候选能否中断取决于原分类器的完整行条件。链接/图片标签可跨同作用域 lazy 正文；数学也可跨同作用域 lazy 正文，块边界与失败方言继续沿用[行内续行](#容器内行内续行)。

```rust
use mdtui3::{Document, Span, Style};
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

每行只扫描实际提供的前缀和一个有界正文分类窗口；全部省略时只检查一次活动帧，不遍历余下深度。开放容器与章节沿用共享末端，普通 lazy 追加不产生祖先关闭/重开事件或折叠索引写入；叶块、范围失效、预算消费和 EOF 收尾沿用既有契约。`tests/container_lazy.rs` 与分块测试覆盖千层引用、250 层列表、4096 个短续行、百万字追加、1200 行独立范围/通知模型、共享预算和折叠回归。`cargo run -r --example container_lazy` 演示部分及全部省略前缀、晚闭合、折叠保持与分帧显示读取。此为有界项目方言；tight/loose、完整 tab 缩进、分类上限扩展和更完整的块确认/撤销规则仍未实现。

## 容器内标题与章节作用域

显式引用和列表条目内识别 ATX h1–h6，以及连续一行或多行正文后、紧邻下一物理行的 Setext `=` / `-` 下划线。先消费显式容器前缀、列表续行缩进及任务标记，再按既有标题方言分类；相对正文最多三格缩进。标题 `source` 保留完整物理源码；ATX 的 `content` 去掉容器/标题前缀、CRLF 和受支持的 closing hashes，多行 Setext 的内容包络见下方；行内显示、实体和源码映射沿用原实现。条目即使变成标题，任务/编号仍可从 `ContainerKind::Item` 读取。

容器 Setext 确认同一 `ContainerId` 中自上次行内序列边界起的连续非空正文，支持显式、等价前缀及 lazy 续行；不跨空行、兄弟条目、实际容器切换或已识别块中断。仅缺少部分前缀的正文可能仍是最内层容器的 lazy 续行，不能据前缀长短推断作用域已退出。下划线必须显式匹配确切作用域；确认时新增一个 `HeadingId` 并隐藏整条下划线，旧正文及行内语义不重扫。`> text\n> -\n` 与 `- text\n  -\n` 中下划线优先于新建空子列表；`- text\n-\n` 的第二个 `-` 是兄弟条目。超长标题内容可以流式增长，但下划线仍须在包含容器前缀的 1 KiB 分类窗口内完整出现；ATX 的 hash run 必须在窗口内遇到空白分隔符或完整行末，否则作为正文；超长 ATX 的尾部 closing hashes 不回看。显式正文的强调、代码 span 与换行续接见[容器内行内续行](#容器内行内续行)；更完整容器规则及下划线长度上限扩展继续待办。

多行标题保留全部原有 `BlockId` 和源码有序、不重叠的叶块范围。`Heading.block` 指向首叶块，只有首块改为 `BlockKind::Heading`；最后一个叶块向后包含下划线，中间叶块完全不改写。`Heading.source` 从首物理行开始到下划线结束，可能大于首叶块范围；标题排版应依据这个范围，不能只凭各叶块的 `kind` 推断标题内容。`Heading.content` 从首正文 byte 到最后正文行的 CRLF 之前；中间换行和容器前缀保留原始偏移，用显示游标去除隐藏前缀、CR 并保留软/硬换行或代码映射。

表头候选失败后仍属于同一正文序列，仅消费尚未解析的后缀；表格确认会终止序列。非法或超过分类窗口的下划线按原有正文/块规则处理，之后合法下划线可包含这些普通正文行。未完成下划线到换行或 `finish` 才确认；没有下划线则保持普通叶块。Setext 保留已发布的强调、链接、图片、数学与显示映射，结束剩余未闭合候选，不回放失败内容。

解析只保留一份固定大小的首叶块 ID 与内容区间；续行更新末端，确认修改首尾块、插入一个标题并发布一次整段范围失效。`changed_blocks` 保守扩大到首叶块（包含未修改的中间块），无逐块事件；`new_headings` 驱动折叠默认初始化、旧章节边界回退和导航。确认成本为有界前缀分类、O(log n) 范围更新及未闭合行内状态的一次性收尾，不按标题字节或物理行数回扫。收尾仍可能清理很多 opener，遵守累计 O(n log n)/空间 O(n)，不承诺单次确认硬时延。名称搜索、slug 和实际标题读取均在显式共享 Budget 中进行。

```rust
use mdtui3::{BlockId, Document, HeadingId, Span, Style};
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
use mdtui3::{Document, HeadingId};
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

`cargo run -r --example container_setext` 展示多行标题迟确认与跳过百万字折叠正文的预算布局；`tests/container_setext.rs` 覆盖 400 组独立范围/显示模型、千层引用、250 层列表、4096 个连续叶块及历史标题。`cargo run -r --example container_headings` 展示晚确认、作用域切换及小预算 slug。测试覆盖 UTF-8 全切点/逐前缀、1200 行固定种子线性范围模型、千层嵌套、百万字 Setext 无回扫、500 个同时开放的章节作用域、共享预算搜索/slug/折叠导航。

## 容器内代码块

显式 Quote/Item 内识别围栏和缩进代码，复用 `BlockKind::Code { info }`、稳定 `BlockId` 与源码有序块索引；`Block.container` 是确切的最内层容器。块的 `source` 包含每行完整物理源码及容器前缀，显示时隐藏容器前缀，正文保留 CODE 样式、原始字符和 LF，CRLF 的 CR 隐藏。代码内的列表/引用标记、标题、表格、链接/脚注、数学、实体、强调和反斜线均为字面内容，不创建相应对象。

- 围栏 opener 在容器前缀后允许至多三个 ASCII 空格、3..32 个相同反引号或波浪线；反引号 opener 的已分类 info 不得含反引号。`info` 是去掉两端 ASCII 空白的源码区间，不解析实体或复制正文；opener 整行隐藏。
- 围栏续行只匹配已经打开的引用/条目路径，不从正文新建子容器。例如 `> ``` ` 的后续 `> > - text` 正文是 `> - text`。同一确切容器内，完整行以至多三格缩进、至少 opener 长度的同字符组成的串及纯 ASCII 空白结尾才闭合，closer 整行隐藏。短串、另一字符、尾随非空白、四格缩进均为正文；正文不按 opener 缩进额外去缩进。
- 每条引用行必须重复显式 `>`，列表行必须补足原 Item 的续行缩进。缺失某层前缀、兄弟条目或退出容器使旧代码止于新物理行开头，再按新作用域分类该行；外层的围栏可能因此新开一个独立块。显式空行仍属于围栏正文，无前缀空行关闭容器及代码。EOF 封闭未闭合围栏，保留代码语义，不回退/重扫正文。
- 缩进代码要求去掉容器前缀后至少四个 ASCII 空格，仅额外隐藏四格；tab 不代替四格。相邻缩进代码行仅在相同 `ContainerId` 下合并，支持流式长行。沿用既有顶层方言：可中断普通正文，完整空白行立即终止代码（即使缩进足够），不延迟保存空行；下一行重新开块。新列表/任务标记后的全部水平空白仍属于已有容器前缀，因此缩进按消费该前缀后的剩余字节计算，通常从条目续行开始。完整 CommonMark 缩进/空行规则仍待扩展。

结构识别沿用含容器前缀的 `BLOCK_LOOKAHEAD = 1024` 字节窗口（UTF-8 最多再取三个字节）。opener 在窗口内确认后，过长 info 后续部分整行隐藏，不因晚来的反引号撤销；未识别的超长/失败 opener 走普通行内方言。closer 必须连同行末在窗口内完整出现，超长 closer 行按代码正文流式消费，即使尾部只是空白也不回看。未分类尾部的块状态保持 Growing，`finish` 按 EOF 分类短尾行或封闭已流式消费的长行。

```rust
use mdtui3::{BlockId, BlockKind, Document, FoldId, FoldView, SyntaxState};
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

围栏只增加一个常数大小的作用域检查，容器匹配每保留一层消费正长度前缀；列表缩进串只扫描一次。超长代码行的后续追加仅延长当前块和共享已解析末端，不做行内扫描、不更新祖先。闭合只访问当前行及实际关闭节点，不读取旧正文；累计解析 O(n log n)、空间 O(n) 不变。折叠仍至多刷新一个开放代码/数学/表格块，容器与内层章节共用尾部，不枚举历史代码块。解析及 `apply` 仍按输入/事件摊还，不承诺任意大输入的单帧时延。

`tests/container_code.rs`、`tests/container_sections.rs` 和分块/fuzz 测试覆盖作用域/失败候选、UTF-8 全切点及中间快照、长行/CRLF/EOF、预算与组合；400 个代码块的生成范围模型、240 次流式区间/高度模型、千层容器/百万字正文、250 层列表和 4096 个历史代码块验证增量成本。折叠百万字代码的布局字节计费和正文回调均为零。`cargo run -r --example container_code` 演示流式围栏、预算预览、兄弟关闭以及只展开目标路径。

## 容器内定义

显式 Quote/Item 内支持既有单行链接定义 `[label]: destination title` 和脚注定义 `[^label]: body`。先匹配容器前缀、列表续行缩进及新条目的任务标记，再允许至多三个前导 ASCII 空格；四格缩进和已有围栏优先，代码里的定义保持字面内容。`BlockKind::Definition` / `FootnoteDefinition` 保存原有符号，`Block.container` 记录最内层容器，`source` 覆盖完整物理行和前缀。目标/title 与脚注 content 均是借用源码的绝对区间。任务信息仍从 Item 读取。

定义的**符号作用域是整个文档**：根、嵌套、兄弟条目和已经关闭的容器共享链接符号表，也共享独立的脚注符号表；不创建局部命名空间或按祖先查找。每种符号的首个有效定义获胜，后续重复定义仍产生隐藏块但不覆盖、不改变符号版本。链接标签沿用最多 999 字节、Unicode 空白折叠及转小写；脚注标签沿用区分大小写的字母数字/下划线/连字符规则。脚注编号仍按全局首次引用顺序分配，定义先到不占号。容器结束或折叠不删除定义，任意位置的引用链接、引用图片及脚注都能解析到它。

- 链接定义必须连同目标、可选 title 和行末在含容器前缀的 1 KiB 分类窗口内完成（UTF-8 最多延长三个字节）。短行等 LF/EOF 才发布；超过窗口后按普通容器正文消费，不因晚来的行尾再确认。裸目标、尖括号目标及三种 title 沿用顶层方言。
- 脚注只要求合法 `[^label]:` 位于分类前缀内，正文可无限流式增长。冒号后前缀内已有的一个 space/tab 可被省略，其余原样保留；正文不解析 Markdown、链接、数学或嵌套脚注。未分类短行仍显示原文；确认后整行隐藏，长行保存 Growing 定义，LF/EOF 封为 Sealed。CRLF 去掉末端 CR/LF，分块中的裸 CR 暂时属于正文。
- 定义仅占一个物理行，不吞下一行的缩进、空行、兄弟或外层内容；缺前缀按既有规则关闭容器，下一行独立分类。定义可以中断普通正文或同容器表格，即使目标/脚注正文含 `|`；表格单元格内的片段、标题正文和行内代码等仍按自己的上下文处理。定义不会作为下一条 Setext 下划线的标题候选。
- 非法/超长标签、目标/title 失败、未完成候选沿用普通行内处理，可能创建未解析引用，但不创建有效定义。不回放长行已消费的前缀；无效的纯空白链接标签也保留容器前缀隐藏和独立叶块边界，不并入外层段落。多行目标/title、脚注多段正文和更完整的容器续行规则仍未实现。

```rust
use mdtui3::{Budget, Document, FootnoteReferenceId, FootnoteTarget, LinkId, WorkLimit};
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

实现复用现有全局符号表和单行状态机，无新增持久化索引或祖先副本。有界标签寻址 O(log S)，长脚注追加只更新当前块/符号和共享解析末端；容器匹配按实际前缀/开闭节点计费。累计解析 O(n log n)、空间 O(n) 不变，不承诺大 append 或 EOF 批量闭合的单帧硬时延。`tests/container_definitions.rs` 与分块测试覆盖两万处旧引用晚定义、千层容器百万字增长、250 层列表、4096 个历史符号、400 组首定义/物理范围模型、通知重建/合并和共享预算；隐藏百万字定义的布局正文回调和字节计费均为零。`cargo run -r --example container_definitions` 演示全局晚定义、容器折叠及逐帧脚注读取。

## 容器内表格

显式 Quote/Item 的普通正文行可以成为表头；紧接的完整物理行只有在**同一最内层 ContainerId** 中，且列数与分隔单元格有效时才确认表格。识别先匹配已有引用前缀/条目续行，再验证分隔行：`> a|b` 后的 `> - | -` 是表格，分隔行优先于新开子列表；`- a|b` 后的 `- -|-` 已换到兄弟 Item，不能确认旧表头。顶层的分隔行优先级保持。

```rust
use mdtui3::{BlockId, Document, FoldId, TableId};
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

正文只接纳同一容器中的普通文字行，允许无管道的稀疏行。空行、标题、代码/定义/分隔线形式的行，进入子容器、离开容器或换兄弟条目都会终止表格；回到外层后不恢复旧表。随后合法的围栏/缩进代码和链接/脚注定义进入独立块；独立分隔线仍按容器正文规则处理。单元格行内状态按行/列隔离，前缀不进入单元格；任务信息留在 Item 上。普通正文的跨行行内语义见[容器内行内续行](#容器内行内续行)，不会跨确认后的表格单元格；lazy continuation 和完整 GFM 兼容仍未实现。

`Table.block` 指向保存确切容器 ID 的叶块；Table/source、delimiter 和 TableRow/source 包含完整物理行及容器前缀，TableCell/source 仅含去空白的单元格正文。`table_at` 覆盖前缀/分隔行，`row_at` 覆盖行前缀但不覆盖分隔行；按 ID/行列读取仍 O(1)，定位表/行仍 O(log B)/O(log R)。未分类尾部排除在结构索引外，已分类的超长行按追加增长；开放表格折叠暂覆盖 pending 尾部，边界确认后收回。列宽增长、行变化与表格创建沿用既有 Update，旧行不重扫。

同一时间仍只有一张开放表格，整表折叠、默认收起、临时展开、导航和 TablePreview 直接复用现有接口；折叠父节点为确切作用域的当前章节，若尚无本地标题则为该容器（根作用域无标题时无父节点）。摘要的源码总上限和共享 Budget **包含隐藏容器前缀**；千层前缀也不能免费越过。正文追加只消费新字节，不遍历祖先；长分隔行也只匹配一次有界容器前缀，后续按字节更新状态。空间不复制祖先链或创建空单元格矩阵，全库保持累计 O(n log n)、空间 O(n)。`container_bytes` 包含分隔行优先级检查，`table_candidate_bytes` 统计长分隔行探测字节。

`cargo run -r --example container_tables` 演示流式确认、兄弟边界、默认整表收起和小预算摘要。`tests/container_tables.rs` 与折叠测试覆盖 UTF-8 全切点/逐前缀、独立范围模型、千层/百万字追加、超宽稀疏表、隐藏前缀预算和单占位跳过正文。实际表格排版及行范围折叠仍见 TODO。

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

表头/分隔行允许可选首尾竖线，列数必须一致；正文缺列为隐式空单元格，多余列隐藏，不分配 rows × columns 的空矩阵。`\|` 在文本和代码 span 中均表示单元格内的竖线。表格在空行或其它块级结构处结束，可以直接跟在段落后。短表头沿用既有规则；长表头支持窗口内早管道，以及新行内序列、只继承待确认换行、尚未闭合旧强调/代码 span/数学/链接及图片标签的晚管道。候选余文、分隔行、正文行和单元格没有此长度限制。解析器只保存一个候选的源码区间、已消费端点和固定大小摘要，延迟部分以原文预览；`pending()` 仍只报告未分类行的有界前缀。判定失败时表头只语义解析一次，不回扫历史段落。

**长表头的流式规则**：分类窗口内的前缀按既有不完整行规则属于普通正文时，窗口内已出现未转义 `|` 的候选延迟全部行内消费。若窗口内尚无管道，新行内序列和可隔离的继承续行参与晚管道识别：顶层新段落、新容器/空行/标题后的正文，以及前序状态只剩待确认软/硬换行、或还保留未闭合强调 opener / 代码 span 的顶层或同一 Quote/Item 显式续行，以及继承未闭合数学或链接/图片标签的顶层段落及同容器正文。旧强调、代码、数学或标签若在首管道前实际闭合，则取消当前行的表头探测，保留普通正文效果。它在首管道前保持即时普通行内解析，从首管道起延迟后缀。lazy 行、代码、标题、定义等已识别块和既有表格正文不参与；窗口后不再重试块/容器分类。

长候选按新增字节维护列数、反斜线奇偶及首尾管道/空白状态；不存单元格数组，不复制原文。早管道候选不发布行内语义；晚管道候选已发布的首单元格前缀语义保持，延迟后缀不发布样式、链接、数学或脚注对象。LF 后固定列数并等待下一行，CRLF 可跨分块；容器前缀与 CR 隐藏，候选内容仍可按片段显示。已经分配的叶块保持 Growing 和稳定 BlockId，块/容器/章节的末端按新字节增长；紧随顶层段落的早管道候选保持原段落端点；即时消费的晚管道续行先延伸原段落，确认时恢复原端点并新建表格块。

确认分隔行时，早管道候选丢弃前序行内状态，候选通过既有行解析器消费一次。可复用前缀的晚管道候选先丢弃仍未闭合的继承强调 opener 与旧标签候选，再把原有 Inline 移给首单元格；行解析器扫描源码以建立单元格、裁剪空白和隐藏代码内的转义管道反斜线，跳过已消费前缀的行内解析，并在首管道处结束该 Inline；后续单元格独立。分隔行无效、作用域改变或 EOF 无分隔行时，只把未消费后缀送入保留的普通状态，再按当前块规则继续或结束；既有Setext 回退保持。已发布的首单元格语义无需撤销，链接/数学/脚注 ID 不变且只发布一次。长分隔行晚失败沿用一次有界前缀回退，禁止同一行再次开启长表头候选。

**继承续行的隔离**：在消费当前分类前缀之前，O(1) 检查行内状态，仅有待确认换行、可选的强调 opener、代码 span、数学或链接/图片标签时可参与晚管道探测。下面先说明强调/换行状态；代码、数学和标签的规则见后文。把该换行的源码区间、待隐藏尾空白/反斜线和软硬类型移入候选；当前行保留原强调栈即时消费，并记录开始时的栈边界和是否曾闭合旧 opener。前序换行及其尾标记暂按原文显示，当前行的样式、实体和对象仍即时发布。确认表格时丢弃此换行；无管道行到 LF/EOF，或候选因分隔行失败、作用域改变、EOF 而回退时，提交一次原换行效果。此提交产生正常范围失效，旧缓存和测量不能漏掉前序行变化。只增加固定大小状态，不克隆栈、不撤销对象、不重读正文。短行及早管道候选保持原有行为。

**未闭合强调的探测**：支持既有 `*`、`**`、`_`、`__`、`~~` 方言。当前行内部的配对正常发布；只要它尚未闭合任何继承 opener，已发布效果便与独立首单元格一致。确认表格时一次移除旧栈前缀并重建当前行剩余 opener 的前驱下标，保留当前行代码/数学/链接等候选以及所有已发布对象 ID。首管道前仍待定的 delimiter run 在移除旧 opener 后按单元格末端结算。失败、EOF 或无管道行结束时只清除探测标记、恢复前序换行，原栈及尚未消费后缀继续普通解析。

若旧 opener 在首管道前闭合，则一次切回普通流式行，当前行不再尝试表头；已确认的跨行样式和对象保持即时可见。同次追加中首管道后的后缀也只普通消费一次。记录实际闭合事件，不能仅比较当前栈长度：旧 opener 弹出后，新 opener 可能又把栈长恢复。确认时的删除/重建计入 `delimiter_steps`，每个 opener 最多在所属序列变为单元格时被处理一次；失败和普通续行不遍历历史栈。

```rust
use mdtui3::{Document, LinkId, Style};
let mut doc = Document::new();
doc.append("> prior  \n")?;
doc.append(&format!("> [cell](url) {}", "x".repeat(2048)))?;
let original = *doc.link_info(LinkId(0)).unwrap();
assert!(doc.tables().is_empty());
doc.append("|b\n> -|-\n")?;
assert_eq!(doc.tables().len(), 1);
assert_eq!(*doc.link_info(LinkId(0)).unwrap(), original);
assert!(!doc.runs_in(mdtui3::Span::new(9, 10)).unwrap()
    .any(|run| run.style.contains(Style::HARD_BREAK)));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`table_header_bytes` 对每个候选正文摘要字节只计一次（也计无管道的合格序列和短普通正文的候选检查）；`inline_bytes` 对可复用的前缀不重复消费；继承代码/数学的成功路径还计一次当前行的独立单元格解析，失败和 EOF 则保持原候选状态、不重复消费前缀。确认时 `table_bytes` 计一次单元格边界扫描。无管道长行正文语义即时提交，收尾不重写整行注解；隔离的前序换行仅在候选失败、旧候选闭合而取消探测或无管道行结束时提交。候选额外状态 O(1)，已有 Inline 只移动、不克隆；分隔行对齐值 O(列数)，确认后才分配真实列/单元格，稀疏行不补空矩阵。输入 String、tile 和正式语义索引仍按需增长，全库累计 O(n log n)、空间 O(n)。**长表头确认与失败均不承诺单次 append 的硬时延或字节预算**；有预算显示、摘要和折叠读取继续使用既有接口，确认通过范围失效拒绝旧测量。

```rust
use mdtui3::{Document, TableId};
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

**仍保留的边界**：已有段落或同一容器正文续行若旧强调/代码/数学或顶层链接/图片标签在首管道前已经闭合，且首管道晚于窗口，仍按普通正文处理。例如 `*prior\nend*…|b` 可能在管道出现前已闭合跨行强调；现有单调样式与稳定对象 ID 无法直接撤销。此范围仍在 TODO，不能通过延迟所有普通长行来绕过即时消费要求。短表头及早管道长表头继续支持前序段落。尚未移除其它块/容器分类上限，也不承诺单次解析硬预算。

```rust
use mdtui3::{Document, LinkId, TableId};
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

`examples/late_headers.rs` 演示首管道前百万字即时解析、已发布链接身份复用及共享 Budget 摘要；`tests/late_headers.rs` 覆盖首单元格语义对照、成功/失败、未闭合跨行状态限制、千层/百万字、4096 对象与稀疏列、400 组范围模型及布局/折叠预算。

`examples/open_headers.rs` 演示保留旧强调 opener、百万字即时消费、成功丢弃与失败闭合，以及稳定对象和共享预算摘要。`tests/open_headers.rs` 覆盖五种 delimiter、首管道前待定 run、闭合后栈重新增长、512 组局部配对模型、400 组表格范围模型、千层/百万字、4096 旧 opener/对象/列、缓存与合并通知；逐切点及中间快照在 `tests/chunking.rs` 验证。

```rust
use mdtui3::{Document, LinkId, Span, Style};
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

**未闭合代码 span 的探测**：继承代码候选可同时保留旧强调栈，以及顶层未闭合链接/图片标签。当前行继续即时更新原代码的反引号 run、全空白标志和最后一个内容区间；代码内的实体、链接、数学和脚注保持原文，不提前发布独立单元格对象。首管道起的后缀仍延迟。若旧代码在首管道前闭合，立即发布原跨行样式及空白规范化映射，并取消该行探测；即使随后开启新的代码候选也不恢复探测。

确认表格时，旧代码尚未闭合，所以当前行前缀没有已发布的行内效果。一次丢弃原 Inline（包含旧强调栈），**不结算首管道前待定的反引号 run**，再由已有 RowParser 扫描当前候选行并按独立单元格解析。确认前的旧对象 ID 保留，当前行对象只在这次解析中分配一次，源码映射按单元格规则生成；不撤销已发布的显示映射。失败、无管道、作用域退出或 EOF 只取消探测标志，继续原代码状态及未消费后缀，保留原空白摘要，代码闭合仍只读端点、不回扫正文。

代码路径的每个合格字节只做一次表头摘要和一次原代码状态消费；成功后在既有单元格扫描阶段再做一次当前行的行内消费。两次消费均如实计入 `inline_bytes`，全流满足 `inline_bytes <= source().len() + table_bytes`；额外消费仅属于互不重叠的已确认表头，不读前序代码正文、不重新分类、不在失败时重放，因此累计仍为 O(n log n)、空间 O(n)。候选只增加一个布尔标志，不复制正文或栈。这个子项保留单次确认的摊还契约，显示、摘要和折叠的共享预算不变。

```rust
use mdtui3::{Document, LinkId};
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

`examples/code_headers.rs` 演示百万字代码候选的成功/失败、旧对象身份和共享预算摘要。`tests/code_headers.rs` 验证全延迟表头与普通正文对照、363 组反引号组合、400 组范围模型、映射的逐标量/tile/预算读取、千层/百万字、4096 旧 opener/对象/列、缓存与合并通知；`tests/chunking.rs` 覆盖全部 UTF-8 切点和逐前缀快照。首管道前已经闭合的跨行代码映射仍保留 TODO。

**未闭合数学的探测**：顶层段落或同容器正文的 `$` / `$$` 候选可参与晚管道探测，可伴随旧强调栈和未闭合链接/图片标签。首管道前继续即时维护原美元串和转义状态；数学内容仍为原文，不提前发布单元格中的链接、数学或脚注。若旧数学在首管道前实际闭合，立即发布稳定 MathId、原始内容包络和范围失效，并取消当前行探测；即使随后开启新数学也不恢复探测。已有对象不会被撤销或重编号。

若确认表格，旧数学从未闭合，因而首单元格前缀没有已发布的行内效果。复用代码候选的固定大小探测标志，丢弃旧 Inline 及仍待定的美元串（不结算并发布跨行对象），再在已有 RowParser 扫描中独立消费当前表头。当前行对象只在此时发布一次，既有 MathId/LinkId/脚注编号保持。失败、无管道、EOF 或块中断继续原候选与未消费后缀；失败分隔行按原块规则处理，旧数学可随后正常闭合。候选额外状态 O(1)，无正文/栈复制；成功只额外消费互不重叠的当前表头，失败不重放，实际计数满足 `inline_bytes <= source().len() + table_bytes`，累计 O(n log n)、空间 O(n)。确认/回退仍不承诺单次硬预算。

容器内数学现沿用同一隔离/取消规则，Quote/Item 中的跨行内容读取见[容器跨行数学内容](#容器跨行数学内容)。`examples/math_headers.rs` 演示百万字成功/失败、稳定 MathId 与预算读取；`tests/math_headers.rs` 覆盖 338 组定界/转义模型、400 组表格范围、待定美元串、4096 个旧 opener/对象/列、千层容器边界、缓存/合并通知及共享预算，UTF-8 全切点和逐前缀快照在 `tests/chunking.rs` 验证。

```rust
use mdtui3::{Document, MathId};
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

**未闭合链接/图片标签的探测**：顶层段落或同容器正文继承 `[...` 或 `![...` 时记录旧左括号的源码位置，继续即时消费原标签及本行语义。沿用单括号候选方言：新 `[` 替换旧标签，`<` 丢弃标签并尝试自动链接；这些操作不发布旧标签对象。确认时只清除仍与记录位置相同的旧标签，保留当前行新标签、待定后缀、强调/代码/数学以及已经发布的 LinkId/MathId/脚注编号；首单元格前缀直接复用。旧图片的 `!` 保持原文，不能作用到新行标签或后续单元格。

如果首管道前的未转义 `]` 关闭了旧标签，立即取消本行探测，即使标签超过引用长度上限、尚未分配 LinkId，也必须取消：后续 `(destination)` 仍可能完成跨行链接。闭合后重新出现 `[` 不恢复探测。当前行新标签的闭合不触发取消。未闭合标签可伴随旧强调或代码/数学；任一旧候选实际闭合都取消探测，继承代码/数学尚未闭合的成功路径仍独立消费当前表头。

失败、无管道、EOF 或块中断沿用原候选和未消费后缀，前序换行只提交一次；不回扫标签、不重新发布对象、不重建符号索引。标签来源仅一个固定大小偏移，普通路径每个字节消费一次；仅叠加继承代码/数学时沿用其已证明的一次当前表头重读。累计 O(n log n)、空间 O(n)，确认/回退不承诺单次硬预算。容器跨行标签现沿用同一隔离/取消规则，去前缀表示见[容器跨行链接与图片标签](#容器跨行链接与图片标签)；已闭合跨行语义的撤销及多行目标/title 继续保留 TODO。

```rust
use mdtui3::{Document, LinkId};
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

`examples/link_headers.rs` 演示百万字成功/失败、稳定 LinkId 与共享预算摘要；`tests/link_headers.rs` 覆盖 200 组替换/转义模型、400 组表格范围、代码/数学组合、千层容器边界、百万字、4096 个旧 opener/对象/列、缓存及合并通知。UTF-8 全切点和逐标量中间快照见 `tests/chunking.rs`。

`examples/inherited_headers.rs` 演示前序硬换行隔离、百万字即时消费、失败恢复与成功摘要；`tests/inherited_headers.rs` 对照全延迟表头语义，覆盖旧对象与块身份、软硬换行、失败/EOF/Setext、原候选不回归、千层/百万字、4096 对象/列、400 组范围模型和共享预算。

`examples/long_headers.rs` 演示百万字长表头的迟确认与共享 Budget 摘要。测试覆盖 117649 组摘要语法对照、全 UTF-8 切点/逐前缀、千层引用和 250 层列表、百万字成功/失败、4096 列稀疏表、400 组范围模型及布局/折叠组合。

**长分隔行的流式规则**：仅紧邻有效候选表头时启用（包含上述长表头）。到达分类上限先只读匹配已有容器路径，再把分隔行前缀喂给状态机；没有完整路径、兄弟条目、新嵌套前缀或 lazy 行不进入探测。单元格的连字符、首尾冒号及 ASCII 空白可以任意长，首个管道也可以在窗口之后；保留一个单元格状态和至多表头列数个对齐值，不复制候选源码。短行保持原有优先级。长行在 LF 或 EOF 才检查最终列数并发布表格；CRLF 跨分块有效。探测期间表头和分隔行原文可见，分隔行尚无块/表对象，容器与章节的已解析末端保持在表头；确认后一次更新范围并隐藏整条分隔行。

**失败与复杂度**：非法字节、空单元格、多余列或 EOF 列数不足只触发一次回退。块分类只重用原来最多 1 KiB 的前缀及不完整行规则，不把长尾重新当成完整 Setext、定义、容器标记或新表头；延迟的长尾交给普通流式路径一次，此后只消费新增字节。`table_candidate_bytes` 对每个探测字节最多计费一次；成功不重读长分隔行，失败的长尾只有一次普通消费。这里有界的是分类回退窗口和重试次数，**不是失败调用的硬字节/时延上限**：单次晚失败可能消费整个延迟尾部，累计仍 O(n log n)、空间 O(n)。旧强调/代码/数学或链接/图片标签已在首管道前闭合的晚管道表头，以及其它块分类上限继续留在 TODO。

```rust
use mdtui3::{Document, TableId};
let mut doc = Document::new();
doc.append("> a|b\n> |")?;
doc.append(&"-".repeat(4096))?;
assert!(doc.tables().is_empty());
let update = doc.append("|-|\n")?;
assert_eq!(update.new_tables, 0..1);
assert!(doc.table(TableId(0)).unwrap().delimiter.len() > 4096);
# Ok::<(), Box<dyn std::error::Error>>(())
```

`examples/long_delimiters.rs` 演示百万字分隔行以及共享 Budget 的表头/正文摘要；摘要按行索引直接跳过分隔行，旧布局、折叠与合并 Update 的失效语义保持。

`TableRow::complete` 区分完整行与正在增长的行。当前行的最后一个空单元格可能在确认尾部竖线后被移除；先前完整行保持稳定。`source_width_hint` 是已完成单元格的最大原文字节数，单调增长，**不是终端字符宽度**。渲染器需要结合窗口预算、字体/Unicode 宽度、padding 和内容换行决定最终列宽；列宽提示不要求读取旧单元格。提示变大时，`Update.tables_changed` 通知具体表格，同时 `dirty` 覆盖该表范围，旧高度/检查点按现有接口惰性失效。

进入表格视口时先定位行，再读取可见列的单元格；超长单元格仍按片段和检查点恢复。表格一行的显示高度由各单元格排版高度的最大值决定，渲染器应约定显示行归属于哪个源码片段，把总行高回填一次，不能把每列高度相加。标题行、边框、横向滚动、列宽冻结/修正策略属于后续渲染器工作，见 [TODO](TODO.md)。

任务列表通过 `BlockKind::ListItem { task: Some(TaskMarker { checked, source }), .. }` 提供状态；识别列表项开头的 `[ ]`、`[x]`、`[X]`，标记后需要空白或行结束。任务标记不会成为引用链接，不会污染符号表。渲染器据此绘制复选框；当前解析器不修改原文中的勾选状态。

图片复用链接目标解析，但 `link_info(LinkId)` 的 `LinkKind::Image` 与普通链接明确区分，保留 alt 区间和完整语法区间。支持直接目标、引用式图片、延迟定义。可以先把 alt 渲染成链接；空 alt 图片也会保留对象，用 `links_in(visible_span)` 在 O(log L + 结果数) 内查询，避免因没有可见 label run 而遗漏。图片解码和绘制未实现。

链接和图片支持裸目标、`<含空格的目标>`，以及双引号、单引号、括号包围的 title；目标和 title 之间需要空格或 tab。`LinkInfo.title` 保留直接链接的 title 区间；`link_title(id)` 同时处理直接链接和延迟定义的引用 title。直接链接和引用定义共用单遍状态机，返回的字符串仍保留反斜线转义，不分配解码副本。规则参考 [CommonMark 链接](https://spec.commonmark.org/0.31.2/#links)，当前仅支持单行目标和 title。

`Style::SOFT_BREAK` / `Style::HARD_BREAK` 标记继续同一行内序列的 LF，包括同一容器的显式正文续行。两个以上行尾空格或一个未转义的反斜线产生硬换行，普通换行为软换行；CRLF 跨 append 也有效。为避免把块末换行误判为行内换行，直到后续同序列内容到达才确认，并通过 `dirty` 使旧片段失效；序列末和代码内部不加这两种标记。继续行前的尾部空格及硬换行反斜线隐藏，源码仍可寻址。

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

`doc.headings()` 提供文档顺序的 `HeadingIndex`。每个 `Heading` 包含稳定 ID 对应的块、容器作用域、级别、整个标题的源码区间和不含标题标记的内容区间。标题文字不复制；超长 Setext 标题也只保存区间。`at_level(level)` 直接返回该级别的 ID 列表；`next(offset, max_level)` / `previous(...)` 查找严格位于指定源码位置之后/之前、级别不超过上限的标题。`Update.new_headings` 给出新加入的 ID 范围。长 ATX 标题的区间会随追加更新；全局索引包含上述方言支持的容器内标题，结构关系遵循确切容器作用域。

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

章节/代码块折叠、隐藏目标定位、源码锚点恢复、文内 slug 及跳转历史见下方接口。后续的目录及更多折叠对象见 [标题导航与折叠联动规划](TODO.md#标题导航与折叠联动规划)及[折叠支持规划](TODO.md#折叠支持规划)。

## 标题显示名称检索

`HeadingSearch::new(&doc, query, max_level)` 只搜索标题索引中各标题 `content` 的非 `HIDDEN` 显示文本，复用实体解码、代码 span 规范化和源码映射。`max_level` 与标题导航一致：选择级别不超过上限的标题，大于 6 按 6 处理，0 不选择标题。空查询返回 `InvalidRange`。匹配区分大小写，保留重叠命中，不做额外空白处理、Unicode 归一化或模糊匹配；标题之间独立匹配，不串接名称，包含已识别的容器内标题，不读取正文或代码块。

```rust
use mdtui3::{Budget, Completion, Document, HeadingId, HeadingSearch, WorkLimit};
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

`cargo run -r --example heading_search` 演示级别过滤、逐帧搜索、复用输出缓冲区及跳到折叠内命中。`tests/heading_search.rs` 覆盖重复/空标题、跨 run/tile 和长查询、各预算维度、身份/过期、流式前缀/切点、固定种子朴素模型、百万字正文/Setext 和级别过滤跳过测试。

## 文内 slug 与 fragment

`HeadingAnchors::new(&doc)` 在 O(1) 时间创建当前 revision 的空索引；`advance` 按共享 Budget 读取已索引标题的非隐藏显示内容，包括容器内标题，忽略正文和代码块。不会在 `append`、Setext 确认或构造任务时扫描旧标题。`indexed_headings()` 报告完成数量，`get(&doc, id)` 为已完成项返回借用的 `HeadingAnchor`，未处理的合法 ID 返回 `None`，非法 ID 返回 `InvalidRange`。

slug 方言明确采用以下规则，不宣称兼容某个平台的完整 slug 算法：Unicode 字母/数字用 Rust `char::is_alphanumeric` 选择并用 `to_lowercase` 小写化；保留 `_`、`-`；每个 Unicode 空白标量变成一个 `-`；删除其余字符。不额外裁剪或合并空白，不做 Unicode 归一化、音译或 emoji 保留；小写映射产生的组合字符保留。输入先经过既有显示语义，因此隐藏标记/链接目标不参与，实体和代码 span 使用显示值。空结果用 `section`。按源码顺序分配全局唯一名称，依次尝试 base、base-1、base-2 等，跳过已经占用的名称。例如 `A`、`A-1`、`A`、`A-1` 分别得到 `a`、`a-1`、`a-2`、`a-1-1`；空标题也参与同一消歧规则。

```rust
use mdtui3::{Budget, Completion, Document, FragmentStep, HeadingAnchors, HeadingId, WorkLimit};
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

`fragment` 只接收以 `#` 开头的 URI fragment，O(1) 借用输入和索引；完整 URL 的拆分以及 Markdown destination 中的实体/反斜杠解码由调用方完成。解析百分号转义一次，校验解码后的 UTF-8，然后按字节精确匹配；`+` 保持字面值，大小写敏感，不再对 fragment 生成 slug。非法转义、非法 UTF-8 或缺少开头 `#` 返回 `InvalidRange`；即使已知名称不存在，也继续按预算校验剩余编码。`#` 表示源码 0，`heading: None`；空标题通过 `#section` 等名字寻址。未完成索引返回 `IndexIncomplete`，调用方释放 lookup、推进索引后重新查询；完整索引中的缺失目标在流式文档返回 `Unresolved`，在 finish 后返回 `NotFound`。完成的查询可免费重复读取。

标题 slug 和目标在首次 `finish` 前一律标为 `provisional`，包括已经局部封口的标题：长 ATX 继续增长、行内晚闭合、晚到引用定义和 Setext 转换都可能改变显示文本及消歧结果。索引、lookup 和输出绑定文档身份/revision，追加或首次 finish 后须丢弃并显式重建；错误文档/过期任务在扣费前返回 `WrongDocument` / `StaleContinuation`，空追加和重复 finish 不失效。finish 后结果固定。库不自动维护跨 revision 的 slug；反复显式重建的总成本由调用方调度，不能当作跨追加增量索引。稳定的 `HeadingId` 与源码目标可继续用于现有导航/书签，但旧 slug 到该目标的对应关系不能沿用。

标题初始化和消歧候选检查各消费 1 query / 1 object；显示读取沿用 `DisplayCursor` 计费（包含隐藏语法和重复前瞻），每个规范化标量另消费输入 UTF-8 字节数与 1 object，每个输出 trie 字节消费 1 byte / 1 object。fragment 每个原始字节或 `%HH` 分别消费 1 或 3 bytes、1 object，最终查询消费 1 query / 1 object；非法编码也计费。两类任务每帧 6 bytes、1 query、1 object 均可持续推进，长标题不会截断或退化成摘要哈希。目标包含精确标题起点，可交给 `FoldNavigation::Reveal` 展开必要路径；索引和查询自身不改变折叠、高度或布局。

索引以字节 trie 保存名字，每个节点至多 256 条有序边，避免长字符串比较和哈希碰撞；重复 base 的后缀计数单调推进，后缀候选从 base 节点开始，不重复遍历长前缀。设 S 为标题内容源码总字节、H 为标题数，单 revision 构建为 O((S + 1) log(n + 1) + H log(H + 1))，空间 O(S + H)。失败的数字候选已属于旧标题，且由唯一的 base 产生，因此总失败探测 O(H)；连续数字的 trie 前缀共享，后缀总节点 O(H)。`AnchorSlug` 只保存借用的 base 和整数后缀；显式 `Display`/`to_string()` 由调用方承担 O(slug 长度) 的格式化成本。借用结果 O(1)，长度 m 的 fragment 查询总计 O(m)、任务空间 O(1)。String/Vec/树节点的分配成本按摊还计，不承诺 allocator 的硬时延。

`cargo run -r --example anchors` 演示小预算索引、碰撞、晚到中文目标、百分号 fragment 和折叠导航。测试覆盖所有 UTF-8 单切点、长 ATX 逐前缀、显示语义、Unicode、错误身份/过期与各预算维度，包含固定种子模型、百万字 Setext/长 fragment 和 4 万标题消歧的工作量与存储检查。链接点击和目录 UI 继续留待接入阶段。

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

`HeadingIndex` 提供 `current`、`parent`、`first_child`、`next_sibling`、`previous_sibling`、`section` / `section_end` 和严格同级前后查询，对应动作已接入 `Navigator`。跳级标题按最近更浅级别确定父节点，兄弟不等于同级。章节末尾可随追加增长，查询不读取标题文本；父子/兄弟和范围遵循上文容器作用域，`current` 退出容器后恢复外层。

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

`FoldView` 为每个视图保存独立的收起意愿、高度和检查点。当前支持顶层及容器内标题章节、围栏和缩进代码块、表格及显式 Quote/List/Item 容器：章节保留标题，正文及子章节变为一个占位；代码块、表格和容器整体变为占位，表格保留表头源码作为摘要。空章节不生成占位。父节点展开后恢复原来的子节点状态，源码、解析对象和 `Style::HIDDEN` 不变。

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

`Document::fold_candidate` 按稳定 `FoldId` 查询整段范围、隐藏正文、父节点、摘要源码及开放状态；`fold_candidate_at` 查询当前位置的最内层候选。`fold_candidates(span)` 二分定位范围，归并枚举开头块与范围相交的候选以及起点在范围内的容器；同起点先按 ContainerId 发出外到内容器，再发出叶块。其 `advance` 与视口共享 Budget，每检查一个块（包括非候选）或一个容器消耗一次 query 和 object，耗尽可续，完成返回 `Covered`。游标绑定文档和 revision；范围之前开始的祖先通过 `parent` 查询。摘要用 `summary_text` 限定 UTF-8 字节数并计费，不统计正文行数、任务数或复制长标题。`hidden_at` 按预算返回 `FoldLookup::{Visible, Hidden, BudgetExhausted}`，隐藏结果是最外层遮蔽者，适合占位命中和源码定位。

`LayoutPass::folded` / `advance_folded` 接收 `FoldLayoutSink`，复用已有有预算排版循环。每个可见占位只调用一次 `placeholder`，传入稳定 ID、隐藏源码、摘要区间及归属 tile；接入方将占位作为块边界绘制并在 `end_tile` 中计入其高度、重置必要的换行状态。回调只获得元数据，若要读取摘要，应另用共享 Budget。占位高度全部归属隐藏起点所在 tile；同一 tile 可有多个占位、标题和后继正文。隐藏内部的文字、图片和子占位不会交给回调。该接口仍使用 `RenderRun`，显示实体解码、终端宽度及真实软换行由接入方处理。完整示例见 `examples/folding.rs`。

高度树独立保存基础高度/有效性与区间遮蔽计数。`height`、`prefix_rows`、`locate` 和视口查询返回折叠后的高度，`base_height` 返回遮蔽前的储存值；完整隐藏的内部 tile 无须逐个清零或恢复。每个占位及末端边界 tile 保持可检索，再按预算精确测量。单节点操作为 O(log H + log C + log F + log T)，顶层标题祖先最多六级，H/C/F/T 分别是标题、容器、已收起节点和 tile 数。父子交替收放、失效、基础高度变化、树增长及 O(1) resize 保留嵌套遮蔽。重复操作不保存历史快照，新增视图状态 O(F + C + H)，容器与内层章节稀疏索引保留已访问位置，分别受 O(C) / O(H) 上界约束；高度树仍为 O(T)。

`set_collapsed` / `toggle` 返回源码变化范围和视图版本；从受影响边界起惰性过期后续高度与检查点，旧异步测量和布局继续任务会被拒绝。`view.measure` 验证本视图的测量及检查点依赖；`insert_checkpoint` 同时使下游布局失效。`reset` 保留折叠意愿与遮蔽，只重置基础估计和检查点。保留的高度仍可能是过期估计，展开后仅测量请求窗口；选择远处 tile 时默认局部近似，不能承诺精确绝对行号。`folded_from_checkpoint` 只接受本视图的有效起点及指定字节距离内的恢复，接入方恢复 sink 状态，恢复步骤也使用同一 Budget。

每次 append/finish 或连续 Update 批处理后调用 `view.apply`，然后才能查询。对已有折叠，更新检查最多六个开放顶层章节和一个开放代码/数学/表格块；开放容器与内层章节共享一次尾部遮蔽更新；新标题和容器关闭各检查相关作用域的至多六个末尾章节，另检查一个可能增长的旧标题。开启默认收起时还处理新增标题/块/表格/容器的元数据，不枚举历史折叠或开放容器祖先链。新 tile 在 apply 返回前继承遮蔽。开放容器/内层章节/代码/数学/表格的未分类行前缀暂属该范围，闭合/下一块确认后可以收回；Setext 晚确认也能缩短前一章节的范围。普通源码追加不会读取旧正文。重复应用不清掉新测量，错误文档或不连续更新在修改视图之前被拒绝。

折叠导航、锚点恢复、有预算批量操作、临时展开和摘要读取见下文。目录 UI、摘要占位绘制、手工交叉范围继续见 [TODO](TODO.md#折叠支持规划)。

### 整表折叠

`FoldId::Table(TableId)` 把整张已识别表格（表头、分隔行、正文）替换为一个占位，`summary` 指向第 0 行的原始源码。宿主可以用 `summary_text` 读取有界原文前缀，或用 `DisplayExcerpt` 读取显示摘要；两者都需要自己的共享预算。列分隔符、对齐、边框和实际表头布局仍由渲染器处理，摘要接口不会自动拼接单元格或测量全表。仅含表头和分隔行的表也产生一个占位。

```rust
use mdtui3::*;

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

候选查询、切换及更新均不枚举行/列/单元格；排版直接跳过隐藏正文，占位高度只归属表格起点 tile。`examples/folding.rs` 演示百万字单元格收起后追加新行、读取表头前缀及有预算排版。前几行有界摘要读取及连续行范围折叠见下文；表格绘制仍是后续工作。

### 容器折叠

`FoldId::Container(ContainerId)` 支持整个 Quote、List 和 Item，包括单行、空内容、任务条目及嵌套容器。隐藏范围覆盖完整物理行及前缀，替换成一个占位；`summary` 借用整个范围，调用方通过 `summary_text` / `DisplayExcerpt` 限量读取，不复制正文或统计条目数。原始 `Container::source` 保持已分类范围；开放折叠候选暂包含未分类尾部，兄弟/外层行确认或 finish 后收回到精确边界。稳定身份和子折叠意愿在增长、关闭和 resize 后保留。

```rust
use mdtui3::*;

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

`container_index_visits` 计索引读写的节点访问量，`container_index_nodes` 计保留节点数；反复切换不新增历史快照，节点数不超过完整二叉索引的 O(C) 上界。`tests/container_folding.rs` 验证千层开放链、百万字正文、4096 个关闭兄弟、同起点父子、有预算深层导航、UTF-8 全切点/逐前缀和独立区间/高度模型。`cargo run -r --example container_folding` 展示流式追加、父预览、子意愿及按帧恢复单元格源码锚点。类型/任务策略见下方；实际摘要/容器排版和终端 UI 继续独立推进。

### 容器内章节折叠

`FoldId::Heading` 包含显式 Quote/Item 内的 ATX 和多物理行 Setext。保留标题全部物理行（含容器前缀），正文及子章节替换为一个占位；空正文没有占位。范围止于同一确切作用域的同级/更高级标题或容器边界；嵌套作用域的标题不截断外层。`HeadingIndex::section` 保持已分类范围，开放的折叠候选暂包含未分类尾部，边界确认时收回。Setext 晚确认可回退前一章节末端；长 ATX 标题增长会移动隐藏起点，标题文字不被自己的折叠遮住。

折叠父链按包含关系组合章节与容器：章节先找同作用域父标题，否则挂在所属容器；子容器先找其结构父作用域的当前标题，否则挂在结构父容器。表格同理。原始 `Container::parent` 和 `HeadingIndex::parent` 不变。`fold_candidate_at` 返回包含当前位置的最内层候选，标题仍可作为导航目标；Reveal 只展开遮住目标的祖先，不改变目标章节自身正文意愿。All/Subtree/Siblings/AtLevel/Focus、预览、源码锚点和默认 `heading_levels` 直接覆盖内层章节，默认只初始化新 ID。

```rust
use mdtui3::{Document, FoldView, FoldId, HeadingId, ContainerId, TableId};
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

`tests/container_sections.rs` 用独立区间、父链和高度模型验证嵌套边界、同起点、同 tile、流式逐前缀/UTF-8 全切点、合并更新、批量、预览与表格组合。500 个开放章节和百万字正文下逐字追加没有章节索引写入；150 层交替章节/容器的导航每帧展开一个节点；4096 个关闭章节验证对数查询和节点复用。`cargo run -r --example container_sections` 演示共享尾部、预览保留子章节及精确源码恢复。实际摘要/容器排版、终端 UI 及未实现容器方言仍保留 TODO。

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

每个有界步骤消费 1 query、1 object：解析动作、检查可见性并展开/跳过一个遮蔽范围、或查询最终估计行。祖先可能包含任意深度的已识别容器；一次步骤通过索引只定位并展开一个最外层遮蔽者，含固定数量的索引操作，不读取正文或标题文本。只给上述最小预算也可推进。耗尽返回 `BudgetExhausted`，保存目标或已经跳过的区间；`Covered` 表示动作结束，`target: None` 表示无匹配目标。`change` 是**本次调用**已展开区间的包络和最新视图版本，已同步更新高度/检查点失效；取消或失效不会撤销已完成的展开。任务存储 O(1)，Reveal 按实际打开的祖先数量计费，可跨帧恢复；VisibleOnly 总工作按实际跳过的折叠区间数计费，每步 O(log H + log C + log F + log T)，不会逐个检查被隐藏的标题。

任务绑定文档身份/revision、视图身份及折叠版本，文档追加/首次 finish、外部折叠操作和 reset 后拒绝旧任务；错误返回前不消费预算或修改视图。自身展开更新其版本，期间的正常高度测量不使任务失效；最终定位使用最新高度。完成后重复调用返回原结果且不重复展开，返回行仍是当时的估计。任务不改写原 Navigator；调用方只应用当前有效任务的结果，新的用户导航应丢弃旧任务。用 `Navigator::apply_target` 应用结果，它在 source 为 None 的普通滚动后清除旧的精确源码目标。远处定位、同 tile 多标题和折叠占位仍由局部 `LayoutPass` 在共享 Budget 下细化，随后再 reconcile；此接口不排整篇前缀，也不承诺精确绝对行号。

`FoldAnchor::new(&doc, source)` 保存文档身份和精确源码位置，`resolve(&doc, &view, viewport_rows, &mut budget)` 返回当前投影。隐藏时 `hidden_by` 标明最外层折叠，`target.source` 指向其占位起点；可见时返回原位置。保留同一个 FoldAnchor，在收起/展开、append、Setext 晚确认或 resize 后重新 resolve 并应用目标即可恢复；不要用占位位置覆盖保存的原锚点。每次 resolve 消费 1 query、1 object，预算不足返回 None，不改变折叠。锚点允许 EOF，追加后原字节偏移保持不变；不绑定 revision，错误文档或未同步的 FoldView 会被拒绝。它保存源码而非精确屏幕行/列，选区仍使用原源码范围。示例 `examples/folding.rs` 演示占位投影和分帧恢复到 120 万字节正文内的目标。

## 批量折叠与新节点默认设置

`FoldBatch` 提供当前 revision 上的显式命令：`All` 展开/收起全部候选，`Subtree` 包括根及全部后代，`AtLevel` 只修改严格等于指定级别（1–6）的标题，`Focus` 展开目标及祖先并收起所有其他候选。聚焦会修改后代意愿；仅想露出目标并保留无关状态时使用 `FoldNavigationPolicy::Reveal`。

`Siblings { target, collapsed }` 展开/收起与目标具有相同 `FoldCandidate::parent` 的其他候选，保留目标、祖先和各节点后代的原有意愿。兄弟可以是不同级别的标题、代码块、数学块或表格；没有父节点的候选互为根层兄弟，包括首个标题之前的代码块、数学块和表格。目标已被祖先隐藏时仍可操作，但不会自动展开祖先；没有兄弟时完成且无变化。该命令只修改已有节点，不改变新节点默认值。

```rust
use mdtui3::*;

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

可运行示例：`cargo run -r --example fold_batch`。

### 容器深度默认策略

`FoldDefaults::container_depth: Option<usize>` 是包含式阈值：新容器的 `Container::depth >= threshold` 时初始化为收起。`None` 关闭策略，`Some(0)` 与 `Some(1)` 都包括全部根容器，`Some(usize::MAX)` 也无需加减运算。深度按结构父链计数：根为 1，Quote、List、Item 各占一层；例如 `> - text` 的 Quote/List/Item 深度依次为 1/2/3，不按视觉缩进或列表标记个数计算。标题与表格不增加容器深度；该深度阈值自身不区分任务状态，与下节 `containers` 按任一命中组合。

```rust
use mdtui3::*;

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

启用策略时每个新容器检查一次已存深度，计入 `default_visits`，无论是否达到阈值；禁用时不访问这些元数据。设 N 为新增容器数，K 为达到阈值的个数，初始化额外成本为 O(N + K(log H + log C + log F + log T))，只增加 O(1) 策略字段，折叠状态沿用 O(F + C + H) 空间（含内层章节索引）。没有新容器的普通追加不检查历史节点或开放祖先，仍只更新一次共享尾部。apply 按新增对象/开闭事件消费，不承诺任意大更新的单帧时延；查询、批量操作、预览和排版继续共用 Budget。测试覆盖千层/百万字逐字追加、4096 个历史兄弟、UTF-8 全切点/逐前缀及 240 次流式策略/显式操作的区间与高度模型。`cargo run -r --example container_defaults` 演示预览中的新子节点、显式展开优先及批量展开后新条目的默认收起。

### 容器类型与任务完成状态策略

`ContainerFoldFilter` 的五个布尔字段按并集筛选：`quotes` 选 Quote，`lists` 选所有有序/无序 List，`items` 选所有 Item（含任务），`completed_tasks` / `incomplete_tasks` 只选有效状态为已完成/未完成的任务 Item。全部为 false 时不选任何节点；普通条目不会被任务字段选中。List 与 Item 是独立节点，不根据子任务聚合列表完成状态，也不隐式修改后代意愿。

`FoldDefaults::containers` 与 `container_depth` 按“任一命中即默认收起”组合。字段 false 表示不增加选择，不是强制展开；例如 `items: true` 会选中所有任务，不受 `completed_tasks: false` 排除。默认规则只消费新增容器，任务状态取源码 `[ ]`、`[x]`、`[X]`，含晚识别、EOF 与合并更新。已有节点、显式展开、导航展开、预览保存的意愿均不重置；默认变化也不回溯已有任务。

`view.task_checked(&doc, id)` 查询该视图中的有效状态，非任务容器返回 `None`，无效 ID 返回 `InvalidRange`。`set_task_checked(&doc, id, bool)` 仅接受任务 Item；只保存与源码不同的覆盖，设回源码值立即移除。它不修改 Document、任务标记及样式、其它视图、折叠意愿或预览；渲染任务控件时由宿主读取有效状态，通用源码/显示读取仍表示原文。修改返回 marker 的 `FoldChange` 区间和新视图版本，并惰性失效该 tile 起的测量/检查点、旧批量/导航/排版任务；相同值为无操作。覆盖跨追加、finish、reset 保留。状态改变后要重新折叠，显式运行下面的批量命令。

```rust
use mdtui3::*;
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

`tests/container_folding.rs` 与存储测试覆盖 32 种类型/任务筛选 × 4 种深度策略、UTF-8 全切点/逐前缀、CRLF/失败 marker/EOF、320 轮流式区间/高度模型、千层/百万字及 4096 历史条目；共享预算、独立视图、旧任务/测量拒绝、预览/导航和非容器折叠均有回归。`cargo run -r --example container_policies` 演示视图内完成任务、分帧筛选与预览/导航恢复。任务控件、列表完成率聚合和终端 UI 未由此接口实现。

## 表格连续行范围折叠

`FoldView::insert_table_rows(document, table, start..end)` 注册一个视图内的固定正文行范围，返回 `FoldId::TableRows(TableRowRange { table, start, end })`，初始展开。行号沿用 `Table::row`：第 0 行是表头；仅接受 `1 <= start < end <= rows.len()`，end 不含。表头、分隔行和范围外正文保留；范围内完整物理行（含容器前缀）合并成一个占位。摘要指向首个选中行，读取仍受 `summary_text` 或 `DisplayExcerpt` 的字节/帧预算限制。

```rust
use mdtui3::*;
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

`cargo run -r --example table_row_folding` 演示百万字末行、固定行范围、整表状态恢复及后续行可见性。真实表格边框、列宽、软换行、占位绘制，以及自动随新行延长的尾部策略仍由后续工作推进。

## 表格前几行有界预览

`TablePreview` 为整表折叠提供独立的摘要读取任务：`rows` 包含表头（第 0 行），跳过分隔行，最多读取前 N 个已索引行；`source_bytes` 与 `display_bytes` 是所有行共用的总上限，不逐行重置。源码上限包含行内隐藏标记，UTF-8 不截断标量；超长首个单元格也不能绕过上限。返回值保留行号、样式、链接和绝对源码映射，不合成列分隔、隐式空单元格、边框或省略号。`RowsEnd` 表示读完当前已索引行，`RowLimit` 表示仍有行未选入；字节上限返回 `SourceLimit` / `DisplayLimit`，不向后扫描确认余文可见。沿用 `DisplayExcerpt` 的停止语义：多个字节上限同时触及时，读取窗口可能影响报告哪个原因，但不影响输出前缀。

```rust
use mdtui3::*;
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

任务和输出绑定文档身份与 revision；追加或首次 finish 后返回 `StaleContinuation`，调用方按需重建，不自动重读历史行。未分类的 pending 尾部不属于已索引行；已索引但未完成的行按当前源码范围读取，可由 `TableRow::complete` 判断。错误文档/过期任务在扣费前拒绝，完成可免费重复读取。`cargo run -r --example table_preview` 演示收起百万字表格、追加新行后按小预算读取前两行。

## 临时折叠预览

`FoldView::set_preview(&doc, Some(id), &mut budget)` 暂时展开一个已支持的章节、代码块、表格或容器；传入 `None` 结束预览，切换 ID 则先恢复旧节点。每个视图同时只有一个预览。祖先和后代的折叠意愿保持不变，因此被父章节隐藏的节点不会因预览而自动露出；展开父章节的预览仍保留子节点占位。这里提供暂时展开整节点的视图状态，实际只排版当前窗口；按逻辑行/字节截断的摘要读取见下文。

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

开始、切换及结束各原子消费 1 query、1 object，支付最多两个节点的固定次数索引操作；O(log H + log C + log F + log T)，只额外保存一个 ID 和布尔意愿，O(1) 空间。不读取正文、不枚举覆盖的 tile、不保存历史快照。相同目标为无操作且不扣预算；预算不足返回 `BudgetExhausted`，保留当前预览和所有状态，可在后续帧重试。错误文档、未同步视图和非法候选在扣费及修改前被拒绝。预览变化返回旧/新隐藏范围的包络和最新版本，并使受影响的高度、检查点、测量和继续任务失效；即使节点原已展开或范围为空也增加版本。实际绘制继续使用共享 Budget 的剩余额度，或留待后续帧。

`cargo run -r --example folding` 演示父章节预览、子折叠保留及结束恢复。`tests/fold_preview.rs` 和折叠排版模型覆盖预算原子性、显式意愿、流式范围、独立视图和旧任务拒绝；百万字节点的切换不读正文，进入预览后的排版仍受窗口及字节预算限制。

## 跳转历史与书签

`Bookmark::new(&doc, source, viewport_offset)` 保存文档身份、精确 UTF-8 源码锚点、捕获时所在的章节/代码/表格 ID，以及目标行相对视口顶端的行偏移。源码应来自实际排版或精确跳转目标；普通滚动后先用 `Navigator::reconcile_source` 回填映射，不能把估计的 tile 起点当作精确返回位置。书签可单独保存，并通过 `Bookmark::navigation` 创建最小路径展开任务；可以用于同一文档的其他视图。追加、finish、Setext 晚确认和 resize 后仍以保存的源码位置为准，目标 ID 仅是捕获时的元数据。EOF 使用左亲和，新增内容不会移动旧 EOF 书签；重载后的新文档身份会被拒绝。

`NavigationHistory` 保存当前条目和线性的前进/后退分支。普通滚动用 `replace_current` 更新当前返回位置；新跳转成功后才 `push` 目标并截断前进分支。完全相同的书签是无操作，保留前进分支；相同源码、不同偏移或目标元数据则是不同条目。`navigation` 在历史边界返回 `None`，否则创建可暂停的 `HistoryNavigation`；完成后自动移动历史游标，宿主将返回的 `ScrollTarget` 应用到 Navigator。

```rust
use mdtui3::*;
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

捕获书签只做 O(log B + log H) 的索引查询，每个书签/任务占 O(1) 空间。历史共 O(J) 空间，J 为保留条目数；Vec 容量保留历史峰值，追加摊还 O(1)，扩容可能复制现有条目，不承诺追加历史的硬时延。`cargo run -r --example history` 演示返回折叠代码、追加与 resize 后恢复、前进和历史边界。`tests/history.rs` 包含固定种子 2000 次操作模型、同 tile 重复标题、普通滚动、分支截断、身份/过期/取消、流式语义变化和百万字隐藏路径验证。

## 定界数学核心

数学使用本库明确限定的美元符号方言，不声明与某个数学 Markdown 扩展兼容：完整且未转义的 `$` 串长度为 1 时打开 inline 候选，为 2 时打开 display 候选；遇到相同长度的串才闭合。长度大于 2 的串在外部为普通文字，在候选内为原始内容；不同长度不嵌套、不切分匹配。没有空白、数字或货币启发式，例如 `$5 and $10` 会识别 `5 and `。`$$` 在行内上下文单独出现是未闭合 display opener；独占物理行时按下方独立数学块处理。连续 `$$$$` 是文字，无法表示空行内公式；空白内容允许。

代码 span/代码块、链接目标及 title、引用定义、正在消费的自动链接等既有上下文先于数学。数学内容内不解析强调、实体、链接、代码或另一个数学对象；原始反斜线保留，一个反斜线使紧随的一个字节不参与定界，连续反斜线按奇偶处理。外部 `\$` 仍是既有标点转义，内部 `$a\$b$` 内容为 `a\$b`。外层强调/链接 label 可以包含数学，显示 run 可同时带这些样式/链接。

两种候选均可跨同一顶层段落的普通换行，但不跨块或表格单元格边界。行内 `$$…$$` 只是对象的展示类型提示；独占行定界另按[独立数学块](#独立数学块)处理。在行内上下文，空行、标题、Setext、表格、围栏等仍先按块规则分类，管道仍先按表格规则分列；ATX 标题限单物理行，Setext 可迟确认多物理行正文；容器正文支持同一确切作用域的显式/lazy 续行，原始 content 包含前缀，另提供去前缀读取（见下节）。表格中的 `\|` 不分列，数学内容区间仍包含原始反斜线。`MathContent` 返回原始 CRLF；通用显示读取仍采用既有 CR 隐藏规则。块分类的 1 KiB 前缀缓冲、表头候选的延迟解析仍适用；顶层及同容器未闭合数学与晚管道长表头的确认/失败规则见[表格](#表格任务列表与图片)。

只有闭合确认才分配稳定 `MathId` 和 `Math { kind, source, content }`，source 包含两端定界符、content 排除定界符；容器跨行对象的两者都是包含前缀的原始包络。确认时内容标 `Style::MATH`，定界符标 HIDDEN，并发布范围失效及 Structure 变更，即使调用方关闭样式尺寸失效，旧测量仍被拒绝。尚未确认时既有显示通道呈现原始内容；块结束/EOF 未闭合时保留原文、丢弃候选，不重放期间跳过的行内语法。美元串需等下一非美元字节或行内作用域结束才确定长度；流式尾部 `$x$` 在追加更多美元符号前不能提前发布对象。

`Update.new_maths` 只报告新确认 ID 范围，支持 merge、append_into/finish_into、空追加和重复 finish。确认后的区间不会因后续追加、Setext 转换或容器关闭而变更；`maths()` / `math(id)` 借用元数据。`maths_in(range)` 创建 O(1) 游标，第一次 advance 在扣费后用 O(log M) 索引定位重叠对象，后续逐项 O(1)，完整对象按 source 与窗口相交判断；空范围不输出。每次检查（含终止查找）计 1 query / 1 object，不读正文，预算不足时不推进，完成后重复读取免费。

`math_content(id)` 创建 O(1) 原始内容游标，每次 advance 最多读取 `TILE_BYTES` 个 UTF-8 字节，计实际返回字节和 1 query / 1 object。下一标量放不下时原子暂停，4 bytes 足以推进任意标量；与其它消费者共用 Budget。没有复制完整公式，也不执行 TeX 校验、布局或字体塑形。两个游标均借用文档，Rust 借用保证消费期间不能追加；释放后可用稳定 ID 建新游标。通用 `LayoutPass` 仍将数学作为带 MATH 样式的文字回退，实际数学对象布局由后续消费方在预算下接入；收起的章节/容器继续跳过其数学正文。

```rust
use mdtui3::*;
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

解析只增加一个 O(1) 候选；每个美元串只决议一次（`Work::math_steps`），所有正文只走既有单遍 `inline_bytes` 消费。闭合做常数次 O(log n) 区间标记，不查正文；额外对象存储 O(M)，跨行去前缀索引另计 O(G)（见下节），全库保持累计 O(n log n)、空间 O(n)。Vec 扩容及批量解析仍是摊还保证，不承诺单次 append 的硬时延。`tests/math.rs` 和现有分块/fuzz 快照验证方言、预算、失败输入、百万字晚闭合及索引模型；`cargo run -r --example math` 演示流式确认与逐帧原始内容读取。

独立数学块见[独立数学块](#独立数学块)；后续仍保留 `\(...\)` / `\[...\]` 等额外定界方言、TeX 解析/排版和实际对象回调接入；脚注的独立核心接口见[脚注核心](#脚注核心)。

## 独立数学块

顶层或显式 Quote/Item 内，完整物理行去除容器前缀后，至多三个前导空格、恰好 `$$`、可选 space/tab 和 LF/CRLF（或 EOF）构成定界行。第一行开启数学块，相同规则的下一行关闭；不接收 info、额外美元符号、转义定界或四空格缩进定界。`$$x$$` 继续按行内 display 数学解析。空数学块允许，空行也是正文。围栏/缩进代码中的 `$$` 保持代码字面内容；数学块能中断段落、lazy 正文和表格，正文中的 Markdown、代码围栏、表格、定义、美元串及反斜线全部保持字面，不产生行内对象。

定界行沿用含容器前缀的 `BLOCK_LOOKAHEAD` 分类窗口：短行等 LF/EOF，长行在窗口到达时一次分类；超长开闭候选保持普通正文或数学正文，不在长尾重试。已打开的数学块正文可无限追加。容器内只匹配已有的确切路径，不把正文标记解释为新容器；必须显式续接，缺失前缀、兄弟条目或作用域退出封存旧块，当前行再按外部规则处理。不存在数学块的 lazy 续行。EOF 未闭合也封存为数学块，不恢复成普通段落、不回放正文。

稳定身份是 `BlockId`，`BlockKind::Math { content, closed }` 保存当前内容包络与是否见到合法关闭行，`Block::state` 区分 Growing/Sealed。`source` 包括完整开闭物理行和容器前缀，`content` 排除整条定界行，保留正文前缀和原始 CRLF；空块的 content 为 opener 末端的空区间。未分类短尾还不属于已提交 content；长正文分类后只随新增字节增长。`new_blocks` / `changed_blocks` 报告新增、增长和封存，merge、空追加和重复 finish 沿用既有契约。它们不进入只含稳定闭合行内对象的 `maths()` / `new_maths`。

`math_block_content(BlockId)` / `math_block_payload(BlockId)` 均 O(1) 构造 `MathContent`，后者只去除匹配的容器前缀。逐步返回的 `MathChunk.source` 连续覆盖 content，正文借用原文、省略的前缀返回空 text；CRLF、空白、反斜线和实体拼写不变。每次最多 `TILE_BYTES` 个 UTF-8 源码字节，计实际字节与 1 query/1 object，前缀也收费；预算不足原子暂停。首次有效读取付费后 O(log G) 定位间隙，之后 O(1)，不分配或复制整段公式。通过 `block_at` 定位数学块，或在窗口内用有预算 `fold_candidates` 枚举结构；消费期间文档借用阻止追加。

```rust
use mdtui3::*;
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

实现复用围栏路径和叶块索引，不增加数学块 arena；每块固定大小元数据，每个省略前缀一个有序间隙。分类只读有界窗口，正文不走行内解析，同物理行百万字增长不增加间隙或祖先更新；闭合/增长的范围标记是 O(log n)，历史块不参与追加刷新。累计解析 O(n log n)、空间 O(n)，不承诺单次 append/finish 的硬时延。`tests/math_blocks.rs` 及分块快照覆盖 400 组源码/内容模型、240 次流式高度模型、千层引用、250 层列表、百万字、4096 历史块、预算与折叠；`cargo run -r --example math_blocks` 演示去前缀内容与导航分帧消费。

## 容器跨行数学内容

同一确切 Quote/Item 内的连续正文保留 `$` / `$$` 候选，支持显式、等价前缀及 lazy 续行。定界仍按完整美元串长度匹配，保留反斜线逐字节转义和 opaque 内容；前缀不参与定界，LF/CRLF 仍参与原始数学内容。空行、兄弟条目、内外容器切换、标题、代码、定义和表格单元格边界结束候选；失败或 EOF 未闭合不回放正文。多物理行 Setext 晚提升保留已确认对象并结束剩余候选。数学在首管道前闭合会取消长表头探测，未闭合则成功时丢弃旧候选、独立消费当前单元格，失败时继续原候选。

`math_content(id)` 继续读取完整原文；新增 `math_payload(id)` 返回同类型 `MathContent` 游标，只去除未送入数学解析器的容器前缀。它保留 CRLF、首尾空白、实体拼写和反斜线，包括表格中显示层会隐藏的 `\|` 反斜线；不等同于过滤通用 HIDDEN 样式。`MathChunk.source` 连续覆盖原 content 包络，正文 text 精确借用该区间，省略的前缀片段则 text 为空。拼接 text 得到数学正文，保留 chunk 即可映射回原始源码。

两种读取都受共享 Budget 约束：每步最多消费 `TILE_BYTES` 源码字节，计 1 query / 1 object 和实际消费的源码字节数，空文本前缀也计费；四字节预算足以推进任意标量，预算不足时位置和预算均不变。完成后读取免费。构造 O(1)，第一次成功推进用已计费的 query 二分定位前缀间隙，后续逐片 O(1)；游标借用文档、状态固定大小，无正文副本。

```rust
use mdtui3::*;
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

物理叶块和已确认 MathId/区间保持稳定，Update 报告新对象并使受影响的缓存、测量、检查点与旧搜索任务失效，折叠意愿保留。`tests/container_math.rs`、分块快照和内部存储测试覆盖 400 组源码映射模型、UTF-8 全切点/逐前缀、千层引用、250 层列表、百万字、4096 条 lazy 续行及历史对象/失败候选。收起容器的布局不读取数学正文；`cargo run -r --example container_math` 演示分帧映射读取。独立数学块见[独立数学块](#独立数学块)；TeX 排版、更多定界方言、已发布跨行语义撤销和终端 UI 继续保留后续。

## 脚注核心

支持 `[^label]` 与顶层或显式容器内的单物理行 `[^label]: body`。label 为 1..999 个 UTF-8 字节，由 `char::is_alphanumeric` 接受的 Unicode 标量、ASCII `_`、`-` 组成；区分大小写、不做 Unicode 规范化，不支持空白、转义、实体或其它标点。脚注与普通引用链接使用独立命名空间；`[^` 定义前缀保留给脚注，非法定义不会成为普通链接定义。这里只定义本库方言，不承诺其它 Markdown 脚注扩展的全部行为。

定义允许至多三个前导空格，冒号后可紧接正文，也可省略恰好一个 space/tab；其余空白原样保留。空正文有效。正文是原始单物理行区间，不解析 Markdown、数学或嵌套脚注；排除行尾 LF/CRLF，裸 CR 保留。后续缩进/空行不延续定义。第一个合法定义生效，后续同标签定义隐藏但不覆盖正文。顶层和容器内共用文档全局脚注符号，围栏和缩进代码优先；定义可以结束段落或表格。容器前缀、边界和晚定义规则见[容器内定义](#容器内定义)，多行、多段定义仍待实现。

沿用 1 KiB 块分类前缀：短行在 LF/EOF 分类，长行到达前缀上限时确认定义，后续正文单遍追加，无正文长度上限。`FootnoteDefinition { block, content, state }` 先为 Growing，在 LF/EOF 变为 Sealed；未到 LF 的 CR 暂属正文，后续 LF 到来时只调整尾端。引用在行内解析器读到 `]` 时确认，不等待定义、不转换成后续直接链接；未分类行仍暂时显示原文。`[^` 候选不跨物理行、块或单元格，失败、超限、EOF 未闭合时不回放已消费前缀，不分配符号或编号；拒绝字节继续按普通行内规则处理。

行内代码、数学、链接目标/title、引用链接后缀和自动链接等正在消费的上下文先于脚注。`\[^x]` 不创建引用，`![^x]` 沿用普通引用图片，`[text][^x]` 沿用普通引用链接；不支持嵌套链接 label 的特殊脚注语义，遵守已有单括号候选规则。脚注 label 自身不解析行内标记，外层强调仍可覆盖整个标记。标题、显式容器正文及表格单元格均可包含脚注引用。

`FootnoteId` 是按首次遇见引用或定义分配的文档内稳定符号，`FootnoteReferenceId` 是按源码顺序分配的独立引用对象 ID。`Footnote.number` 只在该符号首次确认引用时分配，从 1 开始；定义先到不占号，重复引用共享编号，未定义符号也占号。编号一经分配不变，晚定义与重复定义不会重排。引用只保存符号及完整源码区间；`resolve_footnote` 经符号 O(1) 解析到编号和定义快照，区分流式 Unresolved、EOF Missing、Defined。定义的 Growing/Sealed 与整个文档 EOF 独立。对象及快照只属于原文档，跨 revision 使用已复制快照时由调用方检查语义版本和 EOF 状态。

`Update.new_footnotes` / `new_footnote_references` 报告新增范围，`footnotes_changed` 报告编号分配、定义确认、增长和封口，可有重复 ID，支持 merge 和复用缓冲。每符号 revision 按新增定义源码字节和编号/开闭事件递增，与 append 分块无关；全局 `TileKey.footnote_revision` 保守失效。`DependencyKey` 按预算捕获并校验当前 tile 的脚注符号版本，未定义目标另检查 EOF，无关定义可保留直接语义缓存。晚定义/增长只用符号的 first_use 发出 Structure 范围变化，旧高度及后续检查点按既有区间协议失效，不枚举历史引用；关闭尺寸策略也不能忽略这类对象语义变更。首次 finish 对存在脚注引用的文档发布保守范围失效并改变全局脚注版本，保证 Unresolved → Missing 不漏通知；这一转换不逐符号发事件，消费者也需观察 `is_finished()`。

`footnote_references_in(range)` 构造 O(1) 借用游标，首个 advance 在扣费后 O(log R) 定位重叠标记，其后每项 O(1)。每个结果和终止查询各计 1 query / 1 object，不读取 label/body；空范围与完成后的重复读取免费。`resolve_footnote` 每次计 1 query / 1 object / 1 dependency，无效 ID 返回 None。`footnote_content(id)` 构造 O(1) 原文游标，advance 每次最多 TILE_BYTES 个 UTF-8 字节，计实际返回字节及 1 query / 1 object。所有接口共用调用方 Budget；不足时不推进且不扣费，4 bytes 足以读取任意标量。游标借用文档，消费期间不能追加；释放后可按稳定 ID 重建。

```rust
use mdtui3::{Budget, Document, FootnoteReferenceId, FootnoteTarget, WorkLimit};
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

解析器只保留一个 O(1) 行内候选；有界 label 使用 BTreeMap 确定性 O(log S) 符号寻址，编号/定义/增长均只更新当前符号。额外存储 O(S + R)，全库维持累计 O(n log n)、空间 O(n)，Vec 扩容仍为摊还保证。`Work.footnote_steps` 统计符号操作与定义增长/封口，`footnote_label_bytes` 统计实际 intern 的有界标签，正文与失败候选的检查分别计入已有 input/classified/inline 字节计数。两万次旧引用后定义只做三次符号步骤；百万字定义后逐字追加不回扫正文。窗口索引、分块/fuzz、预算、通知和缓存验收见 `tests/footnotes.rs`、`tests/chunking.rs`；`cargo run -r --example footnotes` 演示晚定义和逐帧正文读取。

通用显示与 LayoutPass 仍原样显示 `[^label]` 标记，并隐藏顶层及容器内定义；脚注编号上标、文末脚注区、定义正文 Markdown 排版、双向跳转/返回及对象回调仍待接入，读取核心不自动展开或排版脚注区。

## 明确支持的 Markdown 方言

- 段落与空行；ATX 标题；Setext 标题；分隔线。
- 顶层及显式容器内缩进代码、反引号/波浪线围栏代码；opening fence 长度 3..32，代码 info 保留源码区间，作用域与边界见[容器内代码块](#容器内代码块)。
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
- `[^label]` 引用、顶层及容器内单物理行脚注定义、稳定编号及有预算读取，方言与布局边界见[脚注核心](#脚注核心)。

取舍：

- 不是完整 CommonMark/GFM 实现。行内 `***` 等超过支持长度的强调 run 按原文处理；flanking 使用 ASCII 空白规则，单词内下划线不匹配。交叉闭合采用最近同类 opener，并丢弃中间未闭合 opener，不做回溯。
- 未闭合的代码 span 在块结束后保留为原文，期间跳过的其它语法不会重新解析。嵌套链接不支持：新 `[` 替换尚未闭合的外层 `[`。
- 已识别的引用立即显示 label，并隐藏引用语法；尚未定义时没有可跳转目标。定义到来后只增加目标。第一个有效定义生效，后续重复定义不覆盖。引用匹配的原始正文标签最多 999 字节（不含容器前缀），折叠 Unicode 空白并转小写，不做完整 Unicode case folding。
- URL 和 title 各最多 4096 字节，左括号后的链接后缀候选最多 8256 字节（包括空白和闭合右括号）。支持平衡括号和转义定界，但返回的 destination/title 是原文；不提供多行目标/title、URL 解码或安全过滤。后缀失败时保留原文及已有 shortcut 引用，不回扫后缀中的其它语法。
- 引用定义、Setext、分隔线、闭合 fence 必须在块分类前缀内完成。超长候选行按普通内容/代码处理；超长 opening fence 的 info 只保留已分类前缀，其余该行隐藏。超长标题末尾的 closing hashes 不再回看识别。
- 自动链接候选最多 4096 字节，URI scheme 为 2..32 个 ASCII 字符；email 按 CommonMark 0.31.2 的 ASCII 地址规则。失败或 EOF 未完成的候选不重放内部行内语法；新自动链接中止未完成的外层方括号候选，保持链接源码有序且不重叠。返回原始 URI，不做 percent encoding。
- 不处理 HTML、GFM 裸 URL 自动链接等额外语义。它们可能包含上述方言仍能识别的普通行内标记。数学仅提供上述定界核心，不解析 TeX。HTML 不在当前需求范围。
- 支持只追加，不支持任意位置编辑、撤回已有源码或移除定义。文档销毁会释放 O(n) 状态，这是一次性清理成本。

## 有界显示摘要

`DisplayExcerpt` 从指定源码范围读取非隐藏显示文本前缀，可用于代码块、章节正文及标题摘要。`ExcerptLimits` 同时限制源码前缀字节、输出 UTF-8 字节和 LF 分隔的逻辑行数；最后一行的 LF 包含在输出内，不添加省略号，空输入不产生空行。源码上限包含隐藏语法，防止巨型围栏 info 或隐藏目标拖延摘要；输出上限防止超长首行绕过行数限制。UTF-8 截断保留完整标量，行内实体/代码空白使用现有显示语义，样式、链接及完整替代源码映射保留。

```rust
use mdtui3::*;
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

读取不改变折叠意愿或高度，也不应用视图中的子折叠。宿主显式选取 `FoldCandidate.hidden` 预览正文、选取标题 `content` 读取名称；开放范围是当前 revision 的快照。逻辑行不等于终端软换行后的屏幕行，摘要占位绘制仍需由排版方限制实际显示行数、测量并归属高度。`cargo run -r --example excerpt` 展示百万字折叠代码的两行摘要；终端宽度、占位高度及 UI 接入继续见 TODO。

## 源码入口

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
nix develop -c cargo check --all-targets
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo nextest run --release --no-fail-fast
nix develop -c cargo test --doc
nix develop -c cargo fmt --all -- --check
nix develop -c cargo run -r --example stream
nix develop -c cargo run -r --example structure
nix develop -c cargo run -r --example containers
nix develop -c cargo run -r --example container_headings
nix develop -c cargo run -r --example container_setext
nix develop -c cargo run -r --example container_sections
nix develop -c cargo run -r --example container_tables
nix develop -c cargo run -r --example scaling
nix develop -c cargo run -r --example budget
nix develop -c cargo run -r --example display
nix develop -c cargo run -r --example excerpt
nix develop -c cargo run -r --example table_preview
nix develop -c cargo run -r --example table_row_folding
nix develop -c cargo run -r --example search
nix develop -c cargo run -r --example heading_search
nix develop -c cargo run -r --example anchors
nix develop -c cargo run -r --example folding
nix develop -c cargo run -r --example fold_batch
nix develop -c cargo run -r --example history
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
