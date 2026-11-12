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
| `Left` / `Right` | 最上方可见表格向左/右一列；Link 焦点同样适用 | 收起自身或选父；展开自身或选首个可见子标题 |
| `Ctrl-Left` / `Ctrl-Right` | 向左/右八列，首尾夹紧 | no-op |
| `Home` | 正文顶部，退出跟随 | 侧栏选择首项 |
| `End` | 持续跟随当前输入流的实际末尾 | 侧栏选择末项 |
| `Ctrl+End` | 只跳到当前末尾，退出持续跟随 | 侧栏选择末项 |
| `q` / `Ctrl-C` | 退出并恢复终端 | 退出并恢复终端 |
| `Tab` / `Shift-Tab` | 切换焦点 | 切换焦点 |
| `s` | 开/关侧栏 | 开/关侧栏 |
| `0` / `1`…`6` | 保留正文键位 | 级别上限；0 为全部，1…6 为级别不超过此值 |
| `/` | 编辑正文显示搜索查询；Enter 提交 | 编辑标题名称查询；Enter 提交 |
| `Esc` | 输入态取消草稿；否则取消正文搜索/复制并清除选区高亮 | 编辑中取消；编辑外清除名称查询，返回树模式 |
| `c` | 保留正文键位 | 树模式选中正文当前章节；随后 Left 可继续选父 |
| `Enter` | Body 为 no-op；Link 焦点激活链接 | 激活选中标题并移动正文 |
| `Alt-Left` / `M-Left` | 历史后退；空历史 no-op | 历史后退；空历史 no-op |
| `Alt-Right` / `M-Right` | 历史前进；无前进分支 no-op | 历史前进；无前进分支 no-op |
| `v` | 进入/离开源码选区光标模式；重新进入从当前屏首个可选字素开始 | 不改变侧栏 selection |
| 选区模式 `Left` / `Right` / `Up` / `Down` | 按实际 cell 的完整字素移动；到边缘继续滚动 | — |
| 选区模式 `Space` / `Shift+方向` | 固定起点并伸缩范围；再次 Space 取消固定起点，只选光标字素 | — |
| 选区模式 `y` / `Y` | 复制逻辑显示文本 / 精确源码范围到终端剪贴板 | — |
| `n` / `N` | 下一个 / 上一个正文显示匹配；首尾不循环 | 不操作正文查询 |


侧栏名称输入限 **256 UTF-8 字节**，Backspace/Delete 删除末尾一个 Unicode 标量；输入期间 `q`、`s`、数字和 `/` 都作为字符，`Esc` 放弃编辑，`Enter` 提交，`Ctrl-C` 仍退出；Tab/Shift-Tab 焦点和 Alt-Left/Right 历史仍可用，草稿保留。编辑时不搜索；相同已提交查询不重复扫描，过期查询可用 `/ Enter` 显式刷新。

名称匹配复用核心 `HeadingSearch` 的非隐藏显示文本（包括实体、图片 alt 与链接标签），按字面、区分大小写，不归一化 Unicode、不跨标题。每个匹配标题只列一次；名称模式为独立平面匹配列表，不受目录收起状态限制。只保留有限当前页：上下键移动，Down/PageDown 继续发现后页，Up/PageUp 精确前页按预算重扫，Home 回首页，End 按预算寻找末页。`scan hN @byte` 显示进度，Esc 随时取消；正文搜索状态独立，按当前焦点分别接收输入，切换焦点保留各自草稿。显式目录 Enter 暂停尚未完成的正文搜索导航，保留正文查询、选区和复制；回正文可用 n/N 或 `/ Enter` 恢复，旧搜索不会抢回视口。正文开始编辑也取消尚未执行的目录激活，不写入无移动历史。追加、finish 或晚定义使名称结果标为 `stale`，保留 HeadingId 选择，等待显式刷新，避免每 append 重搜历史。resize 从原匹配页附近重建有界窗口，不从文首扫描。隐藏侧栏会释放未完成的 guard/查询；未完成查询显示 stale，已完成页和选择保留。

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

## 选区、复制与搜索

`v` 进入显式光标模式，底栏显示 CURSOR / EXTEND、UTF-8 源码半开范围和两个复制动作。普通方向键只在该模式内接管；Tab/Shift-Tab、Alt-左右历史、s 侧栏、Enter 激活和退出键保留。Home/End、Ctrl-End 与焦点切换离开光标模式；End 仍开启持续跟随。显式选区移动和命中导航退出跟随，输入文字、重画高亮及 resize 不追加历史。状态行占一行，但出现或范围变化时复用当前已提交正文，不重新估算顶部。

光标只落在有来源的真实 base cell，跳过宽字符 continuation、装饰边框、缩进和状态文字。CJK、combining、ZWJ、RI 按完整字素移动；实体、转义、代码规范化及图片 alt 使用实际 cell-source 映射，一个替换的多个字素可能共享同一个完整源码区间。范围包括两端字素，原始隐藏 Markdown 与换行只要位于两端之间就属于范围；不是把列号当字节偏移。代码正文现在也保留逐字素来源，代码语言标签仍是装饰。沿用现有图片来源契约：非空 alt 对应标签原文，空 alt 的 `[image]` 占位锚在图片起始 `!` 的单字节 Span，单独选中该占位时 Y 因而复制这个明确显示的源码范围。选区跨屏时固定源码起点；追加、晚定义、EOF、resize 只重投影，不改选区。端点变得隐藏或离屏时保留原范围，显示 HIDDEN；方向键不会改选另一段来源，重新按 v 进入可重置到当前屏。

- `Y` 精确复制选中 UTF-8 源码范围，包括范围内隐藏标记、实体拼写、CRLF 和控制字节，不补齐范围外的行首标记或末尾换行。
- `y` 复制来源属于选区的逻辑显示文本：解码实体/转义、规范化行内代码、保留实际图片 alt/空 alt 的 `[image]`，不加入终端软折行产生的 LF，保留列表呈现生成的逻辑词间空格。不同正文块、代码物理行、表格行之间使用 LF，表格所选可见列之间使用 TAB；不复制边框、padding、代码缩进/语言标签、状态栏、ANSI/OSC。代码及表格按当前正文宽度和共享表宽快照裁剪，未画出的部分不作为屏幕截图补出；巨型字素沿用有来源 `…` 占位。当前正文渲染对软换行使用空格、对硬换行使用零宽连接，显示复制遵守这个既有契约；Y 保留原始换行。原 21 输入 fixture / 11 ANSI golden 不变。
- 两种动作都显式发送 OSC52，不启动服务。源码范围与显示负载均限 64 KiB，超限拒绝而不截断；任务显示已准备字节数，可 Esc 取消。内容/宽度变化取消显示复制，精确源码复制可继续读取已存在的不可变范围。读取、布局投影、编码和发送负载都计入帧共享 Budget；编码与发送计费可分帧，最后一次发送完整且有界的 OSC，帧 ANSI 不进入未闭合字符串。界面使用 `clipboard request sent`，只表示完整请求已写出，不确认终端剪贴板权限或实际写入；写出失败明确报错。预算分帧记录准备、编码以及预先获得的发送额度；最后实际写出至多 **87,392 字节**的完整 OSC52 包，实际 wire 字节计入该发送帧的 `FrameMetrics.clipboard_bytes`、`StreamMetrics.output_bytes` 和 reading 累计计数。1 query / 1 object / 6 bytes 验收证明工作和额度获取可恢复，不表示实际每帧只写六字节，也不承诺任意终端 writer 的 I/O 硬时延。

搜索采用核心 DisplaySearch 的逻辑显示流（拼接非 HIDDEN run），不把额外终端装饰当作核心文字：已解析图片的 alt 和嵌套 alt 可查，空 alt 的终端 `[image]` 占位本身不可查；标题/列表按既有呈现规则省略的普通链接标签仍可能产生逻辑命中。命中实际 cell 才着色，当前终端不画出的匹配标 `match not drawn`，在有界、可暂停的目标解析结束后停留于相应正文行或报告无可绘制行，不冒充屏幕高亮。区分大小写、字面匹配，无正则、无 Unicode 规范化；可跨 run、隐藏标记、实体和嵌套标签，可重叠。查询最多 256 UTF-8 字节，退格删除一个 Unicode 标量；q 是输入文字，Ctrl-C 始终退出。只在 Enter 提交时建任务，编辑新查询取消尚未完成的旧任务，Esc 可保留之前已显示的命中。只保存并高亮当前匹配；n 沿游标向前，N 按预算重扫到前一个匹配，时间与所搜索前缀有关，但不积累全篇命中列表。命中的显示位置与源码范围分别保存，共享同一帧 LinkResolver；取消不同步释放历史 memo，后续逐节点退役。追加、晚语义确认或 EOF 改变 revision 后清除旧命中并标 stale，须 `/` 后 Enter 显式重提，不每次追加自动全文重搜，也不偷偷 finish 输入。

此处交付的是实际终端控件与当前未折叠视图。FoldNavigation、FoldAnchor 的已有核心接口仍未接入终端折叠占位；手工范围折叠动作、折叠中的选区投影与必要路径展开仍在 TODO。表格横移已按固定基线 `4299276` 组合验收：复制复用当前 `ViewportState` 的共享 `TableWidthCache::snapshot_budgeted`，向 FirstFrame 传递 min/max、first_column 和实际选列/行窗口；真实 PTY 解码及手写字节期望覆盖横移、裁剪、跨行选区与 resize。搜索目标通过索引定位实际行/列并显式显示该列，保留宽度策略；处于单元格裁剪部分的匹配仍定位到该行正文并标 `match not drawn`，不会用边框冒充命中。

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

目录头部两行分别显示正文 current 的最近三个标题路径和筛选/查询状态。深路径以 `…/` 开头，`c` 再 Left 可逐级向上导航；不免费读取所有祖先标题。头部只占侧栏，不改变正文高度、源码切口或跟随布局，单元格没有源码/link 命中。树层级来自确切 scope 的父章节与容器入口继承章节，跨容器可以超过六层；缩进在窄栏内夹紧。Left 优先收起有后代的自身，再选父；Right 优先展开自身，再选章节范围内第一个通过过滤的子标题。收起父节点保留子节点意愿。级别过滤是上限，不是严格同级/子树过滤；隐藏 selection 取最近前驱可见项（收起祖先优先），无前驱再取后继，current 始终来自实际正文源码。

目录展开意愿独立于正文 `FoldView`。正文折叠控件、占位绘制与跳转的 UI Reveal 尚未交付，继续由原 TODO 的“单点及批量操作”“可定位的摘要”和“准确目标与渐进定位”跟踪；激活入口留有接入点。

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
- Sidebar 只保留当前窗口/有限匹配页、三个路径标题和有界名称查询；收起子树按章节端点二分跳过，深祖先检查逐项计入共享 Budget。只保存显式收起的 HeadingId 意愿，不缓存所有标题名称或每节点路径。查询/摘要/正文共用唯一 SharedLinks resolver，取消与 revision 变化按帧退役旧 memo；数值查询 scratch 随有界任务释放。
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

当前 TODO 高于旧样式：图片可见并可点击，空 alt 为 `[image]`；表格遵守 Alignment。普通标题/列表仍沿用参考的拼合与普通链接文字省略规则。目录面包屑/筛选和目录树展开已实现；选区、两种复制与正文显示搜索已按上述契约接入；正文折叠与勾选编辑仍按各自 TODO 推进。

交互布局从已预算扫描的窗口准备块，不调用全文静态渲染。每端有 4 KiB 显示上下文及有界边缘探测；源码或显示字素上下文超限时，一段 `…` 占一个局部行，映射当前窗口切口，行尾 tile 归属，HeightIndex 标为 Local。重新定位/resize/追加会重新验证；此估计不宣称巨型字素的全局精确高度。巨大表格 cell 在精测期间使用暂定宽度，随后逐帧收敛；每个待测列仅保留一个已付费 run、4 KiB 当前字素窗口和数字摘要，旧 prefix 不随每帧重扫。超过当前字素窗口的字素继续采用 Local `…`，完整绘制仍属于独立 TODO。

OSC8 单目标最多 4096 字节，每帧目标源码检查及 OSC8 目标负载各限 64 KiB。超限时标签、样式和应用 LinkId 保留，目标不截断发送。`www` 使用 `http://`；邮件 OSC8 保留旧参考的裸地址，外部打开使用 `mailto:`。外部打开先检查原始长度，再用核心预算游标解码实体/转义。
