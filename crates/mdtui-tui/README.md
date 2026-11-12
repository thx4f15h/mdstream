# mdtui-tui

`mdtui-tui` 是本工作区 `mdstream-core` 的终端 Markdown 阅读器。它提供两套互不替代的输出路径：

- 交互 TUI：固定 cell 网格、增量 diff、备用屏和可导航侧栏。
- `--once`：全文读取并 finish 后，输出参考规格对齐的原始 ANSI 字节。

TUI 的验收 oracle 是 `Screen`/vt100/PTY cell 网格；`--once` 的验收 oracle 是
`tests/golden/*.ansi` 原始字节。两者不能互换，详见
[静态规格符合性矩阵](../../docs/tui-render-compliance.md) 和
[渲染基线与差异](../../docs/tui-render-baseline.md)。

## 构建与运行

以下命令从 workspace 根目录执行，使用项目固定的 Nix 工具链；产物目录由 `cargo metadata` 决定。

```bash
nix develop -c cargo build -r -p mdtui-tui
# 需要真实 TTY
nix develop -c cargo r -r -p mdtui-tui -- README.md
# 非 TTY 一次性 ANSI 输出
nix develop -c cargo r -r -p mdtui-tui -- --once crates/mdtui-tui/tests/fixtures/spec/images.md
```

来源和逐项契约见 [workspace 集成说明](../../docs/tui-integration.md)。内部 `mdtui3` import 是 `mdstream-core` 的依赖别名；核心仍无运行时依赖。

## CLI

```text
mdtui-tui [OPTIONS] <FILE>
```

| 选项 | 语义 |
| --- | --- |
| `--once` | 等全文读取并 `Document::finish()` 后输出原始 ANSI。非空内容以一次 `ESC[2J ESC[H` 开始；普通行和最后一行用 CRLF，表格内部行之间用 LF。空输入/纯空白输出零字节。标题、代码和表格不裁剪；段落、列表、引用按 `--width` 折行。 |
| `--width N` | `--once` 宽度，`1..=1000`，默认 `120`。TUI 模式禁止使用。 |
| `--height N` | `--once` 高度，`1..=1000`，默认 `40`。垂直溢出以退出码 `5` 报错。TUI 模式禁止使用。 |
| `--table-min-width N` / `--table-max-width N` | TUI 的单元格内容宽度上下限，不含 padding/边框。min 为 `0..=1000`（默认 0），max 为 `1..=1000`（默认不设上限），须 min ≤ max；`--once` 禁用。 |
| `--open-links` | 允许在 Link 焦点按 Enter 时用平台 opener 打开显式外链。只影响已激活外链，不改变渲染字节或默认聚焦行为。 |
| `--no-links` | 完全关闭 OSC 8。链接样式、Link 焦点和内部 `#fragment` 跳转仍保留。不能与 `--open-links` 同用。 |
| `-h`, `--help` | 输出帮助。 |

链接模式：

- 默认：写出 OSC 8 元数据；Enter 只返回外链状态，不启动进程。
- `--open-links`：只有明确激活外链时才打开；Linux 使用 `xdg-open` 加单个 argv 参数，不经过 shell。非 Linux 返回 Unsupported。
- `--no-links`：原始输出无 `\x1b]8;`，但视觉链接和下划线不变，内部锚点仍可跳转。

退出码：

| 码 | 含义 |
| --- | --- |
| `0` | 成功 |
| `2` | 参数错误 |
| `3` | 文件或终端 I/O 错误 |
| `4` | TUI 模式不是交互式 TTY；非 TTY 请用 `--once` |
| `5` | 静态渲染错误，例如 `--once` 内容超过 `--height` |

## 键位

| 键 | Body 焦点 | Sidebar 焦点 |
| --- | --- | --- |
| `Up` / `Down` | 正文上下移动一行 | 侧栏选择上下移动一项 |
| `PageUp` / `PageDown` | 正文上下移动一屏 | 侧栏选择上下移动一屏 |
| `Left` / `Right` | 最上方可见表格向左/右一列；Link 焦点同样适用 | no-op |
| `Ctrl-Left` / `Ctrl-Right` | 向左/右八列，首尾夹紧 | no-op |
| `Home` | 正文顶部，退出跟随 | 侧栏选择首项 |
| `End` | 持续跟随当前输入流的实际末尾 | 侧栏选择末项 |
| `Ctrl+End` | 只跳到当前末尾，退出持续跟随 | 侧栏选择末项 |
| `q` / `Ctrl-C` | 退出并恢复终端 | 退出并恢复终端 |
| `Tab` / `Shift-Tab` | 切换焦点 | 切换焦点 |
| `s` | 开/关侧栏 | 开/关侧栏 |
| `Enter` | Body 为 no-op；Link 焦点激活链接 | 激活选中标题并移动正文 |
| `Alt-Left` / `M-Left` | 历史后退；空历史 no-op | 历史后退；空历史 no-op |
| `Alt-Right` / `M-Right` | 历史前进；无前进分支 no-op | 历史前进；无前进分支 no-op |

焦点循环稳定且跳过不可用目标：

- 侧栏隐藏时为 `Body ↔ Link`；当前屏无链接时 Link 不进入循环。
- 侧栏可见时为 `Body → Sidebar → Link → Body`，`Shift-Tab` 反向。
- Link 焦点只遍历当前可见正文链接；表格中的图片对象可聚焦，普通表格链接仍沿用仅样式策略。
- 窄屏隐藏侧栏后，Sidebar 焦点归一到 Body。

正文 End 开启跟随，底部一行显示 `FOLLOW`；其余行显示实际末尾。正文上下滚动、Home、标题/链接激活、历史前后退和实际焦点切换退出跟随；无效键和重复 End 保持状态。侧栏开关引起的重排、resize、批量输入、晚语义确认和 EOF 校准保留跟随意愿。Ctrl+End 固定按键时已收到的源码切口，随后到来的输入不会把这次跳转移到新末尾；End 可重新进入跟随。

跟随仅消费当前 Document 收到的 append/finish，不增加文件监听，也不允许 finish 后 append。自动更新不写导航 history。末尾定位先用 HeightIndex 估计，再用最多 8 个 tile 的短窗口恢复和实际行选择，直到凑齐一屏或到达开头；不会为了绝对行号完整排版旧前缀。远距离局部恢复仍标记 Local/Approximate，并保留实际行的 source 映射。隐藏定义通过非空白/非定义块的 O(log B) 候选索引跳过；HTML、空标签等候选仍经既有渲染语义确认。

候选查询消费一个逻辑 query/object；窗口恢复、源码/对象/链接活动、DisplayCursor、LayoutPass 和共享 memo 退休沿用同一 Budget。高度抑制、逐 tile 回填、逐行倒序选择和最终发布各自保存游标，每步收费；1 query / 1 object / 6 bytes 帧可恢复。自动 tail 的恢复检查点只保留在当前短窗口，不向全局检查点历史追加。`ViewportState::end_work()` 报告实际读取、窗口和本地检查点峰值；`Document::content_block_index_stats()` 报告输入侧索引增删次数、候选 entry 数及 key 字节，明确不包含标准库 BTree 节点的私有分配/空余容量，不能作为总堆容量。

表格当前为单行裁剪，每物理行计高一次。min/max 应用于真实显示宽度，源码字节 hint 仅作暂定估计。当前绝对列按 TableId 保存；多个表同时可见时，只横移最上方实际绘制的表格，普通链接历史的 Alt-Left/Right 保持。状态行 `table N cols A-B / C [<L >R] ~` 给出列范围、左右省略数及暂定宽度标记；完整显示且使用默认宽度策略时不额外显示状态。

resize 和同表流式追加保留起始列及仍在窗口内的部分精测，新内容不自动横移；离开当前视口计划后丢弃表宽及横移状态，返回从首列开始。原列的 Alignment、图片目标/OSC8 与 cell 源码保持；整行高度和导航仍归属原物理行。只读取索引选出的 cell span 前缀和有界字素上下文，不读取整条 row envelope 或省略列。省略行按已解析索引保留逻辑行数，不生成整表布局。普通追加可重绑未改变的部分精测，晚引用定义、脚注和较早语义变化则保守重测。End 和 Ctrl+End 使用同一列窗口、宽度策略和源码行归属；横移或改宽会重建末端布局，保留跟随意愿或一次跳转的固定源码切口。表格跟随状态显示 `FOLLOW | table …`，EOF 精测也按预算继续。

## 终端尺寸与布局

TUI 最小尺寸为 **20 列 × 3 行**。更小时不创建正常 `FrameRuntime`，显示
`terminal too small` / `minimum 20x3` 提示，可按 `q` 或 `Ctrl-C` 退出；退出后光标、
备用屏和 raw mode 正常恢复。

侧栏宽度：

- 终端小于 40 列时侧栏保持隐藏。
- 40 列及以上使用 `min(40, max(20, 30%))`，且至少给正文保留 20 列。
- 关闭侧栏：`content_x = 0`，`content_width = terminal_cols`。
- 打开侧栏：`content_x = sidebar_width`，`content_width = terminal_cols - sidebar_width`。分隔线计入侧栏宽度。

正文只在内容矩形内排版和裁剪；`s` 改变内容区宽时会保存顶部源码锚点，失效宽度相关的
高度、检查点、scanner/plan、diff 基线和布局恢复 token，再按新宽度从顶部附近局部重排。
仅终端高度变化时保留宽度相关缓存，只调整屏幕和视口。

侧栏中的 **current chapter** 与 **selection** 是两个独立状态：

- current chapter 随正文顶部源码自动更新，使用当前章节高亮。
- selection 只能由 Sidebar 焦点的导航改变，窗口围绕 selection 保持稳定。
- 正文滚动不覆盖 selection；current 不在当前侧栏窗口时不会把 selection 冒充 current。
- 两者重合时使用组合样式，仍可与单独 current/selection 区分。

## 链接生命周期

- 可见正文的链接以 cell `LinkId` 和 OSC 8 表示；Tab 进入 Link 焦点后，同名链接在跨行可见区间一起高亮。
- 普通表格链接只保留视觉样式；表格内图片保留 LinkId、命中与 OSC 8。
- 只有已解析图片显示 alt 或空 alt 占位 `[image]`；未解析引用/失败候选保持字面语法且不可点击，晚定义到达后再更新。
- 图片 alt 平铺内部图片/链接的文本，内部对象不生成额外控件；图片位于有效外层链接中时点击该链接。未解析/失效父对象不覆盖真实子图片目标。完整标签在窗口内读完且显示为空时，只显示外层一个 `[image]`。
- 选择、显示、排版、目录、表宽和激活复核共享有预算的 resolver。晚定义后激活旧焦点会先复核活动状态；失效对象不能仅凭原始目标启动外链。旧 memo 和辅助索引按帧退役，不为每次 append 保留一个旧副本。
- 内部 `#fragment` 链接在流式读取期间不构建完整锚点索引。激活时显示“待文档完成”，正文不移动；仍可按 `HeadingId`/源码通过侧栏跳转。
- EOF 后空闲帧按预算增量构建一次 `HeadingAnchors`；重复激活不会启动多个索引。完成后 fragment 先在有界缓冲中按 CommonMark 规则解码反斜杠标点和实体，再交给核心 slug 匹配；`#` 指向文档首。
- 成功内部跳转才写入历史；失败、pending、预算耗尽和外链都不改变历史。新跳转截断前进分支。
- `--open-links` 使用 `Command::new(program).arg(url)` 风格显式 argv；空目标、控制字符、内部 fragment 和未知 scheme 被拒绝，不经过 shell。打开外链不写内部历史。

## 流式与有界状态

输入接受普通文件和 Unix FIFO。reader 线程按 64 KiB 块读取，使用容量 1 的同步 channel 形成背压；主线程按 revision
顺序应用每个非空 `Update`。首屏最多预读 9×32 KiB=288 KiB，但首个有可绘制内容的批次会立即
提交首帧，不等待 EOF，也不在首帧前 finish。之后按累计 32 个 update 或 16 ms 截止窗口合并
提交，最后一个 update 不会丢失。EOF 后 drain 尾部、finish，并走最终校准路径。q 会断开 channel 并取消读取；已结束线程会 join，仍阻塞在任意 `Read` 的线程不会拖延终端恢复和退出，待该次读取返回后收尾。

滚动型状态有意限制在可见窗口：

- `Screen` front/back 缓冲只随终端 rows×cols 增长。
- Sidebar 只保留当前可见标题窗口和窗口文本；不缓存全部 10k 标题正文。
- 检查点只保留视口前后双向窗口；陈旧 payload 必须在完整 compaction 后退役/释放。
- 表宽缓存只保留当前 plan 的表；列元数据按预算准备当前绝对列起点后的至多 `终端宽度/3+2` 列，不读取前方列。每列最多缓存 32 个出序行观测，淘汰仍保留测量最大值和顺序扫描前沿；离屏、append、resize 清理量不随滚动历史增长。全表宽未知时明确暂定，不复制表格正文。
- EOF 的 `HeadingAnchors` 允许一次性 O(S+H) slug/trie 缓存，单列 indexed headings/source/work，不计入滚动帧上界。

64 MiB 连续滚动、10k 标题、10k 表格行和超长 Setext 的容量口径见
[P67 证据](../../docs/tui-render-todo.md)。64 MiB 用例默认 ignored，是 debug 手动验收，
不是 CI 时间断言，也不代表 release 墙钟。

## 测试与验收

```bash
nix develop -c cargo check --workspace --all-targets
nix develop -c cargo nextest run --workspace --no-fail-fast
nix develop -c cargo nextest run --workspace --release --no-fail-fast
nix develop -c cargo test --workspace --doc
nix develop -c cargo doc --workspace --no-deps
nix develop -c cargo fmt --all -- --check
nix develop -c cargo clippy --workspace --lib --bins -- -D warnings
nix develop -c cargo clippy -p mdtui-tui --all-targets -- -D warnings
# Cell、raw ANSI、实际读量/容量/预算、Unicode 窗口边界
nix develop -c cargo nextest run -p mdtui-tui --test static_render --test once_ansi --test workspace_integration --test nested_objects
# 手动 64 MiB 验收；不以墙钟作相等断言
nix develop -c cargo nextest run -p mdtui-tui --run-ignored only -E 'test(/manual_.*(64|sixty_four)/)'
```

PTY 测试固定 TERM/COLORTERM/LANG，覆盖真实 CLI、按键、120×40 / 80×24、resize、退出及异常恢复。快照更新需审查当前契约与核心语义差异，不能修改官方输入或为旧视觉回退核心行为。

## 两套 oracle

- TUI：固定终端和正文内容区后的 cell/Screen 等价；正常帧只用绝对定位 CSI、SGR、OSC 8 和
  成对同步输出，不含 `ESC[2J`、裸 `ESC[H` 或 `K`。首帧在备用屏内只清屏+归位一次。
- `--once`：逐语义段原始字节等价；保留 CRLF、表格内部 LF、样式 reset、OSC 8 顺序和参考超宽
  溢出。空/纯空白零字节，不带 clear 前缀。

TUI 不允许用 `--once` 截图替代 committed `Screen`，`--once` 也不允许用 TUI cell 快照替代
raw golden。TUI 的 PTY 验收覆盖真实二进制启动、滚动、侧栏、目录/锚点跳转、历史、外链焦点、
`--no-links`、resize 和退出恢复。

## 已知限制与差异

完整的静态逐节证据见
[`docs/tui-render-compliance.md`](../../docs/tui-render-compliance.md)；参考基线、HTML/嵌套强调/MATH
裁定和双 oracle 约束见
[`docs/tui-render-baseline.md`](../../docs/tui-render-baseline.md)。

| 限制/差异 | 当前行为 |
| --- | --- |
| 流式 fragment 链接 | 未 finish 时不构建完整锚点；显示“待文档完成”，EOF 后分帧完成索引再允许跳转。 |
| 嵌套强调外层前景 | 核心 `Style` 是位并集，采用 P14 固定优先级；无法恢复任意嵌套顺序的外层前景。 |
| MATH | 固定使用终端默认前景并保留外层属性；不猜测参考未定义的嵌套数学视觉。 |
| 超宽不折行块 | TUI 把标题、代码、表格裁剪到正文内容区宽；`--once` 保留完整溢出字节。侧栏开关会改变 TUI 宽度。 |
| HTML 块 | 使用有界启发式安全跳过，不复制参考实现的崩溃；不是完整 CommonMark HTML block 状态机。 |
| 表格内链接 | 普通链接沿用参考的仅样式策略；图片按当前 TODO 保留 LinkId、OSC8 与应用内焦点。 |
| truecolor / OSC 8 | TUI 假定终端支持 24-bit 色和 OSC 8，不做能力探测或静默降级；需要时用 `--no-links` 关闭 OSC 8。 |
| 非 Linux opener | `--open-links` 当前只在 Linux 使用 `xdg-open`；其他平台明确返回 Unsupported，不偷偷走 shell。 |
| 大规模验收 | 64 MiB 滚动、超大 fixture 默认 ignored，使用 debug 手动记录；CI 只跑较小的确定性快测，墙钟不作相等断言。 |
| 一次性锚点索引 | EOF 后 `HeadingAnchors` 保留 O(S+H) 的归一化 slug/base 与共享 trie；核心未公开内部 arena capacity，P67 单列 indexed headings/source/work。 |
| 表格宽度缓存 | 首次快照只分配当前列窗口；顺序测量保存前沿，乱序观察每列至多 32 行。容量报告含 Vec/HashMap 容量与 B-tree 每键保守额度，不冒充进程总堆峰值。 |
| 空输入 | TUI 是固定空白网格；`--once` 输出零字节。两种模式的生命周期不同。 |
| ratatui §8 | 参考实现的后端丢失不代表本 ANSI TUI 可以降级；本实现不继承无 OSC 8、标题下划线丢失或表头底边错误。 |

当前 TODO 高于旧样式：图片可见并可点击，空 alt 为 `[image]`；表格遵守 Alignment。普通标题/列表仍沿用参考的拼合与普通链接文字省略规则。目录面包屑/筛选、折叠、搜索、选区、勾选编辑均未实现。

交互布局从已预算扫描的窗口准备块，不调用全文静态渲染。每端有 4 KiB 显示上下文及有界边缘探测；源码或显示字素上下文超限时，一段 `…` 占一个局部行，映射当前窗口切口，行尾 tile 归属，HeightIndex 标为 Local。重新定位/resize/追加会重新验证；此估计不宣称巨型字素的全局精确高度。巨大表格 cell 在精测期间使用暂定宽度，随后逐帧收敛；每个待测列仅保留一个已付费 run、4 KiB 当前字素窗口和数字摘要，旧 prefix 不随每帧重扫。超过当前字素窗口的字素继续采用 Local `…`，完整绘制仍属于独立 TODO。

OSC8 单目标最多 4096 字节，每帧目标源码检查及 OSC8 目标负载各限 64 KiB。超限时标签、样式和应用 LinkId 保留，目标不截断发送。`www` 使用 `http://`；邮件 OSC8 保留旧参考的裸地址，外部打开使用 `mailto:`。外部打开先检查原始长度，再用核心预算游标解码实体/转义。
