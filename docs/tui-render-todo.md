# TUI 渲染 CLI 任务拆分（TODO）

状态：任务规划，尚未实现。本文只拆分工作，不包含核心库修改，也不把任何任务标记为已完成。

修订（2026-10-31）：按核心行为评审新增 P07 探针、P28/P29 可见区间预扫描与分段调度、P39 检查点维护；修正 resize 重置、更新失效范围、流式锚点构建和侧栏标题预算的假设。

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
3. **宽度变化没有隐含自动失效。** `HeightIndex` 的高度和 `CheckpointIndex` 的折行状态都与宽度绑定。宽度变化时必须执行 `heights.reset(estimated_rows)` 和 `checkpoints.clear()`，再按顶部源码锚点重新局部布局；`reset` 会把隐藏 tile 恢复成估计值，P29 必须重新识别并 `suppress`/`measure`。仅清屏重绘会复用旧宽度结果。
4. **更新失效是保守的。** `HeightIndex::apply` 从最早变化 tile 失效到末尾，`CheckpointIndex::apply` 从该 tile 失效到容量；只有 `LayoutPolicy` 可能缩窄。计划不得声称“只失效可见范围”，也不得把 P64 的结论建立在这一点上。可见范围只负责按预算重新测量；离屏状态允许保持估计值或无效值。
5. **`HeadingAnchors` 是 revision 绑定的。** 任意 append 都会使旧索引返回 `StaleContinuation`，重新构建会重读所有标题字节。流式期间不构建完整 slug 索引；内部链接解析推迟到 `finish` 后一次预算内构建，期间只允许按 `HeadingId`/源码跳转。
6. **检查点 payload 不会随失效自动释放。** `invalidate`/`apply` 只清计数，`drain_before` 和 `into_compaction` 才释放历史状态。滚动型阅读必须有显式淘汰/压缩任务，P67 用 `arena_stats`/`len()` 证明上界。
7. **侧栏标题读取也必须预算化。** `DisplayExcerpt` + `display_cursor` 可以在 `Budget` 下按源码/显示字节/行数截断；侧栏不使用无预算的 `display_runs_in(heading.content)`。

## 5. 任务分解

任务编号用于依赖和验收记录，不表示执行顺序之外的优先级。所有任务初始为 `[ ]`（表中为紧凑未画勾选框）；完成时在 ID 前标记 `[x]`（例如 `| [x] P12 |`），并补一行证据（测试名、命令、快照名或计数报告）。

### 阶段 P0：基线、骨架和测试基础

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P00 | 固化参考基线与差异清单 | 无 | 记录参考文件 SHA-256；建立 §0–§9 对照表；明确 TUI 与 `--once` 的差异：TUI 以 cell 网格等价为准，不做行尾 CRLF，交互帧用光标寻址；`--once` 才逐语义段字节精确；不复制 HTML 块崩溃。 |
| P01 | 新建独立 TUI crate | P00 | 新增 `crates/mdtui-tui`，使用独立 `Cargo.toml` + `mdtui3 = { path = "../.." }`；默认不改根 `Cargo.toml`/根 lock，只改 TUI crate 自身 manifest/lock；如确需 workspace member，先取得用户同意；`git diff -- src Cargo.toml` 为空；`cargo check --manifest-path crates/mdtui-tui/Cargo.toml` 通过。 |
| P02 | CLI 参数与终端生命周期骨架 | P01 | 支持 `--once`、`--width`、`--height`、`--open-links`、`--no-links`、`--help`、文件参数（`--width/--height` 供 `--once`/测试固定几何，TUI 模式以终端尺寸为准）；`--no-links` 只关闭 OSC 8 输出，不关闭样式、内部锚点和应用内链接焦点；非 TTY 时 TUI 模式拒绝启动并提示；raw mode/备用屏/光标/panic 恢复有 PTY 测试；退出码覆盖成功、参数错、I/O 错。 |
| P03 | 测试工装与固定环境 | P01 | 提供 vt100 cell 解析、insta 快照、假时钟、临时文件、PTY 封装；固定 `TERM=xterm-256color`、`COLORTERM=truecolor`、`LANG=C.UTF-8`、120×40；工装自身有 1 个通过/失败自检。 |
| P04 | Screen/Cell 数据模型 | P03 | 定义 `Cell`（符号、样式角色、宽度、链接 ID、宽字符占位）、`Screen`、行来源映射；单元测试覆盖 CJK、emoji、组合字符、宽字符占位和边界裁剪。 |
| P05 | 主题与 SGR 编码器 | P03 | 固定参考 §1 的 RGB；按“粗体→斜体→下划线→删除线→前景→背景”发射；独立复位；单元/golden 覆盖 H1–H6、代码、链接、表格框线。 |
| P06 | 静态 fixture 语料骨架 | P03 | 建立 `tests/fixtures/`：标题、段落、强调、链接、图片、引用、列表、代码、分隔线、表格、脚注/HTML、块间距、宽字符；每个 fixture 有 README 说明期望覆盖的规格章节。 |
| P07 | 核心集成探针 | P01、P03 | 在 TUI crate 的测试中用只读核心 API 固定 §4.1 的事实：空围栏块在 `LayoutPass` 中无回调；`runs_in` 与 `display_cursor` 对 `&amp;`/代码空白输出不同；宽度变化后必须 `reset`/`clear`；append 后高度/检查点保守失效；`HeadingAnchors` 在 append 后失效；检查点 payload 在 `drain_before`/`into_compaction` 前仍占用。探针全部通过后才允许开始 P10；若任一事实不成立，停止并重写 §4.1，不得带假设进入实现。 |

### 阶段 P1：静态语义渲染（先不接视口/终端）

这一阶段把 `Document` 的语义 run 变成 `Screen`，只在内存中验证 cell 网格；不要求交互和流式首屏。

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P10 | run 摄取与归一化 | P04、P05 | 静态整篇可用 `display_runs_in`，逐帧路径只允许 P28 的预算化显示段；跳过 HIDDEN；保留 code 空白、实体替换、软/硬换行、容器前缀隐藏；新增 `a &amp; b`、代码内多空格/tab、CRLF 单元测试和 §3.2/§3.4 fixture。 |
| P11 | Unicode 字素与显示宽度 | P04、P10 | 采用固定版本 `unicode-segmentation`/`unicode-width`；覆盖 `•`、`│`、`─`、`═`、CJK、emoji、组合、ZWJ、NBSP、tab；宽字符只写首 cell，后续 cell 标记占位。 |
| P12 | 标题渲染与文字拼合 | P10、P11 | 按 §3.1/§3.12：整行统一标题样式；H1–H5 下划线变体；H6 无下划线；超宽不折行；链接/图片文字丢弃；强调文字按规则重排。测试至少包含规格中的 ``# **B** *I* `c` [l](u) ~~s~~`` 例子。 |
| P13 | 段落渲染与折行 | P10、P11 | 软换行变空格、硬换行消失、首尾空白裁剪、连续空格保留；逐字符贪心折行；宽 0/1 退化；测试所有 §3.2/§5 段落案例。 |
| P14 | 强调/代码内联样式 | P10、P05 | 粗体、斜体、删除线、代码的前景/背景；嵌套时外层决定前景；代码保留外层标志；每段独立复位；golden 覆盖嵌套组合。 |
| P15 | 链接与 OSC 8 | P10、P14 | 链接文字 `#8BE9FD` + 下划线；OSC 8 开在样式前、关在复位后；同行多链接独立；空地址只样式；标题忽略；引用式与行内式输出一致；邮件自动链接的 OSC 8 目标用裸地址（不写 `mailto:`）；表格内不发 OSC 8；`--no-links` 全局关闭 OSC 8 但保留样式；测试原始字节。 |
| P16 | 图片、脚注、定义行与 HTML | P10 | 图片不可见（独占段落零字节，混排只剩两侧空格）；有定义的脚注引用/定义不可见，无定义按原文；链接/引用定义行不可见；HTML 安全策略明确且不崩溃。 |
| P17 | 列表标记、编号与任务状态 | P10、P11 | `•`、`N.`、`⬜`/`✅` 的颜色；起始编号；完成项整行删除线；标记后空格同色；嵌套缩进公式；任务项按 1 格；空项消失；测试混合嵌套和 0/10/99 起始。 |
| P18 | 列表文字拼合与按词折行 | P17 | 按 §3.12 拼合；超长词硬断；CJK 单字成词；词间空格规则；续行缩进含正文样式；可用宽公式 `总宽−缩进−标记−1`；测试 `hello, world` 等规格案例。 |
| P19 | 引用块前缀 | P10、P11 | 段落行加 `│ `；嵌套只一层；标题/列表/代码/表格/分隔线在引用内无前缀；空引用行消失；折行每行带前缀；测试混合容器。 |
| P20 | 代码块骨架 | P10、P11、P05 | 每行 2 空格缩进；前景/背景作用于缩进和内容；空行输出 2 个带样式空格；源 CRLF 归一；未闭合围栏照常；长行不折行；测试围栏/波浪线/缩进代码。 |
| P21 | 代码块语言标签 | P20 | `BlockKind::Code { info }` 有值时首行 `[完整信息串]`，灰色、无缩进；无信息时无该行；首尾空白裁剪、内部原样保留；测试多余参数。 |
| P22 | 分隔线 | P10、P11 | `---`/`***`/`___` 统一输出 `─` 重复宽度；灰色；引用内无前缀；测试宽度 1/2/80。 |
| P23 | 表格几何与列宽 | P10、P11 | 从 `Table` 的行/列/单元格读；左右各 1 空格；列宽取可见内容最大宽度；CJK 按 2 格；缺列补空、超列丢弃、列数不一致回退段落；测试表头/无表体/空单元格。 |
| P24 | 表格渲染与样式 | P23、P05、P15 | 顶/底/表头分隔/竖线字符准确；表头无特殊样式；框线、padding、补齐空格均灰色且分段独立；内容左对齐；内联样式保留；表格内链接只样式不出 OSC 8；行间 LF、块末 CRLF（`--once`）。 |
| P25 | 顶层块间距 | P10–P24 | 按 §4 实现块间空行表；引用/列表作为整块；代码/分隔线与列表紧贴；首块无前导空行；用组合 fixture 断言最终 cell 行号。 |
| P26 | 静态整篇渲染与 `--once` 骨架 | P12–P25 | 把静态 `Screen` 接到 `--once`；先固定宽度、不可交互；对 fixture × 宽度 1/20/80/120 生成 cell 快照；与规格逐节对照。 |

### 阶段 P2：视口、布局和增量更新

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P28 | 显示文本预扫描 | P26、P07 | 对本次要布局的源码区间用 `display_cursor` + 共享 `Budget` 生成 `DisplaySegment`（源码 span、显示文本/style/link/block），按 tile 边界组织，替换跨边界时用 `DisplayRun::replacement` 归属到包含起始字节的 tile；实体、代码空白、CRLF、HIDDEN 跳过都在此归一。段只覆盖视口 + overscan；记录段数/字节数。测试包含实体跨 tile、代码空白跨 run。预算不足时保留上一屏和已扫描的完整 tile 前缀，下一帧从断点继续；不得回退到原始 run，也不得突破帧预算。 |
| P29 | 可见块预扫描与分段调度 | P25、P26、P28 | 先把 P21/P22/P25 的块级输出抽成无屏幕副作用的 `BlockPlan`，静态和增量共用，避免两套间距/标签规则。在 `Document::blocks()` 上按源码二分，只扫描与可见 tile 相交的块。tile 是否交给 `LayoutPass` 直接复用 `document.next_drawable(tile.start).is_some_and(|at| at < tile.end)`（含空 alt 图片对象），不要另写语义判断。把 tile 分成“可绘制段”“TUI 独占隐藏块 tile”“零行隐藏段”，块级事件按源码位置合并进 sink。段边界落在块边界，保存/恢复 `RenderCheckpoint`，避免跨段折行状态串线；TUI 独占段由 TUI 排版并 `measure`，不再交给 `LayoutPass`。空围栏代码块（有 info、无正文）必须产生标签行和块间距；同一 tile 内“段落 + 空围栏”也按源码顺序输出。用静态渲染做 oracle，逐 cell 相等。 |
| P30 | 渲染检查点类型 | P26、P29 | 定义 `RenderCheckpoint`（块 ID/类型、折行列、列表缩进/标记、引用前缀、代码/表格状态）；`Copy` 或小对象；段边界保存/恢复；单元测试 `CheckpointIndex` 存取和 revision 失效。 |
| P31 | `LayoutSink` 事件采集与 tile 行归属 | P28、P29、P30 | 实现 `begin_tile/text/object/end_tile`，回调只消费 P28/P29 的预扫描结果，不做源码读取或二次查询；规定“一行归属于其首字符所在 tile”；块级事件与文本段按源码顺序合并；返回 tile 实际行数（含隐藏块标签/间距）。用计数器断言单次回调工作量受预扫描结果大小约束。 |
| P32 | 首屏局部排版 | P31 | 从 tile 0 开始按 P29 的分段计划推进：可绘制段交给 `LayoutPass`，TUI 独占隐藏块由 TUI 直接排版并 `measure`；预算在段边界暂停，中途耗尽时保留 sink/显示段游标并在下一帧从同一 tile 继续；不能跨越未预扫描的 tile；不调用 `finish`；测试首帧不含全文布局计数，并包含空围栏代码块首屏用例。 |
| P33 | 滚动与导航 | P31 | Up/Down/PageUp/PageDown/Home/End；`HeightIndex::viewport` + overscan；到达首尾不越界；行/页只处理视口附近；测试大文件上下滚动和边界。 |
| P34 | 源码到行的精确细化 | P30、P33 | 标题/内部链接跳转后，用最近检查点或局部起点把 `ScrollTarget.source` 细化到实际行；同一 tile 多标题可分别到达；细化预算耗尽可继续；测试 `Navigator::reconcile`。 |
| P35 | 增量应用与锚点保持 | P31、P33 | 每个 `Update` 依次合并到 `HeightIndex`/`CheckpointIndex`，并明确记录核心的保守失效契约：高度从最早变化 tile 失效到末尾，检查点从该 tile 失效到容量；TUI 只承诺按预算重新测量可见窗口，不承诺离屏状态仍有效。追加/EOF 后保持顶部源码锚点；缓存命中前校验 revision/token；测试滚动中追加、离屏 tile 回滚后重新测量、无陈旧折行。 |
| P36 | 表格宽度渐进测量 | P23、P31 | 首屏只用 `source_width_hint` + 可见行建立宽度上界；后台/滚动按预算补测未访问行；最终宽度收敛到全表最大值；测试首屏不扫全表、finish 后与全量一致。 |
| P37 | EOF 最终校准 | P32、P36 | `finish` 后执行一次有预算的最终校准；保持当前阅读位置；暂定标题/链接/表格可更新；UI 不阻塞退出；测试流式前缀与同内容一次性解析最终 cell 一致。 |
| P38 | 未分类尾部与暂定内容 | P31 | 按 `Document::pending()` 和当前 revision 显示已提交/暂定文本；追加后只更新变化部分；不把未分类尾部误当隐藏；测试长行/晚闭合分隔符/Setext 迟确认。 |
| P39 | 检查点保留与压缩维护 | P30、P35 | 维护“视口前后各 2 屏”的检查点保留窗口；每帧/空闲用共享 `Budget` 调 `drain_before` 丢弃窗口外 payload，超过阈值或 EOF 后空闲时用 `into_compaction` 压缩并 `finish`。压缩期间只缓冲一个帧周期的 update，不并发 apply。用 `len()`/`arena_stats` 断言 64 MiB 连续滚动后有效 payload 只随窗口增长。 |

### 阶段 P3：屏幕差分、闪烁和流式节奏

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P40 | Cell diff 算法 | P04、P31 | 比较前后 `Screen`，按行生成最小变更 run；宽字符/占位 cell 必须成对失效；行尾裁剪只在必要时使用；单元测试 diff 与全量重绘最终屏幕一致。 |
| P41 | 增量 ANSI 写出器 | P40、P05、P02 | 光标定位 + SGR/OSC 8 + 必要清行；`--no-links` 时不写任何 OSC 8；正常帧不发 `ESC[2J`，也不无条件回到 `ESC[H`（绝对定位 `CSI r;cH` 允许）；样式/链接每行闭合；写出失败可恢复；vt100 最终 cell 与参考屏幕一致，序列白名单通过。 |
| P42 | 首帧、resize 与备用屏 | P41、P02、P07 | 首帧一次清屏+归位；宽度变化时先记录顶部源码锚点，再执行 `heights.reset(非零估计值)`、`checkpoints.clear()`，丢弃当前 `LayoutPass`/恢复 token，清空显示段/块计划/diff 基线，然后重新局部布局；高度不变只 resize 时可保留宽度相关缓存。其余帧不清屏；PTY 覆盖 120×40→80×24→120×40，并断言旧宽度高度/检查点 token 不被复用、顶部源码位置保持。 |
| P43 | 流式读取与帧合并 | P02、P35 | 64 KiB 有界 channel；首帧不合并；之后按截止时间/事件数合并；读取慢时背压；不丢最后一个 update；测试 1000 次追加的帧数上界和最终屏幕。 |
| P44 | 首屏预读策略 | P43、P32 | 默认首批 32 KiB（读取线程可用 64 KiB 块）；若无可绘制行，最多再读 8 批/256 KiB 后必须出帧；首帧前不 `finish`；测试暂停 reader 时首帧仍出现。 |
| P45 | 闪烁检测工装 | P41、P43 | 用 vt100 分块喂入每个输出 write；检查中间帧不全空、不清屏、不跳回顶部；与每步全量渲染对照；输出失败/早退也恢复终端。 |
| P46 | 闪烁加固 | P45 | 双缓冲提交后才写；隐藏光标；必要时使用同步输出 `CSI ? 2026 h/l`；每行独立闭合样式/链接；连续 3 次运行字节/帧数稳定。 |
| P47 | 工作量与帧指标 | P41、P45 | 记录读取字节、产出 update 数、布局预算消耗、恢复 tile 数、diff 行数、输出字节、帧耗时；测试断言计数上界，benchmark 单独记录墙钟。 |

### 阶段 P4：交互、跳转和侧栏

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P50 | 事件循环与键位表 | P42、P43 | `q` 退出；方向/Page/Home/End；`Tab`/`Shift-Tab` 切换正文/侧栏/链接焦点；`s` 开关侧栏；`Enter` 激活；`Alt-Left/M-Left` 后退，`Alt-Right/M-Right` 前进；测试事件到状态转换（含空历史时前进/后退为 no-op）。 |
| P51 | 侧栏可见标题模型 | P11、P31 | 按 `HeadingIndex` 顺序只取当前可见条目；标题用 `DisplayExcerpt::new` + `next(document, &mut Budget)` 读取，限制源码字节、显示字节和行数，达到 cell 宽度立即停止；预算不足时显示省略号/上一帧缓存并在下一帧重试。只缓存当前可见标题文本，10k 标题、单个超长 Setext 标题都有读取字节/对象计数上界。 |
| P52 | 侧栏布局与当前章节 | P51、P33 | 宽度建议 `min(40, max(20, 30%))`，窄屏隐藏；缩进体现层级；当前章节高亮随正文顶部位置更新；无标题空栏；测试 20/40/80/160 列。 |
| P53 | 侧栏增量更新与焦点 | P52、P35 | 用 `Update.new_headings`/`changed_headings` 标记脏项；只重建可见项；选择/滚动/焦点独立于正文；跳转后正文与侧栏都保持正确位置；不重排全文。 |
| P54 | 链接命中表与焦点 | P15、P40 | 每个可见行保存 `x0..x1 -> LinkId`；`Tab` 循环当前屏链接并高亮；表格内链接按规格不进入命中表；滚动/更新后失效重建；测试多链接、跨行、CJK 列宽。 |
| P55 | 内部锚点跳转 | P34、P54、P53 | 识别 `#fragment`；流式期间（未 `finish`）不构建完整 `HeadingAnchors`，Markdown 片段链接显示“待文档完成”，仅允许 `HeadingId`/源码跳转；EOF 后在空闲帧用 `HeadingAnchors::new` + `Budget` 分帧构建一次并缓存，append 不会发生（核心 EOF 后拒绝 append）。构建完成后解析百分号、重复 slug、`#` 到文档首；成功才压入历史；测试大小写、空目标、畸形转义、构建期间重复跳转不重复启动。 |
| P56 | 外链策略与 OSC 8 | P15、P54、P02 | 外链只写入 OSC 8 元数据，不在默认路径启动进程；`--open-links` 时用无 shell 的参数化方式打开（邮件客户端可用 `link_url` 的 `mailto:`）；`--no-links` 完全不发 OSC 8，但保留链接样式和应用内焦点/内部跳转；测试 `http/https/mailto`、空目标、控制字符注入拒绝、`--no-links` 原始字节无 OSC 8。 |
| P57 | 跳转历史 | P50、P55 | 保存源码位置 + 视口内偏移；正文滚动刷新当前项；新跳转截断前进分支；`Alt-Right/M-Right` 恢复前进分支；后退/前进在 append/resize 后仍回到同一源码；测试连续标题跳转、同 tile 多标题、失败跳转不移动历史、前进分支截断与恢复。 |
| P58 | 当前章节与侧栏选中分离 | P52、P55 | 正文当前章节自动高亮；侧栏选中是独立状态；跳转失败/取消不改变正文位置；测试聚焦、滚动、跳转交错。 |

### 阶段 P5：CLI、验收和发布门槛

| ID | 任务 | 依赖 | 验证 / 完成定义 |
| --- | --- | --- | --- |
| P60 | 错误、极窄终端与帮助 | P02、P52 | 文件不存在/不可读/非普通文件、非 TTY、参数错、写出错均有明确退出码；小于 20×3 显示尺寸提示且可退出；`--help` 覆盖模式、`--open-links`/`--no-links`、全部键位（含 `Alt-Right/M-Right` 前进）；测试不 panic。 |
| P61 | `--once` 完整规格模式 | P26、P41 | 等待全文后输出参考 §0–§7：`ESC[2J ESC[H`、CRLF、表格内 LF、零字节空输入；与 fixture 的原始 ANSI golden 对比；TUI 模式不复用 CRLF 假设。 |
| P62 | 静态规格符合性矩阵 | P26、P61 | 逐节核对 §0–§6 和 §9；每条有 fixture/测试或明确差异；HTML 崩溃差异、ratatui 差异、TUI 行寻址/复位分段差异单独记录。 |
| P63 | 首屏与规模验收 | P44、P47 | 8 MiB 快测、64 MiB 手动/夜间；首帧发生在 `finish` 前，读取/布局计数有界；同一视口下 128/256/512 倍无关前缀不改变首帧工作量；记录证据。 |
| P64 | 滚动/跳转规模验收 | P33、P34、P55 | 分别报告“核心保守失效成本”和“TUI 可见窗口重排成本”：前者允许从变化 tile 到末尾失效，不把它算作有界；后者必须受 `Budget` 限制。深标题跳转、连续翻页、远距离返回只重排预算内 tile/恢复；不因文档前面增加无关内容而线性增长；EOF 后锚点索引构建单独记为一次性成本。 |
| P65 | 闪烁与增量验收 | P45、P46、P43 | 流式 1000 次追加：首帧立即出现；中间帧不为空；无正常帧 `2J`；最终屏等于全量渲染（含空围栏代码块、实体、代码空白）；帧数上界；连续 3 次稳定。 |
| P66 | PTY 端到端验收 | P50–P57、P60 | 真实 PTY 跑 120×40 和 80×24；覆盖启动、首屏、滚动、侧栏开关、目录跳转、锚点跳转、`Alt-Left`/`Alt-Right` 历史、外链焦点、`--no-links`、resize 120→80→120、退出、panic 恢复；断言最终 cell 与状态。 |
| P67 | 有界状态与内存审计 | P35、P39、P51、P54 | 证明 frame buffer 只随终端尺寸、侧栏只保留可见标题文本、检查点 payload 只保留保留窗口、表格宽度缓存不复制全表正文、EOF 后不残留标题全文副本；用 `CheckpointIndex::len()`/`arena_stats` 记录 64 MiB 滚动、10k 标题/10k 表格行、超长 Setext 标题的容量计数。 |
| P68 | 文档、差异和发布收口 | P62–P67 | README/TUI 使用说明、键位、`--no-links` 语义、构建命令、规格差异、性能证据、已知限制（含“流式期间 Markdown 片段链接推迟到 EOF 解析”）；`cargo fmt`/`clippy`/测试/快照全部通过；只允许 TUI crate、其独立 manifest/lock 和文档变更。 |

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
| 核心探针 | 只读核心 API 的 TUI 测试 | 隐藏 tile 无回调、显示归一、resize 重置、保守失效、锚点/检查点生命周期 | 每次提交 |
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
- `spec/inline.md`：粗体/斜体/删除线/代码嵌套、双反引号、词中强调、`a &amp; b`、代码内多空格/tab。
- `spec/links.md`：行内/引用式/自动链接/邮件/空目标/标题/多链接/折行。
- `spec/images.md`：独占段落、混排、空 alt。
- `spec/quotes.md`：嵌套、引用内标题/列表/代码/表格、空引用行。
- `spec/lists.md`：无序/有序/任务/0 起始/10 起始/深嵌套/宽松与紧凑。
- `spec/code.md`：围栏/波浪线/缩进、语言标签、空行、未闭合、超长行、只有 info 无正文的空围栏、同 tile 与前后段落混排。
- `spec/hr.md`：三种分隔线、引用内。
- `spec/tables.md`：表头/无表体/缺列/超列/空单元格/CJK/链接/列数不一致。
- `spec/spacing.md`：所有顶层块组合的间距。
- `spec/footnotes.md`：有定义/无定义/多引用。
- `spec/html.md`：安全跳过/原文策略。
- `stream/large.md`：8 MiB 快测、64 MiB 夜间；开头标题、末尾标题、长段落、长表格。
- `links/anchors.md`：重复 slug、百分号转义、大小写、`#`、畸形转义。
- `sidebar/many-headings.md`：10k 标题、长标题、重复标题。
- `sidebar/huge-setext.md`：单个超长 Setext 标题，用于侧栏读取预算和锚点一次性构建计数。
- `stream/hidden-blocks.md`：空围栏代码块夹在长段落/列表之间，覆盖 `LayoutPass` 跳过与 TUI 独占 tile 两条路径。

### 7.4 验收测试模板

- **静态**：`render(fixture, width) -> Screen`，与 `insta` 快照比较；同时比较 `--once` 原始 ANSI。
- **增量**：用假 reader 按指定切点分块 `append`；每个切点渲染一次，最终 `finish` 后与同内容一次性解析的 `Screen` 比较。
- **不闪烁**：把每个输出 `write` 喂给 vt100；断言中间屏不清空、不回到顶部、正常帧不含 `2J`；最终屏等于全量重绘。
- **首屏**：假 reader 在首批后暂停；断言首帧已出现、`document.is_finished() == false`、读取字节不超过首批上限。
- **有界**：用 `Document::work()`、`Budget` 使用量、diff 行数、输出字节和 TUI 自己的计数器断言增长；墙钟只放 benchmark，不作为 CI 断言。

## 8. 阶段门槛

| 门槛 | 必须通过 | 证据 |
| --- | --- | --- |
| G0 骨架 | P00–P07 | `cargo check --manifest-path crates/mdtui-tui/Cargo.toml`；`git diff -- src Cargo.toml` 为空；测试工装自检通过；P07 的核心探针全部通过 |
| G1 静态 | P10–P26 | 所有 spec fixture 在 1/20/80/120 宽通过 cell 快照；`--once` 原始 ANSI 通过 |
| G2 视口 | P28–P39 | 大文件首帧在 finish 前；空围栏/实体/代码空白流式与静态逐 cell 一致；宽度 120→80→120 重置高度与检查点；滚动/跳转预算计数有界；检查点 payload 受保留窗口约束 |
| G3 输出 | P40–P47 | 正常帧无清屏；中间帧不全空；最终屏等于全量；帧数/字节上界 |
| G4 交互 | P50–P58 | PTY 覆盖键位、侧栏、锚点、外链焦点、前进/后退历史、resize |
| G5 收口 | P60–P68 | 规模/闪烁/PTY/`--no-links`/有界状态验收；文档和差异记录完整；核心零改动 |

## 9. 关键风险与处理

| 风险 | 影响 | 处理 |
| --- | --- | --- |
| 表格列宽需全表最大值，与首屏速度冲突 | 首屏可能扫描巨表 | P36：首屏用 hint + 可见行，后台/滚动渐进补测；最终 finish 收敛；测试记录“暂定宽度”与最终值。 |
| `LayoutPass` 跳过完全隐藏的块 | 空围栏标签/块间距丢失，流式与静态不一致 | P29 做可见块预扫描，TUI 独占零可绘制块并复用静态块计划；P31 在 sink 内按源码顺序合并块事件；P65/G2 用空围栏 fixture 断言相等。 |
| `LayoutSink::text` 是原始 run | 实体/代码空白/CRLF 未归一 | P28 先预算化扫描显示文本段；sink 不读源码；P10/P65 覆盖 `a &amp; b` 和代码空白。 |
| `LayoutPass` 的 tile 行归属与折行状态复杂 | 滚动后错行/重复 | P29/P30/P31 先固定分段与归属约定；用静态渲染做 oracle；大文档用计数和随机切点。 |
| 核心更新失效范围大于可见窗口 | 离屏检查点/高度暂不可用，错误假设会导致陈旧行 | §4.1/P35 明确保守失效契约；只承诺可见窗口按预算重测；P64 分开报告失效成本与重排成本。 |
| 流式期间重建 `HeadingAnchors` | 每次 append 扫描全部标题，长标题卡顿 | P55 推迟到 EOF 后一次预算内构建；流式期间只按 `HeadingId`/源码跳转；P67 记录一次性成本。 |
| 检查点 payload 长期累积 | 64 MiB 滚动后内存超界 | P39 保留窗口 + `drain_before`/`into_compaction`；P67 用 `len()`/`arena_stats` 验收。 |
| 长首行/隐藏前缀导致首屏空白 | 用户以为卡住 | P44：最多再读 8 批/256 KiB；必要时显示“加载中”仍不回退到全文解析；测试暂停 reader。 |
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
- 首屏测试证明：首批后出帧、未 finish、读取/布局不随全文长度线性增长。
- 增量测试证明：正常帧不清屏、中间帧不全空、最终屏等于全量渲染，且空围栏代码块、实体、代码空白三类路径与静态一致。
- PTY 测试覆盖启动、滚动、侧栏、标题跳转、内部锚点、外链焦点、`Alt-Left`/`Alt-Right`、`--no-links`、resize 120→80→120、退出和 panic 恢复。
- 规模测试记录 8 MiB/64 MiB、10k 标题、10k 表格行、超长 Setext 标题、长段落/长行的工作量与帧指标；检查点 payload 只随保留窗口增长，EOF 锚点构建单独记录。
- 规格差异文档明确列出：TUI 行寻址 vs `--once` CRLF、HTML 安全差异、终端能力差异、表格渐进宽度暂定行为、流式期间片段链接推迟到 EOF 解析。
- 若仍有未完成项，保持 `[ ]` 并在“已知限制”中说明，不以截图或单次耗时替代验收。

## 11. 执行约定

- 每轮只推进一个任务；任务完成后在 ID 前标记 `[x]`，更新证据和依赖它的下一任务。
- P07 是硬门槛：探针失败时先改计划或另开核心任务，不得先写渲染实现再补适配。
- 每个任务先写失败测试或先固定 golden，再实现；不能先写大段实现再补测试。
- 若任务需要核心 API 变更，停止该任务，另开核心任务；TUI 分支不得夹带 `src/**` 修改。
- 发现参考规格本身矛盾时，先记录到差异清单，选一个可测试的行为，不要静默偏离。
