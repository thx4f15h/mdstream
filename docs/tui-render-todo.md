# TUI 渲染 CLI 任务拆分（TODO）

状态：任务规划，尚未实现。本文只拆分工作，不包含核心库修改，也不把任何任务标记为已完成。

修订（2026-10-31）：按核心行为评审新增 P07 探针、P28/P29 可见区间预扫描与分段调度、P39 检查点维护；修正 resize 重置、更新失效范围、流式锚点构建和侧栏标题预算的假设。第二轮补齐 tile 边界检查点、跨 tile 行归属、append 后预扫描失效、超宽不折行块的固定网格裁剪、检查点双向淘汰、内部锚点 destination 解码和规模验收口径。第三轮修正检查点精确恢复/重放、上界 payload 必须完成压缩才释放，以及正文裁剪和缓存失效按内容区宽（含侧栏开关）计算。

参考基线：`gits/mdtui/docs/render-style-spec.reference.md`（相对 `$HOME`，只读；实现开始时记录其 SHA-256）。样式细则以该文件为准；本文只补充 TUI 交互、增量加载、首屏和测试约束。

## 1. 目标与边界

目标：新增一个独立的终端 Markdown 阅读器 CLI，复用当前 `mdtui3` 的增量解析、视口、样式、标题、表格和锚点能力，实现：

- 流式读取文件，首屏不等全文解析完成。
- 按终端视口渲染 Markdown，视觉规则尽量对齐参考样式。
- 正文内锚点跳转、外链可点击显示、跳转历史。
- 左侧标题目录栏，可开关、聚焦、选择跳转和当前章节高亮。
- 增量更新只重绘变化区域；正常滚动和流式追加不整屏清屏。
- 大文件、长段落、长表格、深容器下仍有界工作。

明确不做（MVP 不阻塞）：

- 不修改核心库 `src/**`、不改变 `mdtui3` 公开 API、不给核心库增加运行时依赖。
- 不做编辑、保存、搜索、折叠、鼠标、多文件、标签页、会话恢复、跟随 tail。
- 不解析主题文件；固定参考文档中的 Monokai 角色。
- 不做外部链接自动打开；默认只在终端中显示/承载外链，是否打开由显式参数决定。
- 不复刻参考实现“HTML 块导致崩溃”的行为；本工具必须安全退出或跳过不可见内容，并在差异文档中记录。

## 2. 不可违反的约束

1. **首屏**：首批数据到达后必须能在不调用 `Document::finish()` 的情况下出帧；首帧不得等待整文件读取、UTF-8 全量校验或最终校准。
2. **增量**：正文只按当前视口及有界 overscan 取片段；不得建立全篇渲染行、全篇行高、全篇行号或全篇标题文本缓存。
3. **闪烁**：正常帧不得使用 `ESC[2J`/`ESC[H` 清屏归位；首帧、resize 和终端恢复除外。帧写入使用双缓冲和 cell diff。
4. **有界**：每帧的布局、恢复、样式读取、表格测量和标题读取都通过 `Budget`/`WorkLimit` 或 TUI 自己的等价预算计量；预算耗尽可暂停并继续。`LayoutSink` 回调本身不得发起无界读取或二次全范围查询；回调只消费已按预算准备好的可见区间数据。
5. **核心只读**：如果发现核心接口缺口，先在 TUI crate 内做有界适配；不得为了 TUI 去改 `src/**`。确实必须改核心时，停下来单独提出核心任务。
6. **可验证**：每个子任务必须有单元测试、cell-grid 快照、原始 ANSI 断言、PTY 断言或确定性计数中的至少一种证据。

## 3. 建议架构

```text
文件读取线程（64 KiB 块，有界 channel）
        |
        v
Utf8Document.append / finish
        |
        +--> Document + Update
        +--> HeightIndex.apply
        +--> CheckpointIndex.apply
        |
        v
可见区间预扫描（块计划 + display_cursor 文本段）
        |
        v
视口布局（分段调度：LayoutPass 可绘制段 + TUI 独占隐藏块）
        |
        +--> 语义 run / 标题 / 表格 / 链接对象
        |
        v
Screen（Cell 网格 + source map + link map）
        |
        +--> 一次性 ANSI 写出器（--once，按参考文档字节）
        +--> TUI 差分 ANSI 写出器（crossterm 终端控制）
```

MVP 选择自绘 cell 网格和差分 ANSI 写出器，不把 ratatui 放进关键路径。原因是参考文档要求 OSC 8、下划线变体和精确 SGR/空白行为，而 ratatui 已知会丢失超链接和下划线变体；自绘还能直接控制“正常帧不清屏”。后续若需要 ratatui，只增加 `Screen -> ratatui Buffer` 适配，不改变布局核心。

### 3.1 模块建议

- `crates/mdtui-tui/src/main.rs`：参数、TTY 检查、终端生命周期、错误退出。
- `crates/mdtui-tui/src/app.rs`：事件循环、加载状态、视口、侧栏、跳转历史。
- `crates/mdtui-tui/src/loader.rs`：有界文件读取、`Utf8Document`、EOF 与错误事件。
- `crates/mdtui-tui/src/screen.rs`：`Cell`、`Screen`、宽字符占位、链接范围、行来源映射。
- `crates/mdtui-tui/src/theme.rs`：角色到 RGB/SGR 的映射。
- `crates/mdtui-tui/src/render/`：内联 run、块、折行、列表、引用、代码、分隔线、表格。
- `crates/mdtui-tui/src/render/display.rs`：预算内的 `display_cursor` 预扫描，产出按源码定位的显示文本段。
- `crates/mdtui-tui/src/render/plan.rs`：可见块预扫描、静态/增量共用的块渲染计划、隐藏块分类。
- `crates/mdtui-tui/src/layout.rs`：`LayoutSink` 事件采集、分段调度、检查点、tile 行归属、视口填充。
- `crates/mdtui-tui/src/output.rs`：一次性 ANSI、差分 ANSI、清屏/恢复策略。
- `crates/mdtui-tui/src/sidebar.rs`：可见标题、截断、当前章节、焦点。
- `crates/mdtui-tui/src/links.rs`：链接命中表、内部锚点、外链策略。
- `crates/mdtui-tui/src/testkit/`：vt100 快照、假加载器、计数器和性能断言。

## 4. 核心 API 映射

| 需求 | 复用接口 | 注意点 |
| --- | --- | --- |
| 流式 UTF-8 | `Utf8Document::append/finish` | 非法序列替换为 U+FFFD；不阻塞首帧 |
| 语义 run | `Document::display_cursor`（增量）；`display_runs_in`（仅静态/测试） | `LayoutPass` 的 `text` 给的是原始 `RenderRun`；增量路径必须先预扫描显示文本，不能直接渲染原始 run |
| 块/容器 | `BlockKind`、`Document::block`、`Document::container` | 列表/引用/代码/表格按块和容器属性排版 |
| 视口高度 | `HeightIndex::viewport/measure` | tile 不是强制换行；真实行高必须回填 |
| 局部排版 | `LayoutPass` + `Budget` | 可暂停；sink 必须返回 tile 实际行数；完全隐藏的 tile 会被 `next_drawable`/`heights.suppress` 跳过 |
| 恢复状态 | `CheckpointIndex<S>` | `S` 由 TUI 定义，保存折行/块内状态，不复制正文；失效只清计数，payload 要显式淘汰或压缩 |
| 标题 | `HeadingIndex` | 按 ID 稳定；标题文本只在需要时读 |
| 标题锚点 | `HeadingAnchors` + `FragmentLookup` | revision 绑定；append/finish 后旧索引直接失效，因此只在 EOF 后构建一次 |
| 表格 | `Table::rows/cells`、`Table::row_at` | `source_width_hint` 不是显示宽度；列宽要单独计算 |
| 链接 | `links_in`、`link_info`、`link_url`、`link_destination` | `link_url` 会为邮件补 `mailto:`；按参考规格写 OSC 8 时用裸目标，启动邮件客户端时才用 `link_url`；表格内不发 OSC 8 |
| 导航 | `Navigator::navigate/reconcile` | 行号是估计值；跳转后要用局部排版细化 |
| 计数 | `Document::work()`、`Budget` 使用量 | 用于证明首屏/滚动/跳转不随全文线性增长 |

### 4.1 已验证的核心集成事实与适配契约

以下行为已在当前 `src/**` 中确认，P07 用可执行探针固定它们；后续任务不得再按相反假设拆分。

1. **`LayoutPass` 只报告可绘制内容。** 完全隐藏的 tile 由 `Document::next_drawable` 和 `HeightIndex::suppress` 跳过，不会调用 `begin_tile`/`text`。因此“只有 info 行、没有正文”的围栏代码块永远不会把 `BlockKind::Code { info }` 送到 sink，P21 的语言标签和 P25 的块间距会丢失。增量路径必须有 TUI 侧的可见块预扫描；块级事件按源码顺序合并，不能假设每个块都有 run。
2. **`LayoutSink::text` 收到的是原始 `RenderRun`。** 它来自 `Document::runs_in`，不经过 `display_runs_in` 的实体替换、代码空白归一和 CRLF 处理。增量渲染必须先用预算内的 `display_cursor` 扫描当前 tile 范围，得到显示文本段；sink 只消费这些段。`display_runs_in` 仅用于静态整篇渲染和测试，不进入逐帧路径。
3. **宽度变化没有隐含自动失效。** `HeightIndex` 的高度和 `CheckpointIndex` 的折行状态都与正文内容区宽度（`content_width`，见 §4.1 第 14 条）绑定。终端 resize 或侧栏开关改变 `content_width` 时，必须执行 `heights.reset(estimated_rows)` 和 `checkpoints.clear()`，再按顶部源码锚点重新局部布局；`reset` 会把隐藏 tile 恢复成估计值，P29 必须重新识别并 `suppress`/`measure`。仅清屏重绘会复用旧宽度结果；只改变终端高度时才可以保留宽度相关缓存。
4. **更新失效是保守的。** `HeightIndex::apply` 从最早变化 tile 失效到末尾，`CheckpointIndex::apply` 从该 tile 失效到容量；只有 `LayoutPolicy` 可能缩窄。计划不得声称“只失效可见范围”，也不得把 P64 的结论建立在这一点上。可见范围只负责按预算重新测量；离屏状态允许保持估计值或无效值。
5. **`HeadingAnchors` 是 revision 绑定的。** 任意 append 都会使旧索引返回 `StaleContinuation`，重新构建会重读所有标题字节。流式期间不构建完整 slug 索引；内部链接解析推迟到 `finish` 后一次预算内构建，期间只允许按 `HeadingId`/源码跳转。
6. **检查点 payload 不会随失效自动释放。** `invalidate`/`apply` 只清计数；`drain_before` 和 `into_compaction` 只把历史 payload 的所有权交给调用方，调用方释放（clear/drop 输出向量）后才真正回收。滚动型阅读必须有显式淘汰/压缩任务；P67 必须在压缩完成且退役输出释放后用 `retired`/`arena_stats` 证明上界，`len()` 只能证明有效状态数。
7. **侧栏标题读取也必须预算化。** `DisplayExcerpt` + `display_cursor` 可以在 `Budget` 下按源码/显示字节/行数截断；侧栏不使用无预算的 `display_runs_in(heading.content)`。
8. **`Style` 是独立位并集，不能恢复嵌套顺序。** `RenderRun.style` 把 `EMPHASIS`/`STRONG`/`STRIKE`/`CODE`/`MATH` 做位并集；`**a ~~b~~**` 与 `~~a **b**~~` 在 `b` 上都会得到 `STRONG|STRIKE`，`BlockKind::Math` 与内联数学都会置 `MATH`（如 `**$x$**` 得到 `STRONG|MATH`）。P14 无法实现“嵌套时外层决定前景”，只能采用固定前景优先级 `CODE > MATH > STRONG/EMPHASIS > STRIKE` 并把差异写入规格差异文档；P07 固定位并集与 `MATH` 事实。
9. **`end_tile` 在 tile 末尾触发，不等于行末。** 一个 Markdown 行可以横跨多个 256 字节 tile；首字符所在 tile 的 `end_tile` 返回时还不知道该行后续折行数。P31 采用“行尾 tile 归属”：sink 跨 tile 保留未完成行，行尾所在 tile 的 `end_tile` 才计入整行折行数；Screen 行来源仍记录行首源码，从行中间进入视口时从行首或精确检查点恢复，更早检查点按 P30 重放。
10. **`CheckpointIndex` 只接受 tile 键。** `insert` 的键是 `TileId`，`LayoutPass` 也只能从 tile 启动；块边界没有对应 tile，不能在同一 tile 内插入两个块级检查点。P29/P30 的段边界必须落在 tile 边界，同一 tile 内的多个块由 sink 按源码顺序合并输出。恢复时 `begin_tile(tile)` 只能使用精确 `get(tile)`；`previous(tile)` 返回的更早状态必须经 `LayoutPass::from_checkpoint` 在恢复预算内重放到目标 tile，不能直接套用。重放期间同一 sink 处于不写 `Screen` 的 replay 模式，到达 `begin_tile(target)` 时在同一 sink 内切换为正式输出，目标 tile 只输出一次。
11. **`DisplayCursor` 绑定 revision。** 任意非空 append/finish 都会使旧游标返回 `StaleContinuation`；append 还可能改变 Setext、表格和未闭合内联样式。P28/P35 在 revision 变化时丢弃已扫描前缀、`DisplayCursor`、`DisplaySegment` 和 `BlockPlan`，从视口锚点重新扫描；只有同一 revision 内才允许断点续扫。
12. **检查点淘汰是单向的。** `drain_before` 只处理 tile 小于 `before` 的 payload；视口之后（源码更后）的有效检查点不会被删除，也没有对称的 `drain_after`。P39 必须同时 `invalidate` 窗口下界之前和窗口上界之后的范围；下界由 `drain_before` 回收，上界失效后留下的陈旧 payload 只能由 `into_compaction` 退役并经调用方释放。P67 在压缩完成且输出释放后验收两侧。
13. **内部链接 destination 保留 Markdown 原文。** `link_destination` 保留反斜杠转义和实体原文；`HeadingAnchors::fragment` 只解码百分号/UTF-8，不做 Markdown entity/backslash 解码。P55 必须先在有界缓冲中解码 destination，再调用 `fragment`。
14. **TUI 的正文按内容区宽裁剪。** `Screen` 是固定终端尺寸的 cell 网格，但正文只占侧栏之外的内容区：侧栏关闭时 `content_x = 0`、`content_width = term_w`；打开时 `content_width = term_w - sidebar_width`（侧栏分隔线/边框计入 `sidebar_width`），正文原点 `content_x = sidebar_width`。标题/代码/表格的“不折行”和段落/列表/引用的折行都按 `content_width` 计算，不能按终端宽计算，也不能靠终端自动折行；P04 定义了内容区矩形，P12/P20/P23/P24 在写入 cell 网格前裁剪到 `content_width`，P41 不写出越过内容区右边界或终端宽度的 cell。侧栏开关改变 `content_width` 时按 P42 失效宽度相关高度/检查点/预扫描缓存。`--once` 无侧栏，`content_width == term_w`，保留参考实现的原始字节和溢出行为。

## 5. 任务分解

任务编号用于依赖和验收记录，不表示执行顺序之外的优先级。所有任务初始为 `[ ]`（表中为紧凑未画勾选框）；完成时在 ID 前标记 `[x]`（例如 `| [x] P12 |`），并补一行证据（测试名、命令、快照名或计数报告）。

### 阶段 P0：基线、骨架和测试基础

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P00 | 固化参考基线与差异清单 | 无 | 记录参考文件 SHA-256；建立 §0–§9 对照表；明确 TUI 与 `--once` 的差异：TUI 以 cell 网格等价为准，不做行尾 CRLF，交互帧用光标寻址；`--once` 才逐语义段字节精确；TUI 对标题/代码/表格等不折行块按正文内容区宽裁剪（无侧栏时等于终端宽），`--once` 保留参考字节及可能的终端自动折行；不复制 HTML 块崩溃。 |
| P01 | 新建独立 TUI crate | P00 | 新增 `crates/mdtui-tui`，使用独立 `Cargo.toml` + `mdtui3 = { path = "../.." }`；默认不改根 `Cargo.toml`/根 lock，只改 TUI crate 自身 manifest/lock；如确需 workspace member，先取得用户同意；`git diff -- src Cargo.toml` 为空；`cargo check --manifest-path crates/mdtui-tui/Cargo.toml` 通过。 |
| P02 | CLI 参数与终端生命周期骨架 | P01 | 支持 `--once`、`--width`、`--height`、`--open-links`、`--no-links`、`--help`、文件参数（`--width/--height` 供 `--once`/测试固定几何，TUI 模式以终端尺寸为准）；`--no-links` 只关闭 OSC 8 输出，不关闭样式、内部锚点和应用内链接焦点；非 TTY 时 TUI 模式拒绝启动并提示；raw mode/备用屏/光标/panic 恢复有 PTY 测试；退出码覆盖成功、参数错、I/O 错。 |
| P03 | 测试工装与固定环境 | P01 | 提供 vt100 cell 解析、insta 快照、假时钟、临时文件、PTY 封装；固定 `TERM=xterm-256color`、`COLORTERM=truecolor`、`LANG=C.UTF-8`、120×40；工装自身有 1 个通过/失败自检。 |
| P04 | Screen/Cell 数据模型 | P03 | 定义 `Cell`（符号、样式角色、宽度、链接 ID、宽字符占位）、`Screen`（终端尺寸 + 正文内容区矩形）、行来源映射；单元测试覆盖 CJK、emoji、组合字符、宽字符占位、边界裁剪，以及超宽不折行块被截断后正文行宽不超过内容区宽、正文 cell 不落到侧栏列或越过终端右边界且行来源映射仍有效。 |
| P05 | 主题与 SGR 编码器 | P03 | 固定参考 §1 的 RGB；按“粗体→斜体→下划线→删除线→前景→背景”发射；独立复位；单元/golden 覆盖 H1–H6、代码、链接、表格框线。 |
| P06 | 静态 fixture 语料骨架 | P03 | 建立 `tests/fixtures/`：标题、段落、强调、链接、图片、引用、列表、代码、分隔线、表格、脚注/HTML、块间距、宽字符；每个 fixture 有 README 说明期望覆盖的规格章节。 |
| P07 | 核心集成探针 | P01、P03 | 在 TUI crate 的测试中用只读核心 API 固定 §4.1 的事实：空围栏块在 `LayoutPass` 中无回调；`runs_in` 与 `display_cursor` 对 `&amp;`/代码空白输出不同；宽度变化后必须 `reset`/`clear`；append 后高度/检查点保守失效；`HeadingAnchors` 在 append 后失效；检查点 payload 在 `drain_before`/`into_compaction` 前仍占用；`Style` 只有位并集、无法恢复嵌套顺序且数学 run 带 `MATH`（含 `**$x$**` → `STRONG|MATH`）；`end_tile` 在 tile 末尾而非行末触发；`CheckpointIndex::insert` 只接受 `TileId`；`display_cursor` 在 append 后返回 `StaleContinuation`；`drain_before` 不会淘汰视口之后的检查点；`previous(tile)` 可能早于 `tile`，更早检查点必须由 `LayoutPass::from_checkpoint` 重放，不能直接套用；`link_destination` 保留实体/反斜杠原文而 `HeadingAnchors::fragment` 只解码百分号。探针全部通过后才允许开始 P10；若任一事实不成立，停止并重写 §4.1，不得带假设进入实现。 |

### 阶段 P1：静态语义渲染（先不接视口/终端）

这一阶段把 `Document` 的语义 run 变成 `Screen`，只在内存中验证 cell 网格；不要求交互和流式首屏。

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P10 | run 摄取与归一化 | P04、P05、P07 | 静态整篇可用 `display_runs_in`，逐帧路径只允许 P28 的预算化显示段；跳过 HIDDEN；保留 code 空白、实体替换、软/硬换行、容器前缀隐藏；新增 `a &amp; b`、代码内多空格/tab、CRLF 单元测试和 §3.2/§3.4 fixture。 |
| P11 | Unicode 字素与显示宽度 | P04、P10 | 采用固定版本 `unicode-segmentation`/`unicode-width`；覆盖 `•`、`│`、`─`、`═`、CJK、emoji、组合、ZWJ、NBSP、tab；宽字符只写首 cell，后续 cell 标记占位。 |
| P12 | 标题渲染与文字拼合 | P10、P11 | 按 §3.1/§3.12：整行统一标题样式；H1–H5 下划线变体；H6 无下划线；超宽不折行；链接/图片文字丢弃；强调文字按规则重排。TUI cell 网格在正文内容区宽处裁剪，禁止把越界 cell 交给终端自动折行；`--once` 保留参考字节。测试至少包含规格中的 ``# **B** *I* `c` [l](u) ~~s~~`` 例子和超宽标题 fixture。 |
| P13 | 段落渲染与折行 | P10、P11 | 软换行变空格、硬换行消失、首尾空白裁剪、连续空格保留；逐字符贪心折行；宽 0/1 退化；测试所有 §3.2/§5 段落案例。 |
| P14 | 强调/代码内联样式 | P10、P05 | 粗体、斜体、删除线、代码、数学的前景/背景；核心 `Style` 只有位并集，无法恢复嵌套顺序，采用固定前景优先级 `CODE > MATH > STRONG/EMPHASIS > STRIKE`（`STRONG` 与 `STRIKE` 同时存在时用强调色叠删除线；`MATH` 覆盖 `STRONG/EMPHASIS` 前景）；`~~a **b**~~` 这类“外删内强/斜”按固定优先级用强调色，`**$x$**` 用 MATH 前景，均作为规格差异记录；代码保留外层标志；每段独立复位；golden 覆盖两种嵌套顺序与 `**$x$**` 并标注差异。 |
| P15 | 链接与 OSC 8 | P10、P14 | 链接文字 `#8BE9FD` + 下划线；OSC 8 开在样式前、关在复位后；同行多链接独立；空地址只样式；标题忽略；引用式与行内式输出一致；邮件自动链接的 OSC 8 目标用裸地址（不写 `mailto:`）；表格内不发 OSC 8；`--no-links` 全局关闭 OSC 8 但保留样式；测试原始字节。 |
| P16 | 图片、脚注、定义行与 HTML | P10 | 图片不可见（独占段落零字节，混排只剩两侧空格）；有定义的脚注引用/定义不可见，无定义按原文；链接/引用定义行不可见；HTML 安全策略明确且不崩溃。 |
| P17 | 列表标记、编号与任务状态 | P10、P11 | `•`、`N.`、`⬜`/`✅` 的颜色；起始编号；完成项整行删除线；标记后空格同色；嵌套缩进公式；任务项按 1 格；空项消失；测试混合嵌套和 0/10/99 起始。 |
| P18 | 列表文字拼合与按词折行 | P17 | 按 §3.12 拼合；超长词硬断；CJK 单字成词；词间空格规则；续行缩进含正文样式；可用宽公式 `content_width−缩进−标记−1`；测试 `hello, world` 等规格案例。 |
| P19 | 引用块前缀 | P10、P11 | 段落行加 `│ `；嵌套只一层；标题/列表/代码/表格/分隔线在引用内无前缀；空引用行消失；折行每行带前缀；测试混合容器。 |
| P20 | 代码块骨架 | P10、P11、P05 | 每行 2 空格缩进；前景/背景作用于缩进和内容；空行输出 2 个带样式空格；源 CRLF 归一；未闭合围栏照常；长行不折行；TUI 在正文内容区宽处裁剪，不能写出越过内容区右边界或 `Screen` 宽度的 cell；`--once` 保留参考行字节；测试围栏/波浪线/缩进代码和超宽代码行。 |
| P21 | 代码块语言标签 | P20 | `BlockKind::Code { info }` 有值时首行 `[完整信息串]`，灰色、无缩进；无信息时无该行；首尾空白裁剪、内部原样保留；测试多余参数。 |
| P22 | 分隔线 | P10、P11 | `---`/`***`/`___` 统一输出 `─` 重复到正文内容区宽；灰色；引用内无前缀；测试宽度 1/2/80。 |
| P23 | 表格几何与列宽 | P10、P11 | 从 `Table` 的行/列/单元格读；左右各 1 空格；列宽取可见内容最大宽度；CJK 按 2 格；缺列补空、超列丢弃、列数不一致回退段落；TUI 只需计算并保留正文内容区宽度内的可见列前缀，超出内容区宽度的列不分配 `Screen` cell；`--once` 保留完整表格几何。测试表头/无表体/空单元格/超宽表格。 |
| P24 | 表格渲染与样式 | P23、P05、P15 | 顶/底/表头分隔/竖线字符准确；表头无特殊样式；框线、padding、补齐空格均灰色且分段独立；内容左对齐；内联样式保留；表格内链接只样式不出 OSC 8；行间 LF、块末 CRLF（`--once`）；TUI 每行在正文内容区宽处裁剪，不触发终端自动折行，行号与 `HeightIndex` 保持 1:1。 |
| P25 | 顶层块间距 | P10–P24 | 按 §4 实现块间空行表；引用/列表作为整块；代码/分隔线与列表紧贴；首块无前导空行；用组合 fixture 断言最终 cell 行号。 |
| P26 | 静态整篇渲染与 `--once` 骨架 | P12–P25 | 把静态 `Screen` 接到 `--once`；先固定内容区宽度（无侧栏时等于终端宽）、不可交互；对 fixture × 宽度 1/20/80/120 生成 cell 快照；与规格逐节对照。 |

### 阶段 P2：视口、布局和增量更新

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P28 | 显示文本预扫描 | P26、P07 | 对本次要布局的源码区间用 `display_cursor` + 共享 `Budget` 生成 `DisplaySegment`（源码 span、显示文本/style/link/block），按 tile 边界组织，替换跨边界时用 `DisplayRun::replacement` 归属到包含起始字节的 tile；实体、代码空白、CRLF、HIDDEN 跳过都在此归一。段只覆盖视口 + overscan；记录段数/字节数。测试包含实体跨 tile、代码空白跨 run。所有段、块计划和 `DisplayCursor` 都携带 revision；同一 revision 内预算不足时保留上一屏和已扫描的完整 tile 前缀，下一帧从断点继续；任何非空 append/finish 改变 revision 后，丢弃未消费游标和前缀，从视口锚点或 P35 的顶部源码锚点重新扫描，不得跨 revision 拼接。不得回退到原始 run，也不得突破帧预算。 |
| P29 | 可见块预扫描与分段调度 | P25、P26、P28 | 先把 P21/P22/P25 的块级输出抽成无屏幕副作用的 `BlockPlan`，静态和增量共用，避免两套间距/标签规则。在 `Document::blocks()` 上按源码二分，只扫描与可见 tile 相交的块。tile 是否交给 `LayoutPass` 直接使用 `document.next_drawable(tile.start)`：返回位置小于 `tile.end`（含空 alt 图片对象）则交给，否则由 TUI 处理；不要另写语义判断。把 tile 分成“可绘制段”“TUI 独占隐藏块 tile”“零行隐藏段”，块级事件按源码位置合并进 sink。段边界只能落在 tile 边界（`RenderCheckpoint` 的键也是 `TileId`）；同一 tile 内的多个块不拆段，块边界没有对应 tile 时不另插检查点，由 sink 按源码顺序合并输出。TUI 独占隐藏块 tile 由 TUI 排版并 `measure`，不再交给 `LayoutPass`。空围栏代码块（有 info、无正文）必须产生标签行和块间距；同一 tile 内“段落 + 空围栏”也在一个段内按源码顺序输出。用静态渲染做 oracle，逐 cell 相等。 |
| P30 | 渲染检查点类型 | P26、P29 | 定义 `RenderCheckpoint`（tile 起始处的折行列、跨 tile 未完成行的源码起点/已累计显示宽度、块 ID/类型、列表缩进/标记、引用前缀、代码/表格状态、待合并块事件）；`Copy` 或小对象；检查点只按 `CheckpointIndex` 的 `TileId` 存取：`end_tile(tile)` 后若存在 `tile + 1` 则保存该 tile 结束时的 sink 状态，`begin_tile(tile)` 只接受精确 `get(tile)` 命中的状态。若只有更早的 `from = previous(tile)`，不得把 `from` 的状态直接套到 `tile`；必须从 `get(from)` 取出状态装入同一个 sink 的 replay 模式（只积累状态、不写 `Screen`），再用 `LayoutPass::from_checkpoint(document, heights, from, tile, rows, max_bytes)` 在同一恢复预算内重放到 `tile`；首次收到 `begin_tile(tile)` 时在同一 sink 内切换为正式输出并继续消费该 tile 的事件，不得先把状态交给另一个 sink 再重放或重发 target tile，目标 tile 的 `begin_tile`/`text`/`object`/`end_tile` 只输出一次。若预算在到达 target 前耗尽，保持 replay 模式并在下帧续放；到达 target 后中途耗尽，则保留同一 sink 的当前 tile 部分状态并在下帧从同一 tile 继续，不得重放 target tile。没有精确检查点且无法在预算内重放时，只允许从段起点/行首开始或显式标记近似，不得假装有精确行首。不声称支持块边界或字节偏移检查点。单元测试覆盖 tile 边界存取、同一 tile 多块、跨 tile 行恢复、缺失检查点、更早检查点必须重放（不得直接套用）、target tile 不重复输出和 revision 失效。 |
| P31 | `LayoutSink` 事件采集与 tile 行归属 | P28、P29、P30 | 实现 `begin_tile/text/object/end_tile`，回调只消费 P28/P29 的预扫描结果，不做源码读取或二次查询；规定“一行归属于其行尾（换行符；无换行则最后一个显示字符）所在 tile”。sink 跨 tile 保留未完成行状态；`end_tile(tile)` 只返回该 tile 内已结束行的高度，跨 tile 行的全部折行数在行尾 tile 一次计入，不能在首字符 tile 提前猜行数。Screen 行来源仍记录行首源码；从行中间进入视口时从行首或精确检查点恢复，更早检查点按 P30 重放，不能把行尾 tile 当成行首。块级事件与文本段按源码顺序合并；返回 tile 实际行数（含隐藏块标签/间距）。用计数器断言单次回调工作量受预扫描结果大小约束。 |
| P32 | 首屏局部排版 | P31 | 从 tile 0 开始按 P29 的分段计划推进：可绘制段交给 `LayoutPass`，TUI 独占隐藏块由 TUI 直接排版并 `measure`；预算在段边界暂停，中途耗尽时保留 sink（含跨 tile 未完成行状态）/显示段游标，并在同一 revision 的下一帧从同一 tile 继续；revision 变化时按 P28 丢弃并重扫；不能跨越未预扫描的 tile；不调用 `finish`；测试首帧不含全文布局计数，并包含空围栏代码块首屏用例。 |
| P33 | 滚动与导航 | P31 | Up/Down/PageUp/PageDown/Home/End；`HeightIndex::viewport` + overscan；到达首尾不越界；行/页只处理视口附近。`HeightIndex` 的跨 tile 行高度归行尾 tile，Screen 的 `source map` 保存行首；视口命中行中间时从行首或精确检查点恢复，只有更早检查点时按 P30 经 `LayoutPass::from_checkpoint` 重放，必要时在预算内局部重排，不能从行尾 tile 直接伪造行首。测试大文件上下滚动、跨 tile 长行和边界。 |
| P34 | 源码到行的精确细化 | P30、P33 | 标题/内部链接跳转后，用精确检查点、按 P30 重放的更早检查点或局部起点把 `ScrollTarget.source` 细化到实际行；同一 tile 多标题可分别到达；细化预算耗尽可继续；测试 `Navigator::reconcile`。 |
| P35 | 增量应用与锚点保持 | P31、P33 | 每个 `Update` 依次合并到 `HeightIndex`/`CheckpointIndex`，并明确记录核心的保守失效契约：高度从最早变化 tile 失效到末尾，检查点从该 tile 失效到容量；TUI 只承诺按预算重新测量可见窗口，不承诺离屏状态仍有效。追加/EOF 后保持顶部源码锚点；任何 revision 变化先丢弃显示预扫描、`BlockPlan`、`DisplayCursor` 和链接命中缓存；缓存命中前校验 revision/token，只复用 revision 与 token 都有效的缓存；测试滚动中追加、预扫描落后于 append、离屏 tile 回滚后重新测量、无陈旧折行。 |
| P36 | 表格宽度渐进测量 | P23、P31 | 首屏只用 `source_width_hint` + 可见行建立宽度上界；后台/滚动按预算补测未访问行；TUI 最终宽度只收敛到正文内容区宽内可见列前缀所需值，静态 oracle 和 `--once` 才收敛到全表最大值；测试首屏不扫全表、finish 后 TUI 可见前缀与全量一致。 |
| P37 | EOF 最终校准 | P32、P36 | `finish` 后执行一次有预算的最终校准；保持当前阅读位置；暂定标题/链接/表格可更新；UI 不阻塞退出；测试流式前缀与同内容一次性解析最终 cell 一致。 |
| P38 | 未分类尾部与暂定内容 | P31 | 按 `Document::pending()` 和当前 revision 显示已提交/暂定文本；核心 `Update` 只更新变化部分，但 TUI 的显示预扫描仍按 P28/P35 在 revision 变化时失效，不承诺跨 append 复用已扫描前缀；不把未分类尾部误当隐藏；测试长行/晚闭合分隔符/Setext 迟确认。 |
| P39 | 检查点保留与压缩维护 | P30、P35 | 维护“视口前后各 2 屏”的检查点保留窗口，换算成 tile 区间 `[lo, hi)`（边界夹到 `0..=tile_count`）。每帧/空闲先从 `CheckpointIndex` 执行 `invalidate(0..lo)` 和 `invalidate(hi..document.tile_count())`，清掉窗口两侧仍有效的检查点；`drain_before(lo)` 只负责窗口下界之前的 payload，不能替代上界失效。再用共享 `Budget` 调 `into_compaction` 压缩并 `finish`；压缩期间只缓冲一个帧周期的 update，不并发 apply。`len()` 只反映有效状态；验收必须在窗口上界失效后完成一次 `into_compaction`（`advance` 到 `Completion::Covered`、`finish`），释放 `drain_before`/`advance` 输出向量，把 `drain_before` 游标归零，再断言 64 MiB 连续滚动后有效 payload 只随窗口增长、窗口上界之后的陈旧 payload 已被压缩退役并释放。 |

### 阶段 P3：屏幕差分、闪烁和流式节奏

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P40 | Cell diff 算法 | P04、P31 | 比较前后 `Screen`，按行生成最小变更 run；宽字符/占位 cell 必须成对失效；行尾裁剪只在必要时使用；单元测试 diff 与全量重绘最终屏幕一致。 |
| P41 | 增量 ANSI 写出器 | P40、P05、P02 | 光标定位 + SGR/OSC 8 + 必要清行；正文 cell 不越过内容区右边界，任何 cell 不越过终端宽度，超宽不折行块已由 P04 定义内容区、P12/P20/P23/P24 按内容区宽裁剪，绝对定位行号与 `Screen`/`HeightIndex` 行号保持 1:1，不能依赖终端自动折行；`--no-links` 时不写任何 OSC 8；正常帧不发 `ESC[2J`，也不无条件回到 `ESC[H`（绝对定位 `CSI r;cH` 允许）；样式/链接每行闭合；写出失败可恢复；vt100 最终 cell 与参考屏幕一致，序列白名单通过。 |
| P42 | 首帧、内容区宽度变化与备用屏 | P41、P02、P07 | 首帧一次清屏+归位；终端 resize 或内容区宽度设置器改变 `content_width` 时，先记录顶部源码锚点，再执行 `heights.reset(非零估计值)`、`checkpoints.clear()`，丢弃当前 `LayoutPass`/恢复 token，清空显示段/块计划/diff 基线，然后按新 `content_width` 重新局部布局；只有 `content_width` 不变、仅高度变化时才可保留宽度相关缓存；`content_width` 变化时无论高度是否变化都不得复用旧宽度的高度/检查点。其余帧不清屏；PTY 覆盖 120×40→80×24→120×40，并在固定 120×40 下通过宽度设置器模拟侧栏开关（P50 再接真实 `s`），断言旧宽度高度/检查点 token 不被复用、顶部源码位置保持。 |
| P43 | 流式读取与帧合并 | P02、P35 | 64 KiB 有界 channel；首帧不合并；之后按截止时间/事件数合并；读取慢时背压；不丢最后一个 update；测试 1000 次追加的帧数上界和最终屏幕。 |
| P44 | 首屏预读策略 | P43、P32 | 默认首批 32 KiB（读取线程 channel 可用 64 KiB 块，块大小不计入预读批次）；若无可绘制行，最多再读 8 批 × 32 KiB = 256 KiB（连同首批累计 288 KiB）后必须出帧；首帧前不 `finish`；测试暂停 reader 时首帧仍出现。 |
| P45 | 闪烁检测工装 | P41、P43 | 用 vt100 分块喂入每个输出 write；检查中间帧不全空、不清屏、不跳回顶部；与每步全量渲染对照；输出失败/早退也恢复终端。 |
| P46 | 闪烁加固 | P45 | 双缓冲提交后才写；隐藏光标；必要时使用同步输出 `CSI ? 2026 h/l`；每行独立闭合样式/链接；连续 3 次运行字节/帧数稳定。 |
| P47 | 工作量与帧指标 | P41、P45 | 记录读取字节、产出 update 数、布局预算消耗、恢复 tile 数、diff 行数、输出字节、帧耗时；测试断言计数上界，benchmark 单独记录墙钟。 |

### 阶段 P4：交互、跳转和侧栏

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P50 | 事件循环与键位表 | P42、P43 | `q` 退出；方向/Page/Home/End；`Tab`/`Shift-Tab` 切换正文/侧栏/链接焦点；`s` 开关侧栏（经 P42 的 `content_width` 变化路径失效宽度相关缓存，并保持顶部源码锚点）；`Enter` 激活；`Alt-Left/M-Left` 后退，`Alt-Right/M-Right` 前进；测试事件到状态转换（含空历史时前进/后退为 no-op、固定终端尺寸下开关侧栏后旧折行/检查点不复用）。 |
| P51 | 侧栏可见标题模型 | P11、P31 | 按 `HeadingIndex` 顺序只取当前可见条目；标题用 `DisplayExcerpt::new` + `next(document, &mut Budget)` 读取，限制源码字节、显示字节和行数，达到侧栏内容区宽度立即停止；预算不足时显示省略号/上一帧缓存并在下一帧重试。只缓存当前可见标题文本，10k 标题、单个超长 Setext 标题都有读取字节/对象计数上界。 |
| P52 | 侧栏布局与当前章节 | P51、P33、P42 | 宽度建议 `min(40, max(20, 30%))`，窄屏隐藏；定义正文内容区矩形：关闭时 `content_x=0, content_width=term_w`，打开时 `content_x=sidebar_width`、`content_width=term_w-sidebar_width`（分隔线/边框计入侧栏宽）；正文折行、不折行块裁剪、表格可见列和 Screen 写入都用 `content_width`，不得把正文放到侧栏列；缩进体现层级；当前章节高亮随正文顶部位置更新；无标题空栏；测试 20/40/80/160 列及固定终端宽下开关侧栏。 |
| P53 | 侧栏增量更新与焦点 | P52、P35 | 用 `Update.new_headings`/`changed_headings` 标记脏项；只重建可见项；选择/滚动/焦点独立于正文；跳转后正文与侧栏都保持正确位置；不重排全文。 |
| P54 | 链接命中表与焦点 | P15、P40 | 每个可见行保存 `x0..x1 -> LinkId`；`Tab` 循环当前屏链接并高亮；表格内链接按规格不进入命中表；滚动/更新后失效重建；测试多链接、跨行、CJK 列宽。 |
| P55 | 内部锚点跳转 | P34、P54、P53 | 识别 `#fragment`；流式期间（未 `finish`）不构建完整 `HeadingAnchors`，Markdown 片段链接显示“待文档完成”，仅允许 `HeadingId`/源码跳转；EOF 后在空闲帧用 `HeadingAnchors::new` + `Budget` 分帧构建一次并缓存，append 不会发生（核心 EOF 后拒绝 append）。构建完成后，先在有界缓冲中按 CommonMark 解码 `link_destination` 的反斜杠标点和命名/数字实体，再交给 `HeadingAnchors::fragment` 处理百分号/UTF-8、重复 slug 和 `#` 到文档首；不能直接把原始 destination 交给 `fragment`。成功才压入历史；测试大小写、空目标、畸形转义、可命中的 `#a\_b`/`#&#x61;`，以及仅验证解码结果的 `#a\&b`/`#&#x41;`/`#a&amp;b`（解码后仍按核心 slug 规则匹配，可能为 NotFound）、构建期间重复跳转不重复启动。 |
| P56 | 外链策略与 OSC 8 | P15、P54、P02 | 外链只写入 OSC 8 元数据，不在默认路径启动进程；`--open-links` 时用无 shell 的参数化方式打开（邮件客户端可用 `link_url` 的 `mailto:`）；`--no-links` 完全不发 OSC 8，但保留链接样式和应用内焦点/内部跳转；测试 `http/https/mailto`、空目标、控制字符注入拒绝、`--no-links` 原始字节无 OSC 8。 |
| P57 | 跳转历史 | P50、P55 | 保存源码位置 + 视口内偏移；正文滚动刷新当前项；新跳转截断前进分支；`Alt-Right/M-Right` 恢复前进分支；后退/前进在 append/resize 后仍回到同一源码；测试连续标题跳转、同 tile 多标题、失败跳转不移动历史、前进分支截断与恢复。 |
| P58 | 当前章节与侧栏选中分离 | P52、P55 | 正文当前章节自动高亮；侧栏选中是独立状态；跳转失败/取消不改变正文位置；测试聚焦、滚动、跳转交错。 |

### 阶段 P5：CLI、验收和发布门槛

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P60 | 错误、极窄终端与帮助 | P02、P52 | 文件不存在/不可读/非普通文件、非 TTY、参数错、写出错均有明确退出码；小于 20×3 显示尺寸提示且可退出；`--help` 覆盖模式、`--open-links`/`--no-links`、全部键位（含 `Alt-Right/M-Right` 前进）；测试不 panic。 |
| P61 | `--once` 完整规格模式 | P26、P41 | 等待全文后输出参考 §0–§7：`ESC[2J ESC[H`、CRLF、表格内 LF、零字节空输入；与 fixture 的原始 ANSI golden 对比；TUI 模式不复用 CRLF 假设。 |
| P62 | 静态规格符合性矩阵 | P26、P61 | 逐节核对 §0–§6 和 §9；每条有 fixture/测试或明确差异；HTML 崩溃差异、ratatui 差异、TUI 行寻址/复位分段差异、嵌套强调外层前景无法区分、超宽不折行块在 TUI 被裁剪到正文内容区宽而 `--once` 溢出，单独记录并链接对应 fixture。 |
| P63 | 首屏与规模验收 | P44、P47 | 8 MiB 快测、64 MiB 手动/夜间；首帧发生在 `finish` 前。首帧读取总量不超过 288 KiB（首批 32 KiB + 最多 8 批 × 32 KiB 预读）；布局、显示段、输出和恢复计数有界。视口之前的无关前缀只允许核心 append-only parser 的 `Document::work().input_bytes` 线性增长，不纳入首帧有界断言；视口之后且超出首帧读取上限的无关内容不改变首帧读取/布局/输出计数。128/256/512 倍前缀分别记录 parser 字节与 TUI 布局/输出计数，不以 parser 字节冒充 TUI 工作量。 |
| P64 | 滚动/跳转规模验收 | P33、P34、P55 | 分别报告“核心保守失效成本”和“TUI 可见窗口重排成本”：前者允许从变化 tile 到末尾失效，不把它算作有界；后者必须受 `Budget` 限制。深标题跳转、连续翻页、远距离返回只重排预算内 tile/恢复；跨 tile 长行从行首/精确检查点恢复（更早检查点必须按 P30 重放）的成本单独记录；这里的“不因文档前面增加无关内容而线性增长”只指 TUI 可见窗口布局/恢复计数，核心 `input_bytes` 和保守失效成本单独报告；EOF 后锚点索引构建单独记为一次性成本。 |
| P65 | 闪烁与增量验收 | P45、P46、P43 | 流式 1000 次追加：首帧立即出现；中间帧不为空；无正常帧 `2J`；最终屏等于全量渲染（含空围栏代码块、实体、代码空白）；帧数上界；连续 3 次稳定。 |
| P66 | PTY 端到端验收 | P50–P57、P60 | 真实 PTY 跑 120×40 和 80×24；覆盖启动、首屏、滚动、侧栏开关、目录跳转、锚点跳转、`Alt-Left`/`Alt-Right` 历史、外链焦点、`--no-links`、resize 120→80→120、退出、panic 恢复；固定终端尺寸下开关侧栏后断言正文按新内容区宽重排、顶部源码锚点保持、旧宽度折行/检查点不复用；断言最终 cell 与状态。 |
| P67 | 有界状态与内存审计 | P35、P39、P51、P54 | 证明 frame buffer 只随终端尺寸、侧栏只保留可见标题文本、检查点 payload 只保留视口前后双向窗口内、表格宽度缓存不复制全表正文、EOF 后不残留标题全文副本（豁免 P55 `HeadingAnchors` 的 slug 索引：核心按 O(S + H) 保存归一化 base 与共享 trie，允许一次性 O(S)，保留该缓存并单列测量，不计入滚动型有界状态断言）；用 `CheckpointIndex::len()`/`arena_stats` 记录 64 MiB 滚动、10k 标题/10k 表格行、超长 Setext 标题的容量计数。`len()` 只统计有效状态，上界 `invalidate` 只清计数，不能据此断言视口之后的陈旧 payload 已释放；必须在窗口两侧失效后完成一次 `into_compaction` → `advance`（到 `Completion::Covered`）→ `finish`，在 `finish` 前读取 `stats()` 中的 `retired` 与 `source`/`destination` 的 `ArenaStats`，断言 `advance` 输出向量中退役的陈旧 payload 数符合预期；随后释放 `drain_before`/`advance` 的输出向量，并断言 `finish` 后的 `arena_stats` 与测试进程的 payload 计数不包含旧 source arena 或已退役 payload；仅 `len()` 下降不算通过。 |
| P68 | 文档、差异和发布收口 | P62–P67 | README/TUI 使用说明、键位、`--no-links` 语义、构建命令、规格差异、性能证据、已知限制（含“流式期间 Markdown 片段链接推迟到 EOF 解析”、嵌套强调固定优先级、超宽不折行块裁剪到正文内容区宽且侧栏开关会改变该宽度）；`cargo fmt`/`clippy`/测试/快照全部通过；只允许 TUI crate、其独立 manifest/lock 和文档变更。 |

## 6. 依赖顺序

```text
P00 → P01 → P02/P03 → P04/P05/P06
                   ↓
                  P07（探针通过后才继续）
                   ↓
                  P10 → P11 → P12–P25 → P26
                                           ↓
                                   P28 → P29 → P30 → P31 → P32/P33
                                                              ↓
                                                        P34/P35/P36/P37/P38
                                                              ↓
                                               P40 → P41 → P42/P43/P44
                                                               ↓
                                                         P45/P46/P47
                                                               ↓
                                                         P50 → P51 → P52/P53
                                                               ↓
                                                         P54 → P55/P56/P57/P58
                                                               ↓
                                                         P60 → P61 → P62–P68
```

可并行的边界：

- P04/P05/P06 可在 P01 后并行；P07 只读核心 API，可与它们并行，但必须在 P10 前通过。
- P12–P25 的块类型可并行，但必须先固定 P10/P11 的 run 和 cell 约定。
- P28/P29 是 P31 的前置；不能先写 sink 再补预扫描。
- P39 是 P35 后的独立维护分支，不阻塞 P40；压缩进行时不能 apply update。
- P40/P41 可与 P32/P33 并行，只要 `Screen`/`Cell` 接口先冻结。
- P51/P54 可以并行；两者都依赖 P40 的命中/失效约定。

## 7. 测试基础（随任务推进）

### 7.1 测试层级

| 层级 | 工装 | 断言对象 | 运行频率 |
| --- | --- | --- | --- |
| 核心探针 | 只读核心 API 的 TUI 测试 | 隐藏 tile 无回调、显示归一、resize 重置、保守失效、锚点/检查点生命周期、`DisplayCursor` append 失效 | 每次提交 |
| 单元 | 普通 `#[test]` | 宽度、SGR、折行、间距、列表公式、锚点解码 | 每次提交 |
| cell 快照 | `vt100` + `insta` | 最终屏幕的字符、样式、宽字符占位、链接范围 | 每次提交 |
| 原始 ANSI | `--once` golden | 转义序列顺序、CRLF/LF、复位、OSC 8 | 每次提交 |
| 增量/闪烁 | vt100 分块喂入 | 中间帧不全空、不清屏、最终屏等于全量 | 每次提交 |
| PTY | `portable-pty` 或 tmux | 按键、resize、备用屏、退出恢复 | CI 或 nightly |
| 规模/性能 | 生成 fixture + 计数/bench | 首帧有界、滚动/跳转有界、帧数上界 | nightly/手动 |

### 7.2 固定测试环境

```bash
export TERM=xterm-256color
export COLORTERM=truecolor
export LANG=C.UTF-8
unset NO_COLOR
export COLUMNS=120
export LINES=40
```

- PTY 固定 120×40 与 80×24；resize 测试至少覆盖 80×24、120×40、160×50。
- 快照禁止包含时钟、随机 ID、绝对临时路径；测试输出必须确定性。
- 宽字符断言按 cell 网格，不按 UTF-8 字节数。

### 7.3 必备 fixture

- `spec/headings.md`：H1–H6、Setext、多物理行、标题内链接/图片/强调乱序。
- `spec/paragraphs.md`：软/硬换行、连续空格、tab、NBSP、emoji、CJK。
- `spec/inline.md`：粗体/斜体/删除线/代码嵌套、`**a ~~b~~**` 与 `~~a **b**~~` 两种顺序、双反引号、词中强调、`a &amp; b`、代码内多空格/tab。
- `spec/links.md`：行内/引用式/自动链接/邮件/空目标/标题/多链接/折行。
- `spec/images.md`：独占段落、混排、空 alt。
- `spec/quotes.md`：嵌套、引用内标题/列表/代码/表格、空引用行。
- `spec/lists.md`：无序/有序/任务/0 起始/10 起始/深嵌套/宽松与紧凑。
- `spec/code.md`：围栏/波浪线/缩进、语言标签、空行、未闭合、超长行、只有 info 无正文的空围栏、同 tile 与前后段落混排。
- `spec/overwide.md`：超宽标题、超宽代码行、宽表格；TUI 按正文内容区宽裁剪（含侧栏开关改变宽度）与 `--once` 溢出字节分别记录。
- `spec/hr.md`：三种分隔线、引用内。
- `spec/tables.md`：表头/无表体/缺列/超列/空单元格/CJK/链接/列数不一致。
- `spec/spacing.md`：所有顶层块组合的间距。
- `spec/footnotes.md`：有定义/无定义/多引用。
- `spec/html.md`：安全跳过/原文策略。
- `stream/large.md`：8 MiB 快测、64 MiB 夜间；开头标题、末尾标题、长段落、长表格。
- `links/anchors.md`：重复 slug、百分号转义、大小写、`#`、畸形转义、可命中的 `#a\_b`/`#&#x61;`，以及解码用例 `#a\&b`/`#&#x41;`/`#a&amp;b`。
- `sidebar/many-headings.md`：10k 标题、长标题、重复标题。
- `sidebar/huge-setext.md`：单个超长 Setext 标题，用于侧栏读取预算和锚点一次性构建计数。
- `stream/hidden-blocks.md`：空围栏代码块夹在长段落/列表之间，覆盖 `LayoutPass` 跳过与 TUI 独占 tile 两条路径。

### 7.4 验收测试模板

- **静态**：`render(fixture, width) -> Screen`，`width` 为正文内容区宽（无侧栏时等于终端宽），与 `insta` 快照比较；同时比较 `--once` 原始 ANSI。
- **增量**：用假 reader 按指定切点分块 `append`；每个切点渲染一次，最终 `finish` 后与同内容一次性解析的 `Screen` 比较。
- **不闪烁**：把每个输出 `write` 喂给 vt100；断言中间屏不清空、不回到顶部、正常帧不含 `2J`；最终屏等于全量重绘。
- **首屏**：假 reader 在首批后暂停；断言首帧已出现、`document.is_finished() == false`、读取字节不超过首批上限。
- **有界**：用 `Document::work()`、`Budget` 使用量、diff 行数、输出字节和 TUI 自己的计数器断言增长；墙钟只放 benchmark，不作为 CI 断言。

## 8. 阶段门槛

| 门槛 | 必须通过 | 证据 |
| --- | --- | --- |
| G0 骨架 | P00–P07 | `cargo check --manifest-path crates/mdtui-tui/Cargo.toml`；`git diff -- src Cargo.toml` 为空；测试工装自检通过；P07 的核心探针全部通过 |
| G1 静态 | P10–P26 | 所有 spec fixture（含 `spec/overwide.md`）在 1/20/80/120 宽通过 cell 快照；`--once` 原始 ANSI 通过 |
| G2 视口 | P28–P39 | 大文件首帧在 finish 前；空围栏/实体/代码空白流式与静态逐 cell 一致；append 后旧预扫描/`DisplayCursor` 不复用；滚动/跳转预算计数有界；检查点 payload 在双向失效并完成压缩后才计入上界 |
| G3 输出 | P40–P47 | 正常帧无清屏；中间帧不全空；正文 cell 不越过内容区右边界、任何 cell 不越过终端宽度；`content_width` 变化（终端 resize 改变宽度与固定终端宽下侧栏开关）重置高度/检查点且不复用旧宽度缓存；height-only resize（如 120×40→120×50）保留宽度相关高度/检查点缓存；最终屏等于全量；帧数/字节上界 |
| G4 交互 | P50–P58 | PTY 覆盖键位、侧栏开关（内容区宽度失效）、锚点、外链焦点、前进/后退历史、resize |
| G5 收口 | P60–P68 | 规模/闪烁/PTY/`--no-links`/有界状态验收；文档和差异记录完整；核心零改动 |

## 9. 关键风险与处理

| 风险 | 影响 | 处理 |
| --- | --- | --- |
| 表格列宽需全表最大值，与首屏速度冲突 | 首屏可能扫描巨表 | P36：首屏用 hint + 可见行，后台/滚动渐进补测；TUI 只需收敛到正文内容区宽内可见列前缀，静态 oracle/`--once` 才需要全表最大值；测试记录“暂定宽度”与最终值。 |
| `LayoutPass` 跳过完全隐藏的块 | 空围栏标签/块间距丢失，流式与静态不一致 | P29 做可见块预扫描，TUI 独占零可绘制块并复用静态块计划；P31 在 sink 内按源码顺序合并块事件；P65/G2 用空围栏 fixture 断言相等。 |
| `LayoutSink::text` 是原始 run | 实体/代码空白/CRLF 未归一 | P28 先预算化扫描显示文本段；sink 不读源码；P10/P65 覆盖 `a &amp; b` 和代码空白。 |
| `LayoutPass` 的 tile 行归属与折行状态复杂 | 滚动后错行/重复 | P29/P30/P31 固定 tile 边界检查点和“行尾 tile 计高、行首来源”约定；用静态渲染做 oracle；大文档用计数和随机切点；跨 tile 长行从行首/精确检查点恢复，更早检查点必须重放，记录重排成本。 |
| `Style` 只有位并集，无法恢复嵌套顺序 | `~~a **b**~~` 的外层删除线前景无法按参考实现；数学与强调并集时前景选择不明确 | P14 固定 `CODE > MATH > STRONG/EMPHASIS > STRIKE` 优先级；P07 探针固定位并集与 `MATH` 事实；P62 记录外层删除线差异。 |
| append 使显示预扫描整体过期 | 继续用旧 `DisplayCursor`/tile 前缀会渲染陈旧实体、Setext 或表格内容 | P28/P35 在 revision 变化时丢弃预扫描、块计划和游标，从锚点重扫；P07 探针覆盖 `StaleContinuation`。 |
| 超宽不折行块与固定 cell 网格冲突 | 终端自动折行会让实际行数偏离 `HeightIndex`/Screen 行号 | P04 定义正文内容区矩形，P12/P20/P23/P24 在进入 `Screen` 前裁剪到内容区宽；侧栏开关改变内容区宽时按 P42 重置；P41 禁止正文写越内容区、任何 cell 写越终端；P62 记录 TUI 裁剪与 `--once` 溢出的差异。 |
| 核心更新失效范围大于可见窗口 | 离屏检查点/高度暂不可用，错误假设会导致陈旧行 | §4.1/P35 明确保守失效契约；只承诺可见窗口按预算重测；P64 分开报告失效成本与重排成本。 |
| 流式期间重建 `HeadingAnchors` | 每次 append 扫描全部标题，长标题卡顿 | P55 推迟到 EOF 后一次预算内构建；流式期间只按 `HeadingId`/源码跳转；P67 记录一次性成本。 |
| 检查点 payload 长期累积 | 64 MiB 滚动后内存超界；视口之后的检查点不会被 `drain_before` 清掉 | P39 先 `invalidate` 窗口两侧、再 `drain_before`/`into_compaction`；上界 payload 只有完成压缩并释放退役输出才回收，P67 用压缩后的 `retired`/`arena_stats` 验收双向窗口，不以 `len()` 代替。 |
| 内部链接 destination 未解码 | 含反斜杠/实体的可匹配 destination 查不到标题 | P55 在 `HeadingAnchors::fragment` 前做有界 Markdown 解码；`links/anchors.md` 覆盖 `#a\_b`/`#&#x61;` 命中与 `#a\&b`/`#&#x41;` 解码结果。 |
| 长首行/隐藏前缀导致首屏空白 | 用户以为卡住 | P44：首批 32 KiB，最多再读 8 批 × 32 KiB（累计 288 KiB）；必要时显示“加载中”仍不回退到全文解析；测试暂停 reader。 |
| 宽字符/组合/emoji 宽度跨版本变化 | 快照漂移 | 固定依赖版本；P11 建立宽度表；快照按 cell 网格。 |
| 终端不支持 truecolor/OSC 8 | 视觉差异 | 固定测试环境；P02/P56/P60 定义并测试 `--no-links`（只关闭 OSC 8，保留样式和应用内链接）；不静默降级到 256 色。 |
| 自绘 ANSI 写出器复杂 | 闪烁/状态泄漏 | P40–P46 分步：diff、写出、首帧/resize、闪烁工装；每步都有 vt100 断言。 |
| 根 manifest 改动被误认为改核心 | 违反约束 | P01 默认使用独立 manifest，不改根 `Cargo.toml`；CI 检查 `git diff -- src Cargo.toml`。 |
| 交互/侧栏偷偷全量重建标题 | 大文档卡顿 | P51/P53 用 `new_headings`/`changed_headings`；每帧只读可见项；P67 审计。 |
| 外链打开命令注入 | 安全问题 | P56 不经过 shell；参数化启动；默认不打开；控制字符/非法 URL 测试。 |

## 10. 完成定义

全部任务标记为 `[x]` 后，必须同时满足：

- `cargo test -p mdtui3` 通过，且 `git diff -- src` 为空。
- `cargo test --manifest-path crates/mdtui-tui/Cargo.toml`、`cargo clippy --manifest-path crates/mdtui-tui/Cargo.toml --all-targets -- -D warnings`、`cargo fmt --manifest-path crates/mdtui-tui/Cargo.toml -- --check` 通过。
- 所有 spec fixture 的 cell 快照和 `--once` 原始 ANSI golden 通过。
- 首屏测试证明：首批后出帧、未 finish；读取字节受首批上限约束，布局/显示/输出不随全文长度线性增长；视口前无关前缀的 parser `input_bytes` 单独记录，不冒充 TUI 工作量。
- 增量测试证明：正常帧不清屏、中间帧不全空、最终屏等于全量渲染，且空围栏代码块、实体、代码空白三类路径与静态一致。
- PTY 测试覆盖启动、滚动、侧栏开关（固定终端宽下的内容区宽度变化）、标题跳转、内部锚点、外链焦点、`Alt-Left`/`Alt-Right`、`--no-links`、resize 120→80→120、退出和 panic 恢复。
- 规模测试记录 8 MiB/64 MiB、10k 标题、10k 表格行、超长 Setext 标题、长段落/长行的工作量与帧指标；检查点 payload 在双向失效并完成压缩后只随双向保留窗口增长，EOF 锚点构建单独记录。
- 规格差异文档明确列出：TUI 行寻址 vs `--once` CRLF、HTML 安全差异、终端能力差异、嵌套强调外层前景的固定优先级差异、超宽不折行块在 TUI 按正文内容区宽裁剪而 `--once` 溢出（侧栏开关会改变该宽度）、表格渐进宽度暂定行为、流式期间片段链接推迟到 EOF 解析及 destination 解码规则。
- 若仍有未完成项，保持 `[ ]` 并在“已知限制”中说明，不以截图或单次耗时替代验收。

## 11. 执行约定

- 每轮只推进一个任务；任务完成后在 ID 前标记 `[x]`，更新证据和依赖它的下一任务。
- P07 是硬门槛：探针失败时先改计划或另开核心任务，不得先写渲染实现再补适配。
- 每个任务先写失败测试或先固定 golden，再实现；不能先写大段实现再补测试。
- 若任务需要核心 API 变更，停止该任务，另开核心任务；TUI 分支不得夹带 `src/**` 修改。
- 发现参考规格本身矛盾时，先记录到差异清单，选一个可测试的行为，不要静默偏离。
