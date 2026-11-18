# 终端阅读器 workspace 集成

2026-11-12：将已有终端实验接入当前 `crates/*` workspace。生产核心及核心测试保持原有实现，`mdstream-core` 仍为零运行时依赖、`forbid(unsafe_code)`；TUI 自身也禁止 unsafe。

| 来源 | 固定记录 |
| --- | --- |
| 复用实现 | `experiment/tui-render`，commit `f8d58d8fe6a80ba4a04ab5c4f52aeb16cc9120eb`，主要为 `crates/mdtui-tui/**` |
| 参考样式 | `/home/ajen/gits/mdtui/docs/render-style-spec.reference.md`，169 行 |
| 样式 SHA-256 | `9a6c14b8cffbc474cd0fb715323c32eece875855f0555c6f134d33702bdfc7bb`；2026-11-12 只读复核一致 |
| 集成验证核心基线 | `1d5907a1d2729e929570d700c8a59ae0f262a674`（含长围栏、统一容器列模型、嵌套候选及完整代码失败重放） |
| 历史资料 | `tui-render-{todo,baseline,compliance}.md`；旧实验勾选不是当前 TODO 的验收依据 |

TUI 内部 `mdtui3` 名称是 `mdstream-core` 的依赖别名。版本统一在根 manifest，TUI 继承 workspace 的版本/edition；删除独立 lock，统一使用根 lock。原有 flake、格式化设置与共享 hooks 沿用。

## 数据路径与有界工作

输入接受普通文件和 Unix FIFO；reader 使用容量 1 的 channel 背压，q 取消并断开接收端，已结束线程 join，阻塞中的任意 Read 不延迟退出、返回后自行收尾；首批 32 KiB，有条件最多预读 9 批，首帧不调用 finish。EOF 是单独事件。交互路径为 `StreamEngine → FrameRuntime → ViewportState / FirstFrame → DisplayScanner → TileLayoutSink → Screen/diff`。显式 `--once` 才全文读取、finish、静态布局。

`DisplayScanner` 在共享 Budget 内读取显示片段；scanner 的逻辑窗口与邻近字素上下文分开计量。每端先取 4 KiB 显示上下文，原始边界探针按小块恢复并限制额外负载/前置上下文。末端探针以已经确认的起始边界为相对起点，避免为 RI 奇偶重复读取历史前缀。`edge_source_bytes`、`edge_queries`、`display_source_work` 记录实际检查窗口预算；`scanned_bytes` 只描述逻辑窗口覆盖，不能用它冒充全部读取量。

显示片段统一分字素后才软换行，保留完整字素源码包络；不在 tile/预算切口独立计算宽度。实体产生的 ZWJ/combining、被隐藏强调标记隔开的字素也按显示层合并；表格单元格之间保持分隔。标题显示限制 4 KiB 前缀并保留最后字素原子性，侧栏另外限制源码/显示字节和一行输出。

边缘字素或必要上下文超出限制时，使用 `…` 表示该块在当前逻辑窗口中的源码交集：一个局部行、一个 cell，由该窗口行尾 tile 计高，标为 `HeightState::Local` 并在同步高度时保留 Local。它保留 UTF-8 源码锚点；滚动、追加或 resize 后重建/验证，不把半个字素或错误 RI 旗帜当作真实文字，也不宣称该估计是整个巨型字素的精确全局高度。完整巨型字素排版仍在 TODO。

深容器由 `WindowContexts` 分帧读取和回填祖先摘要，节点读取/回填分别收费，共享祖先只访问一次。交互块准备消费 scanner 的窗口结果；heading/list/code helpers 不再接收 Document 正文读取接口，表格使用准备好的 token 窗口。`LayoutSink` 回调本身不持有 Document，不隐藏源码读取。`prepared_display_bytes` 和实际 scratch cell 容量单列，不能当作进程总堆峰值。

长围栏行在 LF/EOF 前以原始暂定尾显示，不将 `pending()` 的有界前缀误当完整未定内容。确认后通过索引跳过长隐藏前缀，把标签归到首个正文 tile；标签只按共享 Budget 读取最多 4 KiB，仍保留原块源码映射。百万字 marker/info 的 LF 确认、空正文及失败候选回表头见 `workspace_integration`。

高度依旧按片段行尾/块事件归属；同 tile 的源码目标通过 SourceRefiner 区分。运行时保存预算耗尽的 refiner，后续帧继续；普通滚动取消待定跳转，并将实际首行源码交给 `Navigator::reconcile_source`。标题/fragment 跳转保留原始目标，调用 `Navigator::reconcile`；不再用旧块起点覆盖普通滚动的实际映射。

有限窗口的标题细化复用 heading `content` 索引选择准入 tile，避免恢复前缀占满 `max_tiles` 后只纳入隐藏 marker、漏掉下一 tile 的标题文字；请求的 source 与标题身份保持。恢复行通过已有 `row_limit_anchor` 与目标行限额分开，视口扩展块范围前保证结束边界包含目标 tile。零/微预算仍暂停后续算，实际无渲染行的 Definition 保留 `NotFound`。未测前缀的全局行号仍可近似；全量参考布局的准备成本与局部细化分开计量，不为细化回扫任意文档前缀。

## 已验收组件与边界

| 组件 | 当前行为与主要证据 |
| --- | --- |
| 屏幕、diff、终端恢复 | `frame_runtime`、`flicker`、`flicker_acceptance`、`cli`、`pty_e2e`：真实 cell/vt100、raw ANSI、普通帧不清屏、resize/退出/异常恢复 |
| 流式首屏和滚动 | `stream`、`first_frame`、`scale`、`scale_scroll`：暂停 reader、未 EOF 首帧、按键、源码锚点与预算计数；输入解析工作与窗口工作分列 |
| 字素与源映射 | `workspace_integration`：UTF-8 全切点、小预算、跨 tile/window、ZWJ/RI/combining、实体/隐藏标记的实际滚入滚出与检查点恢复，以及超限 Local 占位 |
| 标题和目录基础 | `refine`、`sidebar*`、`history`、`workspace_integration`：同 tile 标题分别到达、普通滚动真实源码、运行时小预算跳转续跑、可见标题窗口与超长 Setext 摘要 |
| 流式退出 | `pty_q_exits_and_restores_while_fifo_writer_stays_open_without_eof`：写端保持打开，首屏后 q 正常退出并恢复终端，EOF 不参与退出 |
| 表格布局与失效 | `table_widths`、`layout_sink`、`viewport_updates`：Alignment、边框/padding、单行裁剪、行高归属、宽度变化丢弃旧布局/检查点、旧 ticket 拒绝及窗口重排 |
| 表格容量 | 按预算准备当前列窗口的终端宽度/3+2 列；全宽未知时暂定。每列最多 32 项出序观测，淘汰保留最大宽度与扫描前沿。`table_window_budget` 覆盖 80 列视口/10 万列表、零预算读取/分配、2 万行离散观测及 append/离屏/resize 清理计数；`workspace_integration` 覆盖 128 列/3 万缺列行及 1000 次新表追加 |
| 链接与图片 | 当前屏幕 row-major LinkId；迟确认邮箱与较晚对象的源序；所有块中的已解析图片 alt/`[image]`、OSC8/`--no-links`，未解析引用/失败后缀保持字面且不可点击，晚定义失效与有效空目标分别回归；独立 `once_images.ansi` |
| 链接预算 | 单目标先 O(1) 检查 4096 字节上限；同帧同 LinkId 只检查一次；目标读取和 OSC8 目标负载各限 64 KiB。百万字目标保留标签与对象而不读取/复制目标、不截断 OSC8。外部打开先限长，再用核心预算游标解码；`www` 补 `http://`，外部邮件补 `mailto:` |
| 任务框 | 已有 `[ ]`/`[x]`/`[X]` 状态绘制、宽 cell、完成样式和续行；只读，不编辑追加源码 |

表格采用单行单元格裁剪，每物理行总高度只回填一次。`--table-min-width` / `--table-max-width` 校验内容宽度范围，hint 仅作暂定值，实际 Unicode、实体、隐藏标记及图片 alt 由显示语义测量；大于一帧额度的 cell 由 `WidthMeasure` 恢复，保留一个付费 run、4 KiB 当前字素及数字摘要，不复制整份 cell。普通追加重绑未改变的部分精测，较早语义/晚定义变化会丢弃并重建，取消只释放有限本地状态，唯一共享 memo 逐节点付费退役。

Left/Right 按绝对列一列移动，Ctrl-Left/Right 八列；目标为最上方实际可见表格，Sidebar 焦点 no-op，Alt 历史保持。resize/流式追加保留列起点，离开视口计划后重置。状态行报告当前列范围和左右省略数。`TableWindow` 按已解析 cell span 直读当前窗口，省略行的逻辑高度按索引和原源码归属保留；布局回调消费准备好的 token，不回读 row envelope 或远端 guard。混合段落/表格 tile 的深行偏移按已准备前缀几何校正，最多补一个局部窗口。已整合提交 `29eb9443` 的 End/Ctrl+End：末端准备传入同一 min/max 和绝对列起点，稀疏行按逻辑偏移回填，长 cell/分隔行包络按物理行索引跳过；横移/改宽只使旧 prepared tail 失效，保留跟随意愿或固定源码切口。End 的 EOF 校准同样恢复巨大 cell 精测。

`table_policy` 的独立全表模型和手写显示期望覆盖宽度边界、窄屏、Unicode/实体/图片、cell 间字素隔离、百万字 cell/alt、6-byte 微预算、append/晚定义/取消/票据、十万列末端直读及实际 App 键位；既有 `table_window_budget` 继续验证两万行、32 项观测上限及清理。真实私有终端另外验证两个 CLI 策略、左右/大步/边界、resize、FIFO 追加和 EOF、图片 OSC8、q 与恢复。完整超长字素绘制仍沿用 Local 占位，未扩展独立 TODO。

目录面包屑、级别/名称筛选、目录树展开、折叠/搜索/选区/勾选编辑没有据此勾选。End 持续跟随后续已交付：正文 End 开启跟随，Ctrl+End 只跳当前末尾，上滚或显式导航退出；见 [交付记录](../DONE.md#2026-11-13-end-持续跟随与一次末尾跳转)。当前基础列表/引用视觉沿用旧样式，不代表完整容器方言或所有 CommonMark 渲染；数学、脚注排版和原始 HTML 不在本次完成范围。

## 当前契约与旧 oracle 差异

当前 TODO 优先：图片保留 alt/占位及点击对象，表格按 Alignment 对齐。旧样式隐藏图片、忽略对齐，均没有沿用。核心已经把空列表叶块分类为 Blank、容器分隔线分类为 ThematicBreak，未解析 shortcut 保留字面方括号；TUI 按当前语义消费，未改核心来迎合旧测试。

只更新了 `spec_headings`、`spec_images`、`spec_lists`、`spec_quotes`、`spec_tables` 五个 cell 快照：前三个体现图片可见，quotes 体现未定义 `[!NOTE]`/`[!WARNING]` 字面方括号且没有 LinkId/链接样式，tables 体现 Alignment。输入 fixture 保持原样。原有十一份 raw ANSI golden 保持，新增图片的独立字节 oracle。快照中 source 路径改为 workspace 路径属于元数据差异。

其他旧视觉裁定（例如标题/列表强调拼合、普通链接文字省略、终端裁剪与 once 溢出、邮件 OSC8 裸地址）仍明确保留，见基线和符合性矩阵；这些视觉规则不表示核心显示文本语义退回旧实现。

## 嵌套对象选择与 memo 生命周期

交互准备先用可恢复 `LinkWindow` 枚举当前窗口对象，再保留 pending `LinkActivity`，通过同一 `LinkResolver`、同一 Budget 得到 Active/Unresolved/Suppressed。原始 destination/title 不再作为激活证明。`SharedLinks` 只共享一个 memo；scanner 的 `next_with_resolver`、LayoutPass、目录 excerpt、当前 slug session、表宽测量和激活复核均复用它，核心任务中的自有 memo 保持为空。`LayoutSink` 回调不解析父链或远端 guard。

父链按预算构建 presentation 摘要并 memo。外层图片 alt 平铺内部的图片/链接文本；内部对象不多生成控件，内部空图片不多生成 `[image]`。整体图片处于有效外层链接时使用该链接目标；父 Unresolved/Suppressed 不覆盖真实子图片目标。完整 alt 已在当前窗口内读完且显示为空时，只生成外层一个占位；未读完整的长标签不为判断“空”扫描屏外正文。未解析对象只恢复自己开闭边界的原始语法，不重解析 label，也不生成假图片控件；未解析子对象的字面边界仍属于外层 alt，继承外层点击归属，已解析子对象的隐藏标记保持隐藏。列表保留图片及未解析语法的原始间距，避免把占位变成 `[ image ]`。

每次成功视口提交请求回收工作 memo。未完成 scanner、目录读取、锚点构建和激活任务的轻量 lease 保留同 revision 进度；这些任务结束后，在后续 Budget 内逐节点回收。revision 改变时优先退役唯一旧 resolver 及 presentation/literal 索引，Covered 后 reset 到最新 revision；连续 append 不排队旧 memo。取消任务只丢空核心 memo 和 Copy scratch，退役工作由共享状态继续。退出进程时的整体析构不计入帧硬预算。

表宽的待测 cell 使用有上限、可恢复 scanner，并共享 resolver，不再以 `cell_width` 的无预算便利读取求值远端 guard。缓存绑定 revision，append/finish 后按明确 cache miss 重建，保守 dirty/符号失效维护高度和检查点；没有跨 revision 保存带遗漏依赖的 `DependencyKey`，也没有同步 atomic capture。缓存诊断包含 pending 自有显示缓冲与公开 cursor scratch 容量；核心私有查询 scratch、allocator 开销和共享 memo 容量不冒充总堆统计，分别报告实际查询/节点/退役数。

交互计划始终 defer：先按预算准备可见对象，再分类 Drawable/TUI-owned/ZeroRow。显式静态 `plan_visible` 才保留无预算便利查询；`unbudgeted_drawable_queries` 可核对 runtime 为零。长围栏构造器最多查三段、且严格限定在已确认 Code span 内的无条件 literal metadata；空代码后接 4,096 个条件候选时构造/零预算 guard work 均为零，不先查询后面的段落再截范围。

`nested_objects` 的独立 cell/目标期望覆盖普通及复合空 alt、未解析父对象、晚定义抑制父链接、标题/列表/引用/表格、stale focus 激活复核、表宽跨窗口 guard 和 cache miss。512 层跨 tile 图片用 1 query/1 object/6 bytes 推进；1,024 个远端引用的中途取消核对零预算保留、100 次连续 append 和逐节点付费退役。统计通过 `LayoutSinkStats.objects/object_index` 记录真实 resolver 和窗口工作，不能只用最终截图说明成本。

代码失败释放另有两项实际消费者回归：finish 前保留暂定原文，释放后 entity 映射原 `&amp;`、strong、嵌套图片的外层目标及旧 ticket 失效均正确；百万字候选一次重放后，局部图片窗口仍只扫描 82 个逻辑源码字节（显示上下文工作 4,258 字节、边缘探测 768 字节分列），回调内 Document 查询为零。解析 finish 的 1,000,017 个重放字节属于输入收尾，不能冒充窗口工作，也没有为布局再次扫描整个前缀。

## 验证命令

运行命令与支持矩阵见 [crate README](../crates/mdtui-tui/README.md)。验收包含全 workspace debug/release nextest、doc、fmt、生产 clippy、TUI 全目标 clippy、64 MiB 手动计数用例及隔离 tmux 的真实 CLI。最终数量与限制记录在 [DONE](../DONE.md#2026-11-13-终端阅读器-workspace-集成)。


## 025 当前 slug 与 preEOF fragment

`FrameRuntime` 持有一个 `HeadingSlugSession`，输入 Update 用 Arc 与实际消费者共享，不复制事件数组。锚点维护不再等待 EOF，也不随 revision/resize 重建。背景维护、激活和当前 lookup 使用同一个帧 Budget 与唯一 SharedLinks resolver；目录 current/selection 继续按真实 HeadingId/source 导航，显示名称查询的显式 stale 行为保持。

Tab/Enter 与发布 front cell 上的左键命中走同一激活状态。固定大小 `LinkDestinationTask` 复用核心解码器，按预算保存实体探测；至多 MAX_DESTINATION_BYTES 的原始目标对应一次预分配平面输出，移动进 Arc<String> 不再复制。当前 fragment 的 percent/UTF-8 游标和 endpoint lease 保留到结束。晚到 binding、失效 active LinkId 或 refiner 期间 owner 变化时，重新验证当前目标后才能提交。poll 成功和 Enter 成功共享完成序号，history 只写一次，前进/后退不重新解释 slug。

已有当前目标时，未 EOF 不再单独构成 PendingDocument；原 history 的 `# B` / `[go](#b)` 样例现在明确验收 preEOF 成功。真正尚未出现的标题保留等待状态，已有 history 不提前变化，metadata 到达后只提交一次成功事件。fragment 成功后的 resize 按当前视口源码位置重新付费细化，不沿用重置后的估计 tile 行，也不重新解释旧 slug。

`fragments_pty` 保持 FIFO 写端开启，真实按键和 SGR 左键均在没有 EOF 的情况下从等待状态自动跳到晚到标题；`fragments` 另验收 refiner 暂停后晚定义改变全局 owner、源码 history、resize 和侧栏 current。终端退出先恢复 tty/mouse，再在有限 Budget 批次中退役新 slug graph、request 和共享 memo；总析构仍有线性工作，不承诺外部 allocator/OS 操作的硬延迟。完整正文折叠的 Reveal UI 继续保留接缝。

普通标题增长只投影新范围与真实 delimiter 变更；条件样式同一个 Recovery 保留上次事件与 forest，append-only 时只处理新事件/端点并更新实际覆盖 pair 的计数。当前 marker 的 revision/source 索引是当前状态，不存 revision 快照。原严格 resolver rebind API 保持，内部采用明确的 memo 保留/旧 continuation 失效接口；条件标题 lease 由 slug owner 保有，实际失效及退出付费回收。成功使用内部 fragment 后，live 窗口若按原格式只剩空白，会显示链接标签作为可读目的标题；静态 --once/原混合格式化与未启用该交互的 Reading 省略规则保持。
