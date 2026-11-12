# 已完成任务

本文件保存从 [TODO](TODO.md) 拆出的已完成任务、验收条件与交付证据。后续工作及仍适用的共同约束见 TODO。

## 已完成的核心能力

- [x] 表格：表头、行/单元格源码区间、默认/左/中/右对齐；缺列隐式为空，多余列忽略。
- [x] 超长表格行流式解析、独立单元格行内状态、转义竖线、按源码定位行、按行列直接访问。
- [x] 列宽提示增量聚合，宽度变化用区间通知使旧表格布局失效，不扫描旧行。
- [x] 任务列表：`[ ]`、`[x]`、`[X]` 的结构化状态与 marker 区间。
- [x] 图片与普通链接区分；保留 alt、完整语法区间和目标，支持引用式图片及延迟定义；空 alt 图片也能按视口查询。
- [x] 标题索引：稳定 HeadingId、ATX/Setext、六级索引、前后标题查找、零拷贝标题区间。
- [x] 导航核心：逐行、整页、首尾、指定标题、按级别前后标题；返回高度锚点及精确标题源码目标。
- [x] 大表、长单元格、巨型 Setext、分块一致性和导航边界的回归测试。
- [x] 按区间顺序读取语义：查询 O(V + log n)，去掉每字符树查询和块二分；UTF-8 片段边界直接定位。
- [x] 链接/图片的尖括号目标、三种 title、引用 title 延迟解析；定义和直接链接共用单遍状态机。
- [x] 行内软/硬换行语义，跨分块 CRLF，块末延迟判定及旧片段失效。
- [x] 超过 lookahead 的真实行内分块测试、中间前缀一致性、缓存键契约、索引朴素模型和查询访问量测试。

### P0：把有界重排做成可执行的接口契约

- [x] **整帧共享预算**：视口查询、隐藏区域跳过、检查点恢复、表格单元格测量和再次查询共用一个预算。计入重复访问、查询次数和对象处理，禁止通过反复调用各自有上限的接口绕过总量限制。预算耗尽时返回部分结果及继续位置，下帧按需推进。
- [x] **明确视口完成状态**：区分“已覆盖目标窗口”“到达文末”“预算耗尽”，返回可继续的源码/tile 位置。高度修正后从稳定锚点继续定位；跨追加或 resize 使用继续位置前验证其版本。
- [x] **限制恢复距离**：提供带距离限制的检查点查询或排版计划。检查点过远时允许从局部源码锚点建立临时布局，或分帧恢复，并明确结果尚未精确；禁止为了精确行号回扫整个长段落。未知区域的绝对行号保持估计。
- [x] **安全跳过无内容区域**：将可见文本与可绘制对象共同纳入查询。空 alt 图片等对象即使没有可见文本，也必须能够进入视口；`next_visible()` 只能证明源码文本隐藏，不能单独证明该区域无需绘制。

验收：固定窗口和预算，扩大屏外文档后，实际处理字节、片段次数、恢复步数和对象数量仍不超过预算。覆盖连续测得零高度后反复补充视口、百万字隐藏区间、无邻近检查点的巨型段落、空 alt 图片、大表格单元格、resize 和持续追加。用最小排版驱动验证完整循环；仅验证一次 `viewport()` 的返回长度不算通过。索引的对数访问增长单独计数。

还需验证推进性：输入和排版参数稳定后，能够容纳一个基本步骤的正预算应让继续位置前进，不能每帧重新扫描同一前缀。后续工作按用户请求范围推进，不因一次跳转在后台无条件排完整篇；无有效上下文时允许局部近似，不承诺任意位置的一屏精确布局。

### P1：提高缓存复用率，完善增量消费协议

#### 变化分类与高度估计

- [x] **表达变化原因**：区分文本/可见性、样式、块结构、引用目标/title、表格列宽提示等变化。解析器报告事实，渲染策略判断是否影响尺寸；未知策略保持保守失效。例如，文字尺寸不变时的新链接目标无需丢弃高度，固定/封顶列宽未变时的新宽度提示无需触发表格重排。
- [x] **分离高度数值与有效性**：区分有效测量、局部临时测量、未测量估计和已过期估计。失效可保留旧数值作估计，避免全部退回统一高度造成位置突变；正确性不依赖旧估计准确。可能重新可见的旧零高度区域必须恢复可检索性，不能因保留 0 而永久跳过。失效和恢复可检索性仍需通过区间操作或索引完成。
- [x] **按符号验证依赖**：为引用符号维护版本，直接依赖缓存按需检查自身使用的符号。若定义会影响尺寸，还要处理后续布局状态：先记录每个符号最早的使用位置，定义到来时从该位置保守失效后续布局状态，使用 O(log T) 区间操作，不遍历历史使用点。无已知使用点时无需为该定义失效旧布局。检查依赖的数量也计入视口预算。
- [x] **细化测量 ticket**：在全局排版参数版本之外，记录依赖范围和输入检查点的版本。尾部无关追加不应拒绝旧视口的有效测量；前序换行状态变化必须拒绝相关结果，不能只检查当前 tile。需要后台排版时提供有大小上限的视口快照，复制成本计入预算。

验收：单 tile 高度从 3 修正到 5，只增加一次 O(log T) 树更新；后续累计位置增加 2，屏外内容不被访问。不影响尺寸的引用/列宽提示变化保留已测高度；真正改变换行条件时，相关 ticket 必须拒绝。保留旧估计后，原零高度位置出现可绘制内容仍可被定位；源码锚点经高度修正后可恢复。

局部原型的具体起点：

- 高度树在 `sum` 之外聚合零高度数量 `z`。对于需要恢复可检索性的失效范围，以正估计 `e` 替代旧 0、保留其它旧高度时，子树新和为 `sum + e * z`，可用 lazy tag 下传；另行记录有效性代际。需验证它与区间赋值、抑制、增长和 reset 的组合，不能只验证这个求和公式。
- ticket 使用布局参数版本及依赖区间的最新失效代际，并验证起始检查点的版本；范围写入和查询沿用区间树的对数路径。检查点被替换或其输入状态改变时，后续依赖也必须失效。不能仅以当前 tile 的源码版本判定整个测量有效。
- 符号的最早使用位置在每次新增/补全引用时增量维护。引用后来转成直接链接时，允许保留偏早的位置，代价只是保守失效。定义不需要找出所有使用位置。需要反例覆盖“当前 tile 没有该符号，但前面的引用图片改变了传入排版状态”。

#### 更新协议与对象状态

- [x] **完整对象增量**：在新对象范围之外，提供已有链接、块、标题及表格行的变更/完成信息，支持目录和辅助索引按增量更新。区分暂定、正在增长、局部语法已封口；封口不代表晚定义引用永远不变。`pending()` 的行前缀不能作为所有未定语义的完整描述。
- [x] **连续更新批处理**：增加 `base_revision -> revision` 的连续更新契约，支持按帧合并输入产生的更新。合并只处理新事件及区间，不反复复制已累计事件，不展开历史 tile 列表。现有逐次 `apply` 路径继续可用。
- [x] **身份与重复应用语义**：在 Update、缓存 key、测量 ticket 等边界对象中校验文档/视图归属，避免同版本号的不同实例混用。明确空更新、重复 `finish`、重复应用、乱序及漏更新的行为；重复应用不能悄悄清掉新测量。多索引统一消费时先校验，再修改状态。

验收：从相同初始索引出发，固定排版参数且处理中间不插入测量，比较批处理与逐次消费的最终语义和缓存有效性；若合并采用更保守的区间，必须明确允许的额外失效，不能漏失效。不要求内部 epoch 数值、树形或事件列表相同。合并事件工作量随事件数线性增长，索引应用仍按区间计费。跨文档/视图、乱序和漏更新有明确结果；对象在流式补全中的变化不会漏报。

#### 分配与内存常数

- [x] **复用热路径缓冲区**：增加 Update 缓冲区复用、`viewport_into` 或视口迭代器；查询 scratch 是否复用由分配统计决定。保留普通借用接口，避免为零拷贝语义查询引入整段 owned 文本。
- [x] **测量实际内存成本**：统计 arena 的容量字节数及分配次数，再评估 NonZero 索引压缩、语义/修订信息拆分、相邻同类节点摘要。比较峰值内存、写入次数和查询次数，不能只比较 node count，也不能用无界全树压缩隐藏成本。
- [x] **可预算的缓存回收**：如需要限制长期浏览留下的检查点状态，增加显式分批移出旧状态的能力。逻辑失效继续不运行大量用户析构函数；用户状态析构的执行时机和耗时不能算成常数时间保证。

验收：预热后的逐字符普通文本追加、固定视口反复查询减少重复分配；总扫描量和树访问上界不退化。密集标记和反复 resize 下记录实际容量、峰值内存及节点复用情况。

## 2026-10-30 核心基础交付

本轮范围：核心库与可运行示例；终端 UI 后续单独接入。维持零运行时依赖与 `forbid(unsafe_code)`。

| 已交付 | 接口与证据 |
| --- | --- |
| P0 预算、推进性、完成状态 | `Budget` / `WorkLimit`、`viewport_budgeted` / `resume_viewport`、`LayoutPass` / `LayoutSink`；`crates/mdstream-core/tests/frame.rs` 与 `crates/mdstream-core/examples/budget.rs` 覆盖整条消费链 |
| 限距恢复、对象安全跳过 | `previous_within` / `recovery`、`LayoutPass::from_checkpoint`、`next_drawable`；恢复也消耗同一 Budget，远处无上下文时标记局部近似 |
| 增量消费协议 | Update 的文档身份、`base_revision`、`merge`、`append_into` / `finish_into`、`apply_layout_update_with_policy`；`crates/mdstream-core/tests/updates.rs` |
| 变化原因与对象状态 | `ChangeKind`、`SyntaxState`、块/标题/链接变更范围、表格行变更范围。对象范围允许包含最后一个未变化的旧对象；引用解析仍使用符号通知 |
| 高度及缓存依赖 | `HeightState`、`expire` 保留非零估计并恢复旧零值可检索性、`LayoutPolicy`、`DependencyKey`、`ticket_from` / `measure_from`、`insert_for_layout`；`crates/mdstream-core/tests/estimates.rs` / `crates/mdstream-core/tests/cache_search.rs` |
| 有界快照与回收 | `Document::snapshot` 只复制有上限的源码，非整份 AST；`drain_before` 按预算移出状态并将析构时机交给调用者 |
| 内存实测与局部压缩 | 三类树 arena 容量和实际增长分配次数；语义树子节点用 NonZeroUsize 压缩，见 README 实测记录。未将 allocator 元数据、String 或用户状态的外部分配冒充树 arena 数据 |
| UTF-8 字节输入 | `Utf8Document`，最多保留 3 字节不完整尾部；非法序列与 EOF 截断替换为 U+FFFD，偏移基于解码结果；`crates/mdstream-core/tests/utf8.rs` |
| 章节关系与结构导航 | 当前章节、父/子、前后兄弟、严格同级、章节范围；`crates/mdstream-core/tests/sections.rs` |
| 源码交互基础 | `SourceSearch` 可分帧增量字面匹配，`Selection` 保留源码区间，`TaskState` 为独立视图状态；`crates/mdstream-core/tests/cache_search.rs`。该轮尚未包含显示文本搜索，见后续交付 |
| 尖括号自动链接 | URI/email 有界候选；失败/未完成候选不回扫；`crates/mdstream-core/tests/autolinks.rs`。不包含 GFM 裸 URL 扩展 |
| 固定版本样例 | CommonMark 0.31.2 的 19 个 autolink 样例：18 个一致、1 个明确差异；`crates/mdstream-core/tests/compatibility.rs`，不宣称整体合规率 |

该轮结束时仍未完成显示语义映射、基本及完整容器、折叠区间遮蔽与联动、slug、数学/脚注，以及强调/代码回退和语法上限放宽。显示语义映射的后续交付见下一节；其余复杂扩展继续按 TODO 验证模型和算法。终端排版、表格绘制、按键与目录 UI 不属于该轮交付范围。

## 2026-10-30 显示语义片段交付

- [x] **显示文本与源码映射**：`display_runs` / `display_runs_in` 返回 `DisplayRun`；普通文本借用原文，替代文本同时保留本次消费区间和完整原始区间。实体跨 tile/查询边界只输出一次，空文本续片仍可映射源码；原始 `runs` 和 `Selection::text` 保留源码语义。
- [x] **字符引用**：支持固定的 2125 个 WHATWG 分号结尾命名引用，以及十进制/十六进制数字引用。候选有界，失败或 EOF 未完成时保留原文，不回扫历史。零值、代理码点和越界码点替换为 U+FFFD；不将替代文本重新当成 Markdown 解析。目标/title、引用标签匹配、自动链接地址、代码及 fence info 保留现有方言。
- [x] **代码 span 空白规范化**：闭合后按范围读取时规范化换行及首尾空格；全空格正文、内部连续空格和其它空白有明确规则。逐字节维护空白摘要，闭合只读端点，不复制或回扫长正文。未闭合代码保留现有无回退行为。
- [x] **有预算的增量消费**：`DisplayCursor` 绑定文档身份与 revision，耗尽返回进度，继续读取使用同一 Budget。每步计入 query、object 和有界源码检查窗口（含 CRLF 相邻字节）；源码锚点前进，追加后旧游标被拒绝。现有 `LayoutPass` 的回调仍接收 `RenderRun`，终端排版后续接入显示接口。
- [x] **晚确认失效**：实体闭合、代码 span 确认会更新范围 revision 和 Text 变化；即使关闭样式影响尺寸的策略，也拒绝旧布局测量。

验证：`crates/mdstream-core/tests/display.rs` 覆盖 UTF-8、变长及双标量实体、普通文字借用、失败/超长候选、转义及字面上下文、跨 tile/逐标量查询、真实流式逐切点/逐前缀一致性、6 字节预算推进、文档/版本校验、百万字代码晚闭合和局部查询计数。已有分块及确定种子 fuzz 快照加入显示映射。另固定 CommonMark 0.31.2 的 24 个显示文本样例，不对 HTML、属性解码、列表结构或完整代码回退作合规承诺。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 100 项、8 项文档测试通过；`cargo run -p mdstream-core -r --example display` 演示分帧读取与解码输出。保留零运行时依赖和 `forbid(unsafe_code)`；实体表带来源及输入 SHA-256，生成脚本读取本地数据，构建与测试不联网。

## 2026-10-30 显示文本搜索交付

- [x] **明确搜索语义**：`DisplaySearch` 按当前 revision 的非隐藏显示片段进行区分大小写的字面搜索，支持重叠及跨 run/tile 匹配。实体解码、代码空白规范化直接复用显示接口；不添加排版分隔符、不做 Unicode 归一化。空查询返回错误。
- [x] **命中源码映射**：`DisplayMatch` 同时返回显示字节范围和最小源码包络。隐藏标记跨越、实体部分命中、双标量实体、CRLF 及 UTF-8 均保留映射；相同源码区间对应的不同显示命中不会去重。选区复制继续保留原始源码。
- [x] **有预算的恢复与复杂度**：读取、匹配比较、KMP 失败回退及命中输出共用 Budget。6 bytes、1 query、1 object 的小步能持续前进；未处理片段保留在固定大小缓冲中，源码映射使用查询长度的环形缓冲。查询预处理和任务存储 O(m)，单 revision 匹配比较总量 O(d)，含显示查询的累计时间 O(n log(n + 1) + m)，不拼接全文显示文本或在回退时复制长前缀。
- [x] **流式变更边界**：任务绑定文档身份与 revision，追加及首次 EOF 后拒绝旧任务，调用方须丢弃旧命中并按需重建；库不自动重搜。空追加和重复 EOF 保持任务有效，完成后重复调用不产生重复结果。跨 revision 维护显示命中和折叠/高亮 UI 接入仍属后续工作。

验证：新增 `crates/mdstream-core/tests/display_search.rs` 的 10 项测试，使用独立语义预期及完整显示文本上的朴素搜索对照，覆盖源码映射、长查询、重叠匹配、真实流式逐前缀/逐切点、固定种子生成输入、共享预算耗尽/恢复、缓存片段及完成状态下的身份和 revision 拒绝、对抗失败回退计数、百万字隐藏区间的有限读取。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 110 项、9 项文档测试和格式检查通过。`cargo run -p mdstream-core -r --example search` 在 13 个有预算步骤内找到格式标记/实体文本和代码中的两处 `ab&c`，输出各自显示及源码范围。保留零运行时依赖和 `forbid(unsafe_code)`。

## 2026-10-30 基础折叠交付

本轮完成 TODO 折叠规划的第一阶段；继续交付核心库与可运行示例，保持零运行时依赖及 `forbid(unsafe_code)`。

- [x] **范围与身份**：`FoldId` 绑定标题/代码块的稳定 ID；`fold_candidate` / `fold_candidate_at` 查询整段、隐藏正文、父节点、摘要区间和开放状态。`FoldCandidates` 按源码范围发现候选，非候选块也计入共享 Budget，支持暂停/恢复及文档/revision 校验；摘要限量读取，不扫描长正文。
- [x] **独立视图与单节点操作**：`FoldView` 自有收起意愿、高度和检查点；`set_collapsed` / `toggle` 返回源码范围及视图版本。标题保留自身，代码块整体收起，空章节不产生占位；父节点收放保留子状态，多个视图互不影响。
- [x] **高度遮蔽原型及接入**：高度树保存基础高度/有效性与独立遮蔽计数，聚合未遮蔽高度、数量及零值；遮蔽范围成对增减，不逐 tile 清零/恢复。`base_height` 可读取基础值，现有行定位/视口使用可见高度。随机数组模型覆盖重叠遮蔽、赋值、测量、过期、增长和 resize；十亿片段稀疏模型验证对数区间访问与重复操作的节点复用。O(1) reset 保留遮蔽。
- [x] **占位与有预算排版**：`LayoutPass::folded` / `advance_folded` 复用原排版循环，`FoldLayoutSink` 显式接收可定位占位。占位高度只归属隐藏起点 tile，同 tile 多标题/占位及后继正文按精确源码边界处理；父折叠隐藏子占位和图片。百万字正文直接跳过，语法零高度代码块仍能恢复折叠入口；1 query、1 object、4 bytes 的小步预算可持续前进。
- [x] **流式范围与失效**：每次 Update 只检查最多六个开放章节和一个代码块，固定数组保存待更新 ID，原位更新记录，不重新分配全部状态。新增 tile 继承遮蔽；代码未分类前缀暂归开放块，闭合/新块确认可收回，Setext 晚确认可缩短旧章节。单节点切换惰性过期后续高度/检查点，旧测量、继续任务、错误文档和不连续更新受校验；重复应用保留新测量。
- [x] **有限恢复**：`folded_from_checkpoint` 只使用本视图有效且距离有界的起点，恢复步骤共用 Budget；远处直接开始保持局部近似。接入时还修复了 `insert_for_layout` 首次写入超出已有检查点树容量的位置会形成反向失效区间的问题。

验证：`crates/mdstream-core/tests/folding.rs` 的 11 项回归使用独立源码区间并集模型和明确的文本预期，覆盖父子切换、两个视图、空章节/空代码、同 tile 边界、UTF-8、逐字符前缀、批处理、晚 Setext、流式代码、隐藏图片、摘要/查询预算、异步旧结果及有限恢复；高度树另有 2 项数组/稀疏模型测试。项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 123 项、10 项文档测试、格式检查及 `git diff --check` 通过。

`cargo run -p mdstream-core -r --example folding` 在 120 万字节正文上演示父节点收起、展开后保留子折叠，分别用 2 / 3 个有预算步骤输出 4 / 6 行。该示例统计源文本换行与占位，不是终端列宽或完整显示语义排版。

下一阶段继续导航联动、锚点恢复、默认策略与可预算批量操作；容器、表格/预览和手工范围仍按各自前提推进。本轮不将这些总体规划项一并勾选为完成。

## 2026-10-30 折叠导航与锚点恢复交付

本轮推进折叠规划第二阶段的核心接口；默认策略、批量命令和目录 UI 继续保留在 TODO。

- [x] **有预算的隐藏目标导航**：`FoldNavigation` 从 Navigator 快照创建任务，接受已有标题/结构/滚动动作及新增 `NavAction::Source`。`Reveal` 仅打开遮住目标的章节或代码块，保留目标标题的正文折叠及无关子树状态；支持目录 ID、搜索或已解析链接的源码目标。目标保持精确源码，显示行仍使用当前高度估计，交给局部排版细化。
- [x] **可见标题策略**：`VisibleOnly` 对前后标题和严格同级遍历按最外层隐藏范围的端点跳转，避免逐个检查隐藏标题；显式目标及结构动作若隐藏则返回无目标。100 个不相交隐藏区间可在极小预算下逐帧跳过，不重扫已完成部分。
- [x] **暂停、变化与失效**：每步消费共享 Budget 的 1 query、1 object，无正文读取。返回完成状态及本次展开的源码包络/版本；取消不撤销已经展开的节点。文档/revision、视图身份/版本均校验，外部折叠、append/finish 和 resize 拒绝旧任务，自身展开及正常高度测量允许继续。完成结果通过 O(1) 的 `Navigator::apply_target` 应用，普通滚动清除旧的精确源码目标。
- [x] **源码锚点恢复**：`FoldAnchor` 保存文档身份与源码位置，按预算投影到最外层折叠占位；父子依次展开后恢复原源码。跨 append、Setext 晚确认和 resize 重新查询，不复制折叠树、正文或历史布局。允许 EOF，追加保持原偏移，非法 UTF-8 边界与错误文档被拒绝；屏幕行/列仍由接入方局部校准。

验证：新增 `crates/mdstream-core/tests/fold_navigation.rs` 的 12 项回归，覆盖确定种子的朴素可见标题模型、同 tile 多标题、跳级/重复标题和级别筛选、必要路径展开、结构动作边界、预算耗尽及区间合并、测量期间继续定位、错误/过期任务、滚动后起点更新、流式范围与锚点恢复。5 万个隐藏标题的前后跳转各用 4 个预算步骤；六级章节和代码块中的百万字目标用 10 步打开 7 个遮蔽节点，源码字节预算为 0，解析计数不变。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 135 项、11 项文档测试、格式检查及 `git diff --check` 通过。`cargo run -p mdstream-core -r --example folding` 保留原有折叠排版演示，并用 4 个步骤恢复到 120 万字节章节内保存的源码位置。保持零运行时依赖和 `forbid(unsafe_code)`。

下一步实现可按预算推进的按级别、递归、全部及聚焦操作，并定义默认策略和显式状态的组合。实际终端排版、目录/高亮/复制 UI、slug、历史/书签和更多折叠对象仍按 TODO 单独推进。

## 2026-10-30 批量折叠与默认设置交付

本轮完成折叠规划第二阶段的按级别、递归、全部及聚焦核心命令，补齐新节点默认状态。保持零运行时依赖及 `forbid(unsafe_code)`。

- [x] **有预算的批量命令**：`FoldBatch` 支持 `All`、包含根节点的 `Subtree`、严格标题级别 `AtLevel` 及 `Focus`。聚焦展开目标及祖先，显式收起其余全部候选；与仅打开必要遮蔽路径、保留无关状态的导航 Reveal 分开。跨级标题、代码块和空章节均使用已有稳定 ID/范围。
- [x] **分帧进度和中间状态**：构造 O(1)，范围定位及逐块检查各消费 1 query、1 object；非候选、未匹配级别和已处于目标状态的块同样计费。返回累计访问/修改数和本次变化区间包络，最小预算可前进，完成可重复查询。批量操作逐步生效，取消保留已完成修改，不复制候选表或折叠树快照；任务存储 O(1)，工作按范围内实际块数和固定次数的索引操作计量。
- [x] **身份、失效及排版组合**：任务绑定文档/revision、视图身份/版本；外部折叠、默认设置变化、append/finish 和 reset 拒绝旧任务。错误返回前不消费预算、不修改视图。自身修改复用单节点高度遮蔽及检查点失效；旧测量被拒绝，正常测量不中断任务，中间状态可以进入有预算排版。
- [x] **新节点默认设置**：`FoldDefaults` 按六级标题和代码类型设置初始折叠，默认全部展开。O(1) `set_defaults` 只影响后续首次识别的候选，已有意愿与显式操作保留；批量命令不修改默认值。更新仅扫描新增标题/块元数据，Setext 晚确认通过新 HeadingId 初始化，新增隐藏区域在 apply 返回前完成遮蔽。重复 Update、逐字符代码续行和 resize 不覆盖显式展开；`default_visits` 记录初始化检查量。

验证：新增 `crates/mdstream-core/tests/fold_batch.rs` 的 10 项回归及 `crates/mdstream-core/tests/folding.rs` 的 1 项排版组合回归。确定种子的父链/可见区间模型对照 100 轮混合命令；覆盖预算原子扣费、非候选计费、取消、错误/过期任务、跨级和同 tile 边界、变化区间合并、独立视图、流式默认设置、逐字符/合并 Update、Setext、finish、resize 和中间状态绘制。百万字子树访问 2 个块、源码字节预算为 0；2 万标题在极小预算下可恢复完成；已有 1 万标题和百万字 Setext 的新确认只增加 1 次默认检查，不访问旧正文。持续追加隐藏正文的排版回调只读取少量可见源码。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 146 项、12 项文档测试、格式检查及 `git diff --check` 通过。新增 `cargo run -p mdstream-core -r --example fold_batch`：每帧 8 query / 8 object，201 块的全部展开用 26 帧完成，3 块子树操作用 1 帧完成；追加后的新标题按默认收起，已展开的旧标题保持展开。

默认值是新节点的初始状态，不是对历史节点持续强制的规则；合并 Update 在 apply 时使用当前默认值。大批新增对象的初始化成本按输入事件计费，不承诺任意大 apply 的硬时延。兄弟操作、临时预览、历史/书签、容器深度策略和目录等 UI 继续按 TODO 推进。

## 2026-10-30 兄弟节点批量折叠交付

- [x] **同父兄弟操作**：`FoldBatchAction::Siblings { target, collapsed }` 展开/收起相同父节点的其他候选，保留目标、祖先及各节点后代的意愿。跨级标题和代码块可互为兄弟；无父节点的候选互为根层兄弟，覆盖首标题前代码、空章节和无兄弟节点。隐藏目标仍可操作，不自动打开其祖先，不修改新节点默认值。
- [x] **预算与范围**：复用 `FoldBatch` 的渐进修改、进度、变化区间及失效校验。构造 O(1)，首次推进按预算定位父章节正文范围；根层操作检查整个块索引。范围内每个块均计入 1 query、1 object，含非候选和不匹配父节点的后代块，不读取正文、不复制候选或状态快照；任务存储 O(1)。Setext 晚确认后旧任务在扣费和修改前被拒绝，已完成修改保留，调用方可创建新任务处理新兄弟。

验证：新增 5 项回归，扩展固定种子的 100 轮混合操作模型和非法目标检查。覆盖跨级/代码混合、隐藏目标、同 tile 边界、根层/空章节/无兄弟、后代状态保留、重复无操作、极小预算、范围外排除、正常测量与 Setext 晚确认。父范围用例只访问 2004 个块，排除外部 2000 个普通块；百万字后代用例只访问 3 个块，源码字节预算为 0；2 万根标题分帧展开其他 19999 个，保留目标收起状态。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 151 项、12 项文档测试、格式检查及 `git diff --check` 通过。更新 `cargo run -p mdstream-core -r --example fold_batch`：兄弟操作以每帧 8 query / 8 object，26 帧访问 203 个块并收起 100 个兄弟。README 和 TODO 已同步；临时预览、历史/书签、容器深度及 UI 工作继续单独推进。

## 2026-10-30 临时折叠预览交付

- [x] **单节点临时展开及恢复**：`FoldView::set_preview` 为每个视图保留一个章节或代码块预览；传 `None` 结束，切换目标恢复旧节点。祖先/后代意愿不变，隐藏祖先继续生效。`is_collapsed` 返回本地意愿，`preview` 返回当前预览 ID，实际可见性通过现有查询和排版读取。预览期间显式收放或批量命令更新结束后的意愿，避免恢复时覆盖用户的新选择；未显式修改则恢复原状态。
- [x] **预算和存储边界**：开始、切换及结束各原子消费 1 query、1 object，支付最多两个节点的固定次数索引操作，O(log H + log F + log T)；仅额外保存一个 ID 和布尔意愿，O(1) 空间，不复制折叠树、不读取正文或遍历隐藏 tile。预算不足保留全部旧状态，后续帧可重试；相同目标无操作。错误文档、未同步视图及非法候选在扣费/修改前拒绝。
- [x] **流式更新和排版组合**：预览跨 append、finish、默认设置变化及 resize 保留，恢复使用当前候选范围，支持开放代码和 Setext 晚确认。预览变化返回旧/新范围包络及版本，沿用高度、检查点、测量与继续任务失效；新导航按实际遮蔽路径工作，绘制使用剩余共享预算或后续帧。

验证：新增 `crates/mdstream-core/tests/fold_preview.rs` 的 6 项回归及 `crates/mdstream-core/tests/folding.rs` 的 1 项窗口排版回归；扩展 160 轮固定种子区间模型、逐字符前缀模型和空代码零高度用例。覆盖同 tile 边界、嵌套/切换、空章节、独立视图、显式/批量意愿、导航、预算原子性、错误和旧任务、默认值、重复 Update、Setext、开放代码、finish 和 resize。百万字节点连续预览/追加/恢复 20 轮，单次状态切换不消费源码字节且高度树修改访问少于 1500；新增可见内容只按 64 字节预算消费，恢复后排版仍跳过百万字正文。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 158 项、13 项文档测试、格式检查及 `git diff --check` 通过。`cargo run -p mdstream-core -r --example folding` 已加入父章节临时预览和恢复：保留子占位的预览绘制用 3 帧，恢复后绘制用 2 帧。README 与 TODO 已同步；有界行数摘要预览、历史/书签、容器深度策略及 UI 接入仍按各自规划推进。

## 2026-10-30 跳转历史与书签交付

- [x] **文档内书签与偏移**：`Bookmark` 保存 `FoldAnchor`、捕获时所在章节/代码 ID 和相对视口行偏移；单独恢复复用 `FoldNavigation::Reveal`。源码始终为精确目标，追加、finish、Setext 晚确认及 resize 不改变书签；EOF 为左亲和。`viewport_top` 在局部行定位后恢复偏移，对缩小视口和文档开头作限制，最终文档末尾由 Navigator 限制。跨视图可复用，重载后的新文档身份被拒绝。
- [x] **线性历史与分支**：`NavigationHistory::replace_current` 刷新普通滚动后的真实位置；`push` 记录成功的新跳转并截断前进分支，完全相同的条目为无操作。前进/后退边界明确返回无任务，完成时才移动历史游标；取消和预算耗尽不提前提交，已展开的折叠保留。相同源码但不同视口偏移/对象元数据仍可分别保存。
- [x] **预算、身份与失效**：历史任务每步消费 1 query / 1 object，最终定位与 O(1) 游标提交共用一步；不读取正文、不枚举内部 tile、不复制历史或折叠树。完成可重复读取；另一任务完成、条目变更及文档/视图变化使旧任务失效，错误历史返回 `WrongHistory`。高度测量不阻塞任务，最后按最新高度定位。每个书签/任务 O(1) 空间；历史 O(J) 条目，Vec 保留峰值容量，追加摊还 O(1)，不承诺扩容硬时延。

验证：新增 `crates/mdstream-core/tests/history.rs` 的 8 项回归，包含固定种子 2000 步线性模型、同 tile 重复/跨级标题、普通滚动返回及视口缩小、历史分支、预算各维原子性、取消、竞争任务、错误文档/视图/历史、过期与未同步更新、空文档/EOF、UTF-8 非法边界、Setext 晚确认、finish 和跨视图恢复。百万字代码内目标在六级标题和代码折叠下用 10 个步骤、7 次路径展开恢复，字节/tile 消费为 0，高度树修改访问少于 15000，保留无关章节折叠；高度回填发生在中途时最终定位使用最新前缀高度。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 166 项、14 项文档测试、格式检查及 `git diff --check` 通过。新增 `cargo run -p mdstream-core -r --example history`：返回隐藏代码用 6 帧、前进用 3 帧，历史边界返回无任务。README 与 TODO 已同步；实际终端行定位、目录/书签 UI、跨重载持久化、有界行数摘要预览及容器工作继续单独推进。

## 2026-10-30 标题显示名称检索交付

- [x] **按显示名称检索标题**：`HeadingSearch` 在各标题 `content` 的非隐藏显示文本上作区分大小写的字面匹配，复用实体/代码 span 显示语义，支持跨 run/tile、重叠命中及长查询。`HeadingMatch` 返回稳定 `HeadingId`、标题内显示字节范围和绝对源码包络；重复名称独立保留，匹配不跨标题，不读取正文或未索引的容器内标题。级别上限与标题导航一致，0 为空，大于 6 钳制到 6。
- [x] **有预算候选及匹配**：最多六个级别索引通过二分查询跳过被过滤标题，每次候选查找消费 1 query / 1 object；读取、比较/失败回退及输出共用 `DisplaySearch` 的预算契约。6 bytes、1 query、1 object 可持续推进，调用方复用结果 Vec。查询仅预处理一次 O(m)，切换标题 O(1) 重置匹配位置而不清空映射环、复制标题或重新分配查询表；任务存储 O(m + TILE_BYTES)。百万字标题由显式预算调用扫描，Setext 确认和任务构造不生成完整名称。
- [x] **流式失效与折叠导航接入**：任务及已返回命中绑定文档身份和 revision，追加/首次 finish 后必须显式重建，错误文档或过期任务在预算/结果变更前拒绝。空追加和重复 finish 不失效，完成任务重复调用不输出。搜索包含折叠内标题并保留视图状态；命中 ID/源码可交给现有 `FoldNavigation::Reveal`，只展开必要路径。

验证：新增 `crates/mdstream-core/tests/heading_search.rs` 的 9 项回归，覆盖显示实体部分命中、隐藏标记/目标、重复/空标题、长查询与重叠、标题边界重置、UTF-8、小预算各维度、待输出命中、错误文档/过期/完成、长 ATX 逐前缀、Setext 晚确认、逐切点，以及固定种子 60 份文档共 480 次朴素模型对照。百万字正文与一万个被过滤标题不增加读取字节或已计费步骤；一万个全部不匹配级别的标题只用一次候选查询。百万字 Setext 的小预算首帧最多读取 30 字节，2001 字节失败查询的累计字节计费低于内容长度的四倍加常数。折叠内命中恢复精确源码目标，并保留目标正文和无关章节的收起状态。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 175 项、15 项文档测试、格式检查及 `git diff --check` 通过。新增 `cargo run -p mdstream-core -r --example heading_search`：10 帧完成两个标题的显示名称命中，随后展开必要路径并定位首个命中。README、TODO 已同步；目录 UI、slug/fragment、跨 revision 命中维护及其余核心待办继续单独推进。

## 2026-10-30 有界显示摘要交付

- [x] **有界显示前缀**：`DisplayExcerpt` 从指定源码范围返回非隐藏 `DisplayRun`，复用实体、代码空白规范化、样式/链接及源码映射。`ExcerptLimits` 同时限制源码前缀字节、输出 UTF-8 字节和 LF 逻辑行数；源码上限包含隐藏语法，超长首行仍受输出上限限制。UTF-8 不截断标量，部分实体输出保留完整替代源码映射；区分范围结束和三种上限，不向后扫描来断言余文可见，也不自动添加省略号。
- [x] **共享预算和复杂度**：构造不扫描正文，任务 O(1) 空间且不复制完整摘要；每次读取沿用 `DisplayCursor` 的 query/object/byte 计费，隐藏 run 和重复前瞻也计费，每个检查窗口至多 `TILE_BYTES + 2` 字节。每帧 6 bytes、1 query、1 object 可推进，截断扫描仅限已付费的有界 run 或至多 8 字节的替代文本。累计成本 O((S + 1) log(n + 1))，S 为消费的源码前缀；调用方管理输出存储。
- [x] **流式与折叠边界**：任务和输出绑定文档 revision，追加/首次 finish 后明确失效，调用方显式重建；错误文档、过期任务及非法范围在消费前拒绝。可用 `FoldCandidate.hidden` 读取代码/章节正文，用标题 content 读取名称；不改变折叠意愿、高度或测量状态，也不应用子折叠。开放范围是当前 revision 快照，逻辑行与终端软换行后的屏幕行分开定义。

验证：新增 `crates/mdstream-core/tests/excerpt.rs` 的 8 项回归，覆盖围栏/缩进代码、UTF-8、实体部分输出及映射、样式/链接、CRLF、隐藏语法、空范围、零/精确上限、各预算维度、错误文档/过期/完成状态、空追加/重复 finish、Setext 晚确认及流式范围。全部 UTF-8 单切点、跨 tile 和大小预算组合对照完整显示文本的独立前缀模型。千字与百万字首行、超长围栏 info 及失败链接候选在 64 字节源码上限下均计费少于 1500 字节；折叠预览读取保持视图版本、高度和高度树访问计数不变。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 183 项、16 项文档测试、格式检查及 `git diff --check` 通过。新增 `cargo run -p mdstream-core -r --example excerpt`：百万字折叠代码的两行摘要用 3 帧、计费 71 字节。README、TODO 已同步；摘要实际显示行上限、占位绘制/高度归属、容器及其余核心待办继续单独推进。

## 2026-10-30 整表折叠交付

- [x] **整表范围及有界摘要**：`FoldId::Table(TableId)` 复用稳定表格身份，将表头、分隔行及正文替换为一个占位；`summary` 保留第 0 行源码，可用现有 `summary_text` / `DisplayExcerpt` 按预算读取。候选按 ID、源码位置或预算范围查询，父节点为当前章节；空正文表格仍保留入口，占位高度只归属隐藏起点 tile。查询和切换不枚举行、列或单元格。
- [x] **流式范围与默认设置**：`FoldDefaults::table` 只初始化后续首次确认的表格，通过 `Update.new_tables` 捕获已有段落块转为表格的晚确认；显式展开后追加行不重置意愿，合并/重复更新遵守既有契约。未分类行前缀暂归开放表格，确认下一块后收回，finish 封闭范围。已有范围更新仍只检查最多六个开放章节和一个代码/表格块，不遍历历史表格。
- [x] **折叠消费链集成**：整表接入共享预算候选、All/Subtree/Siblings/AtLevel/Focus、临时展开、源码锚点和书签恢复。父折叠隐藏表格占位但保留本地意愿；源码导航仅展开必要路径。隐藏单元格及图片不进入排版回调，复用区间高度遮蔽、旧测量拒绝及 resize 语义，两个视图互不影响。

验证：新增 8 项回归，覆盖仅表头的表格、同 tile 多占位及高度归属、UTF-8 摘要边界、隐藏图片、晚确认、显式展开、更新重放/合并、逐字符前缀及全部 UTF-8 单切点、未分类尾部回收、临时展开期间追加/闭合/resize、无效 ID、批量兄弟/聚焦及精确书签恢复。既有固定种子模型加入表格，与标题/代码混合切换、预览和 resize 后的源码区间并集逐次对照。百万字单元格收起后，排版只处理至多两个 tile、少于 10 字节文字和一个占位；追加只更新一个开放范围。1 万张历史表格后的默认初始化只访问新 TableId，范围更新每次至多访问一个开放表格。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 191 项、17 项文档测试、格式检查及 `git diff --check` 通过。`cargo run -p mdstream-core -r --example folding` 增加百万字表格示例：收起后追加新行、读取有界表头，2 帧完成占位及后继正文的消费。README、TODO 已同步；表格行预览/范围折叠、实际表头/摘要绘制与其余核心待办继续单独推进。

## 2026-10-30 表格前几行有界预览交付

- [x] **行前缀摘要核心**：`TablePreview` 读取包含表头的前 N 个索引行，跳过分隔行，共享源码字节和显示 UTF-8 字节总上限；保留行号及 DisplayRun 样式、链接、替代文本和源码映射。缺列不补空矩阵，不合成分隔符、边框或省略号。明确区分索引行结束、行数上限及源码/显示上限；多个字节上限同时触及时沿用 DisplayExcerpt 的首个检测原因，输出前缀不随预算变化。
- [x] **预算与复杂度**：构造/存储 O(1)，每行初始化消费 1 query / 1 object，显示读取与外部调用方共用 Budget，逐帧 6 bytes、1 query、1 object 可前进。R 个访问行、S 个检查源码字节的成本为 O(R + (S + 1) log(n + 1))；不枚举列、单元格或未选入行，超长首个单元格受总字节上限约束。
- [x] **整表折叠及流式组合**：任务独立于 FoldView，保持整表遮蔽、单占位、高度、版本及祖先/子节点意愿；不是局部展开前 N 行。已索引未完成行读取当前范围，未分类尾部排除；任务和输出绑定 revision，追加/首次 finish 后必须显式重建，错误文档/过期任务在扣费前拒绝。实际行可见性、摘要占位布局和连续行范围折叠继续保留在 TODO。

验证：新增 `crates/mdstream-core/tests/table_preview.rs` 5 项回归，覆盖所有 UTF-8 单切点、稀疏/多余列、行内实体/强调/代码、6/256 字节帧的独立完整显示模型对照、1200 组源码/显示上限组合、零/精确上限、仅表头、预算原子性、错误身份/非法 ID、完成重放、空追加/重复 finish、流式开放行及未分类尾部。百万字单元格加一万行未读取尾部，在 64 字节源码上限下累计计费少于 1500 字节，父章节和整表均收起时折叠版本/高度/高度树访问不变。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 196 项、18 项文档测试、格式检查及 `git diff --check` 通过。新增 `cargo run -p mdstream-core -r --example table_preview`：百万字表格收起后追加一行，9 帧、计费 151 字节读取前两行，结束为 RowLimit，整表保持收起。README 与 TODO 已同步。

## 2026-10-30 文内 slug 与 fragment 交付

- [x] **标题 slug 与全局消歧**：`HeadingAnchors` 显式按预算读取标题非隐藏显示内容；Unicode 字母/数字小写化，保留 `_`/`-`，每个空白变成 `-`，其余字符删除，空结果使用 `section`。不做归一化、音译、裁剪或空白合并。所有标题按源码顺序领取首个未占用的 base、base-1、base-2 等名称，处理天然数字后缀与空标题的碰撞；已完成结果返回借用 base、整数后缀、HeadingId 和精确标题源码起点。
- [x] **预算及复杂度**：构造 O(1)，Setext 确认不读取旧标题；显示读取、标量转换、输出字节、候选检查共用 Budget，每帧 6 bytes、1 query、1 object 可推进。字节 trie 的单节点扇出至多 256，后缀计数单调递增，从 base 节点继续检查，不重复复制/比较长前缀。标题源码总长 S、标题数 H 时，单 revision 构建 O((S + 1) log(n + 1) + H log(H + 1))、空间 O(S + H)；整数后缀与共享十进制路径避免逐项复制后缀字符串。显式格式化完整 slug 的成本由调用方承担，分配按摊还计。
- [x] **fragment、流式及导航**：`FragmentLookup` O(1) 借用输入/索引，按预算解码一次百分号并校验 UTF-8，O(m) 时间、O(1) 任务空间；`+` 不转空格，大小写敏感，`#` 返回文档起点，错误输入返回 InvalidRange。区分 IndexIncomplete、流式 Unresolved 和 finish 后 NotFound；标题结果在 finish 前全部暂定，任务/输出绑定身份及 revision，追加和首次 finish 后显式重建，空追加/重复 finish 保留。错误文档/过期在扣费前拒绝；源码目标复用 FoldNavigation 的必要路径展开，索引和解析不修改折叠状态。

验证：新增 8 项集成回归和 1 项内部复杂度回归。覆盖显示实体/强调/代码/链接、Unicode 小写扩展与组合字符、空标题和天然数字后缀冲突、原始/混合/百分号编码、严格 UTF-8 和非法尾部、逐预算维度与重复完成、错误身份/过期、全部 UTF-8 单切点、长 ATX 逐前缀、Setext 和晚到引用；固定种子 60 份文档共 2400 个标题与朴素全字符串模型对照并百分号回查。百万字 Setext 的确认及构造不扫描标题，小预算首帧计费不超过 64 字节；两个相同百万字标题总计费小于输入长度五倍，百万字 fragment 每个输入字节只解码一次。4 万标题（先占用两万个数字名称，再加入两万个重复 base）验证单调候选计数和线性 trie 节点上界。百万字正文不增加索引读取预算，折叠内目标恢复到精确源码且保留无关折叠。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 205 项、19 项文档测试、格式检查和 `git diff --check` 全部通过。新增 `cargo run -p mdstream-core -r --example anchors`：流式索引 4 帧/78 计费字节返回 Unresolved；晚到目标及 finish 后显式重建 6 帧/115 字节，解析重复 slug 和百分号中文目标，恢复源码 37 并保留无关折叠。验收日志：`/tmp/mdstream-anchors-validation.log`。README 与 TODO 已同步；跨 revision 维护、平台特定 slug 兼容和目录/链接点击 UI 未包含在本次核心接口中。

## 2026-10-30 显式列表与引用容器交付

- [x] **容器身份与结构**：独立 arena 分配稳定 `ContainerId`；Quote/List/Item 保存父 ID、深度、bullet/有序分隔符、条目编号及任务标记。源码有序叶块只保存最内层容器 ID，沿用按物理行解析；列表兄弟保留 List ID 并创建不同 Item ID。容器范围为完整物理行包络，不复制祖先链，不构造递归 AST。
- [x] **显式方言与流式开闭**：混合 `>` 和有序/无序列表，条目按正文前缀宽度匹配 ASCII 空格续行；缺失前缀、兄弟切换、空行和 EOF 有确定关闭语义。沿用 1024 字节分类前缀（至多加 3 字节完成 UTF-8），未分类尾部排除，截断候选及前缀后标记不重解析。开放容器共用已解析末端；Update 提供新 ID 范围、实际关闭 ID 及共享末端，支持 merge 和缓冲区复用，空追加/重复 finish 无开闭。
- [x] **预算与复杂度**：范围、父关系、深度按 ID O(1) 查询；借用文档的 `ContainerAncestors` 每步计 1 query / 1 object，耗尽不移动，游标空间 O(1)。活动栈每次保留一层都消费显式前缀或正长度缩进，共享空格扫描结果；每个容器只开闭一次。新增容器解析累计 O(n + 开闭次数)、空间 O(n)，全库继续 O(n log n)；长行追加只改共享末端，不扫描/更新所有祖先。公开前缀/开闭操作计数及 arena/活动栈容量；EOF 和批量关闭仍按实际事件计费，不承诺解析硬时延。

验证：新增 `crates/mdstream-core/tests/containers.rs` 的 11 项回归，覆盖混合父关系/精确范围/叶块归属、同列表不同条目、不同 bullet/分隔符、数字宽度、空任务、显式/缺失续行、缩进空行、顶层表格及围栏优先级、未实现内层块保持正文、流式尾部/EOF、事件重建所有中间快照、merge/身份/重放、共享预算原子性和恢复。固定种子 2000 行引用深度变化对照独立范围模型；现有单切点、随机 UTF-8、逐前缀与 fuzz 快照新增容器。1000 层引用下 1 万次逐字追加及百万字长行追加不增加容器前缀/祖先步骤，arena/活动栈容量小于 200 KB；EOF 恰好关闭 1000 个节点。64/128/256 层列表的 100 行续行检查次数恰为层数 × 行数，空格检查按输入线性计；退出后 2 万空行不产生重复关闭或祖先步骤。分类前缀截断、9/10 位数字、tab 分隔及 CRLF/UTF-8 的每个观察前缀一致。

项目 devShell 下 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 216 项、20 项文档测试、格式检查及 `git diff --check` 通过。新增 `cargo run -p mdstream-core -r --example containers`：5 个容器，展示未分类尾部、续行和兄弟关闭，2 帧读取 4 层祖先；60 字节输入计 78 容器前缀检查、18 容器步骤。验收日志：`/tmp/mdstream-containers-validation.log`。README、TODO 已同步。

本次仅完成独立可用的显式结构核心。lazy continuation、tight/loose、tab stop 缩进、容器内标题/表格/定义/代码块、跨物理行行内语义、前缀上限扩展及容器折叠/章节作用域继续保留 TODO；不声明完整 CommonMark/GFM 容器兼容性，终端 UI 未包含在本单元。

## 2026-10-30 容器内标题与章节作用域交付

- [x] 显式 Quote/Item 内识别 ATX 和单物理行 Setext；`Heading.container` 保存确切作用域，标题/块稳定身份、行内显示及源码映射复用既有模型。Setext 不跨兄弟条目、嵌套边界或空行，不扫描旧正文；hash run 在分类窗口内缺分隔符/完整行末时保留正文。
- [x] 全局六级索引包含容器内标题；父子/兄弟只比较同一作用域。章节止于下一同作用域的同级/更高标题或容器边界，外层章节包含容器；`current` 通过源码上下文边界和进入时的外层章节摘要恢复，无祖先链扫描。
- [x] 范围随共享已解析末端增长，未分类尾部沿用旧上下文，关闭后末端固定；作用域开闭各 O(1)，标题追加摊还 O(1)，查询 O(log n)，空间 O(H + C + R)。`heading_steps` 对作用域开闭、上下文检查及标题插入计数；关闭和新标题事件驱动派生范围，不枚举历史标题通知。
- [x] 名称搜索、slug/fragment、全局/结构导航沿用预算接口；顶层折叠使用根作用域索引，内层 h1 不会截断外层章节。容器内标题暂不提供独立折叠候选，显式预览/批量目标在扣费前拒绝，保留最多六个开放顶层折叠章节的上界。

验收：项目 devShell 中 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 226 项、`cargo test --doc --workspace` 21 项、`cargo fmt --all -- --check`、`git diff --check` 全通过。`cargo run -p mdstream-core -r --example container_headings` 输出 4 个标题及精确作用域/范围，slug 以 55 帧小预算构建。日志：`/tmp/mdstream-container-headings-validation.log`。

`crates/mdstream-core/tests/container_headings.rs` 的 10 项验收包含混合嵌套/任务/CRLF、单行 Setext 优先级与边界、全部 UTF-8 单切点和逐标量中间快照、1200 行固定种子线性章节/导航模型、千层容器中的百万字追加和 Setext 无回扫、500 个同时开放的章节作用域以及 1 KiB 截断候选。百万字追加的 `heading_steps`/`container_steps` 不增长；Setext 确认只有 2 个 heading steps、行内扫描增量为 0；批量关闭 1000 层只处理 1001 个 heading steps。深层搜索/slug 共用每帧 6 bytes、1 query、1 object，折叠导航不读标题或正文。

后续仍保留多物理行 Setext、其它容器内块、跨行行内语义、lazy continuation、tight/loose、tab 缩进/分类窗口扩展，以及容器与内层章节折叠、深度策略和终端 UI；本交付不宣称完整 CommonMark/GFM 容器支持。

## 2026-10-30 容器内表格核心交付

- [x] **识别、边界与分隔行**：显式 Quote/Item 的普通正文表头与完整分隔行在同一 ContainerId 中确认，去掉容器前缀后复用管道/对齐规则。先匹配已存在的容器，再让有效分隔行优先于新开列表；兄弟条目、进入/退出子容器、空行与其它块级形式终止表格，回到外层不续接。表头/分隔行的 1 KiB 分类窗口计入前缀，正文行无限长流式消费；失败候选仅解析一次，并保留普通行内/单行 Setext 回退。
- [x] **身份、索引与增量**：复用 TableId、BlockId 与 Block.container，确认表头不更换已有块 ID；表/行/分隔行保存完整物理行范围，单元格排除前缀，row_at 覆盖行前缀但不覆盖分隔行。列宽提示、new_tables、rows_changed、未完成行与 EOF 沿用既有协议。前缀只在物理行分类时检查；长行追加不遍历祖先、不回扫旧行；稀疏表不分配空矩阵，保持累计 O(n log n)、空间 O(n)。
- [x] **折叠与预算消费**：整表候选的父节点为当前顶层作用域章节，跳过尚无折叠候选的内层标题；默认收起、pending 尾部回收、父章节意愿、临时展开、resize、源码导航和单占位排版直接复用既有契约。只有一个开放表格，更新不增加开放范围遍历上限。TablePreview 总源码上限及共享 Budget 包含隐藏容器前缀，不读取未选中的行或扫描祖先。

验证：新增 12 项回归（`crates/mdstream-core/tests/container_tables.rs` 10 项、`crates/mdstream-core/tests/folding.rs` 2 项），覆盖混合嵌套/任务/CRLF、分隔行优先级与兄弟边界、失败/EOF/Setext 回退、顶层段落进入容器时的块隔离、稳定 ID 和通知、全部 UTF-8 单切点/逐标量中间快照、分类窗口边界，以及固定种子 400 张表的独立物理范围/行/单元格/深度模型。1000 层引用下百万字追加不增加分类、容器前缀/步骤或标题作用域步骤；400 列加 2 万稀疏行仅分配 20,400 个单元格，追加只扫描新行。900 层前缀在 900 字节摘要总上限下不输出正文；共享预算预留 1 字节给其它消费者后仍可逐帧读取，保持折叠版本/高度不变。百万字容器表格收起后只生成一个占位，正文回调读 0 字节，追加只刷新一个开放折叠范围。

项目 devShell 下 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 238 项、`cargo test --doc --workspace` 22 项、`cargo fmt --all -- --check`、`git diff --check` 全通过。新增 `cargo run -p mdstream-core -r --example container_tables`：演示稳定容器身份、流式创建/行通知、兄弟边界与默认整表收起；14 帧、66 计费字节读取两行摘要。验收日志：`/tmp/mdstream-container-tables-validation.log`。README、TODO 已同步。

本单元完成容器内表格识别和索引核心，不代表完整容器规则或 GFM 兼容。容器内定义/代码块、多行 Setext、跨行行内语义、lazy continuation、tight/loose、tab 缩进/前缀上限扩展，以及容器/内层章节折叠、实际表格排版和行范围折叠仍保留 TODO；未实施终端 UI。

## 其他已完成任务

- [x] **章节关系与动作**：当前所属章节、父章节、首个子章节、同父的前后兄弟、严格同级及按级别上限的前后标题分别定义。跨级标题的父节点取最近有效的级别数值更小的标题；章节关系与列表/引用容器父关系分开保存或查询，避免将相同级别误当兄弟。

- [x] 增补标准语法样例覆盖，单独报告具体支持范围，不把自身回归测试通过当作 CommonMark/GFM 合规率。已固定 CommonMark 0.31.2 autolink 19 个样例及 24 个选定显示文本样例；其余范围及 GFM 全集仍未评分，见 README。

## 2026-10-30 显式容器折叠核心交付

`FoldId::Container(ContainerId)` 将显式 Quote/List/Item 的稳定身份和范围接入独立 `FoldView`，支持整段引用、列表、条目、空内容与任务条目。范围覆盖物理行及前缀，共用一个占位；摘要仅借用源码，读取受字节上限与共享 Budget 约束。容器父节点是结构父容器，根容器挂在顶层章节下；容器内表格改为挂在确切容器下，内层标题仍不作为独立折叠候选。父收放、临时展开、resize 和流式追加保留子意愿，源码及解析属性不变。

候选游标二分并归并块与容器 opener，每步消费一个 query/object，同起点按容器前序外到内输出后再输出叶块。批量 All/Subtree/Siblings/Focus 接入容器，同起点且同范围的 List/Item/Table 通过前序消歧，避免误改祖先/后代。Reveal 每步只展开一个遮蔽者，任意深路径可跨帧恢复；VisibleOnly 跳过整段，源码锚点保留原始位置。旧任务、测量 ticket、错误文档、预算原子性和重复 Update 行为保持。

视图新增按 ContainerId 索引的稀疏最大终点树，开放范围使用开放终点标记，关闭事件才固化；查询最外层容器 O(log C)，不遍历祖先。开放容器同在活动路径，共享一次完整 tile 遮蔽并集更新；每次 apply 的容器成本 O((E + 1)(log C + log F + log T))，E 为关闭事件数，与仍开放的深度无关。独立范围仅处理最多六个顶层章节和一个代码/表格块。`container_index_visits` / `container_index_nodes` 提供访问与存储证据；索引保留位置为 O(C)，反复切换不积累快照。诊断计数保留 FoldView 的 Send/Sync，仍为零运行时依赖、禁止 unsafe。

新增 12 项验收：UTF-8 全切点、逐前缀和合并更新；同 tile/同起点父子与章节/表格组合；空内容/EOF/任务元数据；预览跨增长、关闭和 finish；200 层导航逐帧计费；千层开放链下百万字正文、1000 次逐字追加只做 1000 次共享尾部检查且无容器索引写入；有限布局预算内一个占位、正文回调/计费字节为 0；4096 个关闭兄弟的查询计数与反复切换节点上界；固定种子 2000 次操作及 240 次混合流式更新对照独立区间/高度模型。既有批量测试同步计入新容器候选的访问和状态变化，原来的非候选叶块仍逐步计费。

项目 devShell 下 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 250 项、`cargo test --doc --workspace` 23 项、`cargo fmt --all -- --check` 与 `git diff --check` 全通过。`cargo run -p mdstream-core -r --example container_folding` 展示追加只访问共享容器尾部及开放表格共 2 个范围，父预览保留子意愿，6 个预算帧恢复 quote/item/table 下的源码位置 42。验收日志：`/tmp/mdstream-container-folds-validation.log`。README、TODO 已同步。

本单元交付显式容器折叠核心，保留内层章节独立折叠、容器深度/类型/任务默认策略、实际摘要和容器布局、终端 UI，以及原有未实现容器语法。开放折叠暂纳入未分类尾部、确认边界后回退，原 Container 结构范围仍只包含已分类输入；不声称整个容器/折叠规划完成。

## 2026-10-30 容器深度默认策略交付

`FoldDefaults::container_depth: Option<usize>` 以包含式结构深度阈值初始化新 Quote/List/Item 的折叠意愿，`None` 禁用；根深度为 1，List 与 Item 各占一层，0 与 1 均覆盖所有容器。只处理后续 `Update.new_containers`，包括晚确认和合并更新中已经关闭的节点；合并更新使用 apply 时的当前策略。已有文档创建视图、修改/关闭策略都不追溯已有节点，显式收放、导航及批量选择在普通追加、关闭、finish、重复 apply 和 resize 后保持。预览期间显式选择修改保存意愿，新子节点独立套用默认策略；父预览不消除子折叠。

实现读取已存的 `Container::depth`，不遍历祖先或历史节点。启用时每个新容器增加一次 `default_visits`；新增 N 个容器、其中 K 个达到阈值时，额外初始化成本 O(N + K(log H + log C + log F + log T))，策略只增加 O(1) 空间。既有折叠状态/稀疏索引和共享尾部机制不变；没有新节点的追加不做默认值检查，关闭只按实际事件处理。apply 仍按输入/事件消费，不宣称任意大更新具有单帧硬预算。零运行时依赖、禁止 unsafe 保持。

新增 8 项测试，覆盖 None/0/1/中间阈值/usize::MAX、明确的 Quote/List/Item 深度、独立视图、已有与显式意愿、父子恢复、全部 UTF-8 切点与逐前缀、逐次/合并更新、All/Subtree/Siblings/Focus 与表格/预览组合、策略修改的旧任务拒绝及相同值无操作。千层容器/百万字正文下初始化检查恰好 1000 个节点，随后 1000 次逐字追加默认检查和容器索引写入均不增长，仅增加 1000 次共享尾部检查；有限布局预算产生一个占位，正文计费/回调字节为 0。4096 个历史兄弟后新增一节点只检查一次。固定种子 240 次流式策略切换、显式收放、批量展开、预览和 resize 对照独立区间/可见性/高度模型。

项目 devShell 下 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 258 项、`cargo test --doc --workspace` 24 项、`cargo fmt --all -- --check` 与 `git diff --check` 全通过。新增 `cargo run -p mdstream-core -r --example container_defaults` 演示预览中的新子节点、显式展开优先及批量展开后新条目仍默认收起：8 个预算帧展开已有节点，共初始化 5 个容器。原 `container_folding` 示例仍以 6 帧恢复源码位置 42。验收日志：`/tmp/mdstream-container-defaults-validation.log`。README、TODO 已同步。

本单元完成容器深度默认策略；按类型/任务完成状态筛选、内层章节独立折叠、实际摘要/容器排版与终端 UI，以及原有未实现容器语法继续保留 TODO。

## 2026-10-30 容器内章节独立折叠交付

`FoldId::Heading` 接入全部已识别容器内 ATX/单物理行 Setext，候选保留完整标题行、折叠正文及子章节；空正文没有占位。范围按确切作用域止于同级/更高级标题或容器边界，嵌套标题不截断外层。折叠父链先取同作用域父章节，否则为所属容器；子容器和表格挂在所在结构父作用域的当前章节下，没有本地标题时回退到容器。原始标题/容器结构关系不变。同起点前序、源码定位、预算导航、All/Subtree/Siblings/AtLevel/Focus、预览、独立视图和 `heading_levels` 新节点默认规则覆盖内层章节。

内层章节复用容器稀疏最大终点树实现（模块改为 `fold_ranges`），按 HeadingId 单独索引。开放章节与容器共享一次完整 tile 尾部遮蔽，不随逐字追加更新所有开放叶子。新标题通过同作用域六级索引定位可能结束的旧章节；容器关闭只检查六个级别的末尾标题，已封闭/未收起的节点跳过。长 ATX 只移动一个隐藏起点；Setext 晚确认、待分类行边界回退、合并 Update 与 finish 固化精确范围。N 个新标题、E 个关闭容器的范围维护为 O((N + E + 1)(log H + log C + log F + log T))，默认初始化另按新对象计费；查询不扫描祖先，新增索引空间 O(H)，反复切换不存历史快照。`section_index_visits` / `section_index_nodes` 提供访问和存储证据。保持零运行时依赖、禁止 unsafe，apply 仍按实际事件消费，不承诺任意大 Update 的单帧时延。

新增 `crates/mdstream-core/tests/container_sections.rs` 12 项验收，覆盖明确父链/最内层候选、保留标题和空章节、同 tile/同起点、父子意愿、独立视图、Setext 晚确认、流式待分类尾部、UTF-8 全切点和逐标量中间快照、合并更新/重复 finish、全部批量动作、默认/显式状态及预览/表格组合、预算原子性和旧任务/测量拒绝。240 次流式追加、切换、预览和 resize 对照独立线性区间/父链/高度模型；既有“不支持内层折叠”的回归同步升级为新行为验收。

复杂度证据：500 个开放章节下百万字正文后的 1000 次逐字追加只增加 1000 次共享尾部检查，章节索引访问、解析标题/容器步骤均不增长；有限预算布局只输出一个占位、读取少于 16 字节标题，百万字正文不进入回调。一次关闭 500 层恰好增加 501 次范围访问。150 层交替章节/容器导航以每帧 1 query/1 object 打开 300 个必要节点，正文计费字节为 0；一个内层章节下的一万个标题在前后各 4 步中整段跳过，保留本章节标题。4096 个关闭章节的查询少于 40 个索引节点访问，反复切换不增加节点数。百万字 Setext 确认不增加行内扫描，索引访问少于 40 次。

项目 devShell 中 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 270 项、`cargo test --doc --workspace` 25 项、`cargo fmt --all -- --check` 与 `git diff --check` 全通过。新增 `cargo run -p mdstream-core -r --example container_sections`：追加访问共享章节尾部与表格共 2 个范围，6 帧恢复源码位置 56，兄弟章节仍收起；既有 `container_folding` 示例仍以 6 帧恢复源码位置 42。日志：`/tmp/mdstream-container-sections-validation.log`。README 与 TODO 已同步。

本单元完成容器内章节独立折叠核心。容器定义/代码块、多物理行 Setext、复杂续行与其它未实现方言、类型/任务默认策略、实际摘要/容器排版、表格行范围折叠和终端 UI 继续保留 TODO，不声明整个容器或折叠规划完成。

## 2026-10-30 定界数学核心交付

`$…$` / `$$…$$` 使用完整美元串的精确长度配对；长度超过 2 的串、不同长度和转义定界符有确定的原文语义，无嵌套、货币/空白启发式或 TeX 校验。沿用既有代码/目标优先级与块/单元格边界，两种形式均可跨同一顶层段落的普通换行；display 是展示类型提示，不建立独立块。明确保留容器逐物理行和表格管道优先等方言边界。

闭合后发布稳定 `MathId`、`MathKind` 和完整 source / 原始 content 区间；`Style::MATH` 标记内容，定界符隐藏，Structure 及区间失效覆盖旧 tile 和测量。美元串等待非美元 lookahead 或行内作用域结束才确认，不提前发布可撤销对象。未闭合输入保持原文、单遍消费，块结束或 EOF 丢弃候选且不回扫内部语法。`Update.new_maths` 支持连续更新、merge 和缓冲区复用，空追加与重复 finish 不重复报告；已有区间不随追加变化。

`maths_in` 游标在首个预算步骤中二分定位，之后逐对象 O(1)，每次检查计 1 query / 1 object，不读正文。`math_content` 每步返回最多 TILE_BYTES 个原始 UTF-8 字节，计实际字节及 1 query / 1 object，预算不足不移动；两者 O(1) 状态、借用文档防止消费期间发生 revision 变化。与既有消费者共享预算，可暂停/继续/丢弃，不复制完整公式。通用 LayoutPass 保留 MATH 文字回退；实际 TeX 对象布局仍待后续接入。

解析增加一个常数大小候选，每个美元串只决议一次（`Work::math_steps`），内容沿用 `inline_bytes` 单遍扫描；闭合只做常数次 O(log n) 区间标记。新增对象空间 O(M)，全库保持累计 O(n log n)、空间 O(n)，零运行时依赖且禁止 unsafe；单次 append/Vec 扩容仍是摊还契约。

新增 `crates/mdstream-core/tests/math.rs` 11 项验收：精确/不同/超长串、空白与货币规则、奇偶反斜线、opaque 内容、代码/链接/实体/外层样式组合、CRLF/普通换行/Setext/标题/容器/单元格边界、失败及 EOF、延迟闭合与稳定身份、合并/复用更新、禁用样式尺寸策略时的旧测量拒绝、共享预算与 UTF-8 原子恢复、无效区间、4096 个对象的独立窗口相交模型。既有全 UTF-8 切点、逐前缀与固定种子 fuzz 快照加入数学元数据和语法。

复杂度证据：百万字开放数学后的 1000 次逐字追加只增加 1000 个 inline bytes，分类计数与美元串决议次数不增长；追加 `$$ ` 闭合只消费 3 字节、决议 1 次，区间树访问少于 1000 次。1024/4096/16384 规模的密集失配美元串、长美元串和转义候选逐字追加后 EOF 不扫描旧正文，annotation 节点少于 32。百万字公式仅用 16 bytes 的共享预算读取首片段；收起所在章节后布局只读少于 16 字节标题、生成一个占位，公式正文未进入回调。

项目 devShell 下 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 281 项、`cargo test --doc --workspace` 26 项、`cargo fmt --all -- --check` 及 `git diff --check` 全通过。`cargo run -p mdstream-core -r --example math` 展示流式确认、共享对象预算和 5 帧 / 16 字节读取原始公式；未闭合尾部不新增对象。验收日志：`/tmp/mdstream-math-validation.log`。README 与 TODO 已同步。

本单元交付定界数学区间和预算消费核心；独立数学块、其他定界方言、TeX 解析/排版、实际数学对象回调、脚注及终端 UI 继续保留待办，不宣称完整数学 Markdown 扩展兼容。


## 2026-10-30 脚注核心交付

本轮仅实现脚注解析、符号间接寻址、稳定编号及有预算查询；不交付终端 UI。`[^label]` 引用与顶层单物理行 `[^label]: body` 定义使用明确的有界标签方言，详细规则见 [README](README.md#脚注核心)。

- `FootnoteId` 与 `FootnoteReferenceId` 分离符号和引用对象；首次确认引用分配从 1 开始的永久编号，定义先到不占号、重复引用共享编号、未定义引用也占号。首个合法定义生效，空正文有效，重复定义隐藏但不覆盖。
- 定义正文为原始单物理行，可超出分类窗口持续增长；保持源码区间与 Growing/Sealed 状态，CRLF 仅调整尾端，不解析或复制正文。失败、超长、未闭合引用候选不回放已消费前缀；既有代码/数学/链接上下文和块/表格边界优先。
- `Update.new_footnotes`、`new_footnote_references`、`footnotes_changed` 支持逐次消费、merge 与缓冲复用。晚定义/增长只更新当前符号，以最早使用位置发出 Structure 范围失效；`TileKey` 保守版本、`DependencyKey` 按符号预算校验及 EOF 转换均接入。旧布局 ticket 不能跨语义变化提交，无关定义保留直接依赖缓存。
- `footnote_references_in` 在首次扣费后 O(log R) 定位窗口，后续逐对象 O(1)；`resolve_footnote` O(1) 间接解析，区分 Unresolved/Missing/Defined。`footnote_content` 按共享 Budget 和 TILE_BYTES 上限读取原始 UTF-8 字节，原子暂停并保持继续位置；不复制完整正文。
- 新增 11 项脚注验收，既有分块/fuzz 快照纳入脚注：覆盖 Unicode 标签、失败/未闭合、定义顺序和重复、原文回退、上下文/容器/表格、逐前缀增长及 CRLF、通知重建/合并、预算推进、缓存与布局失效。4096 个符号/引用窗口对照朴素区间模型；两万次旧引用后的晚定义仅 3 次符号步骤 / 1 个标签字节、无旧引用遍历；百万字定义后的 1000 次逐字追加仅 1000 次增长步骤，无正文、标签或分类回扫；失败候选计数线性。

验收在项目 devShell 内通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo nextest run --workspace --no-fail-fast`（292 项）、`cargo nextest run --workspace --release --no-fail-fast`（292 项）、`cargo test --doc --workspace`（27 项）、`cargo fmt --all -- --check`、`git diff --check`。`cargo run -p mdstream-core -r --example footnotes` 验证晚定义保留编号，6 帧读取 40 字节正文。日志：`/tmp/mdstream-footnotes-validation.log`。

保留零运行时依赖、`forbid(unsafe_code)`、累计解析 O(n log n) 与空间 O(n)。多行/多段及容器内定义、正文 Markdown 排版、编号上标和文末脚注区、双向跳转/返回与实际对象回调仍见 TODO；通用显示暂时保留 `[^label]` 原文标记，隐藏顶层定义。


## 2026-10-30 表格连续行范围折叠交付

- [x] **范围身份与边界**：`TableRowRange { table, start, end }` / `FoldId::TableRows` 绑定固定正文行号 `[start, end)`，排除表头/分隔行；不接受空、倒序、未来行或无效 TableId。`FoldView::insert_table_rows` 初始展开、精确重插保留意愿，视图内即使展开/预览也禁止重叠及包含，邻接范围保持独立占位；`remove_table_rows` 可撤销注册并重设边界。文档按 ID 查询 O(1) 元数据，语法候选枚举不生成任意行组合。
- [x] **视图组合与预算消费**：行范围直接父节点是整表；章节/容器/整表收起隐藏子占位，展开恢复局部选择。支持 set/toggle、All/Subtree/Siblings/Focus 批量、临时展开、源码导航和锚点、独立视图、resize 与旧测量/任务拒绝。批量逐个访问已注册范围，1 query / 1 object 原子推进；布局和摘要复用共享 Budget，完整隐藏内部使用高度区间遮蔽，占位归属起点 tile。独立 TablePreview 不改变范围状态。
- [x] **流式与 EOF**：允许选入已索引但未完成末行，其源码终点随该行增长；换行/finish 固定，未分类尾部及新行不自动加入。完整行结束于当前 EOF 时记录唯一边界候选，下一次更新校正同一个可增长 tile 的覆盖；否则旧 EOF 内新增的可见行会被误遮蔽。单次追加只更新一个活动行范围或一次 EOF 边界，不扫描所有历史区间；合并更新、重复更新/finish、预览恢复均使用当前范围。
- [x] **复杂度**：BTreeMap 按稳定物理行起点保存不相交注册，前驱查找点覆盖、相邻项验证重叠；注册 O(log R)，切换/移除 O(log R + log F + log T)，不遍历行/列/源码/覆盖 tile。注册空间 O(正文行数)，重复切换不积累历史；六个根章节 + 整表 + 当前末行最多 8 个逐次刷新范围，容器/内层章节继续共享末端。百万字后 1000 次逐字追加仅消费 1000 table bytes / 1000 次行范围刷新；4096 个历史范围后的普通追加零历史范围刷新。
- [x] **验收**：新增 `crates/mdstream-core/tests/table_row_folding.rs` 13 项测试，覆盖 UTF-8 全切点/中间快照、CRLF、空/越界/重叠输入、邻接占位、同 tile 边界、首行摘要预算、隐藏图片、全部批量命令、导航/预览/旧任务、合并与 EOF；1200 次随机切换/预览/reset 对照独立可见区间与高度模型，240 次嵌套作用域流式增长/跨表验收，4096 个范围查找/切换及 1000 次大范围注册移除验收。独立 EOF 用例覆盖 12 种行长与 ASCII/中/🙂 后继，确认仅一次边界校正且新行可见。
- [x] **项目 devShell**：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 305 项、`cargo test --doc --workspace` 28 项、`cargo fmt --all -- --check` 和 `git diff --check` 通过。`cargo run -p mdstream-core -r --example table_row_folding` 用 6 帧/38 个计费字节跳过百万字隐藏行，保留表头、前行及后续追加行。完整验收日志：`/tmp/mdstream-table-row-folding-validation.log`。

本轮交付固定、不相交正文行范围核心；没有改变 Markdown 源码、解析方言或零运行时依赖/禁止 unsafe 契约。真实表格与占位绘制、自动增长尾部策略、重叠/嵌套行区间及手工任意源码范围仍单独设计，终端 UI 不在本轮范围。


## 2026-10-30 容器内代码块核心交付

显式 Quote/Item 内围栏与四空格缩进代码复用 `BlockKind::Code`、稳定 `BlockId` 和确切 `Block.container`。块范围包含完整物理行，显示隐藏容器前缀/围栏或四格缩进，字面正文不解析行内或容器语法。围栏只匹配已有容器路径；缺失前缀、切换兄弟、显式 closer 和 EOF 封闭范围，未闭合围栏保持代码语义。缩进代码仅在同一确切作用域合并相邻行，沿用完整空白行即终止的顶层方言。失败/超长候选、CRLF、opener info、1024 字节分类窗口及不回看语义详见 [README](README.md#容器内代码块)。

代码候选直接接入已有独立折叠视图：父节点取同作用域当前章节，否则为所属容器；整块占位、info 摘要、`code` 默认初始化、子意愿、批量/兄弟、预览、源码导航、独立视图和旧测量失效均沿用预算接口。Growing 代码暂覆盖未分类尾部，边界确认时回退，合并更新可恢复正确状态；前几行摘要使用 `DisplayExcerpt`，隐藏前缀计入共享预算和源码上限。

生产实现仅增加容器围栏的作用域/前缀决议和同作用域缩进合并检查，无新增依赖、公共对象或独立索引。长行追加只增长当前块与共享尾部，无行内扫描或祖先更新；显式前缀按新增字节匹配，关闭按实际事件消费。全库累计解析 O(n log n)、空间 O(n)、禁止 unsafe 和预算契约保持。

新增 15 项测试（`crates/mdstream-core/tests/container_code.rs` 10 项、`crates/mdstream-core/tests/container_sections.rs` 4 项、`crates/mdstream-core/tests/chunking.rs` 1 项），并扩充现有 UTF-8 全切点/逐前缀/fuzz 样本及旧方言回归。覆盖嵌套/兄弟/作用域退出、字面内容、闭合失败、分类窗口、长行/CRLF/EOF、稳定身份及更新、合并更新、默认/预览/导航/表格组合、预算暂停、源码截断与过期任务/测量。400 个生成代码块与独立物理范围/显示模型一致，240 次流式收放/预览/resize 与独立线性区间/父链/高度模型一致。

复杂度验收：1000 层引用内，围栏与缩进代码各追加 1000 次 × 1000 字节，输入只增长百万字，分类/行内/容器/标题计数及容器/章节索引访问均不增长；离开容器只关闭 1000 个节点，不读取旧正文。250 层列表的两个续行恰好匹配 500 个活动帧，缩进串扫描线性且正文不创建子节点。4096 个已关闭代码块之后追加只检查一个新块，无历史刷新。折叠百万字代码在有限 query/object/tile 预算内生成一个占位，正文回调及计费字节均为零。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 320 项、`cargo test --doc --workspace` 29 项、`cargo fmt --all -- --check` 与 `git diff --check`。`cargo run -p mdstream-core -r --example container_code` 用 7 帧/45 个计费字节读取首行代码摘要，再用 6 帧恢复源码位置 30，缩进代码兄弟保持折叠。日志：`/tmp/mdstream-container-code-validation.log`、`/tmp/mdstream-container-code-release.log`。TODO 与 README 已同步。

本单元交付容器内围栏/缩进代码核心；容器内定义、多物理行 Setext、lazy continuation、tight/loose、完整 CommonMark 缩进/空行/tab 规则、实际代码排版与终端 UI 继续保留待办。

## 2026-10-30 容器内定义核心交付

- [x] **链接与脚注定义接入**：显式 Quote/Item 内按既有方言识别链接定义和单物理行脚注定义，复用 `BlockKind::Definition/FootnoteDefinition` 与稳定 BlockId；块保存确切容器、完整物理行及前缀，目标/title/正文保持源码区间。支持嵌套、条目续行及任务前缀，围栏/四空格缩进代码保持字面语义。
- [x] **全局符号和晚定义**：根、嵌套、兄弟及已经关闭的容器共用文档全局命名空间，链接和脚注各自独立；首个有效定义优先，重复定义隐藏但不覆盖/改版本。引用链接、图片及脚注复用间接寻址，晚定义只改当前符号及 first_use 范围失效；脚注编号仍按首次引用全局分配，容器退出和折叠不改变解析目标。
- [x] **流式、失败与边界**：链接定义连同行末须位于含前缀的分类窗口内；脚注 marker 在窗口内确认后正文可无限增长，LF/EOF 封闭，CRLF 只修正尾端。定义中断同作用域正文/表格、不作为 Setext 候选、不吸收下一物理行。非法/超长/未完成候选沿用行内失败规则，无效纯空白链接标签保留容器前缀隐藏和独立叶块，避免合并进外层段落。
- [x] **复用预算与视图协议**：现有 Update 新块/变化范围、符号通知及 merge 可重建全部中间状态；DependencyKey 和布局 ticket 处理晚定义，容器/章节折叠意愿保持。窗口引用查询、间接解析和 UTF-8 原文游标共用 Budget，原子暂停后可继续。语法隐藏的百万字定义布局跳过正文，字节计费及正文/对象回调均为零。

验证：新增 14 项验收（`crates/mdstream-core/tests/container_definitions.rs` 13 项及 `crates/mdstream-core/tests/chunking.rs` 1 项），并扩充既有 UTF-8 全切点、确定种子逐前缀/随机分块快照与脚注回归。400 组生成输入对照独立首定义、源码物理范围和容器深度模型；通知重建和逐次/合并折叠消费一致。覆盖大小写/Unicode 标签、首定义与重复、代码/标题/表格优先级、分类窗口、失败标签、流式正文、CRLF/EOF、预算及缓存。

复杂度：两万处链接和两万处脚注引用之后，容器内晚定义不改变旧引用数组，链接只 intern 1 字节标签、脚注仅 3 次符号步骤，不增加 inline bytes。千层容器中的百万字脚注追加 1000 次单字，只增加 1000 input bytes 和 1000 次当前脚注操作，无分类/正文/标签回扫、祖先操作或容器/章节索引写入；百万字失败链接候选后追加同样只消费新字节。250 层列表的续行空格单次扫描，4096 对历史链接/脚注符号不增加重复定义刷新；复用现有数据结构，累计 O(n log n)、空间 O(n)、零运行时依赖及 `forbid(unsafe_code)` 保持。

项目 devShell 下 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 334 项、`cargo test --doc --workspace` 30 项、`cargo fmt --all -- --check` 及 `git diff --check` 全部通过。`cargo run -p mdstream-core -r --example container_definitions` 演示全局晚定义、稳定编号和容器折叠，脚注正文 6 帧 / 32 字节读取完成。验收日志：`/tmp/mdstream-container-definitions-validation.log`、`/tmp/mdstream-container-definitions-release.log`。

保留边界：仍采用显式容器与既有 1 KiB 分类前缀；链接多行目标/title、脚注多行/多段及正文 Markdown 排版、脚注区/上标/双向跳转、完整容器续行规则和终端 UI 继续待办。本轮没有新增局部符号作用域或改变现有方言长度上限。

## 2026-10-30 容器内行内续行核心交付

同一确切 `ContainerId` 的显式非空正文续行现在共享强调、加粗、删除线、代码 span 和软/硬换行状态。物理叶块仍保留独立 `BlockId`、源码范围及容器身份；换行封闭本行范围，稍后的 closer 通过区间标记确认旧行样式。前缀只由容器匹配器消费并保持隐藏，不构造去前缀正文副本，也不为每个叶块或祖先复制行内状态。

边界与失败方言已写入 [README](README.md#容器内行内续行)：进入/退出容器、兄弟切换、空白正文及已识别标题/代码/定义/表格中断序列；回到外层不恢复旧状态。未决表头只保存前序状态，失败/EOF 时一次性消费其有界正文，成功时先清理状态再按独立单元格解析。单行 Setext 只提升最近物理叶块，保留之前已经确认的跨行样式及换行，下划线终止未闭合状态，不回滚或合并旧行。跨行链接标签/数学仍待去前缀内容表示；它们在容器 LF 处丢弃未完成候选，失败前缀不重放，单行对象及 EOF 语义保持。

代码 span 增加最后一个实际正文 byte/CRLF 区间；all-spaces 忽略前缀，闭合仅检查端点，正确去除规范化后的首尾一个空白并保留源码包络映射。显示查询、摘要、DependencyKey、旧测量拒绝及检查点失效复用既有范围协议；Budget 包含隐藏前缀和 CRLF lookaround，耗尽可原子暂停并继续。跨行闭合不改变容器/章节折叠意愿，逐次及合并 Update 均可消费。

新增 15 项测试（`crates/mdstream-core/tests/container_inline.rs` 14 项、`crates/mdstream-core/tests/chunking.rs` 1 项），并扩展 UTF-8 全切点/逐前缀及固定种子分块样本。覆盖混合引用/列表/任务、容器切换和块边界、失败/确认表头、非法定义回退、Setext 晚提升、CRLF/末端换行、代码空白与逐标量源码映射、单行对象、EOF、长行分类上限、增量失效和共享预算。400 组生成作用域输入对照独立可见文本/样式模型；原定义及表格回退测试更新为明确的续行预期，原有解析、显示、折叠、脚注和数学回归全部通过。

复杂度证据：千层引用的百万字未闭合强调/代码各追加 1000 次单字，仅增加 1000 inline bytes，分类和祖先计数不增长；下一显式行闭合只增加 6 inline bytes、至多 3 次分隔符步骤和少于 1500 次区间树访问。250 层列表续行恰好匹配 250 个活动帧（每帧拥有一个 List/Item 对），空格串计数线性；4096 个未闭合 opener 后追加一个正文字符不访问旧栈或祖先，EOF 仅弹出这 4096 个候选且不读取旧正文。新增持久状态为一个容器 ID 和一个代码末端区间，全库仍为累计解析 O(n log n)、空间 O(n)、零运行时依赖及 `forbid(unsafe_code)`；解析/EOF 维持摊还契约。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 349 项、`cargo test --doc --workspace` 31 项、`cargo fmt --all -- --check` 与 `git diff --check`。`cargo run -p mdstream-core -r --example container_inline` 用 19 帧 / 437 个计费字节读取跨行显示文本，保留六个物理叶块与条目折叠状态。日志：`/tmp/mdstream-container-inline-validation.log`、`/tmp/mdstream-container-inline-release.log`。

本单元交付显式正文的跨行行内衔接核心；跨行链接/图片标签及数学内容映射、多物理行 Setext、需要撤销已确认样式的更完整规则、lazy continuation、tight/loose、tab/分类上限扩展和终端 UI 继续保留 TODO，不声明完整容器或 CommonMark 兼容。

## 2026-10-30 容器 lazy continuation 核心交付

连续非空正文现在可省略引用或列表条目的部分/全部前缀，保留原最内层 `ContainerId`、物理叶块和既有行内状态。显式匹配在首次失配处停止，一次有界正文判定决定保留整段剩余路径或关闭未匹配内层；不逐祖先试探、复制活动栈或重放旧正文。前缀隐藏、软/硬换行、强调/代码 span 晚闭合、范围通知、容器/章节折叠及预算消费复用原接口。

方言与边界见 [README](README.md#容器-lazy-continuation)：空白、有效标题/围栏/定义/分隔线及新容器标记中断，任意合法编号和空列表标记沿用项目方言；只有已有非空正文具备 lazy 资格。未决表头可退为正文，但 lazy 行不确认/延续表格，也不新建表头；缺省前缀的 `===` 为正文，后续显式 Setext 仍只提升最近物理叶块。非法定义标签及失败候选留作行内正文；纯空白分类前缀不保留路径，长行沿用 1 KiB 分类窗口并在后续追加中不重试。

新增 15 项验收（`crates/mdstream-core/tests/container_lazy.rs` 14 项、`crates/mdstream-core/tests/chunking.rs` 1 项），并补充既有 UTF-8 全切点、逐前缀及确定种子随机分块样本。覆盖混合嵌套/任务、部分前缀、兄弟与块中断、非正文资格、空白/缩进、失败候选、表格/Setext、CRLF/EOF、旧测量失效、稳定叶块、显示预算和折叠状态；1200 行生成输入对照独立容器范围/开闭模型，并通过 Update 重建全部中间快照及合并事件。旧测试中明确要求退出的输入改用空白、标题或定义中断，保留原关闭、索引和折叠断言；普通省略前缀输入按新语义验证。

复杂度：千层引用之后 4096 行无前缀短文本恰好增加 4096 次活动帧检查；百万字 lazy 正文再逐字追加 1000 次，只增加 1000 inline bytes，无分类/前缀/祖先回扫。250 层列表的部分缩进仅检查实际匹配层和一次失配，空格扫描线性。500 个开放章节之后 1000 行 lazy 追加仅更新 1000 次共享折叠末端，容器和章节索引零写入；折叠百万字正文的 LayoutPass 计费 0 字节、无正文回调。没有新增持久状态或运行时依赖，累计解析 O(n log n)、空间 O(n)、`forbid(unsafe_code)` 及摊还 EOF 契约保持。

项目 devShell 验收：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 364 项、`cargo test --doc --workspace` 32 项、`cargo fmt --all -- --check` 与 `git diff --check` 通过。新示例 `cargo run -p mdstream-core -r --example container_lazy` 用 11 帧 / 250 个计费字节读取混合续行，保持 4 个容器及折叠意愿；另外 8 个既有容器相关示例通过。日志：`/tmp/mdstream-lazy-final-validation.log`。

附加巡检发现 `crates/mdstream-core/examples/container_tables.rs:30` 仍把表格折叠父节点断言为顶层标题，实际已是条目容器；在改动前提交 `a976459` 的独立源码副本中复现相同失败（`/tmp/mdstream-lazy-baseline-example.log`）。此为既有示例断言问题，本单元未修改它；表格相关核心测试全部通过。

本单元完成有界 lazy continuation 核心；tight/loose、完整 tab 缩进、分类上限、多物理行 Setext、跨行链接/图片/数学内容表示及更完整块确认/撤销规则保留 TODO，终端 UI 仍留待后续。

## 2026-10-30 列表 tight/loose 显式方言核心交付

列表通过 `Container.list_tightness` 共享 `ListTightness::Tight/Loose` 元数据，Quote/Item 为 None。新列表的 Tight 在 Growing 时暂定、Sealed 后最终确定；Loose 单调成立。只有确切 Item 作用域的显式空行记为待定分隔，后续直接内容、新直接子容器或同列表兄弟才确认 Loose。首行空标记、前导/末尾空行、嵌套作用域隔离、围栏代码空白及有界分类语义见 [README](README.md#列表-tightloose)。沿用无前缀空行关闭容器的既有行为，未扩大为完整 CommonMark 空行规则。

`Update.loosened_lists` 每个列表只报告一次转换，可与新建/关闭事件及共享末端重建全部快照；合并只移动增量，append_into/finish_into 保留 Vec 容量，空追加与重复 EOF 无事件。确认时只改列表节点，并对源码包络做一次 O(log n) 结构/语法版本失效；历史叶块、条目、样式及链接无需回访或改写。旧 TileKey/DependencyKey、测量及检查点正常失效，屏外保留估计高度，折叠意愿不变；查询复用每层 1 query / 1 object 的祖先预算，不实现实际间距排版。

新增 `crates/mdstream-core/tests/list_tightness.rs` 的 15 项验收，并扩充全局 UTF-8 全切点/逐前缀样本。覆盖 bullet/有序/任务/引用列表、空条目、前导/末尾/连续空行、嵌套直接子节点、标记变化、代码/标题/表格/定义及失败候选、CRLF、EOF/长空白分类上限、通知重建/合并/容量复用、缓存/旧测量失效、隐藏布局预算与分块一致性。400 组固定种子混合嵌套模板对照独立 spacing 预期，并以随机分块及一次追加对照全快照。

复杂度证据：64/128/250 层列表各处理 1000 条显式空行，`list_steps` 均只增 1000，后续正文确认只增 2 步且仅最内层列表 Loose；千层引用中的列表经过 4096 条无前缀 lazy 行只增 4096 次帧检查/直接内容检查。1 与 4096 个旧条目后的晚确认同为 3 次 list_steps、3 次 container_steps，区间树访问少于 1000；历史叶块保持原值。百万字正文后 1000 次逐字追加只增 1000 inline bytes，分类/前缀/spacing 计数均不增长；已 Loose 列表不重复全范围失效。折叠百万字列表后晚确认，LayoutPass 不触发正文回调且计费 0 字节。新增持久状态为列表属性及每个活动列表帧的两个布尔摘要，不扫描祖先或子树；累计 O(n log n)、空间 O(n)、零运行时依赖与禁止 unsafe 保持。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 379 项、`cargo test --doc --workspace` 33 项、`cargo fmt --all -- --check` 和 `git diff --check`。`cargo run -p mdstream-core -r --example list_tightness` 验证两个列表的晚确定通知，6 帧读取 5 个容器并完成路径查询。日志：`/tmp/mdstream-list-tightness-validation.log`。

本单元交付有独立价值的显式方言 tight/loose 元数据与失效核心；跨无前缀空行保持列表身份、嵌套末尾空行传播、更完整缩进/块边界、实际段落间距和终端 UI 保留 TODO，不声明整个容器语义已完成。


## 2026-10-30 长表格分隔行识别核心交付

本单元交付语法上限扩展的独立子项：紧邻既有有界表头的顶层及显式容器分隔行不再受 1 KiB 长度限制。保持原有管道、列数、ASCII 空白与左右单冒号方言，允许任意长连字符/空白及窗口之后的首管道；LF/EOF 才最终确认，CRLF 可跨分块。长行开始只读匹配表头的确切容器路径，不新开容器、不接受兄弟或 lazy 续行；确认后复用稳定 BlockId/TableId、行/单元格索引和增量通知。规则见 [README](README.md#表格任务列表与图片)。

算法只保存一个单元格有限状态和至多有界表头列数个对齐值，候选正文保留在原始 String 中，不复制增长中的尾部。`pending()` 仍至多 1024 + 3 字节，探测时返回原始分类前缀；其后原文可由片段读取，尚未形成块/表，容器和章节的已解析末端暂留表头处。新增 `Work.table_candidate_bytes` 统计逐字节探测；成功确认只解析有界表头、一次范围隐藏分隔行，不回读长分隔行。

失败只重新分类原有有界前缀，沿用不完整行规则；延迟长尾普通消费一次后进入既有流式路径，不再尝试表头、Setext、定义或容器重分类。**有界的是分类回退窗口与重试次数，单次晚失败仍可能消费长尾，不是硬时延/字节预算保证。** 每个探测字节至多检查一次、普通正文至多解析一次，附加识别存储 O(表头列数)，全库累计 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持。

新增 15 项验收：`crates/mdstream-core/tests/long_delimiters.rs` 13 项、`crates/mdstream-core/tests/chunking.rs` 1 项、识别器语法对照 1 项。原有长表头拒绝测试保留，原长分隔行拒绝断言改为接受。验证覆盖首尾管道/四种对齐、CRLF/EOF、列数不足/溢出/空单元格/晚 Unicode 失败、显式混合嵌套/任务/兄弟/lazy/代码边界、原段落与缩进代码回退、稳定范围、旧测量和缓存键、Update 合并、章节父关系与折叠、预算原子暂停及摘要跳过长分隔行。117649 种六字节候选 × 3 种列数（352947 组）对照原有独立 split/trim 语法；UTF-8 全切点与逐前缀快照一致；400 组有效/无效输入对照独立表/分隔/正文行范围模型。

复杂度验收：千层引用的百万字成功/失败探测，每次追加只增长新增字节的探测计数，分类、行内、祖先及标题计数不变；确认只读有界表头，晚失败分类输入至多 2 × 1024 字节，后续 1000 次逐字追加不再探测或分类旧尾部。250 层列表匹配线性并保持 500 个容器身份；400 列分隔行及失败后 20000 个多余单元格不积累候选矩阵。共享 Budget 的摘要按行索引跳过百万字分隔行，不按分隔行长度计费。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 394 项、`cargo test --doc --workspace` 34 项、`cargo fmt --all -- --check` 和 `git diff --check`。`cargo run -p mdstream-core -r --example long_delimiters` 检查 1000008 个分隔行字节，13 帧/72 个计费字节读取表头与正文摘要（跨帧读取窗口可重叠）。日志：`/tmp/mdstream-long-delimiters-validation.log`、`/tmp/mdstream-long-delimiters-release.log`。

本单元未移除表头长度上限；长表头成功后的单元格语义提交、失败后的前序行内状态恢复，以及其它 Setext/fence/容器分类上限继续保留 TODO。未实现终端 UI，也不声明整个语法上限方向已完成。

## 2026-10-30 早管道长表头识别核心交付

本单元交付长表头的独立子项：分类前缀内已有未转义 `|` 的顶层或显式容器普通正文，可以延伸任意长的表头候选。沿用既有不完整行分类、确切容器作用域、首尾管道/空白及反斜线奇偶规则；不在长尾重试容器、标题或定义分类。首个管道晚于窗口的长行仍保持即时普通行内解析，余项明确保留 TODO。完整方言与示例见 [README](README.md#表格任务列表与图片)。

算法先用列数和少量布尔状态逐字节摘要，不分配单元格数组或复制候选原文。LF 后固定列数，等待紧邻分隔行；CRLF 可跨分块。候选不提前发布行内样式、链接、数学或脚注对象；成功时结束前序状态、按独立单元格语义消费一次，失败/EOF 时把候选交给保留的段落/容器行内状态一次。原块身份、单行 Setext 回退、原顶层段落端点、容器共享末端及范围失效协议保持。长分隔行晚失败只回退原有有界前缀，禁止在该行重开长表头候选。

`Work.table_header_bytes` 统计摘要检查，候选每个字节至多一次；另一次语义消费只发生在确认或回退时，已解析历史正文不重放。候选附加状态 O(1)，分隔行对齐数组 O(表头列数)，确认后才创建正式列/单元格；稀疏行不补空矩阵，原始 String、tile 和正式语义索引保持总空间 O(n)，累计解析 O(n log n)。早管道长表头已分类，`pending()` 为空，原文可按 tile 预览；长分隔行探测继续保留有界 pending 前缀。长表头确认/失败可能单次消费全部延迟内容，未承诺解析调用硬时延或字节预算。

新增 15 项验收：`crates/mdstream-core/tests/long_headers.rs` 13 项、`crates/mdstream-core/tests/chunking.rs` 1 项、摘要语法模型 1 项。117649 组独立 split/trim 对照验证列数、转义奇偶、首尾管道和空白；UTF-8 全切点/逐标量中间快照覆盖成功、失败、EOF、CRLF、窗口处转义及原有段落状态。表格/容器组合覆盖任务/有序列表、兄弟/嵌套/lazy 拒绝、代码优先、单元格隔离、对象仅发布一次、长分隔行回退、稳定 BlockId 与源码范围、旧测量/缓存失效、合并 Update、章节父关系及父子折叠意愿。

复杂度与内存验收：千层引用下百万字候选只增加新字节摘要计数，分类、行内、容器/标题步骤、容器及注解 arena 容量不变；成功/失败后不重复消费。250 层列表保持 500 个容器身份；1000 个连续失败候选的摘要量等于一次行内消费量。4096 列加 4000 个稀疏正文行只分配 8096 个真实单元格；400 组成功/失败输入对照独立表/行/单元格范围模型。共享 Budget 摘要在 64 字节源码上限停止，零预算不推进；百万字表头折叠后追加新行，实际布局仅回调一个占位、正文及计费字节均为 0。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 409 项、`cargo test --doc --workspace` 35 项、`cargo fmt --all -- --check` 和 `git diff --check`。`cargo run -p mdstream-core -r --example long_headers` 摘要扫描 1000012 字节、显示摘要 15 帧/100 个计费字节；原 `long_delimiters` 示例仍为 13 帧/72 个计费字节。完整日志：`/tmp/mdstream-long-headers-validation.log`。

README、TODO 已同步；只勾选已交付的早管道长表头子项。晚管道表头的语义撤销/延迟策略、其它分类长度限制和终端 UI 继续保留待办。

## 2026-10-30 新行内序列的晚管道长表头核心交付

本单元交付首个未转义管道晚于分类窗口的独立子项：顶层新段落或不继承同一容器前序行内状态的显式正文，允许任意长首单元格。保持原有分类优先级和确切容器路径；已有表格正文、代码、标题、定义及 lazy 行不进入探测。完整流式方言与示例见 [README](README.md#表格任务列表与图片)。

模型依据：在没有前序 Inline、也尚无未转义管道时，普通行内解析和首单元格行内解析消费相同字节；单元格首尾空白及代码内 `\|` 的额外隐藏可在确认时单调补充，不必撤销任何已发布对象。分类窗口后继续即时消费首管道前缀，同时单遍更新原有固定大小摘要；遇到首管道时停在管道之前，冻结 Inline 并延迟后缀。确认时移动该 Inline 到 RowParser，扫描一次表头建立单元格与空白范围，跳过已解析前缀，在首管道处结束首单元格状态。失败/EOF 仅消费未解析后缀；无管道普通长行持续即时消费，收尾不新增整行注解，容器换行和单行 Setext 行为保持。

每个候选字节只做一次摘要、每个行内字节只消费一次；成功时额外有一次单元格边界扫描，分别计入 `table_header_bytes`、`inline_bytes`、`table_bytes`。没有回放历史正文、复制源码、克隆 Inline 或提前建立候选单元格数组；候选额外状态 O(1)，原有 Inline/对象及正式表格索引维持总空间 O(n)、累计解析 O(n log n)。链接、数学、脚注的已发布 ID 和首次引用编号保持。长候选确认仍可能扫描整个旧前缀，失败可能消费整个延迟后缀，不声明单次调用硬预算。

新增 14 项验收：`crates/mdstream-core/tests/late_headers.rs` 13 项、`crates/mdstream-core/tests/chunking.rs` 1 项。另扩充既有 117649 组摘要语法模型，以独立反斜线奇偶检查对照首管道位置，逐字节与整段摘要一致。全 UTF-8 切点/逐标量快照覆盖成功、失败、EOF、CRLF、转义边界、继承状态及长分隔行晚失败；首单元格语义对照既有全延迟早管道路径，覆盖样式、实体、代码、链接/图片、数学、脚注和单元格边界。对象在管道前已可查询，确认不重复发布；覆盖任务/有序列表、兄弟/嵌套/lazy/代码边界、稳定 BlockId、缓存与旧测量拒绝、合并 Update 和章节/容器折叠。

复杂度与预算验收：千层引用下首管道前百万字逐批追加仅增加新字节摘要和行内计数，管道后百万字只增加摘要；分类、容器/标题步骤及容器/注解 arena 容量不变。确认或失败不重复行内消费前缀。250 层列表保持 500 个容器身份；4096 个已发布链接/数学/脚注在确认时没有额外符号步骤，4096 列加 4096 稀疏行仅有 8192 个真实单元格；400 组成功/失败输入符合独立区间模型。共享 Budget 摘要零预算不推进、源码上限 64 字节；百万字表头折叠后追加正文，布局仅回调一个占位，正文及计费字节均为 0。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 423 项、`cargo test --doc --workspace` 36 项、`cargo fmt --all -- --check` 和 `git diff --check`。`cargo run -p mdstream-core -r --example late_headers` 保持首链接身份，摘要扫描 1000049 字节、16 帧/110 个计费字节；原 `long_headers`、`long_delimiters` 示例分别保持 15 帧/100 字节、13 帧/72 字节。完整日志：`/tmp/mdstream-late-headers-validation.log`。

README、TODO 只将本子项记为已交付。已有段落或同一容器续行中的晚管道仍保留待办：例如 `*prior\nend*…|b` 在首管道出现前已发布跨行强调，现有单调样式和稳定对象协议不能直接撤销；不得用延迟全部普通长行来规避即时消费约束。其它分类长度上限、完整容器规则及终端 UI 不属于本次实现。

## 2026-10-30 只继承换行的晚管道长表头核心交付

本单元按账本允许的独立子项交付：顶层段落或同一 Quote/Item 显式续行，在前序状态只剩待确认软/硬换行时，支持首管道晚于分类窗口的长表头。已闭合样式、链接、代码、数学和脚注不阻止探测。保留早管道/短表头、分类优先级、确切容器路径及 lazy/代码等边界；原段落与同容器续行中未闭合的跨行候选仍明确保留 TODO。完整方言和流式显示时机见 [README](README.md#表格任务列表与图片)。

模型验证：一般跨行候选可在首管道前闭合并改变旧行的样式、代码显示映射或链接对象；现有单调注解和稳定对象协议不能直接撤销这些效果。因此本子项在当前行内消费前做 O(1) 状态检查，不遍历 delimiter 栈，只隔离已经证明可分离的前序换行。候选保存该换行的源码区间、待隐藏尾空白/反斜线和软硬类型；首单元格从独立状态即时消费，首管道之后仍延迟。前序换行及尾标记在判定前按原文显示；成功时丢弃换行，失败/EOF 或无管道行结束时提交一次原效果。正文样式、实体和对象在管道前即时发布，ID 不重建、不重复发布，未闭合跨行候选继续普通流式解析。

顶层即时续行先延伸原段落范围，并保存原端点；确认时恢复端点、新建表格块，失败/无管道保持普通段落。容器物理叶块保留稳定 BlockId。换行恢复、块归属变化和表格确认复用原有范围失效与 Update，拒绝相关旧缓存和测量；不回访旧块/引用/祖先。候选只增加固定大小换行描述和可选段落端点，不复制正文、不克隆 Inline、不建立候选单元格数组。每个正文摘要字节一次、每个行内字节最多一次，表格确认额外扫描一次单元格边界；维持累计 O(n log n)、空间 O(n)，不承诺单次确认/回退的硬预算。

新增 14 项测试：`crates/mdstream-core/tests/inherited_headers.rs` 13 项与 `crates/mdstream-core/tests/chunking.rs` 1 项。全延迟早管道路径对照当前行和前序行的样式、显示文本及对象；涵盖软/硬换行、CRLF、EOF、Setext、作用域变化、失败分隔行、即时普通正文、对象身份、根段落范围恢复、合并 Update、旧测量与折叠。UTF-8 全切点和逐前缀快照一致。既有 `long_delimiters` 的前序普通段落拒绝断言改为接受，并保留未闭合强调拒绝与其它分类限制；原跨行样式、代码、链接和数学的行为继续单独回归。

复杂度与资源验收：千层引用下，首管道前百万字追加只消费新字节并更新摘要，首管道后百万字只更新摘要；分类、容器/标题步骤及注解/容器 arena 容量不增长。1000 条普通长续行保持单个根段落；4096 个旧 opener 后的 100 条长续行不增加 delimiter 步骤，EOF 只清理一次。250 层列表、4096 个链接/数学/脚注与 4096 列不重复符号工作；400 组成功/失败输入对照独立范围模型。前序百万字尾空白回退用一次范围隐藏，新增树访问少于 1000、节点少于 200，并使远处旧 tile 测量失效。共享预算摘要零预算不推进，容器与表格折叠后的布局只有一个占位回调，正文及计费字节均为 0。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 437 项、`cargo test --doc --workspace` 37 项、`cargo fmt --all -- --check` 和 `git diff --check`。`cargo run -p mdstream-core -r --example inherited_headers` 验证百万字失败回退与成功预览，后者 16 帧/110 个计费字节；既有 `late_headers`、`long_headers`、`long_delimiters`、`container_inline`、`container_lazy` 示例均通过。完整日志：`/tmp/mdstream-inherited-validation.log`。

README、TODO 已同步，只将“只继承换行”的子项记为已交付。继承未闭合跨行候选的晚管道表头、其它分类上限、完整容器语义和终端 UI 仍保留待办，不声称整个继承状态方向完成。

## 2026-10-30 旧强调尚未闭合的晚管道长表头核心交付

本单元按账本允许的独立子项交付：顶层段落或同一 Quote/Item 显式续行继承未闭合 `*`、`**`、`_`、`__`、`~~` opener 时，只要旧 opener 尚未在首个未转义管道前闭合，就支持晚管道长表头。保留分类窗口、确切容器路径、短行/早管道、lazy 和块中断边界。代码/数学/链接等继承候选，以及已发布跨行强调的撤销仍在 TODO；没有宣称整个未闭合跨行候选方向完成。

模型与实现：未闭合强调只保存候选、尚不影响当前字节；当前行内部配对及对象可即时发布。分类时 O(1) 保存栈边界、隔离前序待确认换行；正常消费中记录是否真正闭合过边界以前的 opener，不能只比较栈长，因为弹栈后可能重新增长。若发生旧配对，当前行一次切回普通流式解析，提交原换行并消费尚未处理的后缀，不再启动表头候选。若确认表格，一次移除继承栈前缀并重建当前行 opener 前驱下标，将剩余 Inline 移入首单元格；首管道前待定的 delimiter run 按单元格末端结算。失败/EOF/无管道只取消探测记录、恢复原换行并继续原栈。确认丢弃的是未发布的候选，不撤销已发布样式，不重放或重建对象；LinkId、MathId、脚注引用和首次编号保持。

每个行内字节最多消费一次，每个合格摘要字节一次；表格确认仍有一次单元格边界扫描。成功删除/重建 opener 计入 `delimiter_steps`，每个 opener 仅在所属序列成为单元格时处理一次；失败及无管道续行不遍历历史栈。只增加固定大小栈边界/闭合事件字段，不克隆栈、不复制源码，维持累计 O(n log n)、空间 O(n)。确认和回退不承诺单次硬预算；显示、预览、折叠继续共用已有 Budget、Update 和范围失效协议。

新增 12 项测试：`crates/mdstream-core/tests/open_headers.rs` 11 项与 `crates/mdstream-core/tests/chunking.rs` 1 项。五种 delimiter 和局部代码/数学/链接状态对照既有全延迟早管道路径；失败/EOF 对照普通正文。512 组局部配对/交叉/转义输入对照独立路径，400 组成功/失败/取消探测输入对照表格范围模型。UTF-8 全切点及逐标量中间快照涵盖 CRLF、待定 run、栈重新增长、无管道、长分隔行失败与容器作用域失配。另覆盖即时对象、稳定 ID/增量范围、原块范围、旧测量拒绝、合并 Update、容器/表格折叠与共享预算。既有长分隔行测试将旧强调未闭合的拒绝断言改为接受，并保留继承代码拒绝；原跨行强调闭合和 opaque 候选回归继续通过。

复杂度与资源验收：千层引用下百万字增长只增加新字节摘要/行内计数，分类、delimiter、容器/标题步骤及注解/容器 arena 容量不变；确认/失败不重读首单元格语义。4096 个旧 opener 经 200 轮长候选失败/无管道续行，delimiter 计数不增加，最终确认只清理一次 4096 项。250 层列表、4096 个链接/数学/脚注及 4096 列保持确切作用域，确认没有额外符号工作。预算摘要零预算不推进，8 字节逐帧读取能够完成。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 449 项、`cargo test --doc --workspace` 38 项、`cargo fmt --all -- --check` 和 `git diff --check`。`cargo run -p mdstream-core -r --example open_headers` 演示百万字失败恢复和成功表头，摘要 16 帧/110 个计费字节；既有 `inherited_headers`、`late_headers`、`long_headers`、`long_delimiters`、`container_inline`、`container_lazy` 示例均通过。完整日志：`/tmp/mdstream-open-headers-validation.log`。README、TODO 已同步，零运行时依赖与 `forbid(unsafe_code)` 保持；其余继承候选和跨行语义撤销、其它分类上限、完整容器语义与终端 UI 继续保留待办。

## 2026-10-30 旧代码尚未闭合的晚管道长表头核心交付

本单元交付独立子项：顶层段落或同一 Quote/Item 显式续行继承未闭合代码 span 时，支持首管道晚于分类窗口的表头。允许保留旧强调栈；不支持代码外仍有未闭合标签等其它候选。旧代码若在首管道前已经闭合，保留原跨行代码样式和显示映射并取消该行探测，即使随后重新开启代码也不恢复。早管道、短表头、lazy、块中断和确切容器作用域规则保持；已发布跨行语义的撤销及其它候选仍在 TODO。

模型与实现：代码状态会抑制实体、链接、数学和脚注语义，因此它的前缀不能直接作为独立首单元格复用。探测期间只维护原代码的 run、全空白标志与最后内容区间；首管道之后仍延迟。确认时原代码尚未闭合，当前行没有已发布的行内效果，可一次丢弃旧 Inline（计入旧强调栈清理），由既有 RowParser 扫描当前行并解析各单元格。丢弃时不结算首管道前待定的反引号 run，避免发布需要撤销的旧代码映射。旧对象 ID 保持，候选中的对象只在确认后分配一次，独立单元格显示映射按原规则建立。失败、无管道、作用域变化及 EOF 保留原代码状态，仅消费剩余后缀；之后闭合代码仍只查询端点，不回扫正文。

复杂度证明：每个候选摘要字节一次、原代码状态消费一次；成功时在已有单元格扫描阶段额外消费当前候选行。成功表头互不重叠，因此这部分总量受 `table_bytes` 约束，`inline_bytes <= source().len() + table_bytes`，两个阶段的实际字节均如实计数。失败不重放，普通增长不回扫，确认不读前序代码正文、不重试分类。只增加一个布尔标志，不复制正文或克隆栈；累计解析仍 O(n log n)、核心空间 O(n)，单次确认/回退仍无硬时延保证。有预算显示、摘要与折叠接口保持原契约。

新增 13 项测试：`crates/mdstream-core/tests/code_headers.rs` 12 项与 `crates/mdstream-core/tests/chunking.rs` 1 项。对照全延迟早管道模型与普通正文模型，覆盖 363 组反引号组合、400 组表格范围、UTF-8 全切点/逐前缀、CRLF/EOF/Setext/长分隔行失败、首管道前待定 run、闭合后重新开启、稳定对象/叶块身份、显示映射的逐标量/tile/预算读取、合并 Update、旧测量拒绝和折叠。既有 `long_delimiters` 将本次支持的拒绝断言改为接受，并保留已经闭合的跨行代码及其它分类上限断言。

资源与复杂度验收：千层引用中保留百万字前序代码，再分 1000 次追加百万字候选，仅增加新字节的摘要/行内计数；分类、delimiter、容器/标题计数与注解/容器 arena 容量不变。成功只重新消费当前候选行；失败只消费 10 个剩余字节。4096 个旧强调 opener 经 200 轮失败/无管道续行不增加 delimiter 步骤，最终代码映射与普通模型一致；250 层列表、4096 个旧 opener、4096 组链接/数学/脚注及 4096 列在确认时各处理一次。零预算摘要不推进，8 字节分帧读取可完成。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 462 项、`cargo test --doc --workspace` 39 项、`cargo fmt --all -- --check` 和 `git diff --check`。新示例 `cargo run -p mdstream-core -r --example code_headers` 验证百万字失败恢复与成功表头，预算摘要 15 帧/101 个计费字节；既有 `open_headers`、`inherited_headers`、`late_headers`、`long_headers`、`long_delimiters`、`container_inline`、`container_lazy` 示例均通过。日志：`/tmp/mdstream-code-headers-validation.log`、`/tmp/mdstream-code-headers-release.log`。README、TODO 已同步，零运行时依赖与 `forbid(unsafe_code)` 保持。

## 2026-10-30 旧数学尚未闭合的晚管道长表头核心交付

本单元交付顶层段落继承未闭合 `$` / `$$` 候选时的晚管道长表头，可伴随旧强调栈；继承未清除链接标签等其它状态仍不参与。首管道前若旧数学已经闭合，保留其稳定 MathId、原始区间和即时样式，取消当前行探测，即使新候选随后开启也不恢复。容器数学仍在物理行末终止，其去前缀跨行内容表示继续留待后续。完整方言与示例见 [README](README.md#表格任务列表与图片)。

模型依据：旧数学尚未闭合时只更新美元串及转义状态，正文保持 opaque，不发布当前行内的数学、链接、脚注或强调。因此可沿用代码候选的固定大小探测标志：首管道前继续即时消费原候选，从首管道起延迟后缀；成功丢弃原 Inline 及待定美元串，不结算跨行对象，再由已有 RowParser 独立消费当前表头并分配对象一次。失败/无管道/EOF 继续原状态及未消费后缀；块中断沿用既有规则。旧对象身份及首次脚注编号不变，无对象撤销、正文复制、栈克隆或历史索引重建。

每个合格正文摘要字节一次，失败时每个行内字节一次；成功仅额外解析当前表头，其源码范围互不重叠，全部计入 `inline_bytes`，实际满足 `inline_bytes <= source().len() + table_bytes`。不读前序数学正文，候选额外状态 O(1)，总空间 O(n)、累计解析 O(n log n)；确认、回退和 EOF 保持摊还契约，不承诺单次硬时延。共享预算对象/内容游标、显示摘要及折叠沿用既有接口。

新增 13 项验收：`crates/mdstream-core/tests/math_headers.rs` 12 项与 `crates/mdstream-core/tests/chunking.rs` 1 项。先在基线复现未闭合数学拒绝表头，再验证实现通过；基线日志 `/tmp/mdstream-math-headers-baseline.log`。确认语义对照全延迟早管道路径，失败对照无管道普通正文；338 组美元串/转义输入对照独立闭合模型，400 组成功/失败/取消探测输入对照表格范围模型。UTF-8 全切点及逐标量快照覆盖 CRLF、待定美元串、已闭合后重开、失败长分隔行、EOF、Setext 和容器边界。另覆盖原段落范围恢复、增量对象范围、稳定 MathId/LinkId/脚注编号、旧测量拒绝、合并 Update 与表格折叠。

复杂度与资源验收：前序百万字数学正文后的当前表头再增长百万字，只增加新增字节的行内/摘要计数；分类、数学/delimiter、容器/标题步骤及注解/容器 arena 容量不变。成功只额外消费当前表头，失败不重读；4096 个旧 opener 经 200 轮失败/无管道长行不增加数学或 delimiter 步骤，最终仅清理一次。4096 个延迟链接/数学/脚注和 4096 列仅在确认时发布，数学窗口与 UTF-8 内容读取共用预算且零预算不推进；摘要每帧最多 8 字节。千层引用和 250 层列表验证原数学物理行边界及稳定容器内表格语义。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 475 项、`cargo test --doc --workspace` 40 项、`cargo fmt --all -- --check` 与 `git diff --check`。`cargo run -p mdstream-core -r --example math_headers` 验证百万字成功/失败与稳定 MathId，预览 19 帧/127 个计费字节；既有 `code_headers`、`math`、`container_inline` 示例通过。完整日志：`/tmp/mdstream-math-headers-validation.log`。

README、TODO 已同步，只标记本独立子项；继承链接等其它候选、已发布跨行语义撤销、容器数学跨行表示、其余分类上限和终端 UI 继续保留待办。零运行时依赖与禁止 unsafe 的约束保持。

## 2026-10-30 旧链接或图片标签尚未闭合的晚管道长表头核心交付

本单元交付顶层段落继承未闭合 `[...` / `![...` 标签时的晚管道长表头，支持与旧强调、代码 span、数学候选组合。确认时旧标签不得进入首单元格；失败、无管道、EOF 和块中断沿用原候选及原块规则。容器标签仍在物理行末终止，其去前缀跨行内容表示、多行目标/title 和已闭合跨行语义撤销继续保留 TODO。方言及可运行片段见 [README](README.md#表格任务列表与图片)。

模型依据：标签闭合前不发布依赖该左括号的链接对象，本行强调、实体、代码、数学及替换标签的对象可即时发布。因此只增加一个旧左括号源码偏移；成功时清除仍与该位置相同的候选，并复用当前首单元格状态与全部已发布对象。新 `[` 替换、`<` 丢弃旧候选沿用原方言，当前行新标签不被清除；旧图片的 `!` 不影响新标签或后续单元格。若首管道前未转义 `]` 关闭旧标签，则永久取消本行探测，即使标签已超过 999 字节且尚无 LinkId，也不能让其后潜在的直接目标跨越表格边界。关闭后重新开启标签不恢复探测。

旧强调/代码/数学的闭合事件继续取消探测；继承代码/数学仍 opaque 的成功路径沿用一次当前表头独立解析，其余标签路径直接复用已消费前缀。失败只消费延迟后缀并一次提交前序换行；不复制正文、克隆栈、回扫标签或撤销对象。新增状态 O(1)，普通标签路径实测 `inline_bytes <= source().len()`；叠加 opaque 候选时沿用 `inline_bytes <= source().len() + table_bytes`。累计 O(n log n)、空间 O(n)，单次确认/失败仍是摊还契约，不承诺硬时延。

新增 13 项验收：`crates/mdstream-core/tests/link_headers.rs` 12 项与 `crates/mdstream-core/tests/chunking.rs` 1 项。基线先复现未闭合标签阻止表头确认（`/tmp/mdstream-link-headers-baseline.log`）。确认对照早管道独立单元格路径，失败对照无管道普通正文；200 组标签替换/转义组合对照独立闭合模型，400 组成功/取消/长分隔行失败对照源码范围模型。UTF-8 全切点及逐标量中间快照覆盖 CRLF、硬换行、旧标签闭合后重开、当前行标签、代码/数学组合、长分隔行失败、Setext、EOF 和容器边界。原数学测试中“旧标签阻止隔离”的边界更新为本次支持的组合验收。

复杂度与资源验收：百万字旧图片标签后的当前行再分批增长百万字，仅增加新增字节的行内/表头摘要计数；分类、标签规范化、delimiter、容器/标题步骤和注解/容器 arena 容量均不变。确认及失败都不重读这段前缀。4096 个旧 opener 经 200 轮失败/无管道续行不增加栈或标签规范化步骤，最终只清理一次；4096 个即时链接/数学/脚注及 4096 列在确认时不重复发布，旧对象身份不变。千层引用与 250 层列表保持标签物理行边界及表格作用域；共享预算摘要零预算不推进、每帧最多 8 字节。旧测量拒绝、TileKey 失效、合并 Update 和两个独立折叠视图一致性通过。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 488 项、`cargo test --doc --workspace` 41 项、`cargo fmt --all -- --check` 与 `git diff --check`。`cargo run -p mdstream-core -r --example link_headers` 验证百万字成功/失败及稳定 LinkId，预算预览 16 帧/107 个计费字节；既有 `math_headers`、`code_headers`、`container_inline` 示例通过。完整日志：`/tmp/mdstream-link-headers-validation.log`。

README、TODO 已同步，只将本独立子项标记完成；已闭合跨行语义撤销、容器跨行标签表示、多行目标/title、其余分类上限和终端 UI 保留待办。零运行时依赖与禁止 unsafe 的约束保持。

## 2026-10-30 容器跨行链接与图片标签核心交付

交付同一确切容器中跨物理行的链接/图片显示标签，支持显式、等价前缀和 lazy 正文续接，接入直接、shortcut、collapsed 和 full 引用形式。`LinkInfo.label` 保持绝对源码包络，现有显示片段与共享预算游标去除隐藏前缀，保留实体/代码替换映射和 LinkId；无需复制整段 label 或增加逐行映射数组。具体方言和示例见 [README](README.md#容器跨行链接与图片标签)。目标、title、full 引用后缀继续沿用单物理行规则；数学跨行去前缀表示及更完整块语义撤销仍在 TODO。

实现依据：行内解析器已经只消费匹配容器后的正文，前缀已由注解隐藏。为引用匹配复用一个最多 1000 字节的原始正文缓冲（999 字节上限及一个潜在闭括号），超过上限后停止存储；关闭时只规范化有界正文，collapsed 后缀直接复用 SymbolId。直接链接/图片的显示标签不受此上限影响。候选被替换/终止不回放，作用域和块边界沿用现有序列规则；表头确认隔离旧标签，失败续接，旧标签在首管道前已闭合则取消探测。源码范围及物理叶块身份保持，暂定引用补全目标只更新同一对象，晚定义仍不遍历旧引用。

新增 15 项验收：`crates/mdstream-core/tests/container_links.rs` 13 项、全切点/逐前缀快照 1 项、有界缓冲容量测试 1 项；扩充既有全切点和固定种子逐前缀样本。400 组输入对照无前缀解析和独立源码偏移模型；覆盖 Unicode/CRLF、实体/代码、转义/替换候选、边界/EOF/Setext、长表头确认/失败、引用长度上限、延迟定义及稳定对象通知。两个原“容器标签限单行”回归更新为跨行对象及闭合后取消表头探测，数学与脚注物理行边界继续验证。

复杂度与资源验收：千层引用、250 层列表中百万字标签后的 1000 次逐字追加只增加 1000 个 inline bytes，不增加分类/标签规范化/祖先步骤或注解 arena 分配；闭合只消费新增 9 字节并做范围标记，少于 2000 次树访问。候选缓冲在百万字增长时容量不变，后续短标签复用容量。千层引用中 4096 条 lazy 短续行每行一次失配、无祖先扫描；两万条跨行引用的晚定义只规范化 3 字节、少于 1000 次树访问，不生成新链接。累计 O(n log n)、空间 O(n)，零运行时依赖且禁止 unsafe。

共享预算验收覆盖逐标量/逐 tile/每步六字节显示映射、零预算原子暂停、显示搜索跨前缀命中及原始源码搜索；晚闭合拒绝旧游标/缓存/测量、清理相应检查点并保留折叠意愿，合并 Update 与重建一致。百万字图片标签收起时布局零正文/对象回调、零字节计费；分帧导航只展开目标路径，不读正文。

项目 devShell 验收：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 503 项、`cargo test --doc --workspace` 42 项、`cargo fmt --all -- --check` 与 `git diff --check` 通过。`cargo run -p mdstream-core -r --example container_links` 验证晚定义、稳定图片及容器折叠，标签读取 4 帧/61 计费字节；既有 `container_inline`、`link_headers`、`container_lazy` 示例通过。验收日志：`/tmp/mdstream-container-links-validation.log`。

README、TODO 已同步，仅将容器跨行标签子项标记完成；数学内容表示、多行目标/title/full 引用后缀、更完整边界撤销、其它容器规则及终端 UI 继续保留后续。

## 2026-10-30 容器跨行数学内容核心交付

交付同一确切 Quote/Item 中跨物理行的 `$` / `$$` 候选，支持显式、等价前缀和 lazy 正文续接。定界串长度、逐字节转义、opaque 内容、失败不回放及块/作用域边界沿用既有方言；空行、兄弟/内外容器切换、标题、代码、定义和表格单元格结束候选。单行 Setext 晚提升保留已确认对象；旧数学在首管道前闭合则取消长表头探测，未闭合则确认时独立消费当前单元格，失败继续原候选。具体规则及可运行片段见 [README](README.md#容器跨行数学内容)。

新增 `Document::math_payload(id)`，复用固定大小 `MathContent` 游标返回去前缀正文和绝对源码映射；`math_content` 保持完整原文。`Math.source/content` 仍为含前缀的稳定源码包络，MathId 仅在闭合时发布。每个 `MathChunk.source` 连续覆盖原文，text 借用对应源码，前缀片段的 text 为空；保留原始 CRLF、首尾空白、实体拼写及所有反斜线，包括表格中显示层隐藏的转义管道反斜线。无需把整段公式复制或按通用 HIDDEN 样式过滤。

模型与复杂度：候选仅增加上次 feed 的末端；跨物理行的不连续 feed 追加一条有序前缀间隙，同物理行的 ATX 尾标记不入索引。每个间隙记录一次，总记录成本 O(G)、存储 O(G)；失败/被表头丢弃的候选允许留下无引用记录，但不与后续对象重叠。正文仍单遍消费，确认只做常数次 O(log n) 范围标记，不扫描旧行、祖先或间隙列表。游标 O(1) 构造，第一次成功推进扣费后通过 O(log G) 二分定位，随后逐片 O(1)。全库累计解析 O(n log n)、空间 O(n)，零运行时依赖且禁止 unsafe；单次 append、闭合和存储扩容继续为摊还保证。

新增 14 项验收：`crates/mdstream-core/tests/container_math.rs` 11 项、`crates/mdstream-core/tests/chunking.rs` 全切点/逐前缀 1 项、内部间隙存储测试 2 项；既有分块及固定种子快照纳入预算读取的 payload 映射。基线先复现容器 LF 丢弃数学候选，日志 `/tmp/mdstream-container-math-baseline.log`。400 组输入对照无前缀数学解析与独立源码偏移模型，覆盖 CRLF/Unicode、精确/失配/转义美元串、代码/链接组合、边界/失败/EOF、Setext、表头确认/取消/长分隔行失败、稳定对象与物理叶块。三个旧“数学限单物理行”断言更新为新对象与闭合取消表头探测的验收。

资源验收：千层引用和 250 层列表中，百万字正文后 1000 次逐字追加仅增加 1000 个 inline bytes，分类、math steps、容器步骤和注解 arena 分配不变；闭合仅消费新增 7 字节，树访问少于 2000 次，前缀匹配至多 1000 步。间隙索引在百万字同物理行增长和 100 条 lazy 行中容量/条目数不变，显式续行才新增记录。千层引用下 4096 条 lazy 续行每行一次失配；4096 个历史对象后新对象只消费 11 字节和 2 次定界步骤，4096 个失败候选不增加后续读取步数。

预算与消费验收：每步至多 TILE_BYTES 源码字节、1 query/1 object，隐藏前缀也按实际源码长度计费；四字节可推进任意标量，预算不足时位置与预算不变，完成后免费。测试覆盖元数据/内容共享预算、空前缀片段、原始 CRLF/转义保留、旧缓存/测量/检查点/搜索拒绝及合并 Update 重建。晚闭合保留折叠意愿，收起百万字数学正文的布局零正文回调、零字节计费。

项目 devShell 验收：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 517 项、`cargo test --doc --workspace` 43 项、`cargo fmt --all -- --check` 和 `git diff --check` 通过。`cargo run -p mdstream-core -r --example container_math` 验证流式对象、稳定叶块、容器折叠与逐帧映射，读取 4 帧/30 源码字节；既有 `math`、`math_headers`、`container_inline`、`container_links` 示例通过。完整日志：`/tmp/mdstream-container-math-validation.log`。

README、TODO 已同步，仅将容器跨行数学内容子项标记完成；数学块、额外定界方言、TeX 排版与实际对象回调、更完整块语义撤销、其余容器规则及终端 UI 继续保留待办。

## 2026-10-30 容器内多物理行 Setext 核心交付

- [x] 同一确切 Quote/Item 中的显式、等价前缀及 lazy 连续正文由显式 Setext 下划线迟确认；空行、兄弟、实际作用域切换和已识别块中断终止序列。失败表头回到同一序列，确认表格清除候选；非法/超长下划线保持既有分类行为，换行/EOF 完成合法下划线后才发布标题。
- [x] 首叶块保留 BlockId 并成为标题 opener，末叶块吸收下划线，中间物理叶块完全不改写；Heading.source/content 保存整段原始源码包络，显示读取去除中间前缀和 CR。已发布跨行强调、代码映射、链接/图片/数学对象及 ID 不撤销、不回放，未闭合状态只在边界收尾。
- [x] changed_blocks 保守覆盖首叶块到末端，确认只发布一次整段结构失效和一个 new_headings；章节边界回退、稳定导航、折叠候选/默认/预览、容器组合、合并 Update、旧缓存/测量/读取任务失效复用既有机制。多行标题排版以 Heading.source 为准，不能假定它等于首叶块 source。

解析继续使用一份固定大小的首块与内容区间，无正文副本或每行候选数组。新增字节单遍消费，确认只修改首尾块及范围索引，不按旧正文长度、叶块数或历史标题数遍历。行内未闭合状态收尾仍按 opener 生命周期计费，全库保持累计 O(n log n)、空间 O(n)，无单次调用硬时延承诺。

验收：新增 13 项测试（12 项专用测试及全局分块快照），覆盖 UTF-8 全切点/逐前缀、CRLF/EOF、任务/兄弟/嵌套/块边界、长表头确认及失败/取消、长分隔行回退、对象/叶块稳定身份、预算暂停、缓存/测量/任务及合并更新。400 组前缀自由显示与独立章节/折叠范围模型通过；千层引用、250 层列表的百万字续行只消费新增字节，确认 inline_bytes 不增长；4096 个连续 lazy 叶块中间块保持原值，4096 个历史标题后的确认仅新增 2 个 heading_steps、1 次默认初始化和至多 8 次折叠范围检查。

项目 devShell 的 check、clippy（-D warnings）、debug/release nextest 各 530 项、44 项文档测试、fmt 和 diff 检查通过。新增 `container_setext` 示例用 16 帧读取完整两行标题的 32 源码字节，跳过 120 万字节折叠正文；另有 container_headings、container_sections、container_inline、container_lazy、container_math 五个回归示例通过。日志：`/tmp/mdstream-setext-validation.log`（check/clippy、两种 nextest 通过；首次新增文档示例的 Option 写法错误已修正）、`/tmp/mdstream-setext-doc-examples-validation.log`（修正后 44 项文档测试及六个示例、fmt/diff 均通过）。

此项交付既有容器方言中的多行 Setext；下划线含前缀的 1 KiB 分类上限、更完整块语义撤销、tight/loose 余项、tab 缩进及终端排版继续保留 TODO。

## 2026-10-30 容器类型与任务完成状态策略交付

- `ContainerFoldFilter` 以 Quote/List/全部 Item/已完成任务/未完成任务五个字段作并集。普通条目不匹配任务字段；`items` 包括任务，List 不聚合子项状态。`FoldDefaults::containers` 与既有深度阈值任一命中即初始化新节点为收起；只检查 `new_containers` 的已存元数据一次，含晚识别、EOF 及合并更新，无历史/祖先回扫。
- `FoldView::task_checked` / `set_task_checked` 提供独立视图内的有效完成状态和稀疏覆盖，只保存与源 marker 不同的值。修改不写 Document/样式、不改折叠意愿或预览，设回源码状态移除覆盖；普通追加、finish 和 reset 保留覆盖与显式意愿。实际变化返回 marker 区间和新版本，惰性失效相关测量/检查点以及旧批量/导航/排版任务；无操作保持版本。源码读取仍表示原文，宿主绘制任务控件时查询有效状态。
- `FoldBatchAction::Containers` 仅遍历容器 arena，以有效状态筛选；包括隐藏祖先下的节点，逐容器扣 1 query / 1 object，不匹配和空筛选也计费，不收初始化费，不读取正文/叶块/表格行。预算不足原子暂停，完成后重读免费，渐进修改可立即排版，取消或失效保留已完成部分。非命中容器、标题/代码/整表/注册行范围及未来默认设置保持意愿；预览节点的保存意愿与原单点/批量规则一致。
- 复杂度：新增 N 个容器、匹配 K 个的默认成本沿用 O(N + K(log H + log C + log F + log T))；普通正文追加只刷新共享尾部。批量任务 O(1) 状态、总访问 C 个容器，每步固定次数元数据/索引操作，含 O(log U) 有效状态查询及既有折叠索引操作。U 个覆盖占 O(U)，单点覆盖 O(log U + log T)，无全祖先/子块/tile 扫描。默认 apply 仍是事件摊还契约，不增加任意大 Update 的单帧硬预算承诺。零运行时依赖、禁止 unsafe 保持。
- 新增 12 项验收：32 种筛选 × 4 种深度组合对照；UTF-8 全切点/逐前缀、CRLF/无效及未完成 marker/EOF/合并更新；旧节点及显式展开优先、两视图隔离、状态/预览/导航组合、非容器和行范围保留、预算原子暂停与共享、身份/版本拒绝、旧测量及检查点失效。320 轮流式默认切换/覆盖/批量/预览/reset 与独立区间/高度模型一致。
- 千层容器与百万字正文后的 1000 次逐字追加无默认重查、无容器索引写入或祖先解析增长；筛选 1002 个不匹配容器按预算推进，隐藏正文布局计费 0 bytes。4096 个历史任务后新条目只初始化一次；4096 个覆盖恢复原状态后存储清空，单节点再切换 1000 次不累积历史或高度索引节点。
- 项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo nextest run` 与 `cargo nextest run --workspace --release` 各 **542** 项、`cargo test --doc --workspace` **45** 项、`cargo fmt --all -- --check`。`cargo run -p mdstream-core -r --example container_policies` 输出 **3 帧 / 3 容器 / 0 源码字节**，导航 **4 帧**；既有 `container_defaults` 和 `fold_batch` 示例通过。日志：`/tmp/mdstream-container-policy-validation.log`。

本单元交付默认策略、视图内状态及有预算筛选核心；任务控件绘制、任务聚合统计、实际摘要/容器排版、手工交叉范围和终端 UI 继续留待各自阶段，不将总体折叠规划标记完成。

## 2026-10-30 独立数学块核心交付

- [x] 独占物理行 `$$` 开闭的数学块，至多三个前导空格和可选 space/tab，支持 LF/CRLF/EOF。顶层和显式 Quote/Item 使用确切作用域，正文保持字面；缺失前缀、兄弟/外层切换和 EOF 封存未闭合块，不回放。独立数学块允许空内容，未完成或超长定界候选沿用有界分类规则。具体方言见 [README](README.md#独立数学块)。
- [x] 稳定 `BlockId` 与 `BlockKind::Math { content, closed }` 保存增长中的内容包络和显式闭合标志，`Block::state` 区分 Growing/Sealed。不增加独立 arena，不混入只含闭合行内对象的 MathId 集合；复用 `new_blocks` / `changed_blocks`、合并通知及旧测量失效。正文增长和封存只做块范围失效，不遍历旧 tile。
- [x] `math_block_content` / `math_block_payload` 复用 MathContent 游标，借用原文或省略匹配的容器前缀，保留 CRLF、反斜线和实体拼写。MathChunk.source 连续覆盖内容原文，前缀返回空 text 且按源码收费。O(1) 构造，首次有效读取付费后 O(log G) 定位，之后每步 O(1)，最多 TILE_BYTES 字节及 1 query/1 object；UTF-8 预算不足原子暂停，不复制完整正文。
- [x] `FoldId::Math` / `FoldDefaults::math` 接入独立视图、章节/容器父关系、预算发现、批量、预览和源码导航；默认只初始化新节点，显式选择保留。扩展已有固定末端刷新机制，数学块当前 EOF 封闭后继续追加会刷新末 tile 遮蔽，避免隐藏相邻新文字。通用显示为 MATH 样式字面回退，实际 TeX 排版和对象回调继续留待后续。

新增 14 项验收：12 项专用测试、1 项 UTF-8 全切点/逐前缀快照、1 项内部间隙/容量测试；既有短样本和固定种子分块快照同时覆盖数学块 payload 映射。独立模型验证 400 组源码/内容/容器范围和 240 次流式折叠区间/高度；覆盖 Unicode/CRLF、空块、失败及超长开闭候选、代码/表格/行内数学优先级、容器边界/EOF、稳定身份与通知合并、旧测量拒绝、共享预算、视图隔离和批量/预览/导航。四个既有折叠模型的穷尽匹配补充 Math 分支，原有断言保留。

复杂度与资源验收：千层引用中百万字同物理行增长不增加分类/行内消费/容器/章节步骤，容器与章节折叠索引无额外访问；关闭少于 2000 次注解树访问。250 层列表续行按输入前缀线性匹配。4096 个封闭历史块后追加只处理新块和当前边界，不遍历旧折叠；百万字后 1000 次逐字追加不增加数学间隙记录或块 arena 容量，行内对象与独立块共享有序间隙索引。收起百万字数学块的布局零正文回调、零字节计费。累计解析 O(n log n)、空间 O(n)，零运行时依赖且禁止 unsafe，不承诺单次 append/finish 的硬时延。

项目 devShell 验收：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run` 各 556 项、`cargo test --doc --workspace` 46 项、`cargo fmt --all -- --check` 和 `git diff --check` 通过。`cargo run -p mdstream-core -r --example math_blocks` 读取 5 帧/34 源码字节、导航 4 帧；既有 `math`、`container_math`、`container_code` 示例通过。完整日志：`/tmp/mdstream-math-block-validation.log`；流式高度模型捕获并修复当前 EOF 末 tile 遮蔽，失败证据 `/tmp/mdstream-math-block-model.log`。

README、TODO 已同步，只标记独立数学块核心完成；额外数学定界方言、TeX 解析/排版与实际对象回调、分类上限扩展、其余容器/脚注语义和终端 UI 继续保留待办。

## 2026-10-30 脚注多行多段定义核心交付

完成顶层及显式 Quote/Item 内脚注定义的四空格续行、空行分段和有预算源码映射读取。起始行沿用 `[^label]: body` 方言；续行显式匹配同一容器路径，再消费恰好四个 ASCII 空格，空白行可省略额外四格。tab 不替代续行缩进，缺前缀、兄弟条目、非续行或 EOF 封口；内部标记和定义形状保持字面内容。保留文档全局首定义优先、首次引用编号和稳定对象身份，重复定义的全部续行隐藏但不覆盖原符号。定义和唯一叶块在 LF 后仍为 Growing，边界或 EOF 才 Sealed，未分类短行不提前加入正文。末尾空行直接归入定义，不回撤已发布范围；内部空行不使外层列表 loose，真正分隔其它子块/兄弟的末尾空行以一个待定标记延续原有间距规则。

`FootnoteDefinition.content` 是排除最后一个 LF/CRLF 的原始源码包络，保留内部换行、缩进和容器前缀。新增 `Document::footnote_payload`，与原文 `footnote_content` 共用 `FootnoteContent` / `FootnoteChunk`；去掉容器前缀和续行四格，保留实体、反斜线和其它空白。每个片段提供完整源码映射，省略前缀时返回空 text，仍扣实际源码 bytes、1 query、1 object。构造 O(1)，首次成功读取 O(log G) 定位有序间隙，其后每步 O(1)，最大 TILE_BYTES，预算不足不推进或扣费，四字节可前进任意 UTF-8 标量。

新增 17 项验收：`crates/mdstream-core/tests/footnote_blocks.rs` 15 项、全量分块快照 1 项和内部存储 1 项。覆盖起始缩进/续行、空正文、多段/末尾空行、CRLF/裸 CR/EOF、未完成输入、非法与超长候选、确切容器路径、字面标记、代码/数学/表格优先级、首定义/重复定义、稳定对象和块身份、增量通知重建/合并、旧缓存/测量/搜索拒绝、共享预算和折叠意愿；分块快照新增去前缀映射及短样本，UTF-8 全切点和逐前缀均一致。原有单行测试仅更新因 LF 后不再封口而改变的状态、事件和计数，原有列表间距回归保留。

复杂度与资源验收：千层引用中百万字续行后的 1000 次追加，分类/行内/容器/章节步骤及折叠索引访问不增加，每次只做一次当前脚注增长；间隙、符号和块 arena 容量不增长。两万旧引用的晚定义首行仅两次符号步骤，三条续行各一次；4096 条续行只检查实际路径，4096 历史定义后追加只处理当前旧封口和新定义。250 层列表前缀线性匹配，400 组独立物理行范围/内容模型及 240 次流式区间/高度模型通过。收起百万字多段定义的布局零正文回调、零字节计费。成功定义每个带前缀续行至多一个间隙，重复定义不保存间隙；累计解析 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持，不承诺任意 append/finish 的单次硬时延。

项目 devShell 验收：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run` 各 573 项、`cargo test --doc --workspace` 47 项、`cargo fmt --all -- --check` 和 `git diff --check` 通过。`cargo run -p mdstream-core -r --example footnote_blocks` 以 6 帧/61 源码字节完成读取并保持折叠；既有 `footnotes`、`container_definitions`、`list_tightness` 示例通过。完整日志：`/tmp/mdstream-footnote-block-validation.log`，失败基线：`/tmp/mdstream-footnote-block-baseline.log`；模型与相关回归日志：`/tmp/mdstream-footnote-block-model.log`、`/tmp/mdstream-footnote-block-chunking.log`。

README、TODO 已同步；固定四空格/显式路径方言已完成，分类前缀上限、lazy/tab 等更完整续行规则、定义正文 Markdown 解析/排版、编号上标、脚注区、双向跳转/返回和实际对象回调继续保留待办，终端 UI 留待后续阶段。

## 2026-10-30 容器完整 tab 边界缩进核心交付

- [x] 容器前缀按物理行每四列对齐，列表标记后的混合水平空白和续行使用列距离；嵌套不重置列号，有序编号/兄弟各自保存宽度，任务标记不增加续行要求。容器标记前至多三列，引用后的可选分隔只消费一个空格或恰好一列的 tab。BlockKind::ListItem.indent 明确为相对父前缀的列数，源码区间继续使用原始字节。
- [x] 仅匹配完整字节边界；目标位于 tab 内部时该层原子失败，连同此前空格原样保留，既有 lazy 正文或作用域关闭规则决定后续处理。被消费的 tab 只隐藏真实字节，不构造虚拟空格；叶块的四空格代码/脚注缩进等既有方言不变。长表格分隔行探测复用同一列匹配器，确认/失败与确切作用域保持一致。
- [x] 保留流式分类时机和含前缀的 1 KiB 字节上限；源码映射、内联对象、tight/loose、稳定块/容器身份、折叠意愿、旧测量/读取任务拒绝及合并 Update 复用原接口。新增 container_tabs 示例以 4 帧/13 源码字节读取去前缀数学内容，容器折叠保持。

匹配器只增加常数大小的当前列号、空白末端列号和 marker 列号，活动节点不增加存储。相邻列表层共享空白末端缓存，混合空白至多两遍，纯空格保持一次扫描；首次失配只检查一次有界 lazy 正文，余下路径 O(1) 保留或按实际关闭节点弹栈。新物理行的匹配按实际前缀长度计费，不遍历历史正文；长行后续追加不重新匹配祖先。累计解析 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持，无单次 append/finish 硬时延承诺。

新增 13 项验收（12 项专用测试和 1 项全局分块快照），并补充既有短样本全切点：3072 组独立列宽/真实字节边界与围栏范围模型；UTF-8 全切点/逐前缀、CRLF/EOF、截断标记/分类上限、部分 tab 原子回退、嵌套及任务/兄弟宽度、标题/多行 Setext/代码/定义/脚注/数学/表格回归；长分隔行成功/失败、源映射收费与预算暂停、旧任务/测量失效和通知重建。250 层列表每轮完整续行只检查 250 帧，部分前缀 lazy 续行只检查 2 帧；1000 层引用及 250 层列表的百万字长行后再逐字追加 1000 次，分类/前缀/容器/章节工作和容器 arena 容量均不增长，inline_bytes 恰好增加 1000。

项目 devShell 验收：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release nextest 各 586 项、48 项文档测试、fmt 和 diff 检查通过。container_tabs、containers、container_code、footnote_blocks 四个示例通过。日志：`/tmp/mdstream-container-tabs-validation.log`（check/clippy、release、doc、示例及格式），`/tmp/mdstream-container-tabs-debug.log`（全量 debug）；初始失败基线：`/tmp/mdstream-container-tabs-baseline.log`。

按账本允许的独立子项交付完整 tab 边界规则。README/TODO 已同步；部分 tab 的虚拟空格表示、叶块 tab 缩进、分类上限扩展及其它容器规则继续保留未完成条目，不宣称完整 CommonMark 缩进或终端排版已完成。


## 2026-10-30 围栏代码续行部分 tab 核心交付

本单元完成容器部分 tab 的独立子项：已开启反引号/波浪线围栏的显式续行。原有字节源码保持不变，Item 缩进和引用分隔可部分消费物理行四列 tab，同一 tab 可供多个嵌套帧使用。整条旧路径先原子探测；失败回到原作用域/兄弟分类，不产生半条路径的替换。余下 1..3 列映射为静态空格，并参与关闭围栏的至多三列缩进判定；超过阈值、错定界、尾随正文和长分类候选保持字面代码。CRLF/EOF、空行、稳定 BlockId/ContainerId、原始范围与流式更新沿用已有契约。

每条含余量的正文行只保存一个单字节源码区间及列数，不复制正文、按虚拟列分配或回扫旧显示记录。前缀游标只增加常数状态，同一个 tab 最多供四列；空白串末端缓存避免按祖先重复扫描，失败至多重试一次原有有界前缀。显示索引在原始 runs 之上返回 `DisplayRun.replacement`，每个 tab 只有一个范围/tile 拥有其替代输出。预算游标在收费失败时保持位置；搜索可把多个空格命中映射回同一 tab，有界摘要即使截断替代文本也保留完整映射。`RenderRun` 继续保留原字节，调用方用显示接口取得替换。

新增 14 项验收（13 项专用测试与 1 项全局分块测试），并扩充既有短样本全切点：UTF-8 全切点/逐前缀与分类窗口、嵌套共享 tab/引用分隔、关闭缩进、CRLF/EOF/空行/失败回退、稳定身份/合并 Update/旧测量与游标拒绝、逐标量/tile/预算显示、搜索/摘要映射及折叠回归。既有列宽模型扩展为 3072 组含部分 tab 的结果对照；另有 400 组嵌套字面正文和围栏范围模型。250 层列表的 200 条短续行每行只访问 250 帧；千层引用/250 层列表百万字正文后 1000 次追加不增加前缀、容器、章节或分类工作，不增加容器容量/注解节点。4096 个历史替换后追加新代码只做常数前缀工作，局部查询少于 200 次注解访问；折叠百万字代码零正文计费。

验收：项目 devShell 的 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 600 项、`cargo test --doc --workspace` 49 项、格式与 diff 检查通过。新示例 `partial_tabs` 9 帧/49 计费字节，输出两列空格及中文/emoji，追加关闭后保持折叠意愿。

按账本允许的独立子项交付，未声称全部部分 tab 方言完成。一般正文、数学内容、新叶块分类及缩进代码的 tab 扩展、分类上限与其它容器规则保留 TODO；终端排版仍属后续阶段。


## 2026-10-30 一般正文续行部分 tab 核心交付

本单元完成已有同作用域非空正文续行的部分 tab 子项：按物理行四列对齐原子匹配整条现有路径，多层 Item 可共享 tab，引用分隔只占一列；余下 1..3 列映射到原 tab 单字节。只接受总剩余水平缩进至多三列、既有有界分类器判为普通文字且无新容器标记的续行。叶块、ContainerId、源码包络和已发布对象 ID 保持稳定；失败不产生半条路径的隐藏/替换。新叶块/表头、空白行、过量缩进、不完整路径及继承数学候选沿用原方言，不宣称全部部分 tab 规则已完成。

虚拟空格直接消费于原行内状态机，衔接强调、软/硬换行、代码 span、链接/图片标签及有界引用匹配；不复制或重放正文。显示索引单独保存有序 tab 记录，与之后确认的代码区间组合。代码晚闭合只检查末端并追加一个区间，不改写历史 tab；规范化若裁掉末端一格，仅减少一个虚拟列。显示/tile/逐标量读取、搜索和有界摘要保留单字节 replacement，空格裁到零列仍按源码计费；revision 失效、合并 Update、折叠意愿和已有预算契约保持。

前缀探测先检查是否存在 tab，不含时不重复遍历活动路径；含 tab 的原子探测失败至多再执行一次原分类器。250 层两列 Item 的成功共享 tab 续行每行访问 250 帧；原四列 Item tab 续行有一次探测和一次原匹配，精确计数测试已记录这两个有界遍历，仍按实际提供前缀收费，缺失深层路径时提前停止。每行最多一个固定大小映射记录；千层/百万字正文后 1000 次追加不增长分类/前缀/祖先工作或容器容量，inline_bytes 恰好增加 1000。4096 个旧映射加百万字开放代码晚闭合，原映射逐项不变、只新增一个代码区间；局部查询少于 200 次注解访问。零运行时依赖、禁止 unsafe、累计 O(n log n)/空间 O(n) 保持，不承诺单次解析硬预算。

新增 13 项验收（11 项专用测试、1 项显示存储测试、1 项全局分块测试），并扩充短样本全切点：400 组随机嵌套列宽与显式空格行内模型、UTF-8 全切点/逐前缀、CRLF/EOF/分类窗口、原子回退、新块/数学边界、Setext 迟确认、代码裁一列/全空白/未闭合、稳定对象及合并通知、旧测量/游标/搜索拒绝、预算原子暂停和搜索/摘要映射。千层引用、250 层列表、4096 历史映射及百万字输入通过；折叠百万字正文零字节读取。

验收：项目 devShell 的 cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 613 项、cargo test --doc --workspace 50 项、格式与 diff 检查通过。prose_tabs 新示例 20 帧/112 计费字节，代码晚闭合后保留映射和条目折叠；既有 partial_tabs、container_tabs、container_inline 示例通过。日志：`/tmp/mdstream-prose-tabs-validation.log`；初始 3 项失败基线：`/tmp/mdstream-prose-tabs-baseline.log`。

README/TODO 已同步完成范围。数学内容、新叶块/表头分类、空白/过量缩进/不完整路径的部分 tab、叶块缩进、分类上限及其它容器语义继续保留 TODO；终端 UI 与实际排版仍为后续阶段。


## 2026-10-30 数学内容续行部分 tab 核心交付

本单元完成容器行内与已开启独立数学块续行的部分 tab 映射。行内数学沿用已有非空普通正文续行资格：完整路径原子匹配、剩余水平缩进至多三列，不新建叶块规则或表头；虚拟空格衔接美元定界与转义状态，稳定 MathId、物理叶块和源码包络保持。数学块复用围栏路径，接受空白/过量缩进的字面正文，关闭 $$ 前的虚拟列和 ASCII 空格合计须不超过三列；缺失路径、长定界、未闭合 EOF 等继续既有失败语义。

math_content / math_block_content 保持原文；math_payload / math_block_payload 去除前缀并将最终部分 tab 的单字节映射为 1..3 个静态空格，其它 CRLF、正文 tab、反斜线与实体拼写保留。MathChunk.source 连续覆盖原始内容，按实际源码字节、一 query、一 object 收费；一个 tab 字节可原子发出最多三个空格，预算不足不推进也不收费。显示、搜索、摘要复用同一映射，数学读取不应用代码/实体规范化；旧任务与测量拒绝、合并 Update、独立折叠、预览和源码导航保持。

游标固定十个机器字以内、O(1) 构造，首次付费读取 O(log G + log T) 定位已有间隙和 tab，后续 O(1) 合并。复用显示索引的 tab 记录，不保存第二份映射、不按虚拟列分配；候选只记录最后消费位置，省略前缀追加有序间隙。4096 条历史映射后百万字增长及晚闭合不修改旧记录，同一物理行再逐字追加 1000 次不新增间隙或回扫前缀/祖先。数学块仍做 O(log n) 范围失效，注解端点可按新增字节线性增长，未改变原契约。累计 O(n log n)/空间 O(n)、零运行时依赖和禁止 unsafe 保持，不承诺单次解析硬时延。

新增 14 项验收（12 项专用测试、1 项内部存储测试、1 项全局分块测试）：行内/块数学各 400 组独立列宽与源码模型、UTF-8 全切点/逐前缀、CRLF/EOF、转义/定界长度、空白/失败路径/分类窗口、Setext/表头隔离、原文与替代文本的预算读取、稳定对象/合并通知、旧测量/读取/搜索拒绝，以及搜索/摘要/折叠/预览/导航。千层引用、251 层列表及百万字输入通过；收起数学块或容器时正文零字节读取。既有排除样本随本单元方言更新，仍保留未匹配完整路径的回退检查。

验收：项目 devShell 的 cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 627 项、cargo test --doc --workspace 51 项、cargo fmt --all -- --check 与 git diff --check 全部通过。math_tabs 新示例以 6 帧/33 源码字节完成共享预算读取，流式关闭后保持折叠意愿；既有 container_math、math_blocks、prose_tabs 示例通过。验收日志：`/tmp/mdstream-math-tabs-validation.log`；初始两项失败基线：`/tmp/mdstream-math-tabs-baseline.log`。

README/TODO 已同步；新叶块/表头的部分 tab 分类、一般正文空白/过量缩进/不完整路径、叶块 tab 缩进、分类上限及其余容器规则继续保留 TODO。TeX 排版、额外定界方言及终端 UI 仍属后续阶段。


## 2026-10-30 容器所选新叶块部分 tab 核心交付

本单元交付新叶块分类的独立子项：已有容器路径完整原子匹配后，部分 tab 的虚拟余量参与 ATX 标题、反引号/波浪线围栏、独占行 $$ 数学块及四列缩进代码分类。保持 ContainerId、原始物理行范围及标题/代码 info/数学内容的字节偏移；不合成展开源码。虚拟列加 ASCII 空格至多三列时沿用标题/围栏规则，达到四列时优先作为字面代码，只剥离四列，其余真实空格保留。已有围栏仍优先处理字面内容和关闭行。

新叶块结束旧行内候选、待确认表头或已确认表格；四列缩进的 -|- 不确认表格，也不启动长分隔行探测。隐藏的 tab 不送入新标题的行内状态机、不生成替换记录；围栏内容中的余量沿用原有单字节映射。块、章节和对象身份、合并 Update、缓存/测量失效、默认折叠、预览及源码导航复用既有接口。Setext、定义、表头、普通新段落、新容器首行、空白及路径不完整时仍保留原方言；前缀之外的额外 tab 不扩展为叶块缩进。

复杂度：只在当前有界分类窗口内探测一次完整路径，失败至多再执行一次原匹配器；空白末端缓存跨祖先复用，一个 tab 最多提供四列。无 tab 时只增加一次实际前缀字节检查，不遍历活动栈；已有定义的计数验收显式计入这次检查，250 层定义续行仍只访问 250 帧，前缀读取小于 4 × depth + 64。分类器用常数大小的虚拟列参数调整阈值及源码端点，无新持久结构、正文副本或语义回放。含前缀 1 KiB 提交窗口和流式时机不变，长行追加不重新匹配；累计 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持，不承诺单次解析硬时延。

新增 15 项验收（14 项专用测试与 1 项全局分块快照）：576 组列阈值/真实空格对照、400 组随机嵌套空格模型、UTF-8 全切点/逐前缀、CRLF/EOF、边界内外截断标记、长失败候选、百万字分隔行外观代码、失败路径与范围身份。覆盖标题首个多字节字符/行内对象、代码合并、数学闭合、表格中断、旧测量/任务拒绝和通知重建；预算显示、逐标量/tile、搜索/摘要映射及折叠/预览/导航均通过。千层引用和 251 层列表的百万字标题/代码/数学再逐字追加 1000 次，前缀/祖先/分类/章节计数和容器容量不变；250 层共享 tab 每行只访问 250 帧，4096 个历史标题后只访问当前一层。收起新叶块正文零字节读取。

项目 devShell 验收：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 642 项、cargo test --doc --workspace 52 项、cargo fmt --all -- --check 及 git diff --check 通过。leaf_tabs 新示例 14 帧/215 计费字节，保持标题和代码/数学的确切作用域与流式折叠意愿；math_tabs、prose_tabs、container_headings、table_preview 示例通过。核心验收及示例日志：`/tmp/mdstream-leaf-tabs-validation.log`、`/tmp/mdstream-leaf-tabs-final-doc.log`；初始三项失败基线：`/tmp/mdstream-leaf-tabs-baseline.log`。

额外检查发现既有 container_tables 示例在第 30 行仍断言表格直接以顶层 Heading 为折叠父级，实际父级已是 Item 容器。在修改前 d47813f 的独立源码副本中复现同一失败，见 `/tmp/mdstream-leaf-tabs-existing-example-baseline.log`；该示例的失败位于核心检查全部通过之后，未将其记为通过，也未扩展本单元去修改这个既有断言。

README/TODO 已同步完成范围。剩余新叶块类型与表头、普通新段落、新容器首行、空白/不完整路径、叶块内部 tab、分类上限及其它容器语义继续保留 TODO；未声称全部部分 tab 方言或终端 UI 已完成。


## 2026-10-30 列表跨无前缀空行核心交付

本单元交付省略列表缩进的完整空白行：保留活动 List/Item 稳定身份，到第一个缺失 Quote 之前为止；缺失引用及其后缀一次关闭。空白叶块归属于保留的最内层 Item，共享容器末端包含空行；EOF、作用域退出或标记类型改变不确认 Loose，也不向前收缩已发布范围。短尾部等 LF/EOF 分类，长空白沿用 1 KiB 分类窗口与既有代码/正文回退，不在晚来的 LF 重试。

用一个深度下界记录连续空行的待确认作用域：最后显式匹配的 Item 与其后省略缩进的列表均可恢复。下一条已分类非空行仅在实际恢复的直接条目或同型兄弟确认分隔，已有内容后的新直接内容/子容器或同型兄弟使该 List 单调 Loose；空条目、首次内容、标记改变和边界各有明确规则。不将内层已确认空行继续传播至祖先，完整缩进内层尾部分隔的祖先传播仍保留 TODO。缺少路径的围栏/数学/脚注结束旧叶块；完整匹配路径的字面空行和定义内部分段保持原方言。表头、Setext 和行内序列不跨该空行。

新增有序活动 Quote 深度索引，以 O(log(Q + 1)) 查找必须关闭的后缀，Q 为活动引用数；不遍历保留的祖先。索引随开闭维护、容量与分配计入 container_arena_stats，查询比较计入 container_steps；待确认区间为 O(1) 状态，历史子块与对象不回访。250 层列表跨 4096 个无缩进空行每行只检查一个活动帧，章节每行仅一次上下文检查且折叠容器索引无写入；百万字正文后 1000 次逐字追加不重扫前缀或祖先、不增加容器容量。4096 个历史条目后确认 Loose 仍仅 3 个 list_steps/3 个 container_steps，旧块完全不改写。累计解析 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持。

新增 14 项验收：12 项专用测试、1 项全局分块快照和 1 项折叠高度模型。覆盖 400 组独立间距/身份/范围模型、240 轮流式折叠/预览/高度与可见性模型、UTF-8 全切点/逐前缀、CRLF/EOF、空条目/任务/编号/不同 marker、深引用一次退出、20,000 空行既有复杂度回归、长空白分类与部分 tab、块边界、通知重建/合并、旧缓存/测量/检查点拒绝。保留列表的百万字折叠正文零读取，祖先查询和布局共享预算。旧断言按本单元的新空行语义更新；需要隔离独立列表的模型和缓冲复用测试改用显式作用域退出，仍保留原验收目标。

项目 devShell 验收：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 656 项、cargo test --doc --workspace 53 项、cargo fmt --all -- --check 及 git diff --check 通过。list_blanks 新示例用 3 帧/0 正文字节读取祖先，验证内外层分别晚确定、EOF 封口及折叠意愿保持；既有 list_tightness、containers、container_folding 示例通过。完整验收日志：`/tmp/mdstream-list-blanks-validation.log`；初始三项失败基线：`/tmp/mdstream-list-blanks-baseline.log`。

README/TODO 已同步。完整缩进内层尾部空行向祖先传播、实际段落间距、其它容器/tab 规则、分类上限及终端 UI 继续保留后续；未声明完整 CommonMark tight/loose 兼容。


## 2026-10-30 列表嵌套末尾空行传播核心交付

完成既有容器方言的嵌套末尾空行传播：已分类的分隔若仍位于最后子容器尾部，随 List/Item/Quote 实际关闭传给恢复的祖先 Item；同型兄弟或已有直接内容后的新内容/子容器确认该父列表 Loose。内层后续内容会消耗分隔，不继续传播；EOF/关闭本身不确认，退出整个列表不污染新作用域，已关闭内层列表可保持 Tight。空 Item/Quote 首行、编号/任务/标记变更、字面代码/数学、脚注内部分段与末尾分隔及分类窗口规则见 [README](README.md#列表嵌套末尾空行)。

实现只增加一个布尔尾部摘要，复用已有后缀关闭循环；任意深度子树一次关闭只向保留 Item 传递一次待定分隔。兄弟切换先关闭旧子树再读取旧 Item 的分隔状态，防止遗漏晚确认。没有历史子块或祖先扫描、不增加深度索引；源码范围、叶块/对象身份、既有一次区间失效和 Update.loosened_lists 契约保持。Work.list_steps 明确计入实际关闭后的分隔传递。累计解析 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持。

新增 13 项验收：11 项专用测试、1 项全局分块快照和 1 项折叠高度模型。覆盖 400 组独立深度/恢复作用域/源码范围模型、240 轮流式折叠/预览/高度、UTF-8 全切点/逐前缀、CRLF/EOF、短尾部未分类与长空白不重试、完整 tab 边界、任务与 marker、新直接子块/失败候选、引用和脚注尾部、事件重建/合并/容量复用、旧缓存/测量/检查点/任务拒绝及共享预算。既有两处明确断言“不传播”的预期随新语义更新，内部空行和字面块隔离仍通过。

复杂度证据：64/128/250 层列表各 1000 条完整空行仅 1000 次 list_steps；恢复最外层兄弟均为 4 次 list_steps，container_steps 恰为 2 × depth + 1，对应实际路径检查/关闭/新建。千层引用尾部关闭同样只需 4 次间距步骤；空行期间无折叠容器索引更新或容量增长。百万字长行后 1000 次逐字追加不增加分类前缀/祖先/间距工作；1 或 4096 个历史条目后，嵌套尾部确认均为 4 次 list_steps、5 次 container_steps，旧块/链接不改写。折叠百万字正文零读取，祖先游标与布局共享预算并可原子暂停。

项目 devShell 验收全部通过：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 669 项、cargo test --doc --workspace 54 项、cargo fmt --all -- --check 及 git diff --check。新 list_tail_blanks 示例用 3 帧/0 正文字节验证父级迟确认、内层 Tight/Sealed、稳定范围和折叠意愿；既有 list_blanks、list_tightness、containers、container_folding、footnote_blocks 示例通过。完整日志：`/tmp/mdstream-list-tail-validation.log`；初始两项失败基线：`/tmp/mdstream-list-tail-baseline.log`。

README/TODO 已同步。实际段落间距、其它容器/tab 规则、分类上限及更完整块确认/撤销、终端 UI 继续保留 TODO；不声明完整 CommonMark 兼容。

## 2026-10-30 手工及宿主固定源码范围折叠交付

本轮交付已定型源码前缀内的手工选区/宿主范围核心；不改变只追加源码、语法结构、零运行时依赖及禁止 unsafe 契约。

- [x] **身份与边界**：`SourceFoldRegistration` 原子注册固定非空 UTF-8 `Span`，生成视图独立、进程内不复用的 `SourceFoldId`；两端固定字节锚点，追加/reset 不改变范围。`source_fold_limit` O(1) 排除当前叶块、待分类行、表头探测及整个待确认容器 Setext 标题；局部关闭后可注册，无需 finish 整篇。
- [x] **交叉与结构组合**：手工范围之间不相交（含展开/预览状态，重复亦拒绝），与语法候选及注册行范围允许包含；源码包络与隐藏范围均拒绝交叉，后注册表格行也遵守。开放结构另拒绝跨越其起点的固定选区，防止章节正文增长后穿过终点。双端祖先路径验证，不遍历中间所有候选/正文；完全相同范围按容器、叶语法、手工的顺序消歧。
- [x] **查询与状态**：`FoldView::fold_candidate`、`source_fold_at`、`SourceFoldCandidates` 提供 ID/位置/预算窗口查询，`remove_source_fold` 释放注册及预览/意愿，旧 ID 不复用。语法父链不重挂，手工 parent 指向最内层包含其源码范围的语法节点。独立视图、批量 All/Subtree/Siblings/Focus、预览、源码锚点、导航及有界摘要/布局全部接入；标题级别/容器筛选及未来默认规则保持各自职责。
- [x] **预算与增量**：注册每个端点查找、祖先检查、路径结束及提交扣 1 query / 1 object，零正文读取；成功前不改视图，取消/错误/失效没有半成品。千层引用两端共 2000 个祖先可逐帧暂停；R 个注册只保存 O(R) 当前索引，单点操作和点查询用有序索引，游标 O(1)。4096 历史范围后的百万字及 1000 次逐字追加不刷新历史范围或增加容器/标题扫描；反复注册/删除不保留历史记录。
- [x] **布局与 EOF**：最外层有效范围占位，完整内部 tile 直接遮蔽，边界按预算消费；百万字与图片折叠不读取隐藏正文/对象。固定范围结束于旧 EOF 时只校正一次 tile 边界；与已关闭代码/数学遮蔽独立共存。新组合测试发现关闭代码块原有 EOF 遮蔽未刷新，已复用代码/数学/行范围的一次性刷新槽修复，不遍历旧范围。
- [x] **模型与回归**：新增 `crates/mdstream-core/tests/source_folding.rs` 17 项验收，包含 2400 组全量结构区间对照、1000 轮随机区间/高度/布局模型、240 次流式模型、UTF-8 全切点/逐前缀、CRLF/迟确认 Setext/表格、千层/百万字/4096 范围、双向行范围交叉拒绝、预览/同父兄弟/子树/聚焦、旧任务和测量拒绝、通知合并/重放、EOF 同 tile 多范围。保留原有兄弟语法扫描的预算计数，仅为手工范围增加标题区域的枚举。

验收通过：项目 devShell 的 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 686 项、`cargo test --doc --workspace` 55 项、`cargo fmt --all -- --check` 和 `git diff --check`。`source_folding` 示例注册 2 帧、布局 6 帧/56 个计费字节跳过百万字，导航 1 帧恢复源码锚点；既有 `table_row_folding`、`folding`、`fold_batch` 示例通过。日志：`/tmp/mdstream-source-folds-validation.log`。开放章节交叉与关闭代码 EOF 的失败证据分别保存在 `/tmp/mdstream-source-folds-boundary-baseline.log`、`/tmp/mdstream-source-folds-eof-baseline.log`。

自动增长手工范围、尚未定型正文、手工嵌套、交叉拆分/优先级、重载后的业务身份映射、实际段内软换行/占位绘制及终端选区 UI 继续保留 TODO；本轮只标记上述固定范围核心完成。

## 2026-10-30 已定型顶层段落有界预览折叠交付

完成长段落折叠的独立子项：`SourceFoldRegistration::paragraph` 选择已定型顶层 Paragraph，以源码字节上限保留 UTF-8/CRLF 对齐前缀、固定收起余文。复用视图独立 `SourceFoldId` 并保存原 `BlockId`；候选 source 为整个段落、summary 为预览、hidden 为余文。局部闭合即可注册，后续追加/晚定义/finish/reset 不改变身份或范围；未完成/可能成为 Setext 或表格的段落、容器叶块及无余文的上限明确拒绝。源码保持只追加，未修改解析器、运行时依赖或 unsafe 约束。

整个段落与手工范围共享不相交注册索引，包括仍可见的前缀；双向重叠/重复拒绝，章节父级、批量/预览/导航与独立视图保持原意愿。注册仍按两个端点及章节链共享预算原子完成；创建只有固定端点探测，注册不扫描前缀/余文，额外游标 O(1)，R 个注册存储 O(R)。4096 历史注册后 1000 次逐字追加无范围刷新或容器扫描；百万字追加不改写历史注册，布局不读取隐藏余文或其链接对象。

段中边界单独接入排版：`FoldPlaceholder::paragraph` 指明占位需结束当前段落，终止行高度归属 owner tile；`FoldLayoutSink::paragraph_end` 在余文跳过后仅恢复段落/换行状态，不重复计高。该回调独立扣 object/recovery，可在占位后暂停且不会重复输出，跨 tile/EOF 和从隐藏区中途开始均可恢复后文，无需读取隐藏换行。父级已隐藏整个段落时不生成内层回调，展开/临时预览继续读原文。原始行内样式、实体/代码/链接源码映射保持，显示规范化和真实终端宽度仍由接入方处理。

新增 15 项专用验收：640 组长度/前缀/换行/列宽布局对照、1000 轮随机与 240 次流式区间/高度模型、UTF-8 全切点/逐前缀、CRLF/无换行 EOF、行内边界逐字节截断、手工范围双向拒绝、零预算/原子注册/段尾暂停、百万字隐藏与检查点恢复、4096 历史注册、有界显示摘要、预览/批量/源码锚点/导航、晚定义及 Setext/表格、通知合并/重放、旧任务/测量/检查点拒绝。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 701 项、`cargo test --doc --workspace` 56 项、`cargo fmt --all -- --check` 和 `git diff --check`。`paragraph_folding` 示例 8 帧/47 个计费字节跳过百万字、产生一次段尾恢复；既有 `source_folding`、`folding`、`table_row_folding` 示例通过。完整日志：`/tmp/mdstream-paragraph-folds-validation.log`；专项日志：`/tmp/mdstream-paragraph-folds-targeted.log`。首轮一个手工测试选区本身跨章节边界，已修正测试输入；没有降级范围校验。

README/TODO 已同步。开放段落自动增长、容器跨物理叶块段落、自动长度策略、逻辑/显示行数预览、完整字素/对象截断、媒体详情及完整终端排版继续保留 TODO；本轮只标记已定型顶层段落的源码前缀预览与段尾恢复核心完成。

## 2026-10-30 旧强调可隔离闭合的晚管道长表头核心交付

按本单元允许的独立子项交付：顶层或同一显式 Quote/Item 续行，旧 `*`、`**`、`_`、`__`、`~~` opener 在首管道前由只能闭合的 run 结束，且不会弹出本行未闭合 opener 时，可继续识别晚管道长表头。保持已有分类窗口、块优先级和作用域规则。完整方言及可运行片段见 [README](README.md#表格任务列表与图片)。

先复现两项失败基线：原实现拒绝这种表头，并立即发布跨行样式。现有单调注解不能直接删除跨行样式而保留重叠标记；本子项使用隔离策略。独立首单元格中，close-only run 没有本行 opener 可匹配，也不能自己开启，所以它应为字面内容、局部栈保持不变。原状态正常弹出旧栈，只暂存配对的两个源码区间及 style；本行局部样式、实体、代码、链接、数学及脚注继续即时发布。探测期间仅这些跨行样式/定界符隐藏及原有前序换行等待判定，没有延迟全部普通长行。

确认表格时丢弃暂存配对并移除剩余旧 opener，直接复用首单元格状态和已发布对象。分隔行失败、无管道行 LF/EOF、作用域退出或随后不可隔离闭合时，按原端点提交每对一次；正常范围失效拒绝旧缓存、测量及读取/搜索任务。不会重放正文、克隆 delimiter 栈、撤销对象或重编号。双向 run（如 `end*next`）、会弹出本行 opener 的配对及已闭合代码/数学/标签仍保持普通即时解析并取消该行探测；后者先提交此前隔离的配对。

复杂度依据：每对必定消费一个旧 opener，保存和提交/丢弃各至多一次；确认或失败计入 `delimiter_steps`。额外空间 O(实际消费的旧 opener 数)，注解仍为 O(log n) 区间操作，全库累计 O(n log n)/空间 O(n)。首管道前和失败路径的 `inline_bytes <= source().len()`；成功只做既有单元格边界扫描，不重复解析已消费前缀。千层引用的百万字增长不增加 delimiter、分类、容器、祖先或章节扫描，注解/容器容量不变；4096 对配对在判定时恰好增加 4096 个 delimiter_steps，1000 次尾部追加不遍历配对，4096 历史链接保持身份。内部存储验证百万字追加不增加配对容量，之后 200 轮无管道续行复用同一缓冲、不积累历史配对。解析调用仍为摊还保证，不承诺单次硬时延。

新增 13 项验收：`crates/mdstream-core/tests/closed_headers.rs` 11 项、UTF-8 全切点/逐前缀快照 1 项、内部存储 1 项。覆盖五种标记及软/硬换行、显式列表/引用、旧栈跨类型弹出和本行重开、120 组 flanking/局部栈模型、400 组长分隔行成功/晚失败范围模型、独立早管道语义对照、CRLF/EOF/Setext/作用域退出、稳定 BlockId/LinkId/MathId/脚注编号、合并 Update、共享预算原子暂停、预览/搜索源码映射与章节/容器/表格折叠。折叠百万字正文零读取。原拒绝测试明确改用双向标记保留拒绝覆盖，旧范围模型同时接受此次新增的 close-only 情形，未删除失败约束。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 714 项、`cargo test --doc --workspace` 57 项、`cargo fmt --all -- --check` 和 `git diff --check`。新示例 `cargo run -p mdstream-core -r --example closed_headers` 验证百万字失败提交与成功丢弃、稳定链接和共享预算摘要，后者 17 帧/119 个计费字节；既有 open/inherited/late/long headers、long_delimiters、code/math/link headers 示例均通过。完整日志 `/tmp/mdstream-closed-headers-validation.log`；两项失败基线 `/tmp/mdstream-closed-headers-baseline.log`；专项 `/tmp/mdstream-closed-headers-targeted.log` 与 `/tmp/mdstream-closed-headers-chunking.log`。

README、TODO 只标记本子项；无法隔离的旧强调闭合、通用已发布样式撤销、其它候选/分类上限、更完整容器语义和终端 UI 继续保留待办。零运行时依赖与禁止 unsafe 保持。

## 2026-10-30 旧代码可隔离闭合的晚管道长表头核心交付

本单元交付顶层或同一显式 Quote/Item 续行中，旧代码 span 已在首管道前闭合、且闭合后只有普通文本/空白及不启动行内语法的标点的晚管道长表头。分类窗口、短行/早管道优先级、lazy/部分 tab 和其它块规则保持既有方言；闭合后遇到实体、链接、强调、代码等语法的起始字符即恢复普通解析，不声称通用代码映射撤销已经完成。完整字符边界与示例见 [README](README.md#表格任务列表与图片)。

先复现两项失败基线：原实现拒绝这种表头，并立即提交跨行代码样式/空白规范化。显示替换索引要求源码有序，不能直接移除已发布代码或把它插回后来的实体之前；本次使用固定大小隔离。等长反引号闭合时只保存原 Code 摘要及 closer，继续逐字消费普通长尾；成功丢弃该闭合与旧 Inline，在已有行扫描中独立解析当前表头。旧代码内的当前行对象只在确认时按单元格语义发布一次，旧 LinkId/MathId/脚注编号不变，不读取前序代码正文。当前行的等长反引号可能在独立单元格里变成 opener，故不复用原代码状态。

闭合后遇到不支持的语法字节，必须先提交旧映射，再处理新字节，防止同一次 feed 中实体/代码映射先于旧源码范围进入索引；后续再开代码也不重启探测。无管道 LF/EOF、非法分隔行、Setext 或作用域退出只提交一次原样式/端点映射，再消费未处理的后缀。正常范围失效覆盖旧缓存、测量、显示游标与搜索任务。没有延迟全部普通长行、回放失败正文、撤销对象或重编号；CRLF、全空白/两端裁剪和历史部分 tab 的源码归属保持。

额外状态 O(1)，每次探测最多保存一个闭合摘要；提交/丢弃计一个 delimiter_steps。成功只额外消费互不重叠的当前表头，失败不重放，满足 inline_bytes <= source().len() + table_bytes；累计解析 O(n log n)/空间 O(n)，不承诺单次确认硬时延。千层引用中前序百万字代码加当前百万字长尾，后续每字只增加一次 inline_bytes/table_header_bytes，无分类/容器/章节/祖先或 delimiter 工作，注解和容器容量不变。4096 历史链接、4096 旧 opener、当前行 4096 链接/数学/脚注及 4096 列保持一次消费；历史对象不变。内部测试确认百万字期间摘要端点不变、200 轮成功/失败/取消后释放且不累积历史。

新增 12 项验收：crates/mdstream-core/tests/closed_code_headers.rs 10 项、全局 UTF-8 全切点/逐前缀快照 1 项、内部状态 1 项。覆盖独立早管道语义对照、四种 run 长度/三个闭合位置/四种作用域、12 种取消后缀、400 组范围/长分隔行模型、CRLF/EOF/Setext/作用域退出、共享预算原子暂停、逐标量/tile/预算源码映射、有界摘要/搜索、稳定 BlockId/对象、通知合并与旧任务失效；容器折叠百万字正文零读取。四处旧拒绝断言改为带语法后缀的明确拒绝案例或接受新增可隔离分支，保留原模型和回退验证。

项目 devShell 全部通过：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 726 项、cargo test --doc --workspace 58 项、cargo fmt --all -- --check 和 git diff --check。新 closed_code_headers 示例验证百万字成功丢弃/失败提交、稳定链接和预算预览（15 帧/101 个计费源码字节）；既有 code_headers、closed_headers、math_headers、link_headers 示例通过。完整日志 `/tmp/mdstream-closed-code-headers-validation.log`；初始两项失败基线 `/tmp/mdstream-closed-code-headers-baseline.log`；专项 `/tmp/mdstream-closed-code-headers-targeted.log`。

README/TODO 已同步，仅标记上述独立子项。闭合后带其它行内语法的代码表头、通用已发布映射撤销、其它分类/容器限制与终端 UI 继续保留 TODO。零运行时依赖与禁止 unsafe 保持。

## 2026-10-30 旧数学可隔离闭合的晚管道长表头核心交付

本单元交付顶层或同一显式 Quote/Item 续行中，旧 `$` / `$$` 在首管道前闭合、闭合后只有普通文本/空白及不启动语法的标点的晚管道长表头。完整字符边界与流式方言见 [README](README.md#表格任务列表与图片)。保留分类窗口、lazy/部分 tab 表头及块优先级，不声称通用已发布对象或样式撤销已完成。

先复现两项失败基线：原实现拒绝这类表头，并在判定前发布跨行 MathId。数学 arena 要求源码有序且 ID 稳定，不能在表头确认后删除或重编号对象。本次匹配等长美元串时只暂存 open/close 两个源码区间，不提前发布 MathId、MATH 样式、定界符隐藏或 new_maths；长尾仍逐字消费。成功丢弃该端点对和旧 Inline，在既有单元格扫描中独立解析当前行；当前行对象发布一次，历史 MathId/LinkId/脚注编号和物理叶块保持稳定。

失败、无管道 LF/EOF、Setext 或作用域退出时，按原端点提交公式一次，再消费未处理的后缀。闭合后遇到其它行内语法起始字节，必须在处理该字节前提交旧公式并取消该行探测，防止同次 feed 中后来的数学对象先入 arena；再次开启候选也不重启探测。原始读取保留源码包络，math_payload 复用已有前缀间隙与部分 tab 映射，闭合不回访或改写历史映射。正常范围失效拒绝旧缓存、测量、显示游标和搜索任务，合并 Update 与逐次应用的折叠视图一致。

额外状态 O(1)，每次探测最多一个定界端点对；提交/丢弃计一次 math_steps，美元串扫描继续原计数。成功只额外消费互不重叠的当前表头，失败不重放，满足 inline_bytes <= source().len() + table_bytes，累计 O(n log n)/空间 O(n)，不承诺单次解析硬时延。千层引用中前序百万字数学正文加当前百万字长尾，每字仅增加一次 inline_bytes/table_header_bytes，无分类、数学、容器、章节或祖先工作，注解/容器容量不变。250 层列表中的 4096 历史公式/链接、旧 opener、当前行对象和列保持一次消费，旧公式逐项不变。内部测试确认百万字期间端点和间隙索引不变，200 轮成功/失败/取消后端点对释放。

新增 13 项验收：crates/mdstream-core/tests/closed_math_headers.rs 11 项、全局 UTF-8 全切点/逐前缀快照 1 项、内部状态 1 项。覆盖独立早管道单元格语义、两种美元串/三个闭合位置/四种作用域、12 种取消后缀、400 组范围与长分隔行模型、CRLF/EOF/Setext/作用域退出、共享预算原子暂停、前缀与 tab 的数学源码映射、摘要/搜索和折叠零正文读取。四处旧拒绝断言更新为新增接受分支的语义模型或带语法后缀的明确拒绝案例，保留失败回退验证。

项目 devShell 全部通过：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 739 项、cargo test --doc --workspace 59 项、cargo fmt --all -- --check 和 git diff --check。新 closed_math_headers 示例验证百万字成功丢弃/失败提交、稳定 MathId 及预算预览（14 帧/96 个计费源码字节）；既有 math_headers、closed_code_headers、closed_headers、container_math、link_headers 示例通过。完整日志 `/tmp/mdstream-closed-math-headers-validation.log`；初始两项失败基线 `/tmp/mdstream-closed-math-headers-baseline.log`；专项 `/tmp/mdstream-closed-math-headers-targeted.log`。

README/TODO 已同步，仅标记上述独立子项。闭合后带其它行内语法的数学表头、通用已发布对象/样式撤销、其它分类/容器限制及终端 UI 继续保留 TODO。零运行时依赖与禁止 unsafe 保持。

## 2026-10-30 旧链接图片 shortcut 可隔离闭合的晚管道长表头核心交付

本单元交付顶层或同一显式 Quote/Item 续行中，旧链接/图片标签已在首管道前闭合、闭合后只有普通文本/空白及不启动语法的标点的 shortcut 子项。分类窗口、短行/早管道、lazy/部分 tab 当前表头及块优先级保持既有方言。直接目标、full/collapsed 引用后缀和闭合后其它行内语法仍取消探测，边界见 [README](README.md#表格任务列表与图片)。

先复现两项失败基线：旧实现立即分配跨行 LinkId、规范化引用并更新首引用位置，随后拒绝此类表头。为避免撤销已发布对象和引用副作用，本次只保存一个 Candidate，并复用原有 MAX_LABEL_BYTES + 1 字节有界标签缓冲；不提前登记符号、first_use、链接样式或括号隐藏。成功丢弃 shortcut，直接复用当前单元格前缀的行内状态、样式和对象；失败、无管道行末、EOF、Setext 或作用域退出时提交一次。标签保持原始源码包络，规范化只读已去前缀的缓冲；空白/超长标签沿用无引用对象的规则。

紧接闭合括号的显式后缀、或后来的语法起始字节，先提交原 shortcut 并取消该行探测。尚未开始的 Tail 恢复同一个 LinkId，后续直接目标/title 或引用后缀更新原对象；非法/未完成后缀仍走原方言。新语法处理前完成提交，因此标签缓冲不会先被覆盖，后续链接不会先于旧源码对象入 arena。已定义但从未使用的符号在成功路径上仍无 first_use；失败保留去前缀匹配及晚定义，首定义优先与现有间接寻址不变。范围通知拒绝旧缓存、测量、显示游标和搜索任务，合并 Update 与逐次应用的折叠视图一致。

额外状态 O(1)，无正文复制、旧样式/对象撤销或前缀重解析；提交/丢弃各只处理一次候选并计入 delimiter_steps，失败的有界规范化计入 label_bytes。累计 O(n log n)/空间 O(n)，不承诺单次判定硬时延。千层引用中的百万字长尾只增加一次 inline_bytes/table_header_bytes，无标签、分类、分隔符、容器或章节/祖先重查，注解和容器容量不变；成功/失败判定的新增行内扫描均小于 12 字节。250 层列表中的 4096 历史链接、4096 旧 opener、4096 当前公式和 4096 列保留对象身份并复用已解析语义。两万旧引用之后晚定义只规范化 7 个标签字节；内部测试验证百万字期间端点和缓冲不变、200 轮成功/失败/取消不积累历史状态。

新增 14 项验收：crates/mdstream-core/tests/closed_link_headers.rs 12 项、UTF-8 全切点/逐前缀快照 1 项、内部状态 1 项。覆盖四种作用域/两个标签类型/三个闭合位置的早管道对照、400 组范围/长分隔行模型、16 种取消后缀、CRLF/EOF/Setext/作用域退出、空白及精确长度上限、部分 tab 源码映射、共享预算暂停/恢复、稳定 BlockId/LinkId/MathId、通知合并和旧任务失效。摘要、显示搜索及百万字容器折叠保持有界，折叠正文和对象零读取。原 200 组替换/转义模型只增加可隔离 shortcut 接受分支，保留其余拒绝断言。

项目 devShell 全部通过：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 753 项、cargo test --doc --workspace 60 项、cargo fmt --all -- --check 和 git diff --check。新 closed_link_headers 示例验证百万字成功丢弃/失败晚定义、稳定图片 ID 和预算预览（14 帧/94 个计费源码字节）。完整日志 `/tmp/mdstream-closed-link-headers-validation.log`；初始两项失败基线 `/tmp/mdstream-closed-link-headers-baseline.log`。

README/TODO 已同步，仅标记此独立子项。显式目标/引用后缀、闭合后其它行内语法、通用已发布对象/引用副作用撤销、其余分类/容器限制及终端 UI 继续保留待办。零运行时依赖与禁止 unsafe 保持。

## 2026-10-30 旧链接图片普通字符显式后缀的晚管道长表头核心交付

本单元交付旧跨行链接/图片标签闭合后，直接裸目标与可选 title、full/collapsed 引用的普通字符后缀子项。继承顶层或同一显式 Quote/Item 的标签时，可在首管道前解析有界后缀并继续即时消费普通长尾；目标中的平衡括号、空目标、Unicode title/引用均沿用原单物理行方言。含行内语法起始字符的后缀、引用内 `^`、完成引用后缀后再遇 `(`、闭合后新行内语法仍取消探测；完整边界见 [README](README.md#表格任务列表与图片)。短表头、早管道、lazy/部分 tab 当前表头分类和其它块优先级保持原规则。

先复现两项失败基线：原实现遇后缀起始就发布旧 LinkId/shortcut 符号并取消探测，不能确认此类长表头。现在复用原 Destination/Tail 单遍状态，只额外保存一个后缀的源码端点，不复制目标、不提前分配对象/符号或更新 first_use。成功丢弃旧跨行对象，直接目标和空引用保持单元格字面正文；完整非空 `[key]` 按独立单元格 shortcut 发布自己的 LinkId、源码映射和首引用位置，在后续单元格对象之前完成，无需回放长前缀。未完成普通后缀由单元格边界丢弃，表格管道优先规则保持。

失败、无管道、EOF、Setext 或作用域退出先提交原 shortcut，再应用完成后缀，保留原符号创建顺序和首引用副作用，直接目标仍可留下旧 shortcut 符号；full 引用按目标标签间接寻址，collapsed 复用已去前缀的旧标签。拒绝字节恢复原 parser 位置和同一 LinkId，后续完成只更新该对象；空白/过长旧标签仍遵循无 shortcut 对象及直接目标回退规则。既有局部样式、代码/数学对象和历史 LinkId 保持稳定；源标签包络、部分 tab 显示映射及晚定义无需回访旧引用。

新增 14 项验收：crates/mdstream-core/tests/link_suffix_headers.rs 12 项、crates/mdstream-core/tests/chunking.rs 全 UTF-8 切点/逐前缀快照 1 项、内部状态 1 项。覆盖四种作用域和两种标签的独立早管道对照，400 组范围/长分隔行模型，显式目标/title/引用与总后缀精确上限，非法/未完成/CRLF/EOF/Setext/作用域退出，17 种取消后缀及稳定临时对象，已定义符号的 first_use、两万旧引用晚定义及首定义优先，部分 tab 映射、共享预算暂停/恢复、合并 Update 与旧缓存/测量/显示/搜索任务拒绝。同步更新既有接受范围断言，并保留含新语法时取消及 Provisional LinkId 更新的检查。

千层容器中三类后缀的百万字长尾每字节仅新增一次 inline/header 摘要计数，标签、delimiter、分类、容器、标题及 arena 计数保持不变。250 层列表、4096 旧对象/旧 opener/列的成功路径保留历史 ID，决议仅 4097 次 delimiter_steps，局部 key 规范化 3 字节；不回扫首单元格。固定后缀状态在每类 200 轮成功/失败/取消后释放，标签缓冲不增长。折叠百万字容器正文与对象零读取。累计 O(n log n)、空间 O(n)，无新增运行时依赖/unsafe，不承诺单次判定硬时延。

项目 devShell 全部通过：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 767 项、cargo test --doc --workspace 61 项、cargo fmt --all -- --check 和 git diff --check。新 link_suffix_headers 示例验证百万字成功局部引用/失败晚定义、稳定图片 ID 和预算预览（16 帧/112 个计费源码字节）；既有 closed_link_headers、link_headers、closed_headers、closed_code_headers、closed_math_headers 示例全部通过。完整日志 `/tmp/mdstream-link-suffix-headers-validation.log`；初始两项失败基线 `/tmp/mdstream-link-suffix-headers-baseline.log`；专项 `/tmp/mdstream-link-suffix-headers-targeted.log`。

按可独立验收子项交付。含行内语法字符的显式后缀、闭合后其它语法、多行目标/title、通用对象/符号撤销及其余分类/容器/终端 UI 需求仍在 TODO。

## 2026-10-30 CommonMark ATX 标准样例评分交付

选定此前未评分的 CommonMark 0.31.2 ATX headings 完整章节（62–79，共 18 例）。`crates/mdstream-core/tests/fixtures/commonmark_atx.rs` 保留官方 Markdown 和 HTML 原值、John MacFarlane / CC BY-SA 4.0 归属、版本、来源及规范行号；`scripts/generate_commonmark_atx.py` 只读取本地官方 `spec.json`，以 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20` 固定完整输入并校验章节成员，支持 `--check` 复核生成结果。字符串转义保留每个字符，避免源文件尾部不可见空格；构建和测试离线且零新增依赖。

独立 `crates/mdstream-core/tests/commonmark_atx.rs` 按块类型/顺序、标题级别、可见文字和强调比较官方 HTML，并核对标题索引。测试适配器保留所有空格、内部换行和代码 LF，只跳过结构空块及段落/标题的一个物理末尾 LF；不重解析 Markdown、不按样例修正输出。评分结果：13 支持（62–66、68–69、74–79）；1 刻意差异（70，既有缩进代码可中断段落）；4 未支持（67、71–73，标题尾部或关闭井号前的空格未裁剪）。四项缺口明确列入 TODO；未修改官方期望或宣称其符合规范。非支持项同时检查与官方不同、与单独固定的当前观察值相同，回归或改善均须复审分类。

新增 3 项测试：逐例分类与固定汇总、18 例全部 430 个切点及逐字符输入、适配器对空标题/空白/强调/级别/块边界/代码换行的敏感性。所有流式结果独立对照官方期望或已分类观察，不以整次解析自比较充当标准评分。样例最长仅 54 字节，不据此声称长输入或完整语法兼容；解析核心、复杂度与预算契约无改动。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 770 项、`cargo test --doc --workspace` 61 项、`cargo fmt --all -- --check`、官方夹具 `--check` 和 `git diff --check`。另外验证修改过的上游输入会被摘要检查拒绝。独立报告命令：`nix develop -c cargo nextest run --workspace --test commonmark_atx --no-fail-fast --success-output immediate`。完整日志 `/tmp/mdstream-commonmark-atx-validation.log`；专项分类日志 `/tmp/mdstream-commonmark-atx-targeted.log`；原始评分观察 `/tmp/mdstream-commonmark-atx-baseline.log`。

本次完成该章节的固定样例覆盖与分类。标题空白缺口、完整缩进规则、其余 CommonMark/GFM 范围及终端 UI 继续保留 TODO。

## 2026-10-30 ATX 原始空白与关闭尾缀交付

修复固定 CommonMark 0.31.2 ATX 样例 67、71–73 的空白缺口。解析核心在行内消费前去除标题原始首尾 space/tab、合法关闭井号及其前面的空白；短行和超过 1 KiB 分类窗口的已识别标题共用流式状态机。开头井号的分隔符限定为 space/tab 或行结束；转义井号、非关闭串、Unicode/form-feed 空白、实体生成的空格及代码 span 内部空白保留各自语义。顶层、显式容器及已支持的部分 tab 标题使用同一路径。

候选只保存正文首尾、井号末端和待确认 CR 的源码位置，空间 O(1)。`Work.atx_bytes` 统计每个新增正文/候选字节；已确认正文即时消费，待确认尾缀暂按原始内容显示，失败时只向行内解析提交一次。LF/EOF 成功确认仅隐藏尾缀并收缩原 Heading.content，无需回扫标题、历史对象或容器祖先；持续前导空白随到达隐藏。源码、BlockId/HeadingId、已完成链接/数学对象身份保持；通知使旧 tile、测量、检查点及 revision 绑定任务失效。CRLF 支持跨追加，物理 LF 保留；孤立 CR 仍按库的 LF/CRLF 方言作为正文。

官方夹具及投影的空白保留规则未改动。18 例独立评分更新为 17 支持（62–69、71–79）、1 刻意差异（70，既有缩进代码可中断段落）、0 未支持；全部 430 个切点及逐字符输入通过。适配器继续检查内部重复空格、强调、级别和块边界，未以 trim 或替换官方期望消除缺口。两个既有容器测试的尾空格断言随修复更新。

新增 10 项验收：crates/mdstream-core/tests/atx_whitespace.rs 9 项及 crates/mdstream-core/tests/chunking.rs 1 项。包括 1600 组固定种子的独立原始区间模型、UTF-8 全切点/逐前缀快照、空标题/CRLF/EOF/失败尾缀、显式容器/部分 tab、原始和显示映射、逐 tile/逐标量读取、小共享预算暂停/恢复、名称搜索/slug、通知合并和旧任务/缓存/测量拒绝。百万字正文加百万字空白或井号尾缀，在千层引用中逐字增长不扫描祖先、不增加注解节点或容器容量，EOF 不读取隐藏尾缀；百万字前导空白不进入行内解析，失败尾缀仅一次消费。4096 个历史标题和行内对象后关闭只处理当前标题。累计 O(n log n)、空间 O(n)，不新增运行时依赖或 unsafe；失败尾缀的一次性消费仍属摊还保证，不承诺单次 append 硬时延。

项目 devShell 验收全部通过：cargo check --workspace --all-targets、cargo clippy --workspace --all-targets -- -D warnings、debug/release cargo nextest run --workspace --no-fail-fast 各 780 项、cargo test --doc --workspace 62 项、cargo fmt --all -- --check、官方夹具摘要与生成结果 --check、git diff --check。anchors、container_headings、heading_search 三个相关 release 示例通过；新增 README 示例演示长空白与跨追加 CRLF。完整日志 `/tmp/mdstream-atx-whitespace-validation.log`；独立评分与专项日志 `/tmp/mdstream-atx-whitespace-targeted.log`、`/tmp/mdstream-atx-whitespace-regression.log`；修改前两项失败基线 `/tmp/mdstream-atx-whitespace-baseline.log`。

本单元完成 ATX 原始空白与长尾缀需求。分类前缀上限、完整缩进/容器规则、其它 CommonMark/GFM 范围及终端 UI 继续保留 TODO。

## 2026-10-30 CommonMark Setext 标准样例评分交付

- [x] **固定完整官方章节**：`crates/mdstream-core/tests/fixtures/commonmark_setext.rs` 收录 CommonMark 0.31.2 Setext headings 的全部 27 例（80–106），逐字保存 Markdown 与 HTML，包括空格、tab 和 LF；附 John MacFarlane / CC BY-SA 4.0 归属、规范行号和完整 `spec.json` SHA-256。`scripts/generate_commonmark_setext.py` 只读另行下载的本地来源，校验固定摘要及章节成员后生成或 `--check`，来源被改动时拒绝；构建与测试不联网。
- [x] **独立分类和可重复报告**：`crates/mdstream-core/tests/commonmark_setext.rs` 对照官方 HTML，当前 **22 支持、2 刻意差异、3 未支持**。支持 80–81、83、86、88、90–106；85/87 分别沿用空行终止缩进代码、缩进代码中断段落的已记录方言；82/84/89 暴露 Setext 标题缩进及尾随 space/tab 未裁剪的缺口。非支持项另存当前观察值，同时断言仍不同于官方、且当前行为未改变；改善与回归都必须审查分类，不自动提高分数。
- [x] **保留待评分语义**：测试投影检查块类型/顺序、标题级别和索引范围、显示文字/强调、根引用及根无序 tight 列表边界。同容器物理正文叶块只按已发布软/硬换行合并；空行两侧的代码块保持分离。仅省略结构空行和非代码块的一个物理末尾 LF，保留内部换行、所有空格/tab 和代码 LF；不 `trim`、不重解析 Markdown、不接收样例编号做特殊修正。投影是该章节测试适配器，不是库 HTML 渲染器；样例 91 不能作为支持 HTML 的证明。
- [x] **流式及适配器验收**：全部 533 个单切点和 506 个逐字符前缀检查块/标题/容器稳定身份、源码范围及显示映射/样式、待分类范围，EOF 后与固定分类结果对照。独立敏感性检查覆盖空格/tab、强调、标题级别、内部软/硬换行、引用/列表边界及代码空行分块，避免适配器把待修复差异抹掉。

独立运行：`nix develop -c cargo nextest run --workspace --test commonmark_setext --no-fail-fast --success-output immediate`；夹具复核：`python3 scripts/generate_commonmark_setext.py /path/to/spec.json --check`。3 项新增测试全部通过。项目 devShell 的 `cargo check --workspace --all-targets`、clippy `-D warnings`、debug/release nextest 各 **783 项**、**62 项文档测试**、格式检查、ATX/Setext 夹具复核及 `git diff --check` 通过。完整日志 `/tmp/mdstream-commonmark-setext-validation.log`，独立分类与分块日志 `/tmp/mdstream-commonmark-setext-targeted.log`，初始公开语义观察 `/tmp/mdstream-commonmark-setext-inspect.log`；另以独立 Python 读取逐对核验 27 份官方输入/HTML 并验证来源摘要篡改被拒绝。

本次完成评分而非全部语法支持，没有修改解析核心、依赖或终端 UI。Setext 首尾空白（82、84、89）明确保留 TODO；完整缩进/空行规则、分类长度上限、其余 CommonMark/GFM 范围及终端 UI 继续保留，短标准样例不替代已有长输入和预算/复杂度验收。

## 2026-10-30 Setext 原始首尾空白交付

- [x] **官方空白缺口修复**：Setext 的整段原文首尾 space/tab 在下划线确认时隐藏，`Heading.content` 收缩到裁剪后的源码包络。固定 CommonMark 0.31.2 样例 82、84、89 改为支持，完整章节现为 **25 支持、2 刻意差异、0 未支持**；官方 Markdown/HTML 夹具、摘要和 HTML 投影代码不变。85/87 仍保留已记录的缩进代码方言差异，未改成官方期望。
- [x] **单遍摘要与迟确认**：行内扫描同时维护首尾非 space/tab 字节及待定 CR，计入现有 `Work.inline_bytes`，只增加固定状态。LF/CRLF 后的 Setext 下划线完整确认时读取端点并写至多两个隐藏区间；不回扫正文、容器祖先或物理叶块。内部空白、软/硬换行、已确认代码/数学/链接、实体生成空格和原始源码映射保持；普通段落及失败下划线继续既有规则。序列关闭时清除端点，表头失败只消费未解析后缀，成功则隔离单元格。
- [x] **身份与预算消费**：保留 BlockId、原始物理范围及已发布链接/数学对象；新 HeadingId 使用既有整段 Update 失效及合并协议。首尾 tile 的旧 key/测量/检查点、显示游标、名称搜索和 slug 任务正确失效；小预算显示读取原子暂停并推进，跨 tile/逐标量显示与全文一致，命中的源码映射、共享预算名称搜索/slug、折叠默认/预览/导航通过。
- [x] **独立模型与对抗验收**：新增 10 项验收（`crates/mdstream-core/tests/setext_whitespace.rs` 9 项及全局分块快照 1 项）。1200 组原始区间模型对照独立整串 trim；覆盖 UTF-8 全切点/逐字符前缀、CRLF/EOF/孤立 CR、实体/代码/未闭合候选、内部硬换行、列表 padding/任务/部分 tab、分类上限、作用域切换及表头失败/成功。既有官方 533 切点、506 逐字符前缀评分保持通过。

顶层及千层引用中的百万字正文加百万字尾部空白，逐字增长只计新增行内字节，分类/容器/标题上下文计数不变；下划线确认的旧正文/标签读取为零、区间树访问少于 1000。4096 个历史标题/链接/数学对象之后，再确认含 4096 个中间物理叶块的标题，旧对象和中间块不改写、标题步骤仅 2 次；空间保持固定摘要而非缓存原文。追加百万字折叠正文后，范围查询零正文计费；既有 `container_setext` release 示例以 16 帧/32 源码字节完成并跳过隐藏正文。整体仍是既有摊还 O(n log n)/空间 O(n) 契约，未把未闭合 opener 的实际收尾成本称作常数时间。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **793 项**、`cargo test --doc --workspace` **63 项**、`cargo fmt --all -- --check`、官方 ATX/Setext 夹具 `--check` 和 `git diff --check`。`container_setext`、`heading_search`、`anchors` 三个 release 示例通过。完整日志 `/tmp/mdstream-setext-whitespace-validation.log`；修改前官方评分的三项失败基线 `/tmp/mdstream-setext-whitespace-baseline.log`；初次独立评分与容器回归 `/tmp/mdstream-setext-whitespace-targeted.log`；专项分块/预算/复杂度 `/tmp/mdstream-setext-whitespace-new-tests.log`；最终独立评分和文档复核 `/tmp/mdstream-setext-whitespace-final-doc.log`。

本单元完成已识别 Setext 标题的原始首尾空白需求。普通段落空白、完整缩进/空行规则、Setext 分类上限、剩余容器/tab 规则、其它 CommonMark/GFM 范围及终端 UI 继续按 TODO 推进；零运行时依赖及禁止 unsafe 保持。

## 2026-10-30 GFM 表格标准样例评分交付

- [x] **固定完整官方章节**：`crates/mdstream-core/tests/fixtures/gfm_tables.rs` 收录 GFM 0.29-gfm（2019-04-06）Tables (extension) 全部 8 例（198–205）的官方 Markdown 和 HTML，保留每个空格及 LF，附 GitHub / John MacFarlane 归属、CC BY-SA 4.0 许可、HTML 源码行号和样例锚点。`scripts/generate_gfm_tables.py` 读取另行下载的官方规范 HTML，以 SHA-256 `b153d814fdfc8624bb6da7449162c1cd707a637f7d1c1b636eb44b9cf63fa220` 固定整页来源；校验完整章节成员，只解码代码块的展示标记，离线生成或 `--check`。独立正则提取与 HTML 解码逐字符核验了 8 对输入/期望，篡改来源会被摘要校验拒绝。
- [x] **独立分类和报告**：`crates/mdstream-core/tests/gfm_tables.rs` 当前 **8 支持、0 刻意差异、0 未支持**。覆盖表头/正文、居中/右对齐、可选外侧管道、文本/代码/粗体中的转义管道、引用和空行终止、无管道稀疏行、表头/分隔行列数不符回退、缺失及多余单元格、无正文表格。每例直接比较固定官方 HTML，任一不符都会失败；没有用库输出替换标准期望或动态重分类。
- [x] **保留评分语义**：窄投影仅读取公开表/列/行/单元格元数据及显示片段，核对表 ID、所属块与源码范围，保持表头/正文、顺序、对齐、代码/强调/粗体和根引用边界。隐式空单元格沿用公开语义，多余单元格必须已被解析器排除；不重新切管道、去反斜线、裁剪空白或按样例编号调整输出。只跳过结构空块及段落的一个物理末尾 LF；内部空格/tab 和实体生成的首尾空格均保留。
- [x] **流式和适配器验收**：全部 **361 个切点、353 个逐字符前缀**对照块/标题/容器/表格身份、列宽提示、行/单元格源码范围及完成状态、原始/显示样式与待分类范围；最终输出仍直接比较官方期望。敏感性验收覆盖列/行顺序、默认与左/中/右对齐、无正文与空正文行、缺失/多余单元格、嵌套样式、空格/tab/实体、段落和引用边界。新增 3 项独立测试。

独立报告：`nix develop -c cargo nextest run --workspace --test gfm_tables --no-fail-fast --success-output immediate`；夹具复核：`python3 scripts/generate_gfm_tables.py /path/to/gfm.html --check`。项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **796 项**、`cargo test --doc --workspace` **63 项**、`cargo fmt --all -- --check`、夹具复核及 `git diff --check`。完整日志 `/tmp/mdstream-gfm-tables-validation.log`；独立分类与分块日志 `/tmp/mdstream-gfm-tables-targeted.log`；首次公开语义观察 `/tmp/mdstream-gfm-tables-inspect.log`；独立来源复核和篡改拒绝证据 `/tmp/mdstream-gfm-tables-fixture-check.log`。

本单元完成官方表格章节评分。8 例均为 ASCII、最长 56 字节，不能据此推断所有 Unicode、长输入、容器表格或完整 GFM 合规；其余标准范围、语法边界和终端表格排版继续保留 TODO。解析核心及其复杂度/预算契约未改动，保持零运行时依赖及禁止 unsafe。

## 2026-10-30 GFM 任务列表标准样例评分交付

- [x] **固定完整官方章节**：`crates/mdstream-core/tests/fixtures/gfm_tasks.rs` 收录 GFM 0.29-gfm（2019-04-06）Task list items (extension) 全部 2 例（279–280）的官方 Markdown/HTML，保留每个空格及 LF、HTML 源码行号和样例锚点，附 GitHub / John MacFarlane 归属与 CC BY-SA 4.0 许可。`scripts/generate_gfm_tasks.py` 从本地规范 HTML 离线生成或 `--check`；完整来源 SHA-256 为 `b153d814fdfc8624bb6da7449162c1cd707a637f7d1c1b636eb44b9cf63fa220`，校验章节成员，仅解码展示标记。独立正则提取/HTML 解码与生成器结果逐字符一致，来源篡改会被摘要检查拒绝。
- [x] **独立分类报告**：`crates/mdstream-core/tests/gfm_tasks.rs` 当前 **2 支持、0 刻意差异、0 未支持**。279 覆盖未勾选/已勾选兄弟任务；280 覆盖嵌套任务、各层独立状态和父子边界。逐例直接对照固定官方 HTML，差异即失败，不使用库输出重写期望或动态重分类。
- [x] **保留评分语义与边界**：投影读取公开容器父关系、tight 状态、块/容器一致的任务元数据及显示片段，保持嵌套、条目顺序、勾选状态和正文/样式。核对任务标记的源码范围、原文与隐藏映射，根据 `checked` 输出语义复选框。两个样例标记后的单个 ASCII 空格经原文和隐藏映射校验后保留；不把其它 padding 归一化为单空格，不扫描源码发现任务标记。正文只去掉一个物理末尾 LF，内部空白及实体生成的首尾空格均保留。多行条目、引用、有序/loose 列表、其它块及不同任务 padding 显式拒绝，不把未覆盖语义静默丢弃后计为通过。
- [x] **流式与适配器验收**：所有 **66 个切点、64 个逐字符前缀**对照块/容器身份、父关系、范围/状态、任务标记范围和勾选状态、原始/显示片段映射及待分类范围；结束后仍直接比较官方期望。另检查大小写 X 的状态、非任务条目、正文和条目数量、父子关系/兄弟顺序/新列表边界、代码/强调/粗体、空格/tab/实体以及超出投影范围的输入拒绝。共新增 4 项测试，手工变体与官方评分分开。

独立报告：`nix develop -c cargo nextest run --workspace --test gfm_tasks --no-fail-fast --success-output immediate`；夹具复核：`python3 scripts/generate_gfm_tasks.py /path/to/gfm.html --check`。项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **800 项**、`cargo test --doc --workspace` **63 项**、`cargo fmt --all -- --check`、夹具复核及 `git diff --check`。完整日志 `/tmp/mdstream-gfm-tasks-validation.log`；独立评分和分块日志 `/tmp/mdstream-gfm-tasks-targeted.log`；独立来源核验及篡改拒绝 `/tmp/mdstream-gfm-tasks-fixture-check.log`。

本单元完成官方任务列表章节评分。两个输入均为 ASCII、20/44 字节、最多两层列表，不能据此推断完整任务标记空白、空内容、多行/有序/loose 任务、任意深度、Unicode 或长输入合规。其它标准范围和交互绘制继续保留 TODO；复选框 HTML 仅为评分投影。解析核心及预算/复杂度契约未改动，保持零运行时依赖及禁止 unsafe。

## 2026-10-30 开放顶层段落有界前缀折叠交付

完成开放段落折叠的独立子项：`SourceFoldRegistration::open_paragraph` 显式注册当前已分类但尚未封口的顶层 Paragraph。复用视图独立 SourceFoldId、原 BlockId、源码字节上限、UTF-8/CRLF 对齐与两个端点路径的共享预算；注册原子完成且初始展开，整段包络与其它源码注册保持不相交。原有 `paragraph` 固定范围接口及源码只追加语义保持，未更改解析器、运行时依赖或 unsafe 约束。

活动候选的尾部暂定到源码 EOF，包含未分类末行；前缀与隐藏起点固定。逐次/合并 `apply` 保留增长、普通闭合和 reset 下的身份、收起与临时预览意愿；封口后冻结到实际段尾。晚表头从原段落分离时收缩范围，仍有余文才保留；整块改型为 Setext/表格或收缩后余文消失则移除注册、意愿及预览，旧 ID 不复用，不将选择自动转交新结构。宿主查询候选/paragraph_fold_block 的 None 观察移除；后续可以显式注册新段落。

视图只有一个活动顶层段落更新槽，展开/临时预览期间同样维护；每次通知只按 BlockId 查元数据和更新一个注册，O(log R + log F + log T)，不枚举历史范围、祖先或正文。普通封口/移除退出槽，EOF tile 的一次性校正保持。4096 历史注册加一个百万字开放段落，追加只计一次 range_visits；百万字后再逐字追加 1000 次恰好刷新 1000 次，input/inline 各增长 1000，分类与容器计数不增长。退出段落进入千层引用后，再追加百万字及 1000 字不刷新旧段落或遍历容器祖先。

保留段中占位和段尾状态恢复的独立计费：placeholder 计入 owner tile，paragraph_end 只重置换行上下文；开放 EOF、范围收缩、远处隐藏内部起步和检查点恢复均不回放隐藏正文。收起范围变化从固定隐藏起点失效高度/检查点；文档/视图版本拒绝旧测量、注册、枚举和布局任务。章节、手工范围、批量、预览与 Reveal 导航继续组合，移除注册释放预留区间。

新增 15 项专项验收：384 组独立标量换行布局/EOF/tile 边界模型、240 轮（480 次追加）区间/高度模型、UTF-8 全切点及逐前缀、CRLF/失败候选、Setext/整表改型与晚表头收缩、展开/收起/预览三种移除状态、原子注册与零预算、占位后段尾暂停、共享 recovery 恢复、百万字/4096 历史注册/千层容器、晚定义与行内闭合、源码导航/批量、手工预留与通知合并/重放。已有段落 1000 轮随机及其它折叠测试全部保留并通过。专项首轮修正两处测试输入假设：未定义 shortcut 标签沿用现有隐藏方括号语义；千层引用使用 1000 个连续 `>`，避免空格前缀超出既定 1024 字节分类窗口，没有更改解析期望或扩大方言。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 815 项、`cargo test --doc --workspace` 64 项、`cargo fmt --all -- --check` 及 `git diff --check`。更新的 `paragraph_folding` release 示例先收起开放段落再追加百万字，8 帧/47 计费字节、一次段尾恢复；既有 `source_folding`、`folding`、`table_row_folding` 示例通过。完整日志：`/tmp/mdstream-open-paragraph-validation.log`；专项日志：`/tmp/mdstream-open-paragraph-targeted2.log`、`/tmp/mdstream-open-paragraph-targeted3.log`（后者的千层测试输入修正已由完整验收复核）。

README/TODO 已同步，只勾选显式开放顶层段落前缀折叠。容器跨物理叶块段落身份、自动发现/长度默认策略、逻辑/显示行数预览、完整字素/对象截断、媒体详情、通用手工范围增长和终端 UI 继续保留 TODO。

## 2026-10-30 容器内段落有界前缀折叠交付

完成已识别容器方言内的独立段落折叠核心：`SourceFoldRegistration::container_paragraph` 以首物理叶块 BlockId 标识同一确切作用域内的连续文本，显式/lazy 续行共用一个段落，物理叶块与行内对象保持原身份。开放或已闭合段落均可显式注册，初始展开，生成视图独立 SourceFoldId；源码字节前缀包含物理标记，向下对齐 UTF-8/CRLF，允许跨叶块或为零，必须在已分类包络内留下余文。其它叶块、空标记和非正文结构不接受。

开放尾部暂定到 EOF，逐次/合并 apply 后保留增长、闭合和 reset 的身份与意愿。空行、块中断、子容器/兄弟条目及作用域退出冻结原段尾；返回外层的后续段落使用新首叶块。多行 Setext 或整段表格改型移除注册及预览，晚表头分离将段尾收缩到该物理行起点；若余文消失也移除。成功/失败长表头与超过分类窗口的分隔行均覆盖；不把选择自动转交新语法对象。

解析器复用活动 container_title/current，闭合时仅插入一次 first -> end 有序索引，`Work::paragraph_closures` 计数；P 个闭合段落占 O(P)，封口 O(log P)，无物理叶块遍历。构造 O(log P + log R)，两端祖先路径逐项共享 Budget，注册游标 O(1)；视图继续共用一个活动段落槽，普通增长 O(log R + log F + log T)，闭合额外一次 O(log P) 查询。千层引用下百万字后再逐字追加 1000 次恰好 1000 次刷新，input/inline 各增加 1000，容器/标题步骤及段尾索引零增长；4096 叶块只在封口时写一次，旧叶块未改写。4096 历史段落注册不增加活动追加刷新；250 层列表注册的两条祖先路径总计 1000 次，完成前没有任何部分注册。

整段与手工/其它段落注册不相交，保留章节/容器 parent、祖先折叠、批量、临时预览及 Reveal 导航。占位与 paragraph_end(first) 分别计费；占位贡献 owner tile 行高，结束回调只复位段落局部状态。远处隐藏内部起步与检查点恢复不回放隐藏正文；新可见叶块保留自身容器 ID，完整祖先上下文仍由宿主按预算查询。旧注册/枚举/布局任务、测量和检查点按既有通知及版本契约拒绝或失效。

新增 15 项专项测试：288 组独立标量换行布局模型、240 轮流式/960 次切换的区间与高度模型、UTF-8 全切点及逐前缀、CRLF/EOF/孤立 CR、部分 tab/lazy/任务标记、普通闭合与迟确认、长表头/分隔行成功失败、百万字/千层引用/250 层列表/4096 历史段落和叶块、注册原子性与重叠拒绝、段尾零预算暂停、共享 recovery、独立视图、导航/批量/预览及合并通知重放。既有 30 项段落测试继续通过。首轮两处测试输入计算修正为实际字节端点和隐藏内部 tile 起点；Clippy 提示的两处整除写法也已修正，无解析期望降级。

项目 devShell 全部验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **830 项**、`cargo test --doc --workspace` **65 项**、`cargo fmt --all -- --check` 及 `git diff --check`。新增 `container_paragraph_folding` release 示例在百万字追加和兄弟条目恢复后仅 **6 帧/32 计费字节/一次段尾回调**；原 paragraph_folding、source_folding、container_folding release 示例均通过。完整日志 `/tmp/mdstream-container-paragraph-validation.log`，专项日志 `/tmp/mdstream-container-paragraph-targeted.log`。

README/TODO 已同步；已交付显式源码前缀核心。自动发现/长度默认策略、逻辑/显示行数前缀、完整字素/对象截断、媒体详情、完整容器/tab 方言及终端排版继续保留 TODO。保持零运行时依赖、禁止 unsafe 和累计 O(n log n)/空间 O(n) 契约。

## 2026-10-30 长段落有预算候选发现交付

新增 `ParagraphFoldCandidates` / `ParagraphFoldCandidate`：按半开源码窗口发现顶层及容器逻辑段落，并按整段已分类源码字节下限筛选。结果包含首物理叶块 BlockId、确切容器、完整源码包络、classified_end 和开放状态；从段中、后续物理叶块或待分类尾部起步可定位更早的首叶块，结果有序且无重复。长度包含原始语法、容器前缀及 CRLF，待分类字节不帮助达到下限；空窗口没有结果，允许非 UTF-8 边界的元数据窗口。

构造 O(1)，每步原子消费 1 query / 1 object；不合格段落、非段落叶块和结束查找同样计费，零正文/tiles/recovery 读取。多叶块段落无论是否达到长度下限都直接跳到段尾。复用有序块范围、已有闭合容器段尾索引与活动首尾，每步固定次数 O(log B + log P) 查询，游标 O(1)，无新增解析索引或逐帧历史枚举；输出 Vec 可复用，分配沿用摊还契约。千层引用/百万字、4096 物理叶块、4096 历史段落中的单段窗口均只需一个步骤；250 层列表发现同样一个步骤，注册祖先路径另按既定预算处理。

游标绑定文档及 revision，非空追加/首次 finish 使旧任务失效；验证错误不改变输出或消费预算。开放尾部暂定到 EOF，Setext/整表迟确认移除候选、晚表头分离收缩范围、作用域边界和 EOF 冻结。宿主可按新增字节窗口重新发现普通增长的原段落，缓存的未注册候选在迟确认后需重查受影响的原窗口。发现不绑定视图，注册、折叠、临时预览和 resize 不影响游标，也不排除已注册/被隐藏段落；选择后用现有三个段落注册入口继续共享当帧预算，原有重叠及端点路径验证保持。已注册对象仍由 FoldView::apply 维护。

新增 **14 项**专项验收：400 个独立源码包络/1600 组窗口和长度筛选模型、UTF-8 全切点及逐前缀、CRLF/EOF/部分 tab/lazy、窗口每个字节边界、开放待分类长度、迟确认成功/失败、预算原子暂停与 4096 非候选逐项计费、共享注册预算及 2000 次祖先检查、独立视图与注册/预览/reset、错误文档/旧 revision/无效窗口、合并通知/重放和旧测量/注册拒绝、240 轮流式范围/可见性模型、千层/百万字/250 层列表/4096 历史及物理叶块。百万字后逐字追加 1000 次，各局部窗口恰好一个发现步骤、视图恰好 1000 次活动刷新；inline 只增加 1000，祖先及段尾索引不增长。未修改解析器或既有语法预期。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **844 项**、`cargo test --doc --workspace` **66 项**、`cargo fmt --all -- --check` 及 `git diff --check`。新 `paragraph_discovery` release 示例：从百万字中段发现原首叶块仅 **1 query/object、0 正文字节**，共享预算注册合计 **8 帧**，追加后局部窗口保持身份；既有 container_paragraph_folding 示例仍为 **6 帧/32 字节/一次段尾回调**。完整日志 `/tmp/mdstream-paragraph-discovery-validation.log`；专项日志 `/tmp/mdstream-paragraph-discovery-targeted.log`。

README/TODO 已同步，本单元完成有预算候选发现。自动注册/默认长度策略、逻辑/显示行数前缀、完整字素/对象截断、媒体详情、完整容器方言及终端 UI 保留 TODO；零运行时依赖、禁止 unsafe 和累计复杂度契约保持。

## 2026-10-30 长段落自动注册与长度默认策略交付

新增 `ParagraphFoldPolicy`、`FoldDefaults::paragraphs` 与 `ParagraphAutoFold` / `ParagraphAutoFoldProgress`。以已分类原始源码字节为包含式准入下限，固定源码前缀向下对齐 UTF-8/CRLF，自动注册并收起有剩余后缀的顶层及容器逻辑段落。任务按窗口或 `for_update` 的首个变化块/追加起点到 EOF 发现，包含新段落、流式跨阈值、finish 和合并通知；不按旧正文样式/引用失效范围扫描历史。复用既有发现和注册算法，没有新增解析状态、索引或运行时依赖。

默认策略只初始化新注册。已有手工/自动注册、显式展开及临时预览保持；原始区间重叠或结构交叉计入跳过并继续。`set_paragraph_auto_excluded` 支持阈值前排除/恢复，显式移除段落注册也排除首叶块，避免任务重加；排除状态属于视图、跨 reset/默认值变更保留，O(E) 空间且不积累历史快照。其它视图、父节点及批量意愿独立，手工注册不受排除限制。

普通增长/封口保持身份；晚表头收缩仍有后缀时保留 ID、前缀和用户意愿，即使低于准入阈值。Setext/整表改型或后缀消失沿用 apply 移除。准入时机明确：回退前已准入与回退后才驱动的视图可有不同注册结果，不声称不同通知/预算调度下默认选择相同；相同处理时机的增量窗口与全窗策略逐前缀一致。未完成任务的 `remaining_range` 可与新窗口合并后重建，防止只处理最新通知丢失积压；没有隐式后台队列。

共享 Budget 原子扣发现、两端祖先与最终注册步骤；最后一步同时初始化收起。任务 O(1) 空间、无正文回扫，未完成注册不修改视图，取消/失效保留已完成部分。结果返回累计检查/注册/跳过及当次变化包络；外部文档/视图变更使旧任务失效，新的收起区间按既有规则使测量、检查点与布局任务失效。

新增 16 项验收（14 项 `crates/mdstream-core/tests/paragraph_auto_folding.rs`、2 项复用既有布局模型），覆盖：

- 400 个独立段落包络/400 随机窗口长度模型，240 轮/960 次切换的流式区间与高度模型，72 组自动/手工及大小预算布局对照。
- UTF-8 全切点/逐前缀、CRLF/无换行 EOF/孤立 CR、部分 tab/lazy、分类阈值、极值参数、晚表头收缩与 Setext/整表改型、多视图/排除/移除/预览/批量、未完成窗口跨合并更新恢复、通知重放及旧任务/测量拒绝。
- 千层引用首次注册需 2006 个 query/object 步骤，250 层列表需 1006 步；预算耗尽时未完成注册不可见，已完成部分可见。4096 非段落逐个付费，4096 物理叶块整段只发现一次。
- 4096 历史注册后追加百万字，随后逐字追加 1000 次（顶层/千层引用各一组），每次仅一步发现和一次活动范围刷新；无历史/正文/祖先回扫。隐藏布局只读取前缀与可见后文。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 860 项、`cargo test --doc --workspace` 67 项、`cargo fmt --all -- --check`、`git diff --check`。完整日志 `/tmp/mdstream-paragraph-auto-validation.log`；专项日志 `/tmp/mdstream-paragraph-auto-targeted.log`、`/tmp/mdstream-paragraph-auto-targeted-extra.log`。新 release 示例 `paragraph_auto_folding` 为 8 帧、零正文读取、显式展开跨封口保留；既有 `paragraph_discovery` 与 `container_paragraph_folding` release 示例也通过（后者 6 帧/32 源码字节）。

README/TODO 已同步。本单元完成有预算自动注册与源码字节长度默认策略；逻辑/显示行数前缀、完整字素/对象截断、媒体详情、完整容器方言及终端 UI 继续保留 TODO。零运行时依赖、禁止 unsafe 和累计复杂度契约保持。


## 2026-10-30 可复现语法生成 fuzz 与失败缩减交付

新增 `crates/mdstream-core/tests/support/syntax_fuzz.rs` 和 `crates/mdstream-core/examples/syntax_fuzz.rs`，共享固定版本生成器、失败属性检查、精确输入/分块重放及确定性缩减。四个持久化 u64 种子各 36 例，共 144 例、19,536 个开放前缀；源码/分块/种子用 FNV-1a 指纹 8310356521655359848 锁定。十二类块产生式组合两层行内语法、显式引用/列表、晚定义、未闭合/错误候选、中文/emoji、CRLF 和跨 1 KiB 分类窗口长行，逐字符/随机/语法标记分块均参与验收。独立覆盖检查实际到达 46 个标题、53 张表、707 个容器、191 个链接、26 个行内数学、61 个脚注引用、72 个代码块和 32 个数学块；68 例含 CR、38 例含 Unicode、24 例跨分类窗口。

`crates/mdstream-core/tests/chunking.rs` 的完整快照原样移入 `crates/mdstream-core/tests/support/snapshot.rs`，只放开测试模块可见性，没有删减或规范化旧语义投影。每个开放前缀及 EOF 与一次 append 对照，覆盖块/容器/标题/表格、链接/引用符号、数学/脚注及内容映射、样式与显示映射；独立验证 UTF-8 范围、源码覆盖、父先于子与深度/包含、对象 ID、行/单元格和 payload 顺序、pending 上限、重复 finish 和 EOF 后 append 拒绝。未修改解析核心或官方标准样例，零新增依赖，现有复杂度和预算测试全部保留。

失败文件 `mdstream-syntax-fuzz-v1` 保存种子、case 序号、注入模式、绝对字节切点及原始输入，保留空追加、NUL、CRLF 和未终止 EOF；重放不重新生成。CLI 失败先写原件，再写 `.min`，退出 1；正常退出 0，格式/参数/IO 错误退出 2。缩减按固定次序删除字符区间和分块切点、映射剩余端点，直到单字符/单切点删除均不能保持原失败阶段/属性；结果确定且保持 UTF-8，不承诺全局最小。工具可捕获 panic unwind；对全文重复解析属于测试成本，不声称解析核心的增量预算适用于缩减器，abort/OOM/挂死恢复不在本单元范围。

新增 7 项验收覆盖固定语料、指纹与文件往返、非法切点/版本拒绝、实际语法覆盖，以及三个独立注入故障：

- strong 样式丢失命中 `live.semantic.runs`，144 字节/144 切点缩减为 `**g**\n`（6 字节、0 显式切点）。
- 块区间越界命中 `live.range.utf8`，缩减为 `\n`（1 字节）。
- CRLF 分块时漏传 LF 命中 `live.semantic.source`，缩减为 `\r\n`（2 字节），保留唯一必需切点 1。

三种注入只改变驱动的观察快照或输入适配器，不进入库。测试重复缩减得到同一 case，并逐字符/逐切点验证不可再减；取消注入后同一输入通过。CLI 独立验收额外 256 例（seed 305419896）通过，三个故障及缩减文件重放均按预期退出 1，重复缩减文件 `cmp` 字节一致，去掉注入的重放均退出 0。

项目 devShell 验收通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **867 项**、`cargo test --doc --workspace` **67 项**、`cargo fmt --all -- --check`、`git diff --check`。完整日志 `/tmp/mdstream-syntax-fuzz-validation.log`；专项及旧分块回归 `/tmp/mdstream-syntax-fuzz-targeted.log`；CLI 生成/故障/重放 `/tmp/mdstream-syntax-fuzz-cli.log`，原件及缩减文件在 `/tmp/mdstream-syntax-fuzz-{lose-style,bad-range,split-crlf}.case[.min]`。

README/TODO 已同步。完成可复现语法生成和失败缩减，不将自比较当作 CommonMark/GFM 合规率或覆盖率引导 fuzz；32 位索引执行、更广性能矩阵、其余语法边界及终端 UI 继续保留 TODO。

## 2026-10-30 32 位索引边界验证交付

审计四类稀疏索引的容量倍增、中点、端点/开放哨兵、arena 索引和容量字节乘积，以及高度宽整数转换、预算累计、源码窗口、对象查询和有界语法整数。`HeightIndex::grow_to` 保持长度不超过 `isize::MAX`，`CheckpointIndex::insert` 保持 ID 小于该上限；树容量最多到 `isize::MAX + 1`。内部标注范围来自 String 或已验证的高度端点，折叠树键来自 arena；开放终点 `usize::MAX` 不作为树键。高度先扩为 u128 再乘，定位商/余数分别受子树宽度/u64 高度限制。README 的审计表记录了各项前提及未验证边界。

修复 `HeightIndex::ticket(TileId(usize::MAX))` 在检查结果为 false 时仍因 `then_some` 急切构造而计算 `+1` 的问题。新增回归在原代码上确实触发 debug 溢出（`/tmp/mdstream-index32-reproduction.log`），改成 `then` 延迟构造后正常返回 None；公开 API、有效测量行为和复杂度不变。其余生产代码仅补充边界说明，没有增加运行时依赖或 unsafe，也没有改动解析方言/标准期望。

新增 `crates/mdstream-core/src/index_boundaries.rs` 共 **11 项**独立验收，使用 u128 坐标的线性区间、稀疏写入日志和 BTreeMap 对照实际索引，不复制二叉树实现、不分配巨型源码/叶数组。32 位运行覆盖全部二次幂附近、2,147,483,647 长度与最后有效 tile；64 位同时运行上述边界和本机 `isize::MAX` 边界。覆盖：

- 每个边界的增长、前缀高度、超过 u64 的总高度、`u64::MAX` 单片高度与双向定位；每种长度上限 160 次赋值/置零/失效/重置模型及嵌套遮蔽/基础高度保留。
- 检查点 80 次插入/区间失效与有序前驱模型，最大距离查询、版本令牌/旧测量拒绝、最后有效位置及单步共享预算回收。
- 标注 48 次覆盖的全部常值区间/点样式、链接优先级、最新版本和可见位置；折叠 128 次稀疏登记/关闭与严格 before/offset、开放终点模型。
- 六维极值预算的宽整数原子扣费模型，`u128::MAX` 视口与尾部三片分帧恢复，最大无效 ID/源码范围拒绝及源码窗口裁剪。

四类树的修改和折叠候选查询访问数按字长上限验收，标注遍历按写入数×字长限制访问数；高度每次修改至多分配两条边界路径、reset 仅一次操作。没有通过增大预算或枚举稀疏空洞来使测试通过。

新增仅 `x86_64-linux` 提供的 `index32` devShell，使用现有 flake.lock 的 Rust **1.99.1**、i686 std、Nix i686 GCC/glibc；默认环境保持原工具链，flake.lock 未改。`nix develop .#index32 -c bash scripts/check-index32.sh` 可复跑目标的 check/clippy、debug/release nextest、文档与格式检查，无法链接/执行时直接报错。通过 nextest JSON 查询真实测试路径并读取 ELF 头确认 **ELFCLASS32 / little endian / EM_386**，不是仅模拟 u32 或仅完成交叉编译。

项目 devShell 完整验收通过：x86_64 与 i686 各执行 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **878 项、0 skipped**，两个目标的文档测试各 **67 项**；`cargo fmt --all -- --check`、`nixfmt --check flake.nix`、`bash -n scripts/check-index32.sh`、`git diff --check` 通过。既有标准样例、144 例固定种子语法 fuzz/19,536 开放前缀、百万字/深嵌套及预算回归全部保留并在两种字长执行。日志：

- x86_64 完整验收：`/tmp/mdstream-index32-native-validation.log`。
- i686 完整验收：`/tmp/mdstream-index32-validation.log`；工具链准备/首次目标检查：`/tmp/mdstream-index32-bootstrap.log`。
- 两种字长的 11 项专项：`/tmp/mdstream-index32-targeted-final.log`；ELF 核验：`/tmp/mdstream-index32-elf.log`。

README/TODO 已同步。稀疏索引验证不保证接近 2 GiB 的真实文档/密集 arena 可分配；OOM/分配失败恢复、进程级身份耗尽、u64 revision/序列/计数器绕回及其它 32 位 ABI/无 64 位原子的平台未实测。更广性能矩阵、其余语法边界及终端 UI 继续保留 TODO。

## 2026-10-30 核心性能矩阵交付

- `scaling` 扩展为 17 个固定场景：1/64/4096/整块 append 与 Update 复用、稀疏/密集标记、中文/emoji/组合音标/ZWJ/CRLF 字节流、宽稀疏表、强调/代码/数学晚闭合、晚引用、深 opener、反复 resize 和高度修正。基准与测试共用驱动，解析核心和公共 API 未改动，维持零运行时依赖与禁止 unsafe。
- `scripts/measure-performance.sh` 在项目 devShell 内按新目录保存环境、Git 状态、工具链/flake/源码哈希、Cargo metadata 和逐进程 CSV。输出 schema 2 包括完整解析计数、闭合增量、实际公开 arena/表格 Vec/scratch 容量、吞吐、共享预算消费及 nearest-rank p50/p95/p99/max。测量边界、部分帧与容量遗漏项明确记录在 [性能矩阵](docs/performance.md)。
- 默认三轮 17 × 3 规模共 153 行、4,743 个查询样本已保存在 [原始 CSV](docs/performance-baseline.csv)；跨进程所有非时间字段逐项一致。百万字专项验证晚闭合只消费 2 个新 inline 字节，晚定义仅规范化一个 3 字节标签；稀疏表单元格数为 columns + rows。resize 每次一个索引访问、不改变 arena；高度修正沿对数路径更新，65/130 轮复用固定工作集节点。
- 9 项新验收覆盖完整语义快照及 UTF-8 字节切分、固定指纹/重复输出、三规模 n/log n 计数界、宽表物理存储、百万字迟确认、两次查询/恢复/文本对象消费/测量共享预算、resize/修正和离屏增长、分位数定义。复杂度断言不依赖任何机器时间阈值。
- 验收：项目 devShell `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **887 项通过**、`cargo test --doc --workspace` **67 项通过**、`cargo fmt --all -- --check`；脚本语法、CSV 重复计数/字段及 `git diff --check` 通过。完整日志 `/tmp/mdstream-performance-validation.log`，专项 `/tmp/mdstream-performance-targeted.log`，原始三轮与环境 `/tmp/mdstream-performance-matrix-final/`。

边界：按 Unicode 标量计列的合成消费器不代表实际终端排版；部分高密度帧会耗尽预算，分位数不冒充完整视口完成时延。记录的是指定公开存储的容量与扩容次数，不是总堆/RSS/峰值，解析仍无单次 append 硬时延保证。多平台性能、全部语法组合、完整内存统计、真实缓存依赖链及终端 UI 继续保留；现有独立模型和 32 位验收保持。

## 2026-10-30 存储压缩与缓存回收测量交付

- [x] **可复跑存储实验**：`crates/mdstream-core/examples/storage.rs`、共享 `crates/mdstream-core/tests/support/storage.rs` 和 `scripts/measure-storage.sh` 覆盖三规模纯文本/密集语法/容器、语义窗口与整篇栈、视口输出新建/复用、三种检查点 payload 表示及稀疏位置的 filled/clear/drain/refill。脚本保存环境/源码哈希/权威 target 目录，拒绝覆盖和非法参数，核对重复非时间指标；[方法与结果](docs/storage.md)、[原始 CSV](docs/storage-baseline.csv) 和[环境](docs/storage-environment.txt) 随库保存。
- [x] **查询 scratch 可观测性**：`Runs::scratch_stats()` O(1) 返回待处理帧、实际容量及分配/扩容调用，不读取正文或修改查询结果。64 次 dense 窗口查询共 192 次栈分配/扩容、单次峰值 1,152 字节；plain 为 64 次/288 字节。百万字与全篇查询证明栈按深度增长；不把此单栈统计当作全部临时分配。现有 `viewport_into` 预热后 64 次查询容量增长为零，新建 Vec 为 64，结果相同。
- [x] **有证据的最小压缩**：检查点子索引改用下标 + 1 的 `Option<NonZeroUsize>`，保留树形、身份、版本和所有公开操作语义。x86_64 的 Vec payload 节点从 88 到 72 字节，2,048 个稀疏状态 arena 从 2,883,584 到 2,359,296 字节（下降 18.2%）；内联数组/Box 节点也各减少 16 字节。前后一轮/三轮共 2,928 条指标、324 个阶段，60 个尺寸/容量值改变，其余非时间字段（含访问数、节点数、回收帧数和校验值）全部相同，不声称耗时提速。
- [x] **回收收益与边界**：clear 一次访问、不析构；2,048 个 Vec/Box 状态经分帧 drain 及调用方显式丢弃释放 524,288 字节外部 payload。31 节点/帧时，相邻/稀疏树分别扫描 4,095/17,444 节点、133/563 帧；arena 不缩容、相同位置 refill 不新增 arena 分配。独立二分区间集合/深度模型、BTreeMap 前驱及析构计数验证零/非对称预算、部分回收、旧 token 拒绝与 32 次 clear/refill，不把 Budget 当作用户析构时间上界。

验收：新增 `crates/mdstream-core/tests/storage.rs` 7 项测试；项目 devShell 的 check、clippy、debug/release nextest 各 894 项、67 项文档测试、fmt、脚本语法、原始 CSV/指纹/源码哈希复核及 diff 检查通过。同一套 check、clippy、debug/release 各 894 项与 67 项文档测试在真实 i686 目标通过。三轮每轮 81 阶段的全部非时间字段一致；CLI 非法参数和拒绝覆盖检查通过。

完整日志：`/tmp/mdstream-storage-validation.log`、`/tmp/mdstream-storage-index32.log`；专项与三轮驱动：`/tmp/mdstream-storage-targeted-final.log`；原始目录：`/tmp/mdstream-storage-measurements/`；独立前后/哈希核验：`/tmp/mdstream-storage-artifacts.log`；CLI：`/tmp/mdstream-storage-cli.log`。查询 scratch 复用有分配收益线索，语义/修订拆分和大型状态叶存储也有容量优化空间，但尚无实现与端到端对照；历史树节点回收、完整堆/RSS 峰值、其它 ABI/跨机器性能及终端 UI 继续保留。保持零运行时依赖与禁止 unsafe。

## 2026-10-30 查询 scratch 复用交付

- [x] **安全可归还查询栈**：`QueryScratch` 与 `runs_in_with_scratch` / `display_runs_in_with_scratch` 及对应 tile 入口保留标注遍历 Vec。两个带 scratch 的迭代器独占借用调用方缓冲，drop 时清空待消费帧并归还容量，包含提前退出和 panic unwind；返回文本只借用 Document，可在 scratch 再次使用时继续读取。跨文档、追加、迟确认后重新建立查询，不保留旧遍历语义；空范围不分配，无效范围不改缓冲，调用方可 drop/default 释放容量。原有查询 API、遍历算法、样式/源码映射及访问计数保持。
- [x] **预算游标自动复用**：`DisplayCursor` 跨步骤保留自己的栈，原身份/revision 校验、检查窗口、六维共享预算和源码位置恢复不变；预算不足、错误和完成状态不分配、不扣费。`QueryScratch::stats()` 与游标统计累计分配/扩容调用，迭代器 `scratch_stats()` 统计本次查询；全部只覆盖遍历 Vec，无全局缓存、线程局部缓存、新依赖或 unsafe。
- [x] **可复跑分配对照**：扩展原 storage 驱动，三规模/三轮共 3,303 条指标、378 个阶段，非时间字段逐项相同。65,536 参数、64 次窗口查询：plain 语义/显示新栈各 64 次分配，dense/containers 各 192 次，预热后全部为 0；预热分别 1/3/3 次，容量 288/1,152/1,152 字节。对应预算游标 plain 256→64、dense 21,072→128、containers 7,344→128，包含每个新窗口的冷启动。输出、访问数、容量及预算消费对照相同；旧压缩后基线的全部非时间指标不变。原始 CSV、环境和限制见 [测量记录](docs/storage.md#查询-scratch-复用实验)。时间记录与回归并行，不据此声称吞吐或真实终端时延改善。
- [x] **模型和回归**：新增 11 项测试、3 项文档样例（含 2 项 compile-fail）。400 组独立字节样式/1,200 窗口模型、字面显示预期、全部 UTF-8 范围/切点、实体/代码 CRLF/部分 tab/表格和数学映射、双查询与保留结果、跨文档/流式迟确认/异常退出通过。百万字/千层引用/密集语法及稀疏最大字长树验证 O(log n) 栈和窗口访问界；多种非对称预算、共享依赖消费、旧版本/文档拒绝与空查询验证原子性。既有分块、搜索、摘要/预览、导航、折叠与预算回归全部保留。

验收：x86_64 和实际 i686 的项目 devShell check、clippy、debug/release nextest **各 905 项通过、0 skipped**；文档测试各 **70 项**（68 运行、2 compile-fail），fmt、脚本语法、CSV/哈希复核与 diff 检查通过。完整日志 `/tmp/mdstream-query-scratch-validation.log`、`/tmp/mdstream-query-scratch-index32.log`；专项 `/tmp/mdstream-query-scratch-targeted.log`、`/tmp/mdstream-query-scratch-targeted-extra.log`；三轮记录 `/tmp/mdstream-query-scratch-measurements/`，跨轮/旧基线/源码校验 `/tmp/mdstream-query-scratch-artifacts.log`。

README/TODO 已同步。scratch 保留历史峰值容量，更深工作集可能继续增长；每个活动游标有独立栈，重建需重新预热。其它消费者的瞬时分配、语义/修订存储拆分、历史树节点回收、完整堆/RSS 峰值、多平台性能及终端 UI 继续保留后续。

## 2026-10-30 检查点历史树压缩交付

针对存储实验中 drain 后仍保留历史节点/arena 的缺口，交付 `CheckpointIndex::into_compaction`、`CheckpointCompaction<S>` 和容量/工作统计。任务拥有缓存，DFS 按有效计数跳过失效子树，把有效叶状态和原版本迁至新树，再按预算逐节点退役旧 arena；失效 payload 返回调用方，最后一个节点移出后直接释放长度为零的旧 Vec。无 `Clone`、新依赖或 unsafe，不重新读取文档，不在结束或任务 Drop 时自动执行整树压缩。

接口与边界：

- 每步原子扣 1 object + 1 query，与整帧其它工作共用 Budget；有效叶附带两次 O(log T) 树操作，工作访问单独计数。N 个历史节点、L 个有效状态的总成本 O(N + L log T)、scratch O(log T)；旧插入/查询/失效复杂度及常规 Vec 摊销增长不变。
- 暂停期间 `previous/get/len/token/validates` 查询迁移前后两树的并集，有效 token、serial、文档身份及已消费 revision 原样保留，失效状态不复活。完成后恢复正常查询、按原依赖验证旧测量 ticket；晚更新按既有顺序检查。
- 未完成 `finish` 返回拥有进度的原任务；零或非对称额度不推进，Covered 重放零工作。失效状态只转移所有权，析构由调用方决定。丢弃任务等同于丢弃缓存，保持任务即可暂停；不会在 Drop 中无预算完成迁移。
- 维护期间不能写缓存，完成后继续消费暂存的有序/合并 Update，漏通知需要 rebuild；旧 drain arena 游标须置零。临时峰值包括新旧 arena，旧分配在最后一个节点退役时归还；不承诺每种状态分布都节省容量，没有严格内存上限或分配器/析构的硬实时保证。

实现、示例和实验分别见 `crates/mdstream-core/src/checkpoints.rs`、[README](README.md#检查点历史树压缩) 与 [存储实验](docs/storage.md#检查点历史树压缩实验)。沿用共享 storage 驱动和三轮脚本，新增内联/Box/Vec 相邻与 Vec 稀疏四组，每组填充保留 1/16、压缩和回填三个阶段。仓库保存 [4,563 条指标/486 阶段原始记录](docs/checkpoint-compaction-baseline.csv) 及 [环境/源码哈希](docs/checkpoint-compaction-environment.txt)；各轮非时间字段一致，前一查询复用基线的 975 个非时间字段逐项不变。

2,048 个状态保留 128 个：相邻 Vec arena 294,912 → 73,728 字节（下降 75%），稀疏 Vec arena 2,359,296 → 147,456 字节（下降 93.75%）；内联/Box 相邻同为 75%。旧 arena 最终容量为零，外置 payload 各释放 491,520 字节、保留 32,768 字节。相邻/稀疏分别 5,374 / 19,548 步、174 / 631 帧、9,723 / 25,680 次树访问，scratch 均 128 字节。回填恢复原节点数/arena 容量但增加分配次数，说明压缩适用于历史工作集收缩，固定工作集可继续使用原 drain 复用。测量记录的是明确 arena/Vec 容量和已知 payload，不冒充进程或总堆峰值，不以时间阈值证明复杂度。

新增 11 项验收：

- BTreeMap 与 u128 二分区间独立模型覆盖每个暂停位置的前驱/精确查询和最终节点数；100 轮随机历史、9,600 次操作、900,000 个完成后查询，以及 32 次不同位置回填后的节点回收和旧 token 拒绝。
- 惰性祖先失效、单路径恢复、空/全有效树、反向插入版本、多轮压缩、异缓存 token、部分 drain 前后组合、有效及失效测量 ticket、文档更新/漏通知和错误文档。
- 共享预消费、零/非对称预算、逐步推进、未完成 finish、完成重放；4,096 个非 Clone 稀疏状态验证零库内用户析构、返回值所有权与容量释放，两阶段中途丢弃验证每个状态恰好析构一次。
- 接近 `isize::MAX` 的稀疏树、最大合法 tile 与非法极值、字长级 scratch/每步访问界；实机 i686 和 x86_64 均执行。共享测量另按 3 种状态数 × 2 种间隔 × 3 种预算独立核对节点、容量、返回状态、帧数和回填校验值。
- README 新增 1 个调用样例和 1 个所有权编译失败样例；已有语法、预算、布局、随机模型和流式回归全部保留。

验收：项目 devShell `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **916/916**，文档测试 **72/72**，`cargo fmt --all -- --check`、脚本语法与 diff 检查通过；`nix develop .#index32 -c bash scripts/check-index32.sh` 实际 i686 的对应 check/clippy、debug/release 各 **916/916**、文档 **72/72** 及格式检查通过。三轮 storage release 驱动、CSV/旧基线/源码哈希复核通过。

日志：`/tmp/mdstream-checkpoint-compaction-validation.log`、`/tmp/mdstream-checkpoint-compaction-index32.log`；专项 `...-targeted-final.log`；原始测量 `/tmp/mdstream-checkpoint-compaction-measurements/`；复核 `/tmp/mdstream-checkpoint-compaction-artifacts.log`。保留并发可写压缩、渐进释放旧 Vec 分配、硬内存上限、总堆/OOM、序号耗尽及其它 ABI；语义/修订拆分和终端 UI 未扩展。

## 2026-10-31 TODO 归档

以下已完成且不再关联待办实现的独立条目从 [TODO](TODO.md) 移入；实现、验收条件和证据仍以各条链接的交付记录为准。

- [x] **可复现语法生成 fuzz 与失败缩减**：四个固定种子、版本/指纹锁定 144 例和 19,536 个开放前缀；复用完整语义快照并独立检查源码/对象/容器不变量，精确输入/分块文件重放及保持失败属性的确定性字符删除/分块合并。三个注入故障验证检测、缩减、重放和取消注入恢复，见 [生成 fuzz 交付](DONE.md#2026-10-30-可复现语法生成-fuzz-与失败缩减交付)。不是标准兼容率或覆盖率引导引擎；既有标准样例、随机模型与复杂度验收继续保留。
- [x] **32 位索引边界验证**：四类稀疏索引、宽高度/定位转换、预算和源码/对象 ID 的 11 项独立边界模型；i686 devShell 与可复跑检查脚本，32/64 位 debug/release 全套回归，修复最大无效 TileId 的提前加法溢出，见 [32 位验收交付](DONE.md#2026-10-30-32-位索引边界验证交付)。不以稀疏上限代表巨型文档可分配；接近 2 GiB 的真实存储、OOM 恢复、身份/版本计数耗尽与其它 32 位 ABI 尚未实测，核心性能矩阵见 [性能交付](DONE.md#2026-10-30-核心性能矩阵交付)，多平台性能与完整内存峰值继续保留。
- [x] **核心性能矩阵**：固定 17 场景覆盖 append 粒度、稀疏/密集标记、中文/emoji、超宽稀疏表、延迟闭合、反复 resize 与高度修正；共享 release 驱动及 9 项计数验收，记录操作/实际容量、吞吐与有预算帧分位数。三轮 153 行原始记录、环境与测量边界见 [性能矩阵](docs/performance.md)，验收见 [性能交付](DONE.md#2026-10-30-核心性能矩阵交付)。不是总堆内存/RSS 峰值或真实终端排版测量；多平台性能、完整分配统计及其它语法组合继续保留。
- [x] **存储压缩与缓存回收收益测量**：三规模语义/查询栈/视口复用及检查点填充、失效、回收、再填充实验；独立节点/预算/析构模型，检查点非零子索引将本机 Vec 状态节点从 88 压到 72 字节。方法、前后一轮/三轮原始记录及优化取舍见 [存储实验](docs/storage.md)，验收见 [存储交付](DONE.md#2026-10-30-存储压缩与缓存回收测量交付)。查询 scratch 复用及历史树压缩分别见 [复用交付](DONE.md#2026-10-30-查询-scratch-复用交付) 与 [压缩交付](DONE.md#2026-10-30-检查点历史树压缩交付)；保留语义/修订存储拆分及完整内存剖析；不宣称总堆峰值或硬内存上限。
- [x] **语义/显示查询 scratch 复用**：`QueryScratch` 与四个范围/tile 查询入口独占借用可归还栈，文本维持文档借用；提前退出/unwind、跨文档/追加及空/非法范围保持安全。`DisplayCursor` 自动保留栈容量，原共享预算和版本拒绝不变；预热窗口零新增栈分配，结果/映射、访问数、预算、百万字/千层及极限字长模型见 [复用交付](DONE.md#2026-10-30-查询-scratch-复用交付) 与 [存储实验](docs/storage.md#查询-scratch-复用实验)。保留容量可由调用方丢弃；其它消费者、全局分配剖析和硬内存上限不在此项范围。
- [x] **检查点缓存历史树节点回收**：`CheckpointCompaction` 接管缓存，按共享对象/查询预算迁移有效状态并逐节点退役旧 arena；暂停期间前驱/精确查询和 token 保持，失效 payload 返回调用方，有效身份/版本及更新顺序不变。容量/预算/析构、100 轮随机模型、32 次回填、极限字长及测量 ticket 验收见 [压缩交付](DONE.md#2026-10-30-检查点历史树压缩交付) 与 [存储实验](docs/storage.md#检查点历史树压缩实验)。保留暂停期间可写协议、硬内存上限和总堆剖析：当前任务完成后才可继续写入，峰值包含新旧 arena，不承诺所有输入均缩小容量。
- [x] **CommonMark ATX 章节独立评分**：固定 0.31.2 全部 18 个 ATX 样例（62–79）的官方输入和 HTML，当前 17 支持、1 刻意差异、0 未支持（空白修复后）；来源摘要校验、离线独立报告、全切点/逐字符输入及投影敏感性验收见 [ATX 评分交付](DONE.md#2026-10-30-commonmark-atx-标准样例评分交付)。评分完成不代表 18 项语义全部支持。
- [x] **ATX 标题尾部空白（ATX whitespace）**：原始首尾 space/tab 及关闭井号前空白在解析核心裁剪，短/长标题共用固定端点流式摘要；样例 67、71–73 改为支持，官方输入/期望与投影空白保留规则不变。源码映射、预算/失效、分块/1600 组模型、千层/百万字及历史对象验收见 [ATX 空白交付](DONE.md#2026-10-30-atx-原始空白与关闭尾缀交付)。LF/CRLF 方言、分类前缀上限及其它 CommonMark 差异继续保留。
- [x] **CommonMark Setext 章节独立评分**：固定 0.31.2 全部 27 个 Setext 样例（80–106）的官方输入和 HTML，当前 25 支持、2 刻意差异、0 未支持（空白修复后）；来源摘要校验、离线独立报告、533 个切点/506 个逐字符前缀及投影敏感性验收见 [Setext 评分交付](DONE.md#2026-10-30-commonmark-setext-标准样例评分交付)。评分完成不代表全部语义支持。
- [x] **Setext 标题首尾空白**：官方样例 82、84、89 已由原文端点摘要与迟确认隐藏修复，官方输入/期望和评分投影不变；保持多行、源码映射、稳定对象、分块及复杂度。1200 组模型、千层/百万字、4096 历史对象/叶块、共享预算和失效验收见 [Setext 空白交付](DONE.md#2026-10-30-setext-原始首尾空白交付)。85、87 的缩进代码差异继续沿用已记录方言，完整缩进/空行和分类上限另行推进。
- [x] **GFM 表格章节独立评分**：固定 0.29-gfm 全部 8 个表格样例（198–205）的官方输入和 HTML，当前 8 支持、0 刻意差异、0 未支持；来源摘要校验、离线独立报告、361 个切点/353 个逐字符前缀及投影敏感性验收见 [GFM 表格评分交付](DONE.md#2026-10-30-gfm-表格标准样例评分交付)。仅说明这 8 个短样例通过，不代表完整 GFM 兼容或实际表格排版已完成。
- [x] **GFM 任务列表章节独立评分**：固定 0.29-gfm 全部 2 个任务列表样例（279–280）的官方输入和 HTML，当前 2 支持、0 刻意差异、0 未支持；来源摘要校验、离线独立报告、66 个切点/64 个逐字符前缀和投影敏感性/边界拒绝验收见 [GFM 任务列表评分交付](DONE.md#2026-10-30-gfm-任务列表标准样例评分交付)。仅涵盖这两个短样例的勾选状态、兄弟及嵌套结构，不代表完整任务语法、任意嵌套或交互绘制已完成。

## 2026-11-03 CommonMark Paragraphs 标准样例评分交付

- [x] **完整官方章节与离线重建**：`crates/mdstream-core/tests/fixtures/commonmark_paragraphs.rs` 固定 CommonMark 0.31.2 Paragraphs 全部 8 例（219–226），逐字保存官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属和完整 `spec.json` SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。`scripts/generate_commonmark_paragraphs.py` 只读取本地官方来源，校验摘要及完整章节成员后生成或 `--check`；改动来源会被拒绝。构建/测试不联网，零新增依赖。
- [x] **独立分类与固定观察**：`crates/mdstream-core/tests/commonmark_paragraphs.rs` 报告 **4 支持（219–221、225）、1 刻意差异（223）、3 未支持（222、224、226）**。223 沿用已记录的缩进代码可中断段落方言；222/224 保留段落行首缩进，226 的内部硬换行正确但段落末尾空格未裁剪。非支持项同时断言与官方不同、与单独固定的观察相同；改善和回归都须复核分类，不用当前输出代替官方期望。
- [x] **保留评分语义**：窄投影读取公开块和显示片段，核对段落/代码类型、顺序、边界、可见文字、软/硬换行和代码末尾 LF。仅省略结构空块和段落的一个物理末尾 LF，保留所有缩进、内部空白与尾空格，不 trim、不重解析 Markdown、不按样例编号修正输出。容器、标题、行内代码/链接/数学等超出范围的结构和样式明确拒绝；投影不是库 HTML 渲染器。
- [x] **流式和适配器验收**：新增 4 项测试，覆盖独立评分、全部 **160 个单切点及 152 个逐字符前缀**、空白/强调/换行/块边界敏感性、范围外输入拒绝。快照核对块身份/范围、显示映射/样式和 pending；EOF 后独立对照官方期望或固定分类观察。8 例均为 ASCII，最长 64 字节，不代表 Unicode、长输入、容器段落或其余章节已评分。

独立报告：`nix develop -c cargo nextest run --workspace --test commonmark_paragraphs --no-fail-fast --success-output immediate`；夹具复核：`python3 scripts/generate_commonmark_paragraphs.py /path/to/spec.json --check`。本单元只扩充章节评分，解析核心及复杂度/预算契约未改动，保持零运行时依赖及禁止 unsafe。TODO 保留宽泛标准覆盖项，并新增段落空白缺口；完整缩进规则、其余 CommonMark/GFM 范围及 UI 继续待办。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **920 项**、`cargo test --doc --workspace` **72 项（69 + 3）**、`cargo fmt --all -- --check`、夹具逐字重建/篡改拒绝及 `git diff --check`。专项评分和切点日志 `/tmp/mdtui3-goal-run/paragraphs-targeted.log`；全量回归日志 `/tmp/mdtui3-goal-run/paragraphs-validation.log`；来源核验日志 `/tmp/mdtui3-goal-run/paragraphs-fixture-check.log`。

## 2026-11-03 顶层段落行首缩进交付

- [x] **修复 222/224，更新真实评分**：已分类顶层段落每个物理行开头的 1–3 个 ASCII 空格隐藏后再进入行内解析；源码、块身份/范围及原始映射不变。Paragraphs 当前评分为 **6 支持（219–222、224–225）、1 刻意差异（223）、1 未支持（226）**；上一节的 4/1/3 为首次评分历史。官方夹具逐字保留，评分投影未增加 trim 或按编号修正，相关投影敏感性测试同步核对新显示结果。
- [x] **明确方言与流式边界**：四空格代码仍可中断段落；容器、tab/混合缩进和段末空格保持已有规则。短尾部 LF/EOF 分类前原文可见；长行在有界前瞻提交后立即处理缩进。早管道候选在失败后首次消费时隐藏缩进，长候选的已消费前缀不重放。跨行代码/强调/链接/数学及后续 Setext 继承段落缩进语义；数学 payload 复用已有 gap 索引省略缩进，raw content 保留原文。
- [x] **复杂度与预算**：每个物理行首最多检查三个空格和一个后继字节，隐藏空格计入 `Work.inline_bytes`，增加至多一个有界隐藏区间，无新增解析状态及历史/正文/祖先回扫；累计 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 保持。百万字后逐字追加 1000 次，inline 工作恰为 1000，分类/容器/区间树访问均不增加；4096 个缩进行的工作量有明确上界。隐藏缩进和 CR 仍按显示窗口及前瞻计费，零预算不移动，6 bytes / 1 query / 1 object 持续推进。
- [x] **专项边界验收**：新增 `tests/paragraph_indent.rs` 5 项，覆盖 UTF-8/CRLF/EOF、13 组短/长输入的全切点与逐字符前缀及完整元数据快照、表头成功/失败、跨行对象、原始源码/隐藏映射、tile 与预算读取、旧任务失效、百万字/4096 行复杂度。官方 8 例继续通过 160 切点/152 前缀；Setext、代码空白及表格投影中的旧顶层缩进预期按新方言更新。

项目 devShell 验收：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **925 项**、`cargo test --doc --workspace` **72 项（69 + 3）**、`cargo fmt --all -- --check` 全通过。专项日志 `/tmp/mdtui3-goal-run/paragraph-indent-targeted.log`；完整验收 `/tmp/mdtui3-goal-run/paragraph-indent-validation.log`；固定官方来源运行 `scripts/generate_commonmark_paragraphs.py --check` 通过，日志 `/tmp/mdtui3-goal-run/paragraph-indent-fixture-check.log`。README/TODO 已记录新评分与适用边界；226、容器/完整缩进规则和宽泛标准覆盖继续未完成。

## 2026-11-03 顶层段落末尾空白交付

- [x] **修复 226，保留官方判据**：顶层段落关闭时隐藏最后一个物理 LF/CRLF（或无换行 EOF）之前的原始 space/tab 尾部。Paragraphs 219–226 的当前评分为 **7 支持、1 刻意差异（223）、0 未支持**；以上 4/1/3 和 6/1/1 保留为历史交付记录。官方 Markdown/HTML 及投影逻辑不变，未在投影中 trim。相邻 ATX/Setext/表格投影与预算读取测试的旧段尾预期更新，并保留内部空白及实体生成尾空格的敏感性。
- [x] **流式、硬换行及源码边界**：开放尾部保持暂定可见，续行提交才确认已有软/硬换行；EOF、空行或后继块关闭段落时仅隐藏原始尾部，末尾物理 LF 无换行语义标记，字面反斜杠保留。原文、块范围/身份、显示源码映射不变；隐藏区间产生可见性与 dirty 通知，旧 tile key 和 revision 绑定游标失效。实体产生的空格及已闭合代码/数学/链接内部空白保留，未闭合候选按原文端点裁剪；孤立 CR、Unicode 空白、form feed 不裁剪。容器段末、内部 tab 换行、完整缩进与 223 方言继续留在 TODO。
- [x] **单遍与表头状态移交**：复用既有行内扫描的原始端点，关闭无需回扫尾部或正文，仅一次 O(log n) 隐藏标记；晚表头保存旧段尾的 O(1) 范围，成功时在移交行内状态后提交，失败时保留续行语义。保持累计 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe。百万空格后逐字追加 1000 次只有 1000 inline 字节工作，分类/容器/注解树不增长；EOF 无 inline/分类回扫且树访问少于 300，4096 次段落关闭满足计数上界。
- [x] **6 项专项回归**：`tests/paragraph_tail.rs` 覆盖 UTF-8/CRLF/EOF、硬/软换行、实体/代码/数学/链接、失败候选、代码/容器/Setext 边界、早/晚表头成功与失败、全切点/逐前缀完整快照、跨 tile 映射、零预算暂停及 6 bytes / 1 query / 1 object 推进、百万空格及 4096 段关闭。修复前 226 的显示缺口由同一测试复现，日志 `/tmp/mdtui3-goal-run/paragraph-tail-repro.log`；专项通过日志 `/tmp/mdtui3-goal-run/paragraph-tail-targeted.log`。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **931 项**、`cargo test --doc --workspace` **72 项（69 + 3）**、`cargo fmt --all -- --check`、官方 Paragraphs 夹具逐字重建 `--check`、`git diff --check`。官方 8 例的 **160 个切点 / 152 个逐字符前缀**继续通过。完整日志 `/tmp/mdtui3-goal-run/paragraph-tail-validation.log`。README/TODO 已同步真实评分与开放尾部契约，宽泛标准覆盖及完整空白/容器规则未勾选。

## 2026-11-03 CommonMark Hard line breaks 标准样例评分交付

- [x] **完整章节与独立来源**：固定 CommonMark 0.31.2 Hard line breaks 633–647 全部 15 例，`tests/fixtures/commonmark_hard_breaks.rs` 逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整来源 SHA-256。`scripts/generate_commonmark_hard_breaks.py` 从本地固定来源生成或 `--check`，先核对摘要和章节成员；重新下载官方数据摘要一致、夹具逐字重建及篡改拒绝均通过，构建与测试不联网。
- [x] **真实分类与范围**：`tests/commonmark_hard_breaks.rs` 报告 **11 支持（633–635、638–641、644–647）、2 刻意差异（636–637）、2 未支持（642–643）**。支持项覆盖空格/反斜线硬换行、强调内换行、代码 span 的 LF 归一化及段落/ATX 末尾边界；636–637 沿用已记录的缩进代码中断段落方言，642–643 的原始 HTML 属需求范围外。四个非支持样例保留独立当前观察，断言与官方不同且行为未变；不替换官方期望、不删样例、不把未支持算成符合。
- [x] **流式与投影验证**：4 项测试包括逐例评分、全部 **199 个单切点 / 184 个逐字符前缀**、投影敏感性和边界拒绝。快照核对块/标题身份与范围、显示映射/样式及 pending；EOF 后仍对照官方期望或固定分类差异。投影仅读取公开语义，保留空格/tab、代码 LF、强调/代码 span 和软/硬换行，只省略结构空块与无行内换行标记的末尾物理 LF；不 trim、不重解析 Markdown、不按编号调整输出，拒绝未评分结构/样式。
- [x] **交付边界**：15 例均为 ASCII，最长 21 字节，不将该评分推广为 Unicode、HTML、完整缩进/容器、其它章节或长输入复杂度/预算验收。本单元只新增离线夹具、生成器、评分测试及文档，解析核心和运行时依赖未改；README/TODO 已同步分类，宽泛标准覆盖与完整缩进规则保持待办。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **935 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、官方夹具 `--check` 和 `git diff --check`。专项日志 `/tmp/mdtui3-goal-run/hard-breaks-targeted.log`；来源核验 `/tmp/mdtui3-goal-run/hard-breaks-fixture-check.log`；完整验收 `/tmp/mdtui3-goal-run/hard-breaks-validation.log`，可复跑命令脚本 `/tmp/mdtui3-goal-run/hard-breaks-validate.sh`。

## 2026-11-03 CommonMark Soft line breaks 标准样例评分交付

- [x] **完整章节与独立来源**：固定 CommonMark 0.31.2 Soft line breaks 648–649 全部 2 例；`tests/fixtures/commonmark_soft_breaks.rs` 逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。`scripts/generate_commonmark_soft_breaks.py` 校验本地固定来源摘要和章节成员后生成或 `--check`；重新下载的官方数据摘要一致，夹具逐字重建、篡改来源拒绝及生成器语法检查通过。构建与测试不联网，零新增依赖。
- [x] **真实评分与边界**：**2 支持、0 刻意差异、0 未支持**，验证普通软换行及换行两侧空格的官方结果。两例仅为 8、10 字节 ASCII 输入；不据此声称容器、tab/混合缩进、CR/CRLF、Unicode、长输入复杂度和预算边界全部符合，也不扩充解析方言。
- [x] **流式及语义验证**：4 项测试覆盖逐例报告、全部 **20 个单切点 / 18 个逐字符前缀**、投影敏感性及范围拒绝。快照比较块身份/范围、显示映射/样式与 pending；EOF 后同时对照官方 HTML 和内部 LF 的 `SOFT_BREAK`、原始源码区间、无替代内容及无 `HARD_BREAK`，不会将纯文本 LF 当作正确语义标记。投影仅读取公开段落/显示接口，保留内部空白与软/硬换行区别，只省略结构空块和无行内换行标记的段末物理 LF；不 trim、不重解析 Markdown、不按样例编号调整输出，拒绝未评分结构/样式。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_soft_breaks --no-fail-fast --success-output immediate`；来源复核：`python3 scripts/generate_commonmark_soft_breaks.py /path/to/spec.json --check`。README/TODO 已同步覆盖范围；宽泛标准覆盖、完整空白/容器规则及其它章节继续待办。本单元未修改解析核心、运行时依赖或复杂度/预算契约。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **939 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、官方夹具 `--check` 和 `git diff --check`。专项日志 `/tmp/mdtui3-goal-run/soft-breaks-targeted.log`；来源核验 `/tmp/mdtui3-goal-run/soft-breaks-fixture-check.log`；完整验收 `/tmp/mdtui3-goal-run/soft-breaks-validation.log`；可复跑脚本 `/tmp/mdtui3-goal-run/soft-breaks-validate.sh`。

## 2026-11-03 内部换行混合尾空白交付

- [x] **范围与流式方言**：修复 TODO 的普通内部 tab 换行缺口；同一行内序列的 LF/CRLF 确认为内部换行后，隐藏此前连续的原始 space/tab。顶层及已识别同作用域显式/lazy 容器续行、Setext 正文沿用该规则；紧邻换行的至少两个 ASCII 空格或反斜线仍决定硬换行，tab 不转换成硬换行空格。下一行确认前保持暂定可见，晚表头成功丢弃待定换行、失败提交；原始源码、对象与块身份及映射保持。
- [x] **算法与预算**：`Inline` 仅增加一个 O(1) 混合空白尾部起点，现有单遍扫描同时维护硬换行所需的连续 ASCII 空格起点；CRLF 保留、孤立 CR 重置。换行确认扩大原有隐藏区间，无尾部逆扫、正文重放或祖先访问；累计 O(n log n)、空间 O(n)，零新增依赖、无 unsafe。显示仍通过现有共享 Budget 按原始字节计费，隐藏区间改变会失效旧缓存和 revision 绑定任务。
- [x] **独立回归**：新增 `tests/internal_tab_breaks.rs` 六项测试；修复前 5 失败/1 通过，修复后全部通过。包含 1–6 字节 space/tab 全组合 × 顶层/引用/列表/引用 lazy × LF/CRLF 的 **1008 组模型**，UTF-8 全切点/逐前缀、早/晚表头成功与失败、Setext、行内字面内容、实体、孤立 CR、流式缓存失效、tile/源码映射及零预算暂停/6 字节预算推进。百万字混合尾部后 1000 次单字节追加恰好增加 1000 `inline_bytes`，分类/容器/树计数不增长；换行关闭仅消费新增 4 字节，另验证 4096 个内部换行。
- [x] **已有观察与剩余范围**：Setext 空白测试中的内部 tab、拒绝下划线和过长下划线三处显示期望按新方言更新，原始包络断言保持；九项 Setext 空白测试通过。官方 Paragraphs/Hard line breaks/Soft line breaks 输入、期望和评分保持不变。容器段末裁剪、行首 tab/混合缩进、不透明失败候选的完整回退和完整缩进规则仍待办；不对显示输出 trim，不将宽泛空白或标准覆盖 TODO 标为完成。README/TODO 已同步。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **945 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、Paragraphs/Hard line breaks/Soft line breaks 官方夹具 `--check` 及 `git diff --check`。修复前复现 `/tmp/mdtui3-goal-run/internal-tab-breaks-repro.log`；专项 `/tmp/mdtui3-goal-run/internal-tab-breaks-targeted.log`；Setext 回归 `/tmp/mdtui3-goal-run/internal-tab-breaks-setext.log`；完整验收 `/tmp/mdtui3-goal-run/internal-tab-breaks-validation.log`；复跑脚本 `/tmp/mdtui3-goal-run/internal-tab-breaks-validate.sh`。

## 2026-11-03 容器段落末尾空白交付

- [x] **逻辑段落关闭与方言**：已识别引用、列表（含任务项）及嵌套容器普通段落在 EOF、空行、兄弟/作用域变化或新块关闭时，隐藏原始 space/tab 尾部；显式和 lazy 续行沿用内部软硬换行。物理叶块封口不提前裁剪开放尾部，末尾 LF/CRLF 的 LF 保留且无换行语义标记。源码、块/对象身份及映射不变；代码块、实体生成空白和已闭合行内内容保持，Setext 首叶提升后仍走专属裁剪。
- [x] **端点复用与表头移交**：关闭时以逻辑首叶类型区分普通段落与多行 Setext，复用 `Inline::paragraph_tail` 的原始端点；早/晚表头沿用已有 `HeaderCandidate::paragraph_tail` 保存前一段尾部，成功时提交，失败时保持普通续行规则。容器表头移交后不把新表头区间误作旧段尾隐藏。没有新增持久状态、尾部逆扫、旧正文/叶块/祖先回访，累计 O(n log n)、空间 O(n)、零运行时依赖与禁止 unsafe 保持。
- [x] **六项专项回归**：新增 `tests/container_tail.rs`；修复前 5 项失败/1 项通过，修复后 6 项通过。覆盖 **1512 组** space/tab 组合 × 引用/列表/混合嵌套 × EOF/LF/CRLF 模型、UTF-8 全切点与逐前缀快照、显式/lazy/部分 tab 续行、任务项、空行与兄弟、Setext、早/晚表头成功/失败及首单元格可见内容。另验证块/链接/数学对象稳定、旧 tile key 和游标失效、原始映射、跨 tile 读取、零预算不移动及 6 bytes / 1 query / 1 object 推进。千层容器的百万字尾部后 1000 次逐字追加恰好增加 1000 `inline_bytes`，分类/容器/树计数不增长；EOF 不重新消费正文或前缀，4096 个兄弟段落关闭满足计数上界。
- [x] **准确更新旧观察与剩余范围**：GFM 任务投影敏感性、顶层段尾和内部 tab 测试各一处旧容器尾空白显示断言随新方言更新，内部空白和实体生成尾空格的敏感性保留。官方样例输入、期望、分类与评分投影逻辑未改；README/TODO 已记录范围。tab/混合缩进、不透明失败候选的完整内部回退、完整容器匹配/缩进以及宽泛标准覆盖继续待办。

修复前日志 `/tmp/mdtui3-goal-run/container-tail-repro.log`；专项日志 `/tmp/mdtui3-goal-run/container-tail-targeted.log`；首次全量 debug 记录 `/tmp/mdtui3-goal-run/container-tail-debug-initial.log`（948 通过，3 处上述旧观察待更新）；复跑脚本 `/tmp/mdtui3-goal-run/container-tail-validate.sh`。

项目 devShell 最终验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **951 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、Paragraphs / Hard line breaks / Soft line breaks / ATX / Setext 五组官方夹具 `--check` 及 `git diff --check`。完整验收日志 `/tmp/mdtui3-goal-run/container-tail-validation.log`。

## 2026-11-03 CommonMark Textual content 标准样例评分交付

- [x] **完整章节与独立来源**：固定 CommonMark 0.31.2 Textual content 650–652 全部 3 例；`tests/fixtures/commonmark_textual_content.rs` 逐字保留官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。`scripts/generate_commonmark_textual_content.py` 从本地来源校验摘要与章节成员后生成或 `--check`，保留原始 Unicode，构建/测试不联网、零新增依赖；重新下载来源逐字重建、篡改来源拒绝及生成器语法检查通过。
- [x] **真实评分与适用边界**：**3 支持、0 刻意差异、0 未支持**，覆盖普通标点（含未配对美元符）、希腊文和五个连续空格。只评分这三例，不代表完整 Unicode、非法 UTF-8、容器、长输入或预算边界全部符合标准。官方期望不使用本库输出生成；宽泛标准覆盖和完整语法 TODO 保持未完成。
- [x] **五项流式及语义测试**：全部 **48 个合法 UTF-8 切点 / 45 个逐字符前缀**及 `Utf8Document` 的 **53 个字节切点 / 50 个逐字节前缀**，比较块身份/范围、显示映射/样式和 pending。字节前缀另核对未完成标量的缓冲区和已解码源码；EOF 同时对照官方期望、原始文本借用映射及无替代内容/隐藏/软硬换行标记。投影仅依赖公开段落/显示语义，保留空白与 Unicode 标量，不 trim、不重解析 Markdown、不按编号调整输出；敏感性验证组合字符、实体、连续空格、软硬换行及结构边界，范围外块/样式明确拒绝。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_textual_content --no-fail-fast --success-output immediate`；夹具复核：`python3 scripts/generate_commonmark_textual_content.py /path/to/spec.json --check`。README/TODO 已记录交付与剩余范围，本单元不修改解析核心、运行时依赖、追加复杂度或预算契约。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **956 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、官方夹具 `--check` 及 `git diff --check`。专项日志 `/tmp/mdtui3-goal-run/textual-content-targeted.log`；来源核验 `/tmp/mdtui3-goal-run/textual-content-fixture-check.log`；完整验收 `/tmp/mdtui3-goal-run/textual-content-validation.log`；复跑脚本 `/tmp/mdtui3-goal-run/textual-content-validate.sh`。

## 2026-11-03 CommonMark Blank lines 标准样例评分交付

- [x] **完整章节与独立来源**：固定 CommonMark 0.31.2 Blank lines 全部 1 例（227）；`tests/fixtures/commonmark_blank_lines.rs` 逐字保存官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属及完整来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。`scripts/generate_commonmark_blank_lines.py` 校验本地来源摘要与章节成员后生成或 `--check`，不以解析器输出作为标准期望，构建和测试不联网。
- [x] **真实评分及边界**：**1 支持、0 刻意差异、0 未支持**；只覆盖样例中的文首、段落与 ATX 标题之间、文末的空行及仅空格行。列表松紧、容器空行、代码内空行、tab/CRLF/Unicode 空白及长输入不计入本次标准评分，宽泛标准覆盖和完整语法待办不勾选。
- [x] **四项测试**：独立评分、全部 **23 个切点 / 22 个逐字符前缀**、投影敏感性和范围外拒绝。流式快照复用既有测试辅助，比较源码、块/标题身份及范围、显示映射/样式和 pending；完成后另核对官方期望及逐 run 原始借用映射。投影仅使用公开块、标题索引与显示语义，跳过结构空白块和段落/标题末尾一个物理 LF，不 trim、不重解析 Markdown、不按编号改写输出。敏感性验证正文连续空格、内部 tab、实体空格、空标题、块顺序及软硬换行。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_blank_lines --no-fail-fast --success-output immediate`；夹具复核：`python3 scripts/generate_commonmark_blank_lines.py /path/to/spec.json --check`。README/TODO 已记录实际交付与剩余范围；解析核心、运行时依赖、追加复杂度及共享预算契约未改。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **960 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、官方夹具 `--check` 和 `git diff --check`。重新下载来源逐字重建、篡改来源拒绝及生成器语法检查通过。专项日志 `/tmp/mdtui3-goal-run/blank-lines-targeted.log`；来源日志 `/tmp/mdtui3-goal-run/blank-lines-fixture-check.log`；完整验收 `/tmp/mdtui3-goal-run/blank-lines-validation.log`；复跑脚本 `/tmp/mdtui3-goal-run/blank-lines-validate.sh`。

## 2026-11-03 CommonMark Thematic breaks 标准样例评分交付

- [x] **完整章节与独立来源**：固定 CommonMark 0.31.2 Thematic breaks 43–61 全部 19 例；`tests/fixtures/commonmark_thematic_breaks.rs` 逐字保存官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属和来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。`scripts/generate_commonmark_thematic_breaks.py` 校验本地固定来源和章节成员，提供生成及 `--check`；重新下载来源逐字重建、篡改来源拒绝和脚本语法检查通过，构建/测试不联网。
- [x] **初次评分与剩余范围**：**17 支持、1 刻意差异、1 未支持**（此处记录评分单元的历史结果；后续[容器内分隔线交付](#2026-11-03-容器内分隔线交付)修复 61，当前为 18/1/0）。覆盖分隔线字符、数量、间隔/尾空格、缩进、失败候选、Setext 与列表边界优先级。49 属既有缩进代码中断段落方言；61 的容器内分隔线未识别，重复 bullet 前缀形成嵌套列表。两例当前观察与官方期望分开保存，变化要求复核分类；完整容器/缩进、长候选与其它章节仍待办，不勾选宽泛标准覆盖总项。
- [x] **四项流式及投影验收**：官方评分、全部 **290 个切点 / 271 个逐字符前缀**、投影敏感性和范围外拒绝。复用公开快照辅助，比较源码、块/标题/容器身份及范围、显示映射/样式和 pending，完成后另核对逐 run 原始借用映射。投影只依赖公开块、容器路径、标题索引及显示语义；保留代码空白/LF、正文空白、强调、软硬换行、嵌套与列表分界，不 trim、不重解析 Markdown、不按编号改写输出。仅接纳 tight bullet 列表及每项单个直接物理叶块，引用、有序/任务/loose 列表、多叶块项、容器标题和未评分行内样式明确拒绝。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_thematic_breaks --no-fail-fast --success-output immediate`；夹具复核：`python3 scripts/generate_commonmark_thematic_breaks.py /path/to/spec.json --check`。README/TODO 已准确记录交付与剩余范围；本单元只新增评分测试、夹具和生成器，解析核心、零运行时依赖、禁止 unsafe、累计追加 O(n log n)、空间 O(n) 与共享预算契约不变。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **964 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、官方夹具 `--check` 和 `git diff --check`。专项日志 `/tmp/mdtui3-goal-run/thematic-breaks-targeted.log`；来源日志 `/tmp/mdtui3-goal-run/thematic-breaks-fixture-check.log`；完整验收 `/tmp/mdtui3-goal-run/thematic-breaks-validation.log`；复跑脚本 `/tmp/mdtui3-goal-run/thematic-breaks-validate.sh`。


## 2026-11-03 容器内分隔线交付

- [x] **可闭环方言**：既有显式 Quote/Item 的有界完整行内，正文前三个以内 ASCII 空格、至少三个同种 `*`/`-`/`_`、间隔及末尾 space/tab 识别为 `ThematicBreak`。在容器匹配时优先于同层兄弟和新嵌套列表，保留确切容器与整物理行源码；显示隐藏容器前缀，分隔线保持顶层原文约定。普通段落的 Setext、管道表格分隔行、围栏和四空格代码优先级保留；前一逻辑段落正常关闭并裁剪原始尾空白。
- [x] **算法与边界**：完整行的同种标记后缀只反向扫描一次，保存首标记和倒数第三标记两个位置，各层复用位置判断，不反复探测增长中的剩余行。新增扫描计入 `container_bytes`，无持久状态、旧正文/叶块回访或逐祖先更新；零运行时依赖、禁止 unsafe、累计 O(n log n)/空间 O(n) 和共享预算契约保持。含前缀的 1024 字节分类窗口不放宽：短尾等 LF/CRLF/EOF，未完成长行按既有方言提交后不重试。完整 tab/混合缩进、容器匹配、超长分隔线和单次 append 硬时延仍未交付。
- [x] **七项回归**：`tests/container_rules.rs` 覆盖 648 组标记/间隔/失败模型，列表/引用作用域与嵌套、Setext/表格/代码优先级、UTF-8 全切点/逐前缀、EOF 与分类窗口前后边界、旧对象及缓存失效、逐 run 原始借用映射、零预算暂停及 6 字节共享预算前进。400 层引用/列表及失败链、百万字候选后千次追加无分类/容器回扫、4096 条分隔线工作量验证通过。修复前已有 5 项失败，记录于 `/tmp/mdtui3-goal-run/container-rules-repro.log`。
- [x] **标准评分**：CommonMark 0.31.2 Thematic breaks 61 的公开结构投影已匹配官方 HTML；章节当前 **18 支持、1 刻意差异、0 未支持**，49 仍是既有缩进代码中断段落方言。官方输入/期望及投影算法未改，删除失效的 61 差异观察并更新分类；全部 **290 个切点 / 271 个逐字符前缀**和源码映射通过。README/TODO 已同步支持与剩余范围，完整容器和宽泛标准覆盖保持未勾选。

项目 devShell 验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **971 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、固定官方来源的夹具 `--check`/逐字重建/篡改拒绝和 `git diff --check`。专项（11 项）日志 `/tmp/mdtui3-goal-run/container-rules-targeted.log`；完整验收 `/tmp/mdtui3-goal-run/container-rules-validation.log`；复跑脚本 `/tmp/mdtui3-goal-run/container-rules-validate.sh`。初次全量 debug 970 项通过后补充第七项窗口边界回归，最终 debug/release 均为 971 项。

## 2026-11-03 CommonMark Indented code blocks 标准样例评分交付

- [x] **完整章节与独立来源**：固定 CommonMark 0.31.2 Indented code blocks 107–118 全部 12 例；`tests/fixtures/commonmark_indented_code.rs` 逐字保存官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属和完整来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。`scripts/generate_commonmark_indented_code.py` 校验本地来源摘要与章节成员后生成或 `--check`，不使用解析器输出生成标准期望；重新下载官方来源逐字重建、篡改来源拒绝和脚本语法检查通过，构建/测试不联网。
- [x] **真实评分与既有方言**：**8 支持、4 刻意差异、0 未支持**。107–109、114–118 匹配标准，涵盖额外缩进、代码字面内容、列表优先级及嵌套松紧、标题/分隔线边界和尾空格；110–112 因空行终止缩进代码而分块，113 因缩进代码可中断段落而分块。四例差异均对应 README 已有方言，当前观察与官方期望分开保存，改善或回退均要求复核分类。完整空行/缩进、tab/混合缩进和标准覆盖总项继续待办。
- [x] **四项流式及投影验收**：独立评分、全部 **309 个切点 / 297 个逐字符前缀**、投影敏感性和范围外拒绝。公开快照比较源码、块/标题/容器身份和范围、显示映射/样式及 pending；完成后核对逐 run 原始借用映射。投影仅依赖公开块、容器路径及 tight/loose 元数据、标题索引和显示语义，不 trim、不重解析 Markdown、不按编号改写输出；保留代码字面标记、缩进余量、尾空格、tab/LF、列表嵌套、正文空白及软硬换行。仅接纳该章节所需列表（有序条目编号限 1），引用、任务、容器标题/代码/分隔线、带 info 的代码块及范围外行内样式明确拒绝。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_indented_code --no-fail-fast --success-output immediate`；夹具核验：`python3 scripts/generate_commonmark_indented_code.py /path/to/spec.json --check`。README/TODO 已记录交付与剩余范围；本单元只新增评分测试、夹具、生成器和文档，解析核心、零运行时依赖、禁止 unsafe、累计追加 O(n log n)、空间 O(n) 和共享预算契约不变。短样例的流式验证不代替长输入或预算性能证明。

项目 devShell 最终验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **975 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、官方夹具 `--check` 及 `git diff --check`。专项日志 `/tmp/mdtui3-goal-run/indented-code-targeted.log`；来源核验 `/tmp/mdtui3-goal-run/indented-code-fixture-check.log`；完整验收 `/tmp/mdtui3-goal-run/indented-code-validation.log`；复跑脚本 `/tmp/mdtui3-goal-run/indented-code-validate.sh`。首次 Clippy 检查发现投影中的可合并嵌套 if，修正后全部复跑通过，初次日志保留于 `/tmp/mdtui3-goal-run/indented-code-clippy-initial.log`。

## 2026-11-03 CommonMark Code spans 标准样例评分交付

- [x] **完整章节与独立来源**：固定 CommonMark 0.31.2 Code spans 328–349 全部 22 例；`tests/fixtures/commonmark_code_spans.rs` 逐字保存官方 Markdown/HTML、规范行号、John MacFarlane / CC BY-SA 4.0 归属和完整来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。`scripts/generate_commonmark_code_spans.py` 校验本地来源摘要和章节成员，提供生成及 `--check`，不使用解析器输出生成标准期望，构建/测试不联网。
- [x] **真实评分与剩余范围**：**20 支持、1 刻意差异、1 未支持**。349 的未闭合代码候选不重新解析内部反引号，符合 README 既有失败回退方言；344 为需求范围外的原始 HTML。官方期望与当前差异观察分开保存，改善或回退均要求显式复核分类。346 的 URI 反引号只在测试输出层编码为 `%60`，另验证核心原始目标和标签，不声明通用 URL 编码。完整代码回退、长定界符/长输入、容器边界与宽泛标准覆盖继续待办，HTML 需求范围不变。
- [x] **四项流式及投影验收**：独立评分、全部 **299 个合法 UTF-8 切点 / 277 个逐字符前缀**、投影敏感性和范围外拒绝。公开快照比较源码、块/对象身份和范围、样式/显示映射及 pending；完成后核对逐 run 源码覆盖、UTF-8 边界、原始借用文本或代码空白替代映射。投影只读取公开段落、显示样式和 URI 自动链接，不 trim、不重解析 Markdown、不按编号改写输出；保留代码样式、连续空格、NBSP、tab、内部换行规范化、代码内字面语法和软硬换行。范围外块/样式、图片、引用、普通及 email 链接明确拒绝。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_code_spans --no-fail-fast --success-output immediate`；夹具核验：`python3 scripts/generate_commonmark_code_spans.py /path/to/spec.json --check`。README/TODO 已记录实际交付与剩余范围；本单元只新增评分测试、夹具、生成器与文档，解析核心、零运行时依赖、禁止 unsafe、累计追加 O(n log n)、空间 O(n) 及共享预算契约不变。短样例的流式验证不代替长输入或预算性能证明。

项目 devShell 最终验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **979 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、官方夹具 `--check` 和 `git diff --check`。重新下载来源逐字重建、篡改来源拒绝及生成器语法检查通过；核对本单元开始时的文件摘要，前序解析核心与测试改动均保持。专项日志 `/tmp/mdtui3-goal-run/code-spans-targeted.log`；来源核验 `/tmp/mdtui3-goal-run/code-spans-fixture-check.log`；完整验收 `/tmp/mdtui3-goal-run/code-spans-validation.log`；复跑脚本 `/tmp/mdtui3-goal-run/code-spans-validate.sh`。


## 2026-11-03 未闭合代码候选有界回退交付

- [x] **最小方言与复杂度验证**：不尝试重放完整 Markdown。保留外层优先与开放时字面显示，扫描时最多保存 32 个异长反引号串及其后正文的全空白、语义尾空白和原始端点摘要；失败收尾按第一个后续同长度串配对，跳过已配对内容，找不到则继续下一个串。超过 32 串时整个失败候选保持原方言，成功外层不受上限影响；代码串长/正文长不受此上限限制。至多 496 次摘要比较、16 对提交，比较记入 `delimiter_steps`；新增候选空间 O(1)，无旧正文/物理叶块/祖先回扫，累计解析 O(n log n)、空间 O(n) 保持。
- [x] **边界及显示语义**：EOF、段落/容器/标题/单元格关闭时才提交内部配对，沿用代码空白规范化、源码映射、区间更新及过期游标拒绝。早表头在旧段落关闭时提交；晚表头保存 O(1) 的旧行摘要边界，成功仅提交旧段落配对并独立解析表头，失败恢复原候选。跨表头配对与表头内溢出不能污染旧段落。部分 tab 的尾空白改由代码摘要直接记录，避免延迟提交受到后续 tab 映射干扰。其它语法、反斜线转义与代码外软硬换行不重放；完整回退和超过 32 串的情况继续待办。
- [x] **八项新增回归**：7 项 `tests/code_fallback.rs` 加 1 项内部存储验收，覆盖 255 组独立配对模型、31/32/33/1000 串边界、外层成功及状态清理、Unicode/CRLF、全切点/逐前缀、容器/标题/表格、晚表头成功/失败/跨界/溢出、部分 tab 与空格模型、跨 tile 源码映射和 6 字节预算推进。千层容器百万字候选后千次追加只增加 1000 个行内字节访问，EOF 不重读正文；32 个互不匹配串精确计 496 次比较；4096 候选线性计数、2000 串仍只保留 32 份摘要且复用容量。修复前 4 项失败日志 `/tmp/mdtui3-goal-run/code-fallback-repro.log`；部分 tab 边界失败日志 `/tmp/mdtui3-goal-run/code-fallback-tabs-repro.log`。
- [x] **评分与剩余范围**：CommonMark 0.31.2 Code spans 349 已符合官方期望；章节现为 **21 支持、0 刻意差异、1 未支持**，344 的原始 HTML 仍在需求范围外。官方 Markdown/HTML 夹具和评分投影算法未改，仅删除 349 的旧差异观察、更新分类及计数；299 切点/277 前缀和源码映射继续通过。README/TODO 同步实际方言，上方历史评分记录保留当时结果，完整代码回退与宽泛标准覆盖未勾选。

项目 devShell 最终验收全部通过：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **987 项**、`cargo test --workspace --doc` **72 项（69 + 3）**、固定官方来源夹具 `--check`/逐字重建/篡改拒绝及 `git diff --check`。核心/部分 tab/标准评分专项 **22 项**通过，日志 `/tmp/mdtui3-goal-run/code-fallback-targeted.log`；最终完整验收 `/tmp/mdtui3-goal-run/code-fallback-validation.log`，脚本 `/tmp/mdtui3-goal-run/code-fallback-validate.sh`。前序工作区改动保留；本次仅新增有界代码回退及必要的尾空白摘要、计数说明、回归、评分和文档。


## 2026-11-03 CommonMark Fenced code blocks 标准样例评分交付

本单元推进 TODO「继续扩充标准语法样例覆盖」，固定 CommonMark 0.31.2 **Fenced code blocks 完整章节 119–147，共 29 例**，评分为 **26 支持、3 刻意差异、0 未支持**。131–133 是已记录的围栏正文不按 opener 缩进去缩进方言；保留官方期望和独立当前观察，不将方言输出改作标准期望。

- 新增 `scripts/generate_commonmark_fenced_code.py` 和离线 `tests/fixtures/commonmark_fenced_code.rs`：校验完整 `spec.json` SHA-256 与连续章节成员，逐字复制官方 Markdown/HTML，保留规范行号、作者 John MacFarlane、来源与 CC BY-SA 4.0 归属。构建/测试无需联网；新下载来源重建逐字一致，篡改来源被拒绝。
- 新增 `tests/commonmark_fenced_code.rs` 的 4 项测试：独立逐例评分、全部 **585 个合法 UTF-8 切点 / 556 个逐字符前缀**、投影敏感性、范围外结构/样式拒绝。快照与 pending 核验流式一致性；逐 run 核对连续源码覆盖、原始借用字节及行内代码空白替代映射。
- 测试投影只消费公开块/引用路径/标题索引/info/显示语义，保留空代码、空行、缩进余量、尾空格、tab、LF、字面 Markdown、软硬换行、行内代码和块顺序；不 trim、不重解析 Markdown、不根据样例编号调整输出。代码 language class 仅在测试输出层取原始 info 首词并转义属性，另核对完整 info 原始区间。列表、容器正文/标题、表格、范围外行内样式和需实体/反斜线解码的 info 明确拒绝。

本单元未修改任何已有核心/测试文件、运行时依赖、追加复杂度或预算实现。额外敏感性输入覆盖 Unicode、嵌套引用和无换行 EOF，但不计入标准章节评分；正文去缩进、完整 tab/容器规则、opener/分类长度上限、info 解码和其它标准章节仍待办，未勾选宽泛标准覆盖条目。README/TODO 已同步实际交付与剩余范围。

验收通过：`nix develop -c bash /tmp/mdtui3-goal-run/fenced-code-validate.sh` 完成 workspace all-targets check、clippy `-D warnings`、fmt 检查、debug/release nextest 各 **991 项**、doc **72 项**、夹具复核及 `git diff --check`。完整日志 `/tmp/mdtui3-goal-run/fenced-code-validation.log`；4 项专项日志 `/tmp/mdtui3-goal-run/fenced-code-targeted.log`；官方来源重建/篡改拒绝日志 `/tmp/mdtui3-goal-run/fenced-code-fixture-check.log`。


## 2026-11-03 围栏正文有界空格去缩进交付

本单元选择前序 Fenced code blocks 评分暴露的正文去缩进子项，修复 CommonMark 0.31.2 **131–133**；该章节现为 **29 支持、0 刻意差异、0 未支持**。官方 Markdown/HTML 夹具与评分投影算法保持，只删除旧差异观察、更新报告及一处投影敏感性测试的期望。此前评分记录保留当时结果，宽泛标准覆盖仍未完成。

- 活动反引号/波浪线围栏记录 0–3 列 opener 缩进，正文在既有容器匹配后先扣除前缀 tab 虚拟余量，再隐藏剩余额度内的 ASCII 空格。遇到原始正文 tab/非空格即停止，保留余下空白、字面语法和原始 info；空行/EOF/失败 closer 同样处理。closer 使用未去缩进的列数判定；独立数学块、四空格代码、容器退出及分类窗口方言保持。
- 只增加两个固定大小的缩进字段。1015 层容器前缀可能使正文缩进跨过 1 KiB 分类窗口，最多三列余量随新增字节逐次消费；新增实际检查计入 `Work.classified_bytes`，不回读正文、不再匹配祖先、不枚举历史块。隐藏使用既有区间标记和失效通知，累计解析 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 及共享预算契约保持。
- 新增 `tests/fence_indent.rs` **7 项回归**：840 组根/引用/列表/嵌套作用域独立空格模型，反引号/波浪线、空行、LF/CRLF/EOF、UTF-8 全切点/逐前缀及逐前缀 EOF、部分 tab、原始 tab 停止规则、分类边界、源码/tile/6 字节预算映射、旧缓存/游标/测量拒绝与折叠身份。千层百万字后追加 1000 字节不增加分类/行内/容器/树访问；4096 历史块后新块只做局部工作。修复前 6 项失败，证据 `/tmp/mdtui3-goal-run/fence-indent-repro.log`。
- 两项既有部分 tab 测试更新：数学仍保留虚拟空格，代码按 opener 扣除；映射/搜索/摘要用一列 opener 验证剩余一列仍指向原 tab。README 的旧输出示例和 `examples/leaf_tabs.rs` 同步修正，release 示例通过（13 帧、199 计费字节）。正文原始 tab 去缩进、完整容器/叶块规则、opener/分类长度上限和 info 解码在 TODO 单独保留，未扩展 UI。

验收通过：专项 **25 项**（7 新回归 + 4 评分 + 14 部分 tab）及官方 **585 切点/556 前缀**；workspace debug/release nextest 各 **998 项**；文档 **72 项（69 + 3）**；all-targets check、clippy `-D warnings`、fmt、官方夹具校验/逐字重建/篡改拒绝和 `git diff --check`。完整运行日志 `/tmp/mdtui3-goal-run/fence-indent-validation.log`；首次文档测试揭示旧示例输出，修正后文档/夹具/fmt/diff 验收见 `/tmp/mdtui3-goal-run/fence-indent-doc-final.log`，all-targets check/clippy/fmt 和示例实跑见 `/tmp/mdtui3-goal-run/fence-indent-example-final.log`。专项 `/tmp/mdtui3-goal-run/fence-indent-targeted.log`；来源 `/tmp/mdtui3-goal-run/fence-indent-fixture-check.log`；可复跑完整脚本 `/tmp/mdtui3-goal-run/fence-indent-validate.sh`。前序工作区成果保留。

## 2026-11-03 CommonMark Backslash escapes 标准样例评分交付

本单元推进 TODO「继续扩充标准语法样例覆盖」，固定 CommonMark 0.31.2 **Backslash escapes 完整章节 12–24，共 13 例**，评分为 **8 支持、4 刻意差异、1 未支持**。14 对应既有未闭合代码候选不重放其它行内语法，22–23 对应链接目标/title 保留原始转义，24 对应围栏 info 保留原始转义；21 为需求范围外原始 HTML。官方期望与独立当前观察分开保存，行为改善或回退均要求复核分类。

- 新增 `scripts/generate_commonmark_backslash_escapes.py` 及离线 `tests/fixtures/commonmark_backslash_escapes.rs`：校验完整来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20` 与章节成员，逐字复制官方 Markdown/HTML，保留规范行号、作者 John MacFarlane、来源和 CC BY-SA 4.0 归属。新下载官方来源重建逐字一致，篡改来源拒绝，生成器语法检查通过；构建及测试不联网。
- 新增 `tests/commonmark_backslash_escapes.rs` **4 项测试**：独立逐例评分、全部 **448 个合法 UTF-8 切点 / 435 个逐字符前缀**、投影敏感性与范围外拒绝。公开快照与 pending 验证源码、块/对象身份和范围、样式/显示映射的流式一致性；完成后逐 run 核对连续源码覆盖、原始借用文本和代码空白替代映射。
- 投影只消费公开块/显示片段/链接目标/title/info，保留空白、字面代码、强调、软硬换行、块顺序和原始转义，不 trim、不重解析 Markdown、不按编号改写输出。URL 中的反斜线仅在输出层编码 `%5C`，另核验核心原始地址和标签；直接/引用目标与 title、完整 info 亦独立核对，不在适配器中补做语法解码。未评分容器/标题/表格、嵌套样式、图片/email、未解析引用及额外 URL 编码明确拒绝。

README/TODO 已同步评分与剩余范围；完整代码回退、目标/title/info 解码及宽泛标准覆盖保持待办。未修改已有解析核心、测试、夹具、脚本或运行时依赖；基线 SHA-256 复核保留前序改动。累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算契约不变，短样例不代替长输入或预算性能证明。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_backslash_escapes --no-fail-fast --success-output immediate`；夹具核验：`python3 scripts/generate_commonmark_backslash_escapes.py /path/to/spec.json --check`。专项日志 `/tmp/mdtui3-goal-run/backslash-targeted.log`；来源重建/篡改拒绝 `/tmp/mdtui3-goal-run/backslash-fixture-check.log`；初始评分发现 14 的差异与独立观察见 `/tmp/mdtui3-goal-run/backslash-initial.log`、`/tmp/mdtui3-goal-run/backslash-observations.log`。

完整验收通过：`nix develop -c bash /tmp/mdtui3-goal-run/backslash-validate.sh` 执行 workspace all-targets check、clippy `-D warnings`、fmt 检查、debug/release nextest 各 **1002 项**、doc **72 项（69 + 3）**、官方夹具复核及 `git diff --check`。完整日志 `/tmp/mdtui3-goal-run/backslash-validation.log`；既有文件摘要和新增文件范围核对通过，本单元仅新增三个文件并更新 README/TODO/DONE。

## 2026-11-03 CommonMark Entity and numeric character references 标准样例评分交付

本单元推进 TODO「继续扩充标准语法样例覆盖」，固定 CommonMark 0.31.2 **Entity and numeric character references 完整章节 25–41，共 17 例**：**12 支持、4 刻意差异、1 未支持**。32–33 的链接目标/title 与 34 的围栏 info 保留原始实体拼写；41 的失败后缀保留既有 shortcut 引用，隐藏方括号且没有目标；31 为需求范围外原始 HTML。官方期望与独立当前观察分开保存，改善或回退均要求复核分类。

- 新增 `scripts/generate_commonmark_entities.py` 与离线 `tests/fixtures/commonmark_entities.rs`，校验完整来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20` 及章节成员，逐字复制官方 Markdown/HTML，保留规范行号、作者 John MacFarlane、来源及 CC BY-SA 4.0 归属。构建与测试不联网。
- 新增 `tests/commonmark_entities.rs` **5 项测试**：评分、全部 **524 个合法 UTF-8 切点 / 507 个逐字符前缀**、投影敏感性、范围外拒绝、逐字符切片及 6 字节共享预算恢复。快照核对源码、块/容器/对象身份与范围、显示映射/样式和 pending；实体的原始区间与解码值另用固定独立映射核验，包含双标量实体、替代字符及 tab/LF，切片不得重复输出替换文本。
- 投影只消费公开块、顶层 tight bullet 列表、显示片段及原始链接/info 元数据，不 trim、不重解析、不按编号改写输出。只删除来自原文的物理段末 LF，实体产生的 LF（包括段末）保留；空白、字面代码、强调、软硬换行、块顺序和真实/实体生成的结构标记可区分。未解析引用按核心公开显示结果保留，不补回方括号；目标/title/info 不解码，完整元数据与失败链接对象独立核验。未覆盖容器/嵌套或 loose 列表、多叶块条目、标题/表格/数学、嵌套行内样式、图片/email 与额外 URL 编码明确拒绝。

README/TODO 已记录实际评分与剩余范围；目标/title/info 实体解码、完整失败链接回退和宽泛标准覆盖保持待办。未改变既有解析核心、测试、脚本或运行时依赖；累计追加 O(n log n)、空间 O(n)、禁止 unsafe 和共享预算契约保持，短样例不代替长输入性能证明。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_entities --no-fail-fast --success-output immediate`；夹具核验：`python3 scripts/generate_commonmark_entities.py /path/to/spec.json --check`。专项日志 `/tmp/mdtui3-goal-run/entities-targeted.log`；初始观察与方括号差异见 `/tmp/mdtui3-goal-run/entities-observations.log`、`/tmp/mdtui3-goal-run/entities-initial.log`。

完整验收通过：`nix develop -c bash /tmp/mdtui3-goal-run/entities-validate.sh` 执行 workspace all-targets check、clippy `-D warnings`、fmt 检查、debug/release nextest 各 **1007 项**、doc **72 项（69 + 3）**、官方夹具复核及 `git diff --check`。新下载官方来源逐字重建、篡改来源拒绝与生成器语法检查通过，日志 `/tmp/mdtui3-goal-run/entities-fixture-check.log`；全量日志 `/tmp/mdtui3-goal-run/entities-validation.log`。基线 SHA-256 与新增文件集合核对通过：仅新增三个评分文件并更新 README/TODO/DONE，前序工作区成果保持。


## 2026-11-03 CommonMark Tabs 标准样例评分交付

本单元推进 TODO「继续扩充标准语法样例覆盖」，固定 CommonMark 0.31.2 **Tabs 完整章节 1–11，共 11 例**，初次评分为 **4 支持、7 刻意差异、0 未支持**。3、4、10、11 匹配官方输出；1、2、8 是叶块 tab/混合缩进代码差异，5 是空行后部分 tab 加叶块 tab，6 是新引用首行部分 tab，7 是新列表标记消费全部水平空白，9 是部分 tab 路径的新嵌套列表保持 lazy 正文。差异依据 README 既有容器/叶块方言单独分类，没有把当前输出写成标准期望。

- 新增 `scripts/generate_commonmark_tabs.py` 及离线 `tests/fixtures/commonmark_tabs.rs`：校验完整来源 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20` 与完整章节成员，逐字复制官方 Markdown/HTML，保留规范行号、John MacFarlane、来源及 CC BY-SA 4.0 归属。构建和测试不联网。
- 新增 `tests/commonmark_tabs.rs` **5 项测试**：独立逐例评分，全部 **148 个合法 UTF-8 切点 / 137 个逐字符前缀**，投影敏感性，范围外拒绝，逐字符切片与 6 字节共享预算读取。快照及 pending 核对流式源码、块/容器/对象身份与范围、样式和显示映射；逐 run 独立验证连续源码覆盖、UTF-8 边界及借用原文，包括内部 tab、希腊文字和隐藏前缀。零预算停步、耗尽与恢复也通过同一读取测试验收。
- 投影只读取公开块/容器/标题和显示片段，保留嵌套列表、tight/loose、引用、字面代码、tab/连续空格和软硬换行；不展开 tab、不 trim、不重解析、不按编号改写输出。范围外的行内样式/对象、表格、容器代码/标题/分隔线和带 info 的代码块明确拒绝，后续支持这些结构时须复核投影和分类。当前观察改变会失败，不自动把改善或回退分类为通过。

README/TODO 已记录实际评分和具体剩余范围，宽泛标准覆盖与完整容器/叶块 tab 规则保持待办。仅新增评分/夹具/生成器和更新三份文档，既有核心、测试与脚本保持；累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算契约不变。短样例评分不替代长输入复杂度证明。

复跑：`nix develop -c cargo nextest run --workspace --test commonmark_tabs --no-fail-fast --success-output immediate`；夹具核验：`python3 scripts/generate_commonmark_tabs.py /path/to/spec.json --check`。初始观察 `/tmp/mdtui3-goal-run/tabs-observe.log`；专项 `/tmp/mdtui3-goal-run/tabs-targeted.log`；新下载官方来源逐字重建与篡改拒绝 `/tmp/mdtui3-goal-run/tabs-fixture-check.log`。

完整验收通过：`nix develop -c bash /tmp/mdtui3-goal-run/tabs-validate.sh` 执行 workspace all-targets check、clippy `-D warnings`、fmt 检查、debug/release nextest 各 **1012 项**、doc **72 项（69 + 3）**、官方夹具复核、生成器语法检查及 `git diff --check`。全量日志 `/tmp/mdtui3-goal-run/tabs-validation.log`；新下载官方来源逐字重建及篡改拒绝见上述来源日志。基线 SHA-256 与新增文件集合核对通过：既有 crates/scripts 内容保持，仅新增三个评分文件；README/TODO/DONE 为追加记录，前序工作区成果保留。


## 2026-11-03 顶层 tab 缩进代码交付

本单元从 TODO 的 tab 缩进余项选择顶层一般叶块分类：行首 0–3 个 ASCII 空格加 tab 按四列缩进打开或续接代码块，隐藏原始前缀，余下 tab/空格、标记、实体保持字面。CommonMark 0.31.2 Tabs 的 **1、2、8 已修复，当前 7 支持、4 刻意差异、0 未支持**。5 的空行后路径仍关闭条目，根段落改为根代码，第二个 tab 仍是字面，因此仍与官方条目内代码不同；6、7、9 的容器差异保持。官方 Markdown/HTML、夹具及评分投影算法未改，只调整分类、差异观察和汇总。

- 方言与边界：同作用域相邻代码合并；LF/EOF/分类窗口前的短输入暂定原文，确认后失效旧缓存/显示游标。空行立即终止代码、代码可中断段落的方言保持；完整纯空白短行是 Blank，超过分类窗口仍未完成的长空白行沿用既有一次分类规则。已匹配容器/lazy、围栏/数学、脚注续行和表头分隔行优先级保持，失败容器路径回到根时采用新分类。Setext 下划线不再提升前一 tab 缩进代码。
- 核心只修改 `block::classify`：原分类为 Text 后额外检查最多四个前缀字节，容器叶块仍用原 `classify_indented`；lazy 探测原来就接受 Text/Indented 两者，资格不变。复用字面代码/隐藏区间，没有新增持久状态、替换文本、运行时依赖或 unsafe。分类输入计入既有 Work；无旧正文/历史块/祖先回扫，累计追加 O(n log n)、空间 O(n)、共享查询预算契约保持。
- 新增 `tests/root_tab_code.rs` **5 项回归**：360 组独立四空格模型（缩进余量、内部 tab、UTF-8、代码字面标记、LF/CRLF/EOF），代码合并/空行/容器/表格/围栏边界，合法 UTF-8 全切点与逐前缀快照及 pending、分类窗口与缓存失效，逐 run 原始借用/连续映射、逐字符切片/tile 与 6 字节预算停步恢复，百万字后千次逐字追加及 4096 代码行计数。后续逐字追加不增加分类、行内、容器或树访问，代码保持同一块。
- 既有 paragraph_indent、math_tabs、partial_tabs 与 setext_whitespace 的受影响观察按新规则更新；容器退出和 Setext 用例同时检查块类型及字面结果。其余前序工作区成果保留，宽泛标准覆盖、完整容器叶块/tab 和空行规则继续待办。

验收证据：修复前新增 5 项均失败，日志 `/tmp/mdtui3-goal-run/root-tabs-repro.log`；修复后专项日志 `/tmp/mdtui3-goal-run/root-tabs-targeted.log`，官方 **148 切点 / 137 前缀**、源码映射与小预算检查通过。固定来源逐字重建 Tabs 夹具、篡改来源拒绝及 Indented/Fenced code 两章夹具核对通过，日志 `/tmp/mdtui3-goal-run/root-tabs-fixture-check.log`。首轮全套识别三处旧方言断言，记录于 `/tmp/mdtui3-goal-run/root-tabs-first-full.log`。

复跑：`nix develop -c cargo nextest run --workspace --test root_tab_code --test commonmark_tabs --no-fail-fast --success-output immediate`。

完整验收通过：workspace all-targets check、clippy `-D warnings`、fmt 检查、debug/release nextest 各 **1017 项**、doc **72 项（69 + 3）**，日志 `/tmp/mdtui3-goal-run/root-tabs-validation.log`；`git diff --check` 通过。对照执行前 SHA-256，只有 `block.rs`、五个受影响旧测试、README/TODO/DONE 发生修改，新增 `root_tab_code.rs`；官方夹具/生成器和其余前序工作区成果保持。


## 2026-11-03 引用内 tab 缩进代码交付

本单元从 TODO 的 tab 缩进余项选择显式引用内代码：新 Quote 或已匹配路径最后一个 Quote 后的可选一列空白及四列代码缩进按四列 tab stop 消费，允许部分 tab；未消费的完整 tab 保留字面，最后被部分消费的 tab 复用显示映射输出剩余 1–3 格。CommonMark 0.31.2 Tabs **6 修复，当前 8 支持、3 刻意差异、0 未支持**；5、7、9 的差异观察保持，官方输入/期望、夹具和生成器未修改。

- 方言先限定为显式引用代码，不足缩进、短完整空白、新引用非代码叶块、列表叶块 tab 及缺前缀/lazy 路径沿用既有规则。外层匹配不放宽；已有围栏/数学保持优先，代码中断同作用域段落/表格，连续代码合并。LF/EOF/1024 字节分类窗口前原文暂定，窗口之后不重试分类；完整空行终止代码、缩进代码中断段落仍是既有方言。
- `Prefix::quote_code` 用固定大小游标原子探测，只读当前有界引用/空白前缀，失败不改位置及虚拟列，实际字节检查计入 Work。`ContainerLine` 仅传递本行代码分类标记；不新增持久解析状态，不复制祖先或读取旧正文，显示映射按已有记录存储。累计 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 和共享预算契约保持。
- 新增 `tests/quote_tab_code.rs` 5 项回归：504 组独立列宽模型及其中 336 组显式续接，UTF-8/CRLF/EOF、嵌套引用与列表外层、字面 Markdown/实体、合并及失败/lazy/围栏/数学/表格边界；全合法切点与逐前缀、分类窗口和缓存失效；原始单字节替换、tile 边界、逐字符切片、零预算停步与六字节恢复；千层引用百万字后千次追加、4096 行线性计数。
- Tabs 投影放行 Quote 内无 info 代码，仅读取公开块/容器与显示输出，不自行展开 tab；增加字面空白敏感性检查，条目代码和其它范围外结构仍拒绝。映射检查允许核心发布的 tab 替换，其余内容仍核验借用原文。原有根 tab 测试中 `>\t\tfoo` 的旧非代码断言由本项精确代码及映射回归接替，其余旧测试保持。

验收：官方 148 个合法 UTF-8 切点 / 137 个逐字符前缀、映射/切片/六字节预算继续通过。专项日志 `/tmp/mdtui3-goal-run/quote-tabs-targeted.log`；修复前新增测试失败记录 `/tmp/mdtui3-goal-run/quote-tabs-repro.log`。README/TODO 已同步实际范围，宽泛标准覆盖及完整容器 tab 规则仍未勾选。

完整验收通过：workspace all-targets check、clippy `-D warnings`、fmt、release nextest **1022 项**、doc **72 项（69 + 3）**，日志 `/tmp/mdtui3-goal-run/quote-tabs-validation.log`；debug 全套 **1022 项**见 `/tmp/mdtui3-goal-run/quote-tabs-first-full.log`，加强续接模型后的 debug 专项 **15 项**见上述专项日志。Tabs 夹具逐字重建与篡改来源拒绝见 `/tmp/mdtui3-goal-run/quote-tabs-fixture-check.log`；Indented/Fenced code 两章夹具核验及 `git diff --check` 通过。对照执行前 SHA-256，只改两个核心文件、两项既有测试和 README/TODO/DONE，新增 `quote_tab_code.rs`，其余前序成果保持。


## 2026-11-03 列表续行部分 tab 缩进代码交付

本单元从 TODO 的 tab 缩进余项选择已有列表的部分 tab 代码续行：完整已有路径以 Item 结束、路径消费部分 tab、普通段落续行未启用时，原子扣除四列代码缩进，最后 tab 的余量映射到原单字节，后续完整 tab 保留字面。CommonMark 0.31.2 Tabs **5 修复，当前 9 支持、2 刻意差异、0 未支持**；7、9 差异保持，官方输入/期望、夹具和生成器未改。

- 方言限定为现有路径的代码叶块；空行后内容、相邻代码续行和阻止普通续行的表头候选适用。普通段落/lazy、失败路径、围栏/数学保持原规则；新列表首行、非部分 tab 路径、新嵌套列表、完整容器及空行规则仍待办。代码同作用域合并，空行终止代码；短未完成行暂定原文，1024 字节窗口后不重试分类。
- `Containers::partial_prefix` 复用固定大小游标和缓存空白末端，在成功后传递已有 `ContainerLine.indented`；失败不改变位置或余量，检查计入 Work。`Document` 沿用已有代码分支的 tab 替换映射，只将局部变量更名以覆盖 Quote/Item。没有新增持久状态、旧正文/历史块/无输入支撑的祖先回扫、运行时依赖或 unsafe；累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。
- 新增 `tests/item_tab_code.rs` **6 项回归**：396 组独立列宽模型，其中 264 组另验续接；有序/嵌套列表、UTF-8/CRLF/EOF、字面语法、松紧和身份、空行/兄弟/失败/lazy/围栏/数学/表格边界；全切点与逐前缀、分类窗口及缓存失效；tile 边界单字节替换、连续源码覆盖、逐字符切片及零预算/六字节恢复。998 层容器百万字后千次追加不增加分类/行内/容器/树访问，4096 行保留同一代码块。
- Tabs 评分投影接纳 Item 内无 info 代码，直接消费公开结构和核心显示映射；加入 ASCII 与 tab 代码的结构/空白敏感性检查，不展开 tab、不 trim、不重解析。官方 **148 切点 / 137 前缀**、映射/切片/六字节预算继续通过。根代码回退测试改用确实缺少引用前缀的输入，原样例 5 的旧断言由新测试替代；其余前序成果保持。README/TODO 同步实际交付和剩余范围，宽泛标准覆盖保持未勾选。

复现日志 `/tmp/mdtui3-goal-run/item-tabs-repro.log` 证明修复前样例 5 退出条目；专项 `/tmp/mdtui3-goal-run/item-tabs-targeted.log`；首轮 debug 全套 `/tmp/mdtui3-goal-run/item-tabs-first-full.log` 定位上述旧回退断言。复跑：`nix develop -c cargo nextest run --workspace --test item_tab_code --test commonmark_tabs --no-fail-fast --success-output immediate`。

完整验收通过：workspace all-targets check、clippy `-D warnings`、fmt、debug/release nextest 各 **1028 项**、doc **72 项（69 + 3）**，以及 Tabs/Indented/Fenced code 夹具核验和 `git diff --check`；日志 `/tmp/mdtui3-goal-run/item-tabs-validation.log`。Tabs 夹具逐字重建与篡改来源拒绝见 `/tmp/mdtui3-goal-run/item-tabs-fixture-check.log`。对照执行前 SHA-256，仅修改两个核心文件、两个既有测试、README/TODO/DONE，新增 `item_tab_code.rs`；其它前序工作区成果保持。


## 2026-11-03 列表首行 tab 缩进代码交付

本单元从 TODO 的 tab 缩进余项选择新列表及兄弟条目的代码首行：标记后的空白含 tab 且至少五列时，消费一列分隔加四列代码缩进，条目续行宽度按标记加一列保存；最后 tab 的余量映射原单字节，未消费的 tab/空格保留字面。CommonMark 0.31.2 Tabs **7 修复，当前 10 支持、1 刻意差异、0 未支持**；9 的部分 tab 新嵌套列表仍待办，官方输入/期望、夹具与生成器保持。

- 方言限定为当前分类窗口可识别的含 tab 首行；新列表、兄弟条目及显式嵌套适用。纯空格、少于五列、完整空白行、失败/lazy 路径、已打开围栏/数学正文保持原规则；代码里的任务标记、标题、实体和围栏均保留字面。短输入等 LF/CRLF/EOF，1024 字节窗口后不重试分类；完整空行仍终止代码，完整容器和纯空格首行规则继续待办。
- `Prefix::marker` 复用扫描过的空白末端，仅额外推进最多五字节；临时 Marker 携带可选余量，既有代码分支消费原始映射。兄弟代码先关闭旧条目及子层，停止继续匹配旧子层；代码不再走任务标记解析。没有新增持久状态、旧正文/历史块/无输入支撑祖先回扫、运行时依赖或 unsafe；累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。
- 新增 `tests/list_start_tab_code.rs` **6 项回归**，包含 **2268 组独立列宽模型（1512 组另验续接）**；有序/嵌套/兄弟/任务字面、列表身份及松紧、UTF-8/CRLF/EOF、全切点与逐字符前缀、分类窗口和缓存失效；tile 边界单字节替换、连续源码覆盖、逐字符切片、零预算停步及六字节恢复。1000 层容器百万字后千次追加不增加分类/行内/容器/树访问，4096 兄弟代码条目的容器工作保持线性。
- Tabs 投影仅补充紧凑条目以代码块开头时的 HTML 换行，并新增格式/余量敏感性检查；直接消费公开结构和核心映射，不展开 tab、不 trim、不重解析。官方 **148 切点 / 137 前缀**、映射/切片与六字节预算通过。既有列表续行测试的一处旧首行观察同步；旧围栏列宽模型的 13 种首行现在为字面代码，单独断言代码，剩余 2240 组围栏续行模型继续验证，首行代码由新增模型覆盖。

修复前 6 项新增测试中 4 项失败，日志 `/tmp/mdtui3-goal-run/list-start-tabs-repro.log`；专项 23 项通过见 `/tmp/mdtui3-goal-run/list-start-tabs-targeted.log`。首轮全套定位旧围栏假设见 `/tmp/mdtui3-goal-run/list-start-tabs-first-full.log`。Tabs 夹具逐字重建及篡改来源拒绝通过，日志 `/tmp/mdtui3-goal-run/list-start-tabs-fixture-check.log`。复跑：`nix develop -c bash /tmp/mdtui3-goal-run/list-start-tabs-validate.sh`。

完整验收通过：workspace all-targets check、clippy `-D warnings`、fmt、debug/release nextest 各 **1034 项**、doc **72 项（69 + 3）**，以及 Tabs/Indented/Fenced code 夹具核验和 `git diff --check`；日志 `/tmp/mdtui3-goal-run/list-start-tabs-validation.log`。对照执行前 SHA-256，仅修改一个核心文件、三项既有测试与 README/TODO/DONE，新增 `list_start_tab_code.rs`；其它前序工作区成果保持，范围核对见 `/tmp/mdtui3-goal-run/list-start-tabs-scope-check.log`。

## 2026-11-03 部分 tab 路径新嵌套列表交付

- 方言：已有路径完整匹配且确实部分消费 tab 后，剩余 0–3 列缩进可建立新 List/Item。按整条物理行四列 tab stop 计算，多层条目共享 tab；余量作为标记缩进消费，原始前缀隐藏且字节映射保留。祖先 ID 不变，任务、有序、连续嵌套、首行代码和正常容器通知/松紧复用既有流程。官方 Tabs 9 修复，当前完整章节 11 支持、0 刻意差异、0 未支持；官方 Markdown/HTML、来源生成器和评分投影算法保持。
- 边界：失败路径、lazy、分隔线、Setext、表头分隔行、围栏/数学及分类窗口规则保持；短尾在 LF/CRLF/EOF 前暂定。部分 tab 才能匹配的兄弟条目、新引用、不完整路径、纯空格首行及完整容器/空行规则仍待办。README/TODO 已同步当前评分及剩余范围，未勾选宽泛标准覆盖。
- 复杂度：复用既有部分 tab 探测筛出新列表候选，仅对候选额外检查当前有界前缀与结构优先级；固定大小游标共享空白末端，每层消耗正列数，不走未匹配祖先后缀。无新增持久状态、旧正文或历史块回扫；累计 O(n log n)、空间 O(n)、零运行时依赖和共享预算保持。初版对普通行增加额外检查的做法已移除，既有工作量上界断言保持。
- 验证：新增 `tests/nested_tabs.rs` 6 项测试，1695 组独立列宽展开模型；嵌套/有序/任务/引用外层、UTF-8/CRLF/EOF、祖先身份与通知、列表松紧和续接、失败/优先级、全切点与逐字符前缀、分类窗口与读取游标失效。源码连续覆盖/借用、tile 边界、逐字符切片、零预算停步和六字节恢复通过。1000 层容器百万字后千次追加不增加分类/容器工作，4096 组根兄弟与部分 tab 新子列表保持线性前缀工作。
- 验收命令：`nix develop -c bash /tmp/mdtui3-goal-run/nested-tabs-validate.sh`，包含 workspace check、clippy `-D warnings`、fmt、debug/release nextest、doc tests、Tabs/Indented code/Fenced code 官方夹具核验及 `git diff --check`。结果：debug/release 各 1040 项通过，doc 共 72 项通过，check/clippy/fmt/三组夹具核验和 diff 检查通过。日志 `/tmp/mdtui3-goal-run/nested-tabs-validation.log`；专项 `/tmp/mdtui3-goal-run/nested-tabs-targeted-final.log`，修复前失败 `/tmp/mdtui3-goal-run/nested-tabs-repro.log`。

## 2026-11-03 部分 tab 路径兄弟条目交付

- 方言：祖先前缀完整匹配并部分消费 tab 后，允许在已有列表原缩进位置识别同类标记兄弟；bullet 字符或有序分隔符须相同，数字可变。保留祖先与 List ID，仅关闭旧 Item 及后代并新增 Item；复用任务、标题、嵌套标记、首行代码、松紧与通知。标记前虚拟列隐藏，首行代码余量另按原规则映射到原 tab 字节。
- 边界：分隔线优先；兄弟改变作用域，不受旧 Item 的 Setext/表头分隔行优先级约束。失败、异类标记、错位缩进、缺失引用、已开启围栏/数学及分类窗口方言保持。部分 tab 表格续行、新引用、一般不完整路径、纯空格首行、完整容器/空行和宽泛标准覆盖继续待办，README/TODO 准确记录范围。
- 复杂度：只读探测在兄弟层停止，不匹配旧 Item 正文缩进及后代；结构分类确认后提交祖先游标，复用正常兄弟变更流程。固定大小游标缓存空白末端，每个成功祖先消费正列数，一个 tab 至多四列；候选增加固定次数有界前缀扫描，关闭后代各弹出一次。无新增持久状态、旧正文/历史块回扫、运行时依赖或 unsafe；累计追加 O(n log n)、空间 O(n) 和共享预算保持。
- 验证：新增 `tests/sibling_tabs.rs` 六项回归，15,660 组独立列宽展开模型；祖先/List 身份、后代关闭、松紧/通知、失败与结构优先级、UTF-8/CRLF/EOF、全切点和逐字符前缀、分类窗口与读取游标失效；tile 边界、源码连续覆盖/借用、逐字符切片、零预算停步与六字节恢复。千层容器百万字后千次追加不增加分类/容器访问，4096 个部分 tab 兄弟条目保持有界前缀工作。官方 Tabs 保持 11 支持、0 刻意差异、0 未支持，官方夹具和评分投影未改。

修复前最小复现失败见 `/tmp/mdtui3-goal-run/sibling-tabs-repro.log`；专项六项通过见 `/tmp/mdtui3-goal-run/sibling-tabs-targeted-final.log`。debug 全套 1046 项通过见 `/tmp/mdtui3-goal-run/sibling-tabs-first-full.log`；其后仅补充两条专项断言并通过 debug 专项复验，核心只调整说明注释。其余验收命令：`nix develop -c bash /tmp/mdtui3-goal-run/sibling-tabs-validate.sh`。

最终验收通过：workspace all-targets check、clippy `-D warnings`、fmt、release nextest 1046 项、doc 72 项（69 + 3）、Tabs/Indented code/Fenced code 三组官方夹具核验和 `git diff --check`；日志 `/tmp/mdtui3-goal-run/sibling-tabs-validation.log`。结合上述 debug 全套与最终专项，所有验收完成。执行前 SHA-256 范围核对确认仅修改一个核心文件及 README/TODO/DONE，新增 `sibling_tabs.rs`；其它前序成果逐字保持，见 `/tmp/mdtui3-goal-run/sibling-tabs-scope-check.log`。


## 2026-11-03 列表首行纯空格缩进代码交付

- 方言：已识别新列表及兄弟条目标记后至少五个 ASCII 空格，消费一格分隔加四格代码缩进，剩余空格及正文保持字面；条目续行宽度仅含前置缩进、标记及一格分隔。复用有序/嵌套/部分 tab 祖先路径，保留 List ID、松紧、关闭及新增通知，代码中的任务标记、强调、实体、围栏和容器标记不激活语法。
- 边界：1–4 格、完整空白行、失败/lazy、围栏/数学正文和空行终止方言保持。短尾等到 LF/CRLF/EOF；只在当前分类窗口识别，超长行按既有窗口规则提交，不回扫重分类。完整容器/空行、异类或错位兄弟、新引用、不完整路径及宽泛标准覆盖继续待办，README/TODO 同步实际范围。
- 复杂度：移除既有首行代码探测的“必须含 tab”限制，复用扫描后的空白末端和固定大小 Prefix，额外消费至多五字节（纯空格命中常量时间快路径）。没有新增持久状态、旧正文/历史块或无输入支撑的祖先回扫；累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算契约保持。
- 验证：新增 `tests/list_start_space_code.rs` 六项回归，2268 组独立空格模型（1512 组另验续接）；列表身份/子层关闭、部分 tab 祖先、松紧/通知、字面代码、空白/不足边界、UTF-8/CRLF/EOF、全切点与逐字符前缀、分类窗口、源码连续覆盖/借用、逐字符切片/tile、零预算停步及六字节恢复、缓存失效。1000 层容器百万字后千次逐字追加不增加分类/行内/容器/树访问，4096 个兄弟代码条目符合前缀工作上界。

修复前六项中四项失败，见 `/tmp/mdtui3-goal-run/space-start-repro.log`；修复后六项专项通过，见 `/tmp/mdtui3-goal-run/space-start-targeted-final.log`。全套中一条旧 `container_code` 断言按新方言更新：`-     text` 从首行开启代码，十空格续行保留四格正文；没有修改官方夹具或评分投影。

最终验收：`nix develop -c bash /tmp/mdtui3-goal-run/space-start-validate.sh` 通过；workspace all-targets check、clippy `-D warnings`、fmt、debug/release nextest 各 **1052 项**、doc **72 项（69 + 3）**、Tabs/Indented code/Fenced code 三组官方夹具核验及 `git diff --check` 均通过。完整日志 `/tmp/mdtui3-goal-run/space-start-validation.log`。SHA-256 范围核对仅修改 `containers.rs`、一条既有 `container_code` 测试及 README/TODO/DONE，新增 `list_start_space_code.rs`；其它前序成果逐字保持，核对记录 `/tmp/mdtui3-goal-run/space-start-scope-check.log`。


## 2026-11-03 部分 tab 路径新引用交付

- 方言：完整匹配已有容器路径并部分消费 tab 后，剩余 0–3 列允许开启显式 Quote；多个祖先可共用 tab，原字节前缀隐藏且祖先 ID 保持。新引用内标题、列表/任务、表格、定义、围栏/数学及代码复用原规则，范围、松紧、通知与行内作用域正常更新。官方 Tabs 保持 11 支持、0 刻意差异、0 未支持，官方夹具、生成器和评分投影均未改。
- 边界：新标记自身的 tab 分隔不另行扩展；不完整路径、缺失引用、异类/错位兄弟、超额缩进和已有围栏/数学正文保持。短尾等待 LF/CRLF/EOF，窗口外标记不回扫追认。完整容器/空行、其余叶块部分 tab 与宽泛标准覆盖继续待办，README/TODO 已同步。
- 复杂度：将 `>` 候选纳入既有部分 tab 结构探测和提交路径，内部路由字段由列表专用名称改为容器名称。固定大小 Prefix 缓存空白末端，每个成功帧消费正列数，不扫描未匹配祖先后缀；无新增持久状态、旧正文/历史块回扫、运行时依赖或 unsafe，累计 O(n log n)、空间 O(n) 和共享预算契约保持。
- 验证：新增 `tests/nested_quotes.rs` 六项回归，3051 组独立列宽模型；列表及引用祖先身份、松紧/通知、内层叶块、旧行内候选隔离、失败路径、UTF-8/CRLF/EOF、全切点与逐字符前缀及分类窗口。源码连续覆盖/借用、逐字符切片/tile、代码余量映射、零预算停步与六字节恢复、读取游标失效通过。999 层容器百万字后千次追加不增加分类/容器访问，4096 组根兄弟及新引用符合前缀工作上界。

修复前最小复现失败见 `/tmp/mdtui3-goal-run/nested-quotes-repro.log`；两处旧 leaf/prose tab 测试观察按新引用语义更新，保留其余失败边界。验收命令：`nix develop -c bash /tmp/mdtui3-goal-run/nested-quotes-validate.sh`。

最终验收全部通过：workspace all-targets check、clippy `-D warnings`、fmt、debug/release nextest 各 **1058 项**、doc **72 项（69 + 3）**、Tabs/Indented code/Fenced code 三组官方夹具核验及 `git diff --check`。完整日志 `/tmp/mdtui3-goal-run/nested-quotes-validation.log`。SHA-256 范围核对仅修改两个核心文件、两处既有测试及 README/TODO/DONE，新增 `nested_quotes.rs`；其它前序成果逐字保持，记录 `/tmp/mdtui3-goal-run/nested-quotes-scope-check.log`。


## 2026-11-03 列表完整边界续行 tab 缩进代码交付

- 方言：已有路径完整匹配且最内层为 Item，在普通正文续接未启用时，完整字节边界也接受四列 tab/混合代码缩进。沿物理行 tab stop 消费，代码缩进末端的部分 tab 余量映射原字节，其后完整空白/语法保留字面；保留原 Item/List 身份、Loose 确认和相邻代码合并。
- 边界：普通正文/lazy、不完整路径、完整纯空白、围栏和数学正文维持原行为；空行终止代码，短尾等待 LF/CRLF/EOF，分类窗口之后不重试。完整容器/空行及宽泛标准覆盖未勾选，README/TODO 已同步。
- 复杂度：移除代码探测的“路径已经部分消费 tab”前提，成功代码探测可独立提交；仍先要求有界前缀含 tab。固定大小 Prefix 复用空白末端，额外消费至多四列，没有新增持久状态、旧正文/历史块或无输入支撑的祖先回扫。累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 和共享预算保持。
- 验证：新增 `tests/full_item_tab_code.rs` 五项回归，414 组独立列宽模型（276 组另验续接），嵌套/有序/引用路径、身份/松紧、失败边界、UTF-8/CRLF/EOF 全切点及逐字符前缀、分类窗口与缓存失效。源码连续覆盖/借用、tile 边界单字节替换、逐字符切片、零预算停步及六字节恢复通过；500 层容器百万字后千次追加不增加分类/行内/容器/树访问，4096 行代码合并并符合工作上界。

修复前专项五项中四项失败见 `/tmp/mdtui3-goal-run/full-item-repro.log`；其中映射用例随后收窄到完整字节边界路径。验收命令：`nix develop -c bash /tmp/mdtui3-goal-run/full-item-validate.sh`。


最终验收通过：workspace all-targets check、clippy `-D warnings`、fmt、release nextest **1063 项**、doc **72 项（69 + 3）**、Tabs/Indented code/Fenced code 三组官方夹具核验及 `git diff --check`，日志 `/tmp/mdtui3-goal-run/full-item-validation.log`。debug 全套 **1063 项**通过见 `/tmp/mdtui3-goal-run/full-item-first-full.log`；随后仅收窄两个分块测试输入、固定模型数量断言及调整核心注释，最终 debug 专项 **5 项**再次通过（同最终验收日志）。官方夹具/生成器/评分投影和既有测试均未修改。

SHA-256 范围核对仅修改 `containers.rs` 与 README/TODO/DONE，新增 `full_item_tab_code.rs`；其余前序成果逐字保持，记录 `/tmp/mdtui3-goal-run/full-item-scope-check.log`。


## 2026-11-03 围栏正文 tab 列宽去缩进交付

- 方言：已开启反引号/波浪线围栏的正文先消费已有容器部分 tab 虚拟列，再按 opener 的剩余 0–3 列额度消费 ASCII space/tab。tab 沿物理行四列对齐，部分消费后的余量显示为空格、映射原单字节；其余 tab 与代码语法保留字面，源文与 info 借用不变。
- 边界：零缩进 opener、数学块、缺失路径、开闭定界与分类窗口保持原方言；closer 在去缩进前判断，原始 tab 开头的候选仍作为正文。未完成短行等 LF/CRLF/EOF 或分类窗口提交，既有身份、通知与缓存失效保持。完整围栏/容器规则、长度上限、info 解码和宽泛标准覆盖未勾选。
- 复杂度：活动围栏新增一个列模四字节。需要正文去缩进时，当前有界分类前缀做一次字节列位置计算，后续追加只携带剩余额度和列位置；最多消费三列，没有历史正文/块/祖先节点回扫或续传前缀重读。实际检查计入 classified_bytes，部分 tab 复用显示映射与预算接口；累计追加 O(n log n)、空间 O(n)、零运行时依赖与禁止 unsafe 保持。
- 验证：新增 `tests/fence_tabs.rs` 五项回归，1008 组独立列宽模型，部分 tab 祖先与作用域、失败 closer/数学/空白、UTF-8/CRLF/EOF 全切点/逐前缀、缓存失效、连续源码映射、tile 边界替换/逐字符切片及六字节预算恢复。1015 层分类窗口续传、百万字后千次追加与 4096 行代码通过；百万字后新增输入不增加分类/行内/容器/树访问。修复前五项均失败，见 `/tmp/mdtui3-goal-run/fence-tabs-repro.log`；窗口用例的预期随后按物理列宽修正为剩余两列。

旧 `fence_indent.rs` 一条 tab 字面观察按新方言更新；官方夹具、生成器及评分投影不变。验收命令：`nix develop -c bash /tmp/mdtui3-goal-run/fence-tabs-validate.sh`。

最终验收全部通过：workspace all-targets check、clippy `-D warnings`、fmt、debug/release nextest 各 **1068 项**、doc **72 项（69 + 3）**、Tabs/Indented code/Fenced code 三组官方夹具核验及 `git diff --check`。debug 全套日志 `/tmp/mdtui3-goal-run/fence-tabs-first-full.log`；最终专项 16 项与其余验收日志 `/tmp/mdtui3-goal-run/fence-tabs-validation.log`。

SHA-256 范围核对仅修改 `lib.rs`、`block.rs`、既有 `fence_indent.rs` 与 README/TODO/DONE，新增 `fence_tabs.rs`；其它前序文件逐字保持，记录 `/tmp/mdtui3-goal-run/fence-tabs-scope-check.log`。


## 2026-11-03 分类窗口内的长围栏交付

- 方言：反引号/波浪线 opener 从 3..32 扩展为 3..1023，必须在原 1024 字节分类窗口内看到定界串结束（非同字符或完整行末）；容器前缀与缩进占用同一窗口。窗口末端仍在串内时保持普通行内消费，也避免深容器中的短串被提前截断确认。短尾等待 LF/CRLF/EOF，已确认围栏在 EOF 保留代码。
- 边界：同字符且不短于 opener 的 closer 仍须连同行末在窗口内完整出现，短串/异字符/非空白后缀/四格缩进及超窗口 closer 保留字面。原始 info、部分 tab 正文映射、容器作用域退出和长 info 隐藏方言保持；窗口外定界识别、info 解码及完整容器/tab 规则仍待办，不勾选宽泛标准覆盖。
- 复杂度：只调整 `MAX_FENCE_LENGTH` 和一次切片长度比较，仍单次扫描当前有界前缀；Fence 已用 usize 保存长度，无新增持久状态、额外窗口、旧正文/祖先回扫、运行时依赖或 unsafe。累计 O(n log n)、空间 O(n) 及共享预算保持。
- 验证：新增 `tests/long_fences.rs` 五项回归，504 组独立长度/缩进/容器/行末模型，33/63/255/1000/1023 边界、失败 closer、原始 info、UTF-8/CRLF/EOF 全切点与逐字符前缀、完整源码覆盖/借用、部分 tab 替换/切片及零预算停步与六字节恢复。成功和失败路径在千层容器、百万字正文后千次追加不增加分类/容器计数，代码路径不增加行内计数，普通回退按新增字节消费。

修复前 33 字符 opener 模型失败见 `/tmp/mdtui3-goal-run/long-fences-repro.log`；专项验收见 `/tmp/mdtui3-goal-run/long-fences-targeted.log`。完整验收命令：`nix develop -c bash /tmp/mdtui3-goal-run/long-fences-validate.sh`。


全套首次验收指出 `container_code.rs` 原有“33 个波浪线无效”断言已不适用；仅将该输入换成超过 `MAX_FENCE_LENGTH` 的串，继续验证超长失败，33 字符成功由新模型覆盖。首次日志 `/tmp/mdtui3-goal-run/long-fences-first-full.log`；修改范围核对 `/tmp/mdtui3-goal-run/long-fences-scope-check.log`，其它前序文件 SHA-256 保持；核心及既有测试撤去本单元精确改动后亦与前序摘要一致。

最终验收通过：workspace all-targets check、clippy `-D warnings`、fmt、debug/release nextest 各 **1073 项**、doc **72 项（69 + 3）**、Tabs/Indented code/Fenced code 三组官方夹具逐字核验及 `git diff --check`。完整日志 `/tmp/mdtui3-goal-run/long-fences-validation.log`。官方输入/期望、生成器和评分投影未修改。


## 2026-11-04 围栏 closer 的 tab 缩进交付

- 方言：完整匹配已有容器路径后，反引号/波浪线 closer 前的 space/tab 沿物理行四列制表位计量，含部分 tab 虚拟余量不超过三列才闭合。关闭判断先于正文去缩进，复用相同字符、至少 opener 长度及既有 ASCII 空白后缀规则；容器和代码块身份保持。
- 边界：短行等待 LF/CRLF/EOF；超过分类窗口的候选维持字面代码，不追认后续串尾。超过三列、短串、异字符、非空白后缀仍为正文；数学块、opener、缺失路径和兄弟切换规则保持。完整容器/opener、窗口外识别、info 解码及宽泛标准覆盖仍未勾选。
- 复杂度：当前有界窗口内读取一次已匹配前缀的字节取得列模四，计入 `Work.classified_bytes`，并复用于正文去缩进。只检查当前输入提供的前缀，不遍历祖先节点，不增持久状态，不重读旧正文或在续传时重算前缀。缩进超过三列即失败，累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 和共享预算保持。
- 验证：新增 `tests/fence_closer_tabs.rs` 五项回归，1920 组独立列宽/长度/行尾模型，根/引用/列表/部分 tab、UTF-8 全切点/逐前缀/EOF、作用域/数学/失败/窗口边界、稳定身份/缓存失效、tile 边界源码连续覆盖/逐字符切片及六字节预算停步恢复。千层容器百万字正文、长失败 closer 后千次追加不增加分类/行内/容器/树访问；4096 次围栏开闭验证工作计数上界。

README/TODO 已同步。原有测试、官方夹具、生成器与评分投影均保持；范围核对见 `/tmp/mdtui3-goal-run/closer-tabs-scope-check.log`。首轮专项只修正新测试夹具的一个字节位置，使 tab 恰在 tile 边界；随后 debug 全套 1078 项通过，日志 `/tmp/mdtui3-goal-run/closer-tabs-first-full.log`。独立副本使用单元开始前核心运行新测试，日志 `/tmp/mdtui3-goal-run/closer-tabs-repro.log`。

最终验收通过：workspace all-targets check、clippy `-D warnings`、fmt、debug/release nextest 各 **1078 项**、doc **72 项（69 + 3）**、Tabs/Indented code/Fenced code 三组官方夹具逐字核验及 `git diff --check`。静态检查仅要求将新测试的一处等价边界表达式简化，核心自首次全套验收后未再改变。release/静态/文档与夹具日志 `/tmp/mdtui3-goal-run/closer-tabs-validation.log`；最终 debug 专项 `/tmp/mdtui3-goal-run/closer-tabs-targeted-final.log`。单元开始前核心在独立副本和独立构建目录中运行新增测试，**5 项全部失败**，确认回归能检出缺失行为；当前实现五项全部通过。


## 2026-11-04 围栏 opener 的 tab 缩进交付

- 方言：已识别容器前缀之后，反引号/波浪线 opener 前的 space/tab 按物理行四列制表位计量，含部分 tab 虚拟余量至多三列；该列数用于既有正文去缩进。开启行隐藏，原始 info、容器身份、源码字节与通知契约保持。
- 边界：现有匹配器确定前缀边界，新引用标记自身的 tab 分隔不在本项内。顶层 tab/超过三列、短串、反引号 info 内反引号、缺失路径、数学及其它叶块维持原规则；短行在 LF/CRLF/EOF 确认，分类窗口和定界长度上限保持，超窗串不追认，长 info 仍截在窗口内。完整容器/其它叶块、窗口外识别、info 解码及宽泛标准覆盖仍未勾选。
- 复杂度：原始 tab 缩进候选与活动代码围栏从当前有界前缀取得列模四，并计入 `Work.classified_bytes`；部分 tab 探测复用现成列位置。普通无 tab 叶块不额外扫描前缀；缩进探测至多四字节，无新增持久状态、旧正文或祖先节点回扫。累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 和共享预算保持。
- 验证：新增 `tests/fence_opener_tabs.rs` 五项回归，720 组独立列宽/定界/行尾模型、UTF-8 全切点/逐前缀/EOF、失败/数学/作用域/窗口边界、原始 info 与晚确认身份/缓存失效、tile 边界连续源码映射/逐字符切片及六字节预算停步恢复。千层容器百万字后千次追加保持分类/行内/容器/树访问不变，超窗失败 opener 无重新分类，4096 次开闭符合计数上界。

修复前的模型测试检出 `> \t``` rs` 未识别，日志 `/tmp/mdtui3-goal-run/opener-tabs-repro.log`。一条旧容器代码测试将相同输入的失败观察改为成功断言；官方夹具、生成器及评分投影均未改。首次全套发现普通深层续行多扫前缀超出已有计数门槛，随后将扫描限制到 tab 候选；该测试及新专项合计 19 项通过，日志 `/tmp/mdtui3-goal-run/opener-tabs-targeted-final.log`。README/TODO 已同步，宽泛条目保持未完成。

最终验收通过：workspace all-targets check、clippy `-D warnings`、fmt、debug/release nextest 各 **1083 项**、doc **72 项（69 + 3）**、Tabs/Indented code/Fenced code 三组官方夹具逐字核验及 `git diff --check`。日志 `/tmp/mdtui3-goal-run/opener-tabs-validation.log`；范围核对 `/tmp/mdtui3-goal-run/opener-tabs-scope-check.log`，除本单元三处核心、一处旧断言、新测试及 README/TODO/DONE 外，其余 265 个基线文件 SHA-256 保持。


## 2026-11-04 ATX 标题的 tab 缩进交付

- 方言：已识别容器前缀后，ATX 井号前 space/tab 按物理行四列制表位计量，含部分 tab 余量至多三列即可识别；`> \t# title` 和 `>\t# title` 分别使用两列、三列叶块缩进。标题身份/作用域、1–6 级、原始源码、关闭井号及既有行内语义保持。
- 边界：不修改引用标记自身可选一列分隔；顶层 tab/超过三列、七个井号、缺少分隔和不完整路径保留原分类，围栏/数学字面正文保持。短尾在 LF/CRLF/EOF 确认，井号串/必要分隔受既有共享窗口限制，截断串不追认；完整容器/其它叶块和宽泛标准覆盖保持未完成。
- 复杂度：直接复用围栏分类已取得的列位置与至多四字节缩进结果，无额外前缀扫描、持久状态或旧正文/祖先回扫；累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 及共享预算不变。
- 验证：新增 `tests/atx_tabs.rs` 六项回归，1848 组独立列宽/级别/行尾模型，UTF-8 全切点及逐前缀/EOF、失败/窗口/作用域、晚确认身份/章节与游标失效、tile 边界连续源码映射/逐字符切片/六字节预算；搜索、slug、折叠消费及通知增量；千层百万字标题后千次追加不增加分类/容器工作，仅处理新增行内字节，超长失败串不重新分类，4096 个标题符合计数上界。

修改前五项专项全失败，日志 `/tmp/mdtui3-goal-run/atx-tabs-repro.log`。首次全套发现四条旧测试仍期待 `>\t#` 为普通文字，现与围栏 opener 测试的一条旧 ATX 观察一起准确更新；不修改官方输入/期望、生成器或评分投影。README/TODO 同步记录实际交付和剩余范围。

最终验收：workspace all-targets check、clippy `-D warnings`、fmt，debug/release nextest 各 **1089 项**，doc **72 项（69 + 3）**，ATX/Tabs/Indented code/Fenced code 四组官方夹具逐字核验及 `git diff --check` 全部通过。日志 `/tmp/mdtui3-goal-run/atx-tabs-validation.log`；范围核对 `/tmp/mdtui3-goal-run/atx-tabs-scope-check.log`，除本单元一处核心、五处旧测试、新测试及 README/TODO/DONE 外，其余 264 个基线文件 SHA-256 保持。


## 2026-11-04 分隔线的 tab 缩进交付

- 方言：已识别容器前缀后的 space/tab 按物理行四列制表位计量，含部分 tab 余量至多三列时识别至少三个同类 `*`/`-`/`_` 的分隔线；标记间/末尾允许 space/tab。完整匹配路径保留 ContainerId，分隔线先于嵌套列表；既有 ASCII-space Setext 优先级保留，原始 tab Setext 未扩展。隐藏容器前缀，叶块内原始缩进和标记保持既有字面显示语义，部分 tab 前缀余量不产生可见替换。
- 失败/流式：超额缩进、少于三个标记、异类/非空白后缀、缺失路径及围栏/数学正文沿用原规则；短行等待 LF/CRLF/EOF，整行须在共享分类窗口内完成，超窗候选不回扫追认。完整容器、新引用自身分隔、其它叶块、窗口外识别与宽泛标准覆盖继续待办。
- 实现与复杂度：`block.rs` 复用围栏/ATX 的有界列宽结果；`containers.rs` 的原子部分 tab 探测接纳 Rule，结构优先级使用已有后缀摘要和缓存列宽，删除 ASCII-only 限制。无新增持久状态，不回扫旧正文、历史块或无输入支撑的祖先；累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 及共享预算契约保持。
- 专项：`tests/rule_tabs.rs` 新增 5 项回归，2376 组独立列宽模型覆盖三种标记、计数、混合缩进、完整/部分边界和 LF/CRLF/EOF；验证作用域/身份、优先级/失败/窗口、全部 UTF-8 切点/逐前缀/EOF 快照、源码连续覆盖/借用/切片/tile、零预算停步和六字节恢复。千层引用中的百万字失败候选后千次追加不增加分类字节、容器字节/步数；4096 行验证线性计数。
- 验收：`nix develop -c cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`；devShell 内 nextest debug/release 各 **1094 项通过**，`cargo test --doc` **72 项通过**。CommonMark Thematic breaks/Tabs/Setext/Fenced code 四组固定来源夹具校验通过；官方输入/期望、生成器和评分投影未改。首次全套仅一条旧部分 tab 分隔线回退观察不符，`nested_tabs.rs` 现断言原条目内分隔线显示（新专项另验结构）；最终全套通过。
- 证据：修复前模型/优先级缺口见 `/tmp/mdtui3-goal-run/rule-tabs-repro.log`；最终专项 `/tmp/mdtui3-goal-run/rule-tabs-targeted-final.log`；全套 `/tmp/mdtui3-goal-run/rule-tabs-validation.log`；范围核对 `/tmp/mdtui3-goal-run/rule-tabs-scope-check.log`。README/TODO 同步更新，宽泛待办未勾选。

## 2026-11-04 Setext 下划线的 tab 缩进交付

- 方言：已有同作用域段落或待确认表头之后，完整匹配容器前缀，按物理行四列制表位计量下划线前的 space/tab；含部分 tab 余量至多三列时，连续 `=` / `-` 提升原段落为一级/二级标题。保留 BlockId、ContainerId、跨行内容、索引及通知，整条下划线隐藏且原始源码/字节映射保持；优先于分隔线及空嵌套列表。
- 边界：无同作用域段落、空行隔断、缺失路径、兄弟条目、超额缩进、失败标记/后缀和已开启围栏/数学保持原规则。短尾等待 LF/CRLF/EOF，下划线整行须在包含容器前缀的共享分类窗口内完成，超窗候选不回扫追认。完整容器、其它叶块、窗口外识别及宽泛标准覆盖保持待办。
- 实现与复杂度：`block.rs` 将 Setext 判定接到已有有界列宽结果，`containers.rs` 的部分 tab 探测接纳 Setext，`lib.rs` 传入同作用域段落资格。只读当前有界前缀，不新增持久状态或回扫旧正文/历史叶块/无输入支撑的祖先；保留累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 及共享预算。
- 专项：新增 `tests/setext_tabs.rs` 六项测试，1728 组独立列宽/标记/行尾模型，UTF-8 全切点、逐字符前缀及 EOF 快照，优先级/失败/分类窗口，标题身份与折叠增量，原始源码及切片/tile 显示映射，零预算停步和六字节显示/搜索/slug 恢复。998 层引用中的百万字标题晚确认不增加旧正文行内读取；百万字失败候选追加不重新分类。
- 验收：devShell 内 all-targets check、clippy `-D warnings`、fmt；debug/release nextest 各 **1100 项通过**，doc **72 项（69 + 3）通过**。CommonMark Setext/Tabs/Thematic breaks/Fenced code 四组官方夹具逐字核验通过，官方输入/期望、生成器、评分投影未改。首轮全套发现三项旧回退预期，`leaf_tabs`、`nested_tabs`、`rule_tabs` 已准确更新，最终全套通过；README/TODO 同步实际交付及剩余范围。
- 证据：修改前专项的列宽模型、优先级及标题晚确认复现缺口，见 `/tmp/mdtui3-goal-run/setext-tabs-repro.log`；最终相关专项 **31 项通过**，见 `/tmp/mdtui3-goal-run/setext-tabs-targeted-final.log`；全套 `/tmp/mdtui3-goal-run/setext-tabs-validation.log`；范围核对 `/tmp/mdtui3-goal-run/setext-tabs-scope-check.log`。除三处核心、三处旧测试、新测试及 README/TODO/DONE 外，其余 266 个基线文件 SHA-256 保持，`git diff --check` 通过。


## 2026-11-04 引用链接定义的 tab 缩进交付

- 方言：已识别容器前缀后的 space/tab 按物理四列制表位计量，含部分 tab 虚拟余量至多三列时识别完整有界引用链接定义；保留 ContainerId、原始目标/title 借用区间、全局首定义优先及引用失效，整条定义行隐藏且源码字节保持。
- 边界：短行等待 LF/CRLF/EOF，定义整行必须在共享分类窗口内完成；超额缩进、非法候选、缺失路径、围栏/数学与脚注定义沿用原方言。已有段落/表格可被定义中断；不宣称完整标准引用定义规则，元数据解码、脚注/其它叶块、完整容器与窗口外识别继续待办。
- 实现与复杂度：`block.rs` 的定义判定复用现有有界列宽结果，`containers.rs` 的部分 tab 原子探测接纳 Definition；现有符号更新和显示消费无需改动。无新增持久状态、旧正文/历史块/历史链接回扫；累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算保持。
- 专项：新增 `tests/definition_tabs.rs` 五项测试，864 组独立列宽/目标/行尾模型，全 UTF-8 切点、逐字符前缀及 EOF 快照，失败/窗口/表格优先级、原始元数据/作用域/首定义优先/通知，连续源码及 tile/逐字符显示映射，零预算停步和六字节恢复。998 层引用中百万字后千次追加及晚定义不回读旧行内正文；超窗百万字失败候选不重新分类；两万旧链接的晚定义不枚举旧对象，4096 个定义符合工作计数上界。
- 修复前五项专项中四项失败，分块一致性一项通过，见 `/tmp/mdtui3-goal-run/definition-tabs-repro.log`。首轮全套发现 `leaf_tabs` 一条旧观察仍要求引用定义失败，现改为原条目内定义及原始目标断言，脚注/表头失败观察保留。新窗口测试补入物理 tab 占四列的拒绝条件；核心实现自首轮专项后未改变。官方夹具、生成器和评分投影未改，README/TODO 已同步实际交付与剩余范围。
- 验收：devShell 内 `cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check` 通过；debug/release nextest 各 **1105 项通过**，doc **72 项（69 + 3）通过**，日志 `/tmp/mdtui3-goal-run/definition-tabs-validation.log`。Tabs/Backslash escapes/Entities/Fenced code 四组固定来源夹具逐字核验通过，日志 `/tmp/mdtui3-goal-run/definition-tabs-fixture-check.log`。范围核对 `/tmp/mdtui3-goal-run/definition-tabs-scope-check.log` 确认仅两处核心、一处旧测试、一个新增测试及三份文档发生本单元改动，其余前序文件保持；`git diff --check` 通过。

## 2026-11-04 脚注定义起始行的 tab 缩进交付

- [x] **方言与边界**：已识别容器前缀后的 space/tab 按物理行四列制表位计量，连同部分 tab 余量至多三列时识别合法 `[^label]:`。标记必须落在共享分类窗口内，正文可跨窗口流式增长；短尾等待 LF/CRLF/EOF，失败/未完成/窗口外候选不追认。保留原始正文、作用域、全局首定义优先、稳定编号及间接引用；定义可中断普通段落、待定表头和已确认表格。同路径四个 ASCII 空格续行及空白分段保持，未扩展 tab/lazy 续行、正文解析或实际脚注区排版。
- [x] **实现与复杂度**：`block.rs` 将原有脚注起始标记识别移到共享列宽扫描之后；`containers.rs` 的部分 tab 原子探测接纳脚注定义。复用已有消费/隐藏/通知路径，无新增持久状态、旧正文回扫或历史引用遍历，累计 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持。
- [x] **专项验收**：新增 `tests/footnote_tabs.rs` 的 5 项回归；864 组独立物理列宽模型覆盖根、引用、列表、多层及部分 tab，验证作用域、空正文、原始实体/标记和 CRLF/EOF。全 UTF-8 切点、逐字符前缀及前缀 EOF 快照一致；失败/窗口、四空格续行、代码/数学边界、晚定义与重复定义、稳定身份、源码覆盖、脚注 payload 和显示读取的六字节预算恢复通过。998 层百万字脚注后的 1000 次追加不增加分类/行内/容器扫描；两万旧引用晚定义与 4096 后续定义的工作计数通过。
- [x] **验证结果**：修改前新增专项 4 失败/1 通过（`/tmp/mdtui3-goal-run/footnote-tabs-repro.log`），修复后 5 项通过。首轮全套仅有两处旧脚注失败预期，已更新为新定义成功及作用域/正文断言；最终 `nix develop -c` 中 check/clippy（`-D warnings`）/fmt、debug/release nextest 各 1110 项、doc 72 项和 `git diff --check` 均通过，日志 `/tmp/mdtui3-goal-run/footnote-tabs-validation.log`。其余 270 个前序文件 SHA-256 保持，包括官方夹具、生成器和评分投影；范围记录 `/tmp/mdtui3-goal-run/footnote-tabs-scope-check.log`。

README/TODO 已记录实际交付和剩余范围；脚注正文解析/tab 续行、其它叶块、完整容器与宽泛标准覆盖仍未勾选。


## 2026-11-04 独立数学块定界行的 tab 缩进交付

- [x] **方言与边界**：已识别/完整匹配容器前缀之后，独占行 `$$` opener/closer 的 space/tab 按物理行四列制表位计量，含前缀部分 tab 余量至多三列。整条定界行必须在共享分类窗口内完成，后缀仅 space/tab；短尾等待 LF/CRLF/EOF，超窗候选不在后续追加中追认。失败、四列缩进、额外美元符号、孤立 CR、缺失路径和代码围栏保持既有边界。开栏缩进不从数学正文扣除，完整正文 tab 保持字面；原始源码、BlockId、ContainerId、折叠和通知保持。
- [x] **实现与复杂度**：`block.rs` 的数学 opener 移到既有叶块列宽扫描之后，closer 使用既有围栏 tab 列宽计算；`lib.rs` 在数学行含原始 tab 缩进时计算当前有界前缀列模四，计入分类工作。部分 tab 原子探测和数学消费/隐藏路径复用，无新增持久状态、旧正文或祖先节点回扫；无 tab 的数学正文不增加前缀扫描，累计 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算保持。
- [x] **专项验收**：新增 `tests/math_delimiter_tabs.rs` 六项测试，576 组独立物理列宽模型覆盖 opener/closer、根/引用/列表/部分 tab 和 CRLF/EOF；全 UTF-8 切点、逐字符前缀及前缀 EOF 快照一致。失败路径/分类窗口、稳定身份、折叠意愿、增量合并与过期读取拒绝、原始/去前缀 payload 源码映射及六字节预算通过。998 层百万字正文与超窗失败 closer 后各千次追加不增加分类/容器/行内扫描，4096 次开闭符合计数上界。
- [x] **修复证据**：修改前专项 5 失败/1 通过，见 `/tmp/mdtui3-goal-run/math-delimiter-repro.log`；修复后 6 项通过，见 `/tmp/mdtui3-goal-run/math-delimiter-targeted.log`。首轮全套 1114 通过/2 失败，仅两处围栏旧测试仍要求数学 tab 定界失败，已更新为成功及显示隐藏断言；核心自专项通过后未再修改。首轮记录 `/tmp/mdtui3-goal-run/math-delimiter-first-full.log`。

- [x] **最终验证**：devShell 内 `cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release nextest 各 **1116 项**、doc **72 项（69 + 3）** 和 `git diff --check` 均通过；日志 `/tmp/mdtui3-goal-run/math-delimiter-validation.log`。范围核对 `/tmp/mdtui3-goal-run/math-delimiter-scope-check.log` 确认其余 271 个前序文件 SHA-256 保持，包括官方夹具、生成器和评分投影。

README/TODO 已同步实际交付；新引用自身分隔、完整容器、额外数学方言/实际排版、其它叶块和宽泛标准覆盖继续待办。


## 2026-11-04 脚注续行的完整 tab 边界交付

- [x] **方言与边界**：已有定义完整匹配确切容器路径后，space/tab 按物理行四列制表位计量；四列终点落在完整字节边界时消费缩进并延续同一 BlockId。多余空白/正文保持字面，原始包络、首定义优先、稳定编号、隐藏和范围通知保持。完整空白分段同样消费可匹配的四列；超窗空白已有合法缩进时流式增长。部分 tab 容器路径/四列终点、lazy 省略仍拒绝，不扩展正文 Markdown 解析或脚注区排版。
- [x] **实现与复杂度**：`containers.rs` 抽出只读路径游标，原表格分隔行探测仍返回原字节偏移；脚注探测复用游标列位置、空白末端和完整边界缩进消费，额外扫描计入 `Work.container_bytes`。`footnotes.rs` 复用现有隐藏间隙与定义增长，不新增持久状态，不回扫旧正文/历史引用/无输入支撑的祖先，保持累计 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 和共享预算。
- [x] **专项验收**：新增 `tests/footnote_continuation_tabs.rs` 五项回归，1701 组独立列宽模型覆盖根、引用、列表和原始 tab 容器前缀，验证 UTF-8/CRLF/EOF、身份与原始字面正文；全切点、逐前缀及前缀 EOF 快照一致。空白/失败/重复定义、原文读取及六字节映射预算通过。991 层百万字续行后千次追加不增加分类/容器/行内扫描；两万旧引用与 4096 次续行的通知和操作计数通过。
- [x] **修复证据**：实现前专项四项失败、一项分块一致性通过，见 `/tmp/mdtui3-goal-run/footnote-continuation-repro.log`。原脚注测试中两项包含旧 tab 拒绝/空白保留预期，已按新方言更新成功、正文和长行断言；其余拒绝及完整空格行为保持。最终 debug 全套 1121 项通过，见 `/tmp/mdtui3-goal-run/footnote-continuation-debug.log`。

README/TODO 已同步；只勾选本子项，部分 tab/lazy 续行、完整容器、正文解析/实际排版及宽泛标准覆盖仍待办。

- [x] **最终验证**：devShell 内 `cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release nextest 各 **1121 项**、doc **72 项（69 + 3）** 及 `git diff --check` 均通过；日志 `/tmp/mdtui3-goal-run/footnote-continuation-validation.log`。范围核对 `/tmp/mdtui3-goal-run/footnote-continuation-scope-check.log` 确认其余 273 个仓库文件 SHA-256 保持，包括官方夹具、生成器和评分投影。


## 2026-11-04 脚注续行的部分 tab 容器路径交付

- [x] **方言与边界**：已有脚注完整匹配原容器路径时，允许多层 Item 共享 tab 和 Quote 可选分隔消费一列；路径余量参与四列续行缩进，终点必须落在字节边界。成功后保持 BlockId、容器身份、原始包络、字面正文、稳定编号和首定义优先；失败原子回退到原完整边界/空白方言，不生成虚拟正文。短尾等待 LF/CRLF/EOF，分类窗口与长正文流式消费保持。四列终点的部分 tab、lazy/完整容器、正文 Markdown 解析与实际排版仍待办。
- [x] **实现与复杂度**：仅扩展 `containers.rs` 的只读脚注路径探测；表格分隔路径保持原规则。固定大小游标复用列位置和空白末端，仅消费过部分 tab 的失败路径才重试原拼写；至多两次路径匹配仅检查当前有界前缀，全部计入容器工作。无新增持久状态、旧正文/历史引用回扫，累计 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算保持。
- [x] **专项验收**：新增 `tests/footnote_partial_paths.rs` 五项回归，1512 组独立物理列宽/行尾模型涵盖多层共享 tab、引用/列表交错、余量及既有引用失败回退；UTF-8 全切点、逐字符前缀及前缀 EOF 快照一致。失败/空白/窗口/重复定义、原文包络与连续六字节预算映射通过。982 层容器百万字后千次追加无分类/容器/行内回扫，512 条深层续行及两万旧引用后的 4096 次增长保持身份、通知和工作计数。
- [x] **修复证据**：修改前新增专项四项失败、一项分块一致性通过，见 `/tmp/mdtui3-goal-run/footnote-partial-repro.log`。首次全套发现两处普通失败路径重复探测增加工作计数，已修复且保留原性能断言；一条既有脚注测试的部分 tab 拒绝预期按新增支持更新。相关专项最终 38 项全部通过，见 `/tmp/mdtui3-goal-run/footnote-partial-targeted-final.log`。

README/TODO 已同步实际交付与剩余范围，宽泛待办保持未完成。

- [x] **最终验证**：devShell 内 `cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、debug/release nextest 各 **1126 项**、doc **72 项（69 + 3）** 和 `git diff --check` 均通过；日志 `/tmp/mdtui3-goal-run/footnote-partial-validation.log`。范围核对 `/tmp/mdtui3-goal-run/footnote-partial-scope-check.log` 确认其余 275 个前序文件 SHA-256 保持，包括官方夹具、生成器和评分投影。


## 2026-11-04 脚注续行的部分 tab 终点交付

- [x] **方言与边界**：完整匹配已有容器路径后消费恰好四列续行缩进，终点可落在 tab 内，余下 1..3 列映射到末尾 tab 的单字节源码；原始读取、后续完整 tab、字面正文、空白分段、首定义优先、稳定身份与通知保持。不足缩进/缺失路径沿用原子回退，短尾与分类窗口规则保持；lazy/完整容器、正文 Markdown 解析与实际排版仍待办。
- [x] **实现与复杂度**：复用只读前缀游标的部分 tab 缩进消费；每个获胜续行前缀增加固定 `u8` 余量（含平台结构对齐开销），重复定义不保存映射，同物理行增长不增加记录。读取器把隐藏前缀与末尾 tab 分开返回，tab 映射为静态空格，原子收取 `max(source.len(), text.len())` bytes、1 query、1 object，预算不足不推进也不扣费。无正文复制或旧引用/祖先回扫，记录空间 O(续行数)，累计解析 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持。
- [x] **专项验收**：新增 `tests/footnote_endpoint_tabs.rs` 五项回归，1701 组独立列宽模型、UTF-8 全切点/逐前缀/前缀 EOF、1..3 列映射、零/不足/六字节预算、原文与失败/空白/重复定义通过。982 层容器百万字正文后千次追加无分类/容器/行内回扫，512 条深层续行及两万旧引用后的 4096 次增长符合计数边界。内部新增测试验证前缀记录只随获胜续行增长，不随同物理行正文或重复定义增长。
- [x] **修复证据**：新增专项在实现前四项失败、一项分块一致性通过，见 `/tmp/mdtui3-goal-run/footnote-endpoint-repro.log`。首轮全套仅两组旧脚注测试中的六项失败，原因是旧预算/直接借用断言及完整终点模型不再适用；按新方言更新两组模型、映射计费及两条拒绝预期，相关 17 项最终通过，见 `/tmp/mdtui3-goal-run/footnote-endpoint-targeted-final.log`。

README/TODO 已同步实际交付与剩余范围，宽泛待办保持未完成。

- [x] **最终验证**：devShell 内 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、debug/release nextest 各 **1132 项**、doc **72 项（69 + 3）**、`cargo fmt --all -- --check` 及 `git diff --check` 全部通过；日志 `/tmp/mdtui3-goal-run/footnote-endpoint-validation.log`。范围核对 `/tmp/mdtui3-goal-run/footnote-endpoint-scope-check.log` 确认其它 274 个前序文件 SHA-256 保持，包括官方夹具、生成器和评分投影；本单元独立差异保存于 `/tmp/mdtui3-goal-run/footnote-endpoint-unit.patch`。

## 2026-11-04 围栏 info 反斜线解码读取交付

- 从围栏代码及 Backslash escapes 剩余范围中交付 info 的 ASCII 标点反斜线解码读取。新增 `Document::code_info(BlockId)`、`CodeInfo` 和 `CodeInfoChunk`；原始 `BlockKind::Code { info }` 区间、借用读取、块身份及解析行为保持。非标点、Unicode、实体和末尾反斜线保留字面，输出不递归解析、不额外 trim 或选择语言词。
- 游标固定四个机器字、零分配；构造 O(1)，读取 O(info 字节数)。每步一个字符，消费一次 query/object 与实际源码字节数；两字节转义对原子消费，UTF-8 最多四字节，预算不足不推进也不计费。借用防止读取期间修改文档；无新增持久解析状态、正文复制、历史块或祖先回扫。已有分类窗口和长 info 截断方言保持，窗口末尾反斜线不会吞掉窗口外字符。
- 新增 `tests/code_info.rs` 六项回归：3024 组独立 ASCII/连续反斜线/容器模型；UTF-8 全切点与逐字符前缀/EOF；原始范围和借用指针、跨 tile 转义映射；零预算、共享预算及六字节分帧恢复；失败 opener、缺失 info、代码正文和未解码实体；990 层百万字正文、4096 历史代码块及千次追加工作计数。超长隐藏 opener 原有注释树更新按有界操作数验收，不宣称零树操作。
- CommonMark 0.31.2 Backslash escapes 例 24 改为消费核心 `code_info` 输出，章节现为 **9 支持、3 刻意差异、1 未支持**。评分投影不自行解码，官方输入/期望与生成器保持原样；448 个切点和 435 个逐字符前缀及投影敏感性/拒绝测试通过。原始 info 借用仍单独断言；其它评分投影未改。
- 验收通过：`cargo check --workspace --all-targets`；`cargo nextest run --workspace --no-fail-fast` 和 `--release` 各 **1138 项**；`cargo test --workspace --doc` **73 项**；`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、官方 Backslash escapes 夹具核验及 `git diff --check`。均在项目 `nix develop` 环境执行 Cargo 命令。
- 日志：`/tmp/mdtui3-goal-run/info-escapes-debug.log`、`/tmp/mdtui3-goal-run/info-escapes-validation.log`；范围核对 `/tmp/mdtui3-goal-run/info-escapes-scope-check.log`，独立差异 `/tmp/mdtui3-goal-run/info-escapes-unit.patch`。README/TODO 已同步；info 实体、直接/引用链接目标/title 解码、完整围栏/容器及宽泛标准覆盖继续待办。

## 2026-11-04 围栏 info 实体解码读取交付

- 在既有 `Document::code_info` 中交付分号终止的命名/数字实体解码：复用固定 2125 项命名表，数字规则与行内实体一致；反斜线优先，未知/不完整/超长候选按字面继续，解码结果不递归进入语法。原始 info、代码正文、分类窗口截断、块身份及追加路径保持；官方例 34 已通过核心接口支持。
- 游标增加固定探测状态和 8 字节缓冲，无堆分配。每个未转义 `&` 探测收一次 query，至多 33 个源码字节逐字节收费；固定表查找/数字校验包含在该 query 中，结果只缓存一次。成功输出另收一次 query/object 和输出 UTF-8 字节，失败探测的字面输出按普通字符收费。六字节、单 query/object 帧可持续恢复，未输出时 source_position 不变但探测预算可能已消费；输出保持原子性与完整源码映射。读取累计 O(info 字节数)，不访问正文、历史块或祖先。
- `CodeInfoChunk.text` 现在借用当前游标步骤：普通文本仍来自源码，实体来自固定缓冲，需要跨步骤保留时由调用方复制。既有原始借用接口不变。内部实体解码拆出零分配 `decode_into`，原行内调用仍取得原有 String，未增加持久解析状态、运行时依赖或 unsafe。
- 新增 `tests/info_entities.rs` 六项回归：408 组独立有效/失败/容器/行尾模型、全部 2125 命名项及 3588 组数字模型；UTF-8 全切点/逐前缀/EOF、跨 tile 映射、零预算与共享六字节恢复、双标量原子输出、窗口末尾未完成实体、980 层百万字正文/4096 历史块后千次追加计数。修改前五项回归有 3 失败、2 通过，见 `/tmp/mdtui3-goal-run/info-entities-repro.log`；完整表模型在实现后补充。
- 实体章节评分投影改为读取核心 `code_info`，保留原始 info 单独断言，未在投影中实现解码；当前 **13 支持、3 刻意差异、1 未支持**。官方输入、HTML 期望和生成器未改。专项首轮只有一条旧 info 文本期望失败，按新增语义更新；原转义测试仅更新实体预期、失败探测计费和游标固定大小上界。
- 验收通过：项目 `nix develop` 内 `cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1144 项**、`cargo test --workspace --doc` **73 项**、clippy `-D warnings`、fmt check；两个官方章节夹具核验和 `git diff --check` 通过。日志：`/tmp/mdtui3-goal-run/info-entities-debug.log`、`/tmp/mdtui3-goal-run/info-entities-validation.log`；范围核对与独立差异见同目录 `info-entities-scope-check.log`、`info-entities-unit.patch`。
- README/TODO 已同步。链接目标/title 解码、完整围栏/容器、失败链接回退及宽泛标准覆盖继续待办，未将这些宽泛条目标为完成。


## 2026-11-04 链接及图片 title 解码读取交付

- 从 Backslash escapes / Entity 剩余范围中交付 `Document::decoded_link_title(LinkId)`、`LinkTitle` 和 `LinkTitleChunk`。直接链接/图片和晚定义引用共用 ASCII 标点转义及命名/数字实体解码，反斜线优先、不递归、不 trim；未知/未完成候选保留字面。原始 title、目标 URL、身份及首定义优先保持，引用输出映射到获胜定义，空 title 与缺失 title 区分。
- 构造 O(1)，固定大小零分配游标复用 info 解码器；普通字符/转义按源码字节和 query/object 收费，实体探测按扫描字节及 query 收费，成功输出按输出字节及 query/object 收费。探测可分帧，输出原子；六字节、单 query/object 帧保证继续。没有新增持久解析状态、正文复制或历史使用点/祖先回扫，累计解析 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持。
- 新增 `tests/link_title.rs` 七项回归：1440 组独立实体/转义/容器/直接与引用/图片/行尾模型，753 组 ASCII/连续反斜线模型；UTF-8 全切点/逐字符前缀/前缀 EOF；晚定义/首定义优先/无 title 定义获胜、原文借用与目标 URL、空/缺失/失败 title、跨 tile 映射、零/不足/共享六字节预算、4096 字节和引用分类窗口边界；980 层百万字后千次追加及两万旧引用晚定义计数。
- 首次专项 6 项通过、1 项因测试错误假设 SymbolId 为 0 失败，已改为从已有 LinkTarget 取得符号身份。补充短行测试后首次全套 1150 通过/1 失败：测试在行末确认前要求读取 title，现按既有 LF/CRLF/EOF 分类时机断言。修正后 7 项专项通过；日志 `/tmp/mdtui3-goal-run/link-title-targeted.log`、`link-title-first-full.log`、`link-title-targeted-final.log`。未改解析行为或已有测试预期。
- README/TODO 同步实际范围；目标解码、URL 编码、多行 title、失败候选回退与宽泛标准覆盖继续待办。官方章节评分仍消费原始链接元数据，本项没有提升兼容率，官方夹具/生成器/评分投影保持。
- 最终验收通过：项目 `nix develop` 内 `cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1151 项**、`cargo test --workspace --doc` **74 项（71 + 3）**、clippy `-D warnings`、fmt check 和 `git diff --check`。完整日志 `/tmp/mdtui3-goal-run/link-title-validation.log`；范围核对 `link-title-scope-check.log` 确认仅 7 个预期文件变化，其余 280 个前序文件逐字节及 SHA-256 保持，独立差异 `link-title-unit.patch`。

## 2026-11-04 链接及图片目标解码读取交付

- 交付 `Document::decoded_link_destination(LinkId)`、`LinkDestination` 和 `LinkDestinationChunk`。已提交的直接/引用链接及图片目标解码 ASCII 标点转义与分号终止命名/数字实体；反斜线优先、不递归、不 trim。URI/email 自动链接保持字面，email 不添加隐含 `mailto:`；不执行 URL 编码/过滤。引用输出映射到全局首个获胜定义，空目标与未解析引用区分；原始 `link_destination`、`link_url`、title、身份及解析方言保持。
- 构造 O(1)，固定大小零分配游标复用 info 解码器，增加一个游标内字面模式标记；直接目标与标签区间相等即可识别自动链接，不增加解析持久状态。普通字符/转义、实体探测和输出分别按既有源码/输出字节及 query/object 计费，自动链接逐字符计费；共享六字节、单 query/object 帧可恢复，输出原子。总读取 O(目标字节数)，不访问历史引用、正文或祖先；追加 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持。
- 新增 `tests/link_destination.rs` 七项回归：1440 组独立实体/转义/容器/直接与引用/图片/行尾模型及 750 组 ASCII 模型；自动链接字面语义与借用指针、裸/尖括号目标、非递归、原始 URL、空目标/缺失引用、晚定义/首定义优先/身份；UTF-8 全切点与逐前缀/EOF、跨 tile 映射、零/不足/共享六字节预算、4096 字节和引用分类窗口边界；980 层百万字后千次追加与两万旧引用晚定义计数。新增及相关既有专项 26 项首次全部通过，没有修改旧测试预期。
- 验收通过：项目 `nix develop` 内 `cargo check --workspace --all-targets`；debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1158 项**；`cargo test --workspace --doc` **75 项（72 + 3）**；`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` 和 `git diff --check`。日志 `/tmp/mdtui3-goal-run/destination-validation.log`，分项日志同目录 `destination-{check,targeted,debug,release,final-checks}.log`。
- README/TODO 已同步；多行目标/title、完整失败回退、URL 编码与宽泛标准覆盖继续待办。官方夹具、生成器及评分投影未改，当前章节统计未接入目标/title 解码游标。范围核对 `/tmp/mdtui3-goal-run/destination-scope-check.log`，独立差异 `destination-unit.patch`；除 7 个本单元文件外，其余 282 个前序文件保持原 SHA-256。


## 2026-11-04 标准样例评分接入链接元数据解码交付

- 从 TODO「继续扩充标准语法样例覆盖」选择既有链接元数据预算接口的评分接入。CommonMark 0.31.2 Backslash escapes / Entity 两章改为消费 `decoded_link_destination`、`decoded_link_title`，官方 22–23、32–33 与原始 HTML 期望一致；章节分别为 **11 支持/1 刻意差异/1 未支持**及 **15 支持/1 刻意差异/1 未支持**。移除这四例的旧差异观察，其余差异保持，官方 Markdown/HTML、生成器和来源摘要未改。
- 共用 `tests/support/link_metadata.rs` 仅读取核心游标并进行输出层 HTML 转义及 UTF-8/反斜线百分号编码；已有百分号序列保持，不自行解码 Markdown、不递归、不 trim。ASCII 空白/控制字符及其它未评分 URL 编码仍拒绝；自动链接保留核心字面语义，空目标及空/缺失 title 可区分，原始借用接口断言保留。完整 URL 编码、过滤及实际链接交互未交付。
- 目标/title 在每帧六字节、一次 query/object 的预算间共享，耗尽后保留游标及实体探测状态，换预算恢复；连续源码覆盖、源码边界、有限步数及与无限预算的输出对照均有断言。两章全部 **972 个 UTF-8 切点、942 个逐字符前缀**继续通过；新增两项回归覆盖直接/引用、自动链接、非递归、空元数据、原有百分号、中文/emoji、双标量及长命名实体、title 空白保留，补充 Unicode 输入全切点通过。实现前两项新测试均失败，见 `/tmp/mdtui3-goal-run/metadata-score-repro.log`；实现后专项 **11 项**通过，见 `metadata-score-targeted.log`。
- 项目 devShell 验收通过：`cargo check --workspace --all-targets`，debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1160 项**，`cargo test --workspace --doc` **75 项（72 + 3）**，`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、两组固定来源夹具 `--check` 和 `git diff --check`。汇总 `/tmp/mdtui3-goal-run/metadata-score-validation.log`，check/debug/release 分项日志同目录 `metadata-score-{check,debug,release}.log`。
- README/TODO 更新当前统计和剩余范围，DONE 既有交付历史保持。核心解析、预算接口、追加 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 均保持；完整失败回退、多行目标/title、通用 URL 输出和宽泛标准覆盖仍待办。本单元仅修改两个评分测试、增加一个共用测试适配器及同步三份文档，范围核对 `metadata-score-scope-check.log`、独立差异 `metadata-score-unit.patch` 均在上述临时目录。


## 2026-11-04 新引用 tab 分隔后的代码围栏交付

- 从 TODO「围栏代码剩余规则」交付新 Quote 自身 tab 分隔的有界代码围栏子项：`>` 后直接 tab 先消费一列分隔，剩余列参与 opener 至多三列缩进，反引号/波浪线围栏确认成功才采用；正文按该列数去缩进，保留原始 info、容器/块身份和源码映射。失败、数学、其它叶块、短尾与窗口外候选保持既有方言。已有路径、closer 和预算读取接口复用原实现。
- 新增固定大小游标原子探测，无持久状态或正文复制；仅在自身 tab 后的有效缩进以围栏字符开头时分类当前有界后缀，工作计入容器/分类字节。不会对每层祖先扫描后续正文，不回访旧块或历史正文，累计追加 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持。
- 新增 `tests/quote_fence_tabs.rs` 五项回归：576 组物理列宽/定界/行尾模型，正文去缩进与原始 info；失败/其它叶块/共享窗口/作用域边界；UTF-8 全切点和逐字符前缀；连续借用源码映射/逐字符切片/tile、零预算停步和六字节恢复；989 层容器百万字后千次追加不增加分类、容器、行内或树访问。实现前专项 3 失败/2 通过，见 `/tmp/mdtui3-goal-run/quote-fence-repro.log`；初版深度测试的 tab 恰好整字节边界，后改为 989 层以覆盖实际部分 tab 路径。实现后专项 5 项全部通过，见 `quote-fence-targeted.log`。
- 项目 devShell 验收通过：`cargo check --all-targets`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1165 项**、`cargo test --workspace --doc` **75 项（72 + 3）**、clippy `-D warnings`、fmt check 和 `git diff --check`。日志 `/tmp/mdtui3-goal-run/quote-fence-debug.log`、`quote-fence-validation.log`；没有修改旧测试预期。范围核对 `quote-fence-scope-check.log` 确认仅 5 个文件改动，其余 286 个前序文件 SHA-256 保持，包括官方夹具/生成器/评分投影；独立差异 `quote-fence-unit.patch` 位于同目录。
- README/TODO 已同步。完整容器、新引用其它叶块/分隔规则及超过分类窗口的围栏识别继续待办，未将宽泛条目标为完成。

## 2026-11-04 新引用 tab 分隔后的 ATX 标题交付

- 从完整容器/叶块 tab 缩进待办中交付一个子项：新 Quote 的 `>` 后直接 tab 消费一列分隔，剩余列按物理四列制表位计入至多三列 ATX 缩进。`>\t # title` 确认为引用内标题；仅窗口内确认成功的 1–6 级 ATX 采用，失败候选原子回退。短行等待 LF/CRLF/EOF，截断标记与必要分隔不在窗口外追认；既有尾缀裁剪、行内语义、稳定身份、作用域和源码映射保持。
- 将已有 `quote_fence` 原子探测扩展为围栏/ATX 共用的 `quote_leaf`，只增加有效缩进后的 `#` 候选和标题成功分支；固定大小游标与工作计数复用，无新增持久状态、旧正文/历史块/祖先回扫。长标题正文和尾缀沿用单遍消费，累计追加 O(n log n)、空间 O(n)、零运行时依赖与禁止 unsafe 保持。
- 新增 `tests/quote_atx_tabs.rs` 六项回归：1344 组列宽/级别/行尾独立模型、失败及窗口边界、UTF-8 全切点/逐字符前缀、连续源码映射/逐字符切片/tile、零预算停步与六字节恢复、搜索/slug/折叠；989 层百万字标题后千次追加不增加分类/容器工作，失败长井号串与 4096 组新引用标题保持有界计数。修改前专项 5 失败/1 通过，见 `/tmp/mdtui3-goal-run/quote-atx-repro.log`；实现后 6 项通过，见 `quote-atx-targeted.log`。
- 首轮全套 1169 通过/2 失败，原因是旧 ATX 列宽模型及叶块测试仍把 `>\t # ...` 当作超额缩进；更新为一列分隔后的三列缩进，并将叶块旧拒绝断言改为明确成功断言。围栏测试的一条旧标题回退预期同样更新。相关专项 31 项全部通过，见 `quote-atx-targeted-final.log`；首轮全套见 `quote-atx-debug.log`。
- README/TODO 已同步。完整容器、新引用其它叶块及超过分类窗口的识别继续待办，未将宽泛条目标为完成。
- 项目 devShell 最终验收通过：`cargo check --all-targets`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1171 项**、`cargo test --workspace --doc` **75 项（72 + 3）**、clippy `-D warnings`、fmt check 和 `git diff --check`，见 `/tmp/mdtui3-goal-run/quote-atx-validation.log`。范围核对 `quote-atx-scope-check.log` 确认仅 8 个预期文件变化，其余 284 个前序文件 SHA-256 保持，包括官方夹具/生成器/评分投影；独立差异 `quote-atx-unit.patch` 位于同目录。


## 2026-11-04 新引用 tab 分隔后的分隔线交付

- 从完整容器/叶块 tab 缩进待办交付新 Quote 的有界分隔线子项：`>` 后直接 tab 消费一列分隔，余量按物理四列制表位计入至多三列缩进；至少三个同种 `*` / `-` / `_`，仅完整行确认成功采用。分隔线优先于新列表，已有作用域 Setext 优先级保持；失败、普通正文、数学及围栏规则原子回退。隐藏作为容器分隔消费的 tab，实际后缀空白和标记按原字节显示；身份及源码映射保持。
- `quote_leaf` 复用已有单次反向分隔线后缀摘要和缓存列宽，只增加常数判断，无新增后缀分类扫描、持久状态、旧正文/历史块/祖先回扫。累计追加 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 保持；共享预算接口复用原实现。
- 新增 `tests/quote_rule_tabs.rs` 五项回归：1728 组列宽/标记数/行尾独立模型，优先级、部分 tab 祖先、失败和分类窗口；UTF-8 全切点/逐前缀/EOF；连续源码映射/逐字符切片/tile、零预算停步和六字节恢复；989 层百万字后千次追加不增加分类/容器工作，以及 4096 组新引用分隔线的有界计数。修改前 4 失败/1 通过，见 `/tmp/mdtui3-goal-run/quote-rule-repro.log`。初版窗口测试把无 LF 的恰好 1024 字节 EOF 误认为可追认，已按既有窗口规则修正；相关专项 21 项全部通过，见 `quote-rule-targeted-final.log`。
- 首轮 debug 全套 1175 通过/1 失败；唯一旧列宽模型仍拒绝 `>\t ***`，已按新引用分隔消费一列更新预期，未放宽其它断言。首轮日志 `quote-rule-debug.log`。最终项目 devShell 内 `cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1176 项**、`cargo test --workspace --doc` **75 项（72 + 3）**、clippy `-D warnings`、fmt check 与 `git diff --check` 全部通过，日志 `/tmp/mdtui3-goal-run/quote-rule-validation.log`。
- README/TODO 已同步，完整容器、新引用其它叶块、窗口外识别及宽泛标准覆盖继续待办，未将宽泛条目标为完成。范围核对 `quote-rule-scope-check.log` 确认仅 6 个预期文件变化，其余 287 个前序文件 SHA-256 保持，包括官方夹具/生成器/评分投影；独立差异 `quote-rule-unit.patch` 位于上述临时目录。


## 2026-11-04 新引用 tab 分隔后的独立数学块交付

- 从容器/叶块及数学后续待办中交付新 Quote 自身 tab 分隔的数学 opener：`>` 后直接 tab 消费一列可选分隔，余量按物理四列制表位计入至多三列缩进；只有完整有界的独占行 `$$`（可带尾部 space/tab）确认成功时采用。失败候选原子回退，短尾等待 LF/CRLF/EOF，窗口外候选不追认。已有路径、closer、作用域退出及未闭合 EOF 复用既有规则；数学正文保持字面，不按 opener 缩进去缩进。
- 实现仅将 `$` 加入 `quote_leaf` 候选集合，复用固定大小游标和既有数学分类/消费路径；候选的有界后缀扫描计入分类工作，不对每层祖先重扫正文，无新增持久状态、正文复制或旧块回访。累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享 Budget 保持。
- 新增 `tests/quote_math_tabs.rs` 六项回归：384 组独立列宽/行尾/后缀模型，完整及部分 tab 祖先路径、失败/分类窗口，UTF-8 全切点/逐前缀/EOF，稳定身份/折叠/读取失效，原始与正文源码映射、连续显示覆盖/tile、零预算停步与六字节恢复；989 层百万字正文及超长失败 closer 后千次追加无分类/容器/行内回扫，4096 组数学块保持计数边界。修改前专项 5 失败/1 通过，见 `/tmp/mdtui3-goal-run/quote-math-repro.log`；实现后专项 6 项通过，见 `quote-math-targeted.log`。
- 首轮 debug 全套 1179 通过/3 失败；三条旧新引用测试仍预期 `>\t $$` 显示普通文本，已更新为数学定界隐藏，其余断言保持。首轮日志 `/tmp/mdtui3-goal-run/quote-math-debug.log`。README/TODO 已同步，完整容器、其它新引用叶块、窗口外识别与数学实际排版继续待办，未将宽泛条目标为完成。
- 最终项目 devShell 验收通过：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1182 项**、`cargo test --workspace --doc` **75 项（72 + 3）**、clippy `-D warnings`、fmt check 和 `git diff --check`，见 `/tmp/mdtui3-goal-run/quote-math-validation.log`。范围核对 `quote-math-scope-check.log` 确认仅 7 个已有文件及 1 个新增测试变化，其余 286 个前序文件 SHA-256 保持，包括官方夹具/生成器/评分投影；独立差异 `quote-math-unit.patch` 位于上述临时目录。


## 2026-11-04 新引用 tab 分隔后的引用链接定义交付

- 从容器/叶块 tab 缩进待办交付新 Quote 自身 tab 分隔的引用链接定义：`>` 后直接 tab 消费一列可选分隔，余量按物理四列制表位计入至多三列缩进，仅完整有界且标签有效的定义采用。失败候选、纯空白/过长标签和脚注原子回退；短尾等待 LF/CRLF/EOF，分类窗口外不追认。保持原始目标/title、确切作用域、首定义优先、稳定链接身份和引用范围通知。
- `quote_leaf` 复用固定大小游标与既有定义分类/消费路径，增加 `[` 候选和无分配标签有效性检查；有界后缀分类及标签探测均计费。无新增持久状态、正文复制或历史引用/正文/祖先回扫，累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算保持。
- 新增 `tests/quote_definition_tabs.rs` 五项回归：576 组独立列宽/目标/行尾模型，失败/分类窗口及完整/部分 tab 祖先路径，UTF-8 全切点/逐前缀/EOF，首定义优先与引用通知，连续源码映射/逐字符切片/tile、零预算停步与六字节恢复。989 层百万字正文后新开第 990 层引用定义、超长失败定义后千次追加、两万旧引用以及 4096 个后续新引用定义验证有界计数。修改前 4 失败/1 通过见 `/tmp/mdtui3-goal-run/quote-definition-repro.log`；初次实现后 5 项通过见 `quote-definition-targeted.log`，补充映射/新引用分块和容器计数检查后全套通过。
- 项目 devShell 验收通过：`cargo check --workspace --all-targets`，debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1187 项**，`cargo test --workspace --doc` **75 项（72 + 3）**，`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` 及 `git diff --check`。日志 `/tmp/mdtui3-goal-run/quote-definition-debug.log`、`quote-definition-validation.log`；旧测试无需修改。
- README/TODO 已同步，仅追加准确子项，完整容器、新引用其它叶块、多行定义及窗口外识别继续待办。范围核对 `quote-definition-scope-check.log` 确认仅 4 个已有文件及 1 个新增测试变化，其余 290 个基线文件 SHA-256 保持，包括官方夹具/生成器/评分投影及旧测试；独立差异 `quote-definition-unit.patch` 位于上述临时目录。


## 2026-11-04 新引用 tab 分隔后的脚注定义起始行交付

- 从容器/叶块 tab 缩进待办交付新 Quote 自身 tab 分隔的脚注起始行：`>` 后直接 tab 消费一列可选分隔，余量按物理四列制表位计入至多三列缩进；合法 `[^label]:` 才采用部分 tab 前缀，失败原子回退。标签和冒号必须在分类窗口内，正文可跨窗口单遍增长；保留既有标签规则、全局首定义优先、首次引用编号、稳定身份、源码映射和四列续行。
- 核心仅将分类器已验证的 `FootnoteDefinition` 加入 `quote_leaf` 成功分支，复用固定大小游标和已有脚注消费路径；有界后缀及标签验证计入分类工作。无新增持久状态、正文复制或旧正文/引用/祖先回扫，累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算保持。
- 新增 `tests/quote_footnote_tabs.rs` 五项回归：576 组列宽/正文/行尾独立模型，完整及部分 tab 祖先、失败/标签上限/分类窗口、四列续行，UTF-8 全切点/逐前缀/EOF，稳定身份/编号与首定义优先，原始与正文映射、零预算停步及六字节恢复；989 层百万字脚注后千次追加、两万旧引用和 4096 个后续新引用定义验证计数。修改前专项 4 失败/1 通过，见 `/tmp/mdtui3-goal-run/quote-footnote-repro.log`。首次实现后专项 4 通过/1 失败：历史定义压力输入仍在同一脚注续行内，现用空行退出并创建独立 Quote，再验证 4096 组新引用定义。
- 首轮 debug 全套 1191 通过/1 失败；旧引用定义测试将合法脚注起始行列为失败输入，现改用缺少冒号的脚注候选继续验证原子回退，其余断言保持。日志 `/tmp/mdtui3-goal-run/quote-footnote-debug.log`。
- 最终项目 devShell 验收通过：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --no-fail-fast` 各 **1192 项**、`cargo test --workspace --doc` **75 项（72 + 3）**、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` 和 `git diff --check`，见 `/tmp/mdtui3-goal-run/quote-footnote-validation.log`。
- README/TODO 已同步，完整容器、新引用普通正文、窗口外识别及脚注正文解析/实际排版继续待办。范围核对 `quote-footnote-scope-check.log` 确认仅 5 个已有文件及 1 个新增测试变化，其余 290 个基线文件 SHA-256 保持，包括官方夹具/生成器/评分投影；独立差异 `quote-footnote-unit.patch` 位于上述临时目录。


## 2026-11-04 新引用 tab 分隔后的嵌套引用交付

- 从容器后续语义的新容器首行/tab 待办交付新 Quote 自身 tab 分隔后的嵌套引用：`>` 后直接 tab 消费一列可选分隔，余量与后续空白按物理四列制表位合计至多三列，下一个显式 `>` 必须在分类窗口内。修复 `>\t > text` 少识别一层引用的问题；保留父子身份、既有叶块/续行、源码与显示映射及开闭通知。超过三列、无嵌套标记、围栏/数学正文和窗口外识别保持既有方言。
- `quote_leaf` 复用固定大小游标及缓存空白终点，只检查下一引用标记，不在每一层扫描剩余后缀；新增读取计入容器工作。无新增持久状态、正文复制或旧正文/祖先回扫；累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算保持。
- 新增 `tests/quote_nested_tabs.rs` 五项回归：1152 组独立列宽/叶块/行尾模型，完整及部分 tab 祖先、失败/窗口、UTF-8 全切点/逐前缀/EOF；稳定父子身份和开闭通知、原始源码读取、隐藏前缀映射、零预算停步与六字节恢复；连续 330 段 tab 引用链及 990 层百万字正文后千次追加检查计数。修改前 4 失败/1 通过见 `/tmp/mdtui3-goal-run/quote-nested-repro.log`；最初模型中的任务标记源码偏移不宜跨不同长度输入直接比较，已改用普通列表模型，任务输入仍纳入分块快照检查。
- README/TODO 已同步；完整容器、新引用普通正文、其它首行规则及窗口外识别继续待办，未勾选宽泛 TODO。
- 补充分类窗口外首字节的反例后，首轮 release 产物在该断言失败（`quote-nested-release-first.log`）；显式要求内层标记位置小于窗口末端，避免提前采用外层 tab 分隔。最终 debug 全套 **1197 项**通过，见 `/tmp/mdtui3-goal-run/quote-nested-debug-final.log`。
- 最终项目 devShell 验收通过：`cargo check --workspace --all-targets`、release `cargo nextest run --workspace --release --no-fail-fast` **1197 项**、`cargo test --workspace --doc` **75 项（72 + 3）**、clippy `-D warnings`、fmt check 和 `git diff --check`；日志 `/tmp/mdtui3-goal-run/quote-nested-validation.log`。范围核对 `quote-nested-scope-check.log` 确认仅 4 个已有文件及 1 个新增测试变化，其余 292 个基线文件 SHA-256 保持，包括官方夹具/生成器/评分投影及旧测试；独立差异 `quote-nested-unit.patch` 位于同目录。

## 2026-11-04 新引用 tab 分隔后的嵌套列表交付

- 从容器首行/tab 后续待办交付新 Quote 自身 tab 分隔后的嵌套列表：`>` 后直接 tab 消费一列可选分隔，余量加后续空白至多三列时，允许开启有效无序/有序列表；列表标记必须在分类窗口内确认。修复 `>\t - item` 漏识别列表以及 `>\t- item` 列表缩进多算一列的问题。失败原子回退，分隔线优先级、首行代码、任务/编号、父子身份、显式续行/兄弟条目及共享预算保持。
- 核心只扩展 `quote_leaf` 成功分支，复用固定大小游标和既有 `marker` 探测；扫描至多九位数字、定界及随后的有界空白，成功后下一循环消费同一标记，每段只增加常数次访问并计入 `container_bytes`。无新增持久状态、正文复制或旧正文/历史容器/祖先回扫，保持累计追加 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe。
- 新增 `tests/quote_list_tabs.rs` 五项回归：4608 组独立列宽/标记/正文/行尾模型；失败/分隔线优先级/分类窗口，完整及部分 tab 祖先；17 组输入的 365 个合法 UTF-8 切点及 348 个字符前缀/EOF；稳定身份、续行与兄弟通知、原始源码及隐藏前缀映射、零预算停步和六字节恢复；128 组连续 Quote/List/Item、991 层百万字正文后千次追加检查计数。
- 修改前专项 4 失败/1 通过，见 `/tmp/mdtui3-goal-run/quote-list-repro.log`。初次专项中，跨不同长度输入的任务标记源码偏移改由分块快照验证，列宽模型使用普通文本；连续嵌套与兄弟输入的空格按物理列宽校正。最终专项 5 项通过，见 `quote-list-targeted-final.log`；debug 全套 **1202 项**通过，见 `quote-list-debug.log`。
- README/TODO 已同步，仅勾选准确子项；完整容器、新引用普通正文、其它首行规则和窗口外识别继续待办。
- 最终项目 devShell 验收通过：`cargo check --workspace --all-targets`、release `cargo nextest run --workspace --release --no-fail-fast` **1202 项**、`cargo test --workspace --doc` **75 项（72 + 3）**、clippy `-D warnings`、fmt check 和 `git diff --check`；日志 `/tmp/mdtui3-goal-run/quote-list-validation.log`。范围核对 `quote-list-scope-check.log` 确认仅 4 个已有文件及 1 个新增测试变化，其余 293 个基线文件 SHA-256 保持，包括官方夹具/生成器/评分投影及旧测试；独立差异 `quote-list-unit.patch` 位于同目录。


## 2026-11-04 新引用 tab 分隔后的普通正文交付

- 完成 TODO 容器剩余语义中的新引用普通正文子项：直接 tab 消费一列引用分隔，其余至多三列按物理四列制表位映射回原单字节；行内语义、源码读取、父子身份与显式续行保持。失败标记若被分类为普通文本亦适用；完整容器及其它首行规则继续待办。
- 保留容器正文前导空白的既有方言，不在本轮承诺完整 CommonMark 缩进。空白引用行、无有效标签的定义、窗口外起始与四列代码优先级保持；分类窗口内起始的长正文继续即时单遍追加。
- 表头候选增加固定大小的虚拟余量，沿用单遍摘要、源码区间和显示映射；早/晚管道确认时裁剪单元格前导空白，失败或 EOF 只消费尚未解析部分，余量不丢失、不重放。没有新增依赖、unsafe、正文复制或旧正文/祖先回扫，累计解析 O(n log n)、空间 O(n) 与共享预算契约保持。
- 新增 `quote_prose_tabs` 六项回归：1536 组输入、短输入全切点/逐前缀/EOF、48 组长表头的全切点/流式检查点、源码/逐字符/tile 映射、零预算停步和六字节恢复；989 层百万字及后续千次单字追加检查分类/容器计数不再增长。修改前专项 3 失败/2 通过；首轮全套五处旧 tab 显示预期按本轮新语义更新，官方夹具/生成器/评分投影保持。
- 验收：`nix develop -c bash /tmp/mdtui3-goal-run/quote-prose-validate.sh` 通过；debug/release 各 1208 项、doc 75 项及 check/clippy/fmt/diff 通过。日志 `/tmp/mdtui3-goal-run/quote-prose-validation.log`，完整测试分别见 `quote-prose-debug.log` / `quote-prose-release.log`；专项及范围核对见同目录 `quote-prose-targeted.log` / `quote-prose-scope-check.log`，独立差异 `quote-prose-unit.patch`。

## 2026-11-04 完整已有部分 tab 路径的普通新段落交付

- 从容器后续语义的新段落待办交付完整已有部分 tab 路径上的普通新段落：窗口内非空正文、余量与后续空白合计至多三列时，允许空行/标题/围栏/定义后继续使用原容器。`- seed\n\n\t*text*` 保留 Item 并显示两个空格和斜体文字；源码仍是原 tab，列表 Loose 由原观察路径确认。空白/失败路径、四列代码、分类窗口和待确认表头的前缀规则保持，部分路径普通正文仍不作为表头候选。
- 核心只放宽原子路径探测的普通文本条件；区分无前序段落的新正文与待确认表头，复用原列宽、身份、行内消费及显示映射。无新增状态、依赖、unsafe、扫描或旧正文/历史容器/祖先回扫，累计追加 O(n log n)、空间 O(n) 与共享预算保持。
- `tests/new_paragraph_tabs.rs` 六项回归覆盖 1536 组独立列宽模型，UTF-8 全切点/逐前缀/EOF、表头限制/Setext/代码/失败路径/窗口边界、身份/列表松紧、原始/逐字符/tile 映射、零预算停步及六字节恢复；982 层百万字后千次追加无分类/容器回扫，6 组长管道正文全切点和预算消费验证单遍增长。
- 修改前专项 5 项失败，见 `/tmp/mdtui3-goal-run/new-paragraph-repro.log`；其中两处新增测试的引用空行/分隔列预期随后校正。首轮全套 1210 通过/4 失败：两处旧预期按新段落显示/身份/松紧语义更新，另两处暴露待确认表头边界扩大，收窄新段落条件后相关 36 项通过，见 `new-paragraph-targeted-final.log`。README/TODO 已同步；部分路径表头、其它首行/空白/不完整路径和完整容器规则继续待办，未勾选宽泛 TODO。
- 最终验收：`nix develop -c bash /tmp/mdtui3-goal-run/new-paragraph-validate.sh` 通过；debug/release 各 **1214 项**、doc **75 项（72 + 3）**及 check/clippy/fmt/diff 通过。日志 `/tmp/mdtui3-goal-run/new-paragraph-validation.log`；全套见 `new-paragraph-debug-final.log` / `new-paragraph-release.log`，范围核对 `new-paragraph-scope-check.log` 确认仅 6 个已有文件及 1 个新增测试变化，其余 293 个基线文件 SHA-256 保持，包括官方夹具/生成器/评分投影；独立差异 `new-paragraph-unit.patch`（均在同目录）。

## 2026-11-04 CommonMark Block quotes 标准样例评分交付

从标准样例覆盖待办拆出完整 Block quotes 章节，固定 CommonMark 0.31.2 的 228–252 共 25 例；新增离线夹具生成器、固定 SHA-256/章节成员校验及 `--check`。官方 Markdown/HTML 不经过核心解析器，逐字保留。只新增测试评分基础设施，核心解析状态、运行时依赖及追加/预算算法保持。

独立评分为 18 支持、2 刻意差异、5 未支持。238、252 保留容器正文前导空白，属于现有方言；239–241、244、249 暴露空引用行发布为正文叶块的缺口，已单独写入 TODO。观察值与官方期望分离，差异消失或变化时测试要求复核分类。投影只使用公开结构/显示语义，保留空正文叶块及原始空白，不重解析 Markdown、不接收编号；越出投影范围的对象/样式显式拒绝。

六项新增回归覆盖完整章节报告、409 个合法切点、384 个逐字符前缀、EOF 收尾、源码及隐藏前缀连续映射、逐字符切片、零预算与六字节共享预算恢复；独立验证标题级别、容器层级/关闭、显式/lazy 换行、代码字面空白和投影拒绝边界。短章评分不代表完整容器兼容或深层/长输入复杂度证明；既有性能与预算回归继续作为验收门槛。

验收：debug/release 各 1220 项、doc 75 项，以及 `cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、fmt、夹具 `--check` 与 `git diff --check` 全部通过；修改过的上游字节被 SHA-256 校验拒绝。总日志 `/tmp/mdtui3-goal-run/block-quotes-validation.log`，专项报告 `block-quotes-targeted.log`，来源校验 `block-quotes-provenance.log`。范围核对 `block-quotes-scope-check.log` 确认只追加 README/TODO/DONE，新增本章测试/夹具/生成器，其余 297 个前序文件 SHA-256 保持；独立差异 `block-quotes-unit.patch`（均在同目录）。

## 2026-11-04 有界空白引用叶块交付

- 完成 TODO 中 Block quotes 239–241、244、249 暴露的空白叶块缺口：完整有界空引用/空白引用行发布为 `Blank`，保留物理叶块、容器身份、原始源码与剩余空白映射。已有段落终止、显式续行、缺失引用退出和列表尾部空白传播路径保持；marker-only 列表、部分/缺失路径、代码/数学正文及窗口外长空白不扩展。
- 核心只扩展既有 `Line::Container` 空白分支的叶块类型选择，无新增状态、扫描、依赖、unsafe 或旧正文/引用/祖先回扫；累计解析 O(n log n)、空间 O(n) 和共享预算契约保持。显示中的剩余空白保留原字节，不能将 `Blank` 理解为删除源码或 trim 文本。
- CommonMark 0.31.2 Block quotes 评分更新为 **23 支持、2 刻意差异、0 未支持**；238、252 继续保留前导空白方言。官方夹具、生成器与投影函数逐字保持，只更新独立分类和相应断言。完整容器和宽泛标准覆盖仍待办。
- 新增 `tests/quote_blanks.rs` 六项回归：1152 组空白/嵌套/行尾组合，UTF-8 全切点/逐前缀/EOF，段落、行内、lazy、Setext、列表松紧、代码/数学/表头边界，原始/切片映射与零预算停步、六字节恢复；980 层中百万字正文后 100 个空白行按新增前缀计费，后续百万字长行及千次追加不回扫。官方全章仍覆盖 409 切点与 384 前缀。修改前专项 5 失败/1 通过，修改后相关 12 项全部通过，未调整其它旧测试。
- 验收通过：debug/release 各 **1226 项**、doc **75 项（72 + 3）**，check/clippy/fmt、官方夹具 `--check` 和 diff 检查通过。日志 `/tmp/mdtui3-goal-run/quote-blank-validation.log`；debug 全套见 `quote-blank-debug.log`，专项及修复前证据见 `quote-blank-targeted.log` / `quote-blank-repro.log`。范围核对 `quote-blank-scope-check.log` 确认仅 5 个已有文件及 1 个新增测试变化，其余 298 个基线文件 SHA-256 保持；独立差异 `quote-blank-unit.patch`（均在同目录）。


## 2026-11-04 有界空列表条目叶块交付

- 完成容器剩余语义中的有界无任务空列表叶块分类：现有分类器已确认为空白的 ListItem 发布为 `Blank`，列表标记、有序编号和父子身份仍从容器 arena 读取；物理叶块、源码和前缀隐藏保持。空首行不直接使列表 Loose，后续正文和兄弟条目的间距继续沿用原观察路径。
- 任务条目即使没有正文仍发布带 `TaskMarker` 的 `ListItem`；非空正文、代码/数学、部分或缺失路径匹配、窗口外空白和 EOF 不追认规则保持。仅扩展类型选择，不增加扫描、持久状态、依赖或 unsafe，不回访旧正文或祖先，累计解析 O(n log n)、空间 O(n) 与共享预算契约不变。完整列表/容器和宽泛标准覆盖仍待办。
- 新增 `tests/empty_items.rs` 六项回归：1728 组标记/编号/缩进/作用域/行尾模型，UTF-8 全切点/逐前缀/EOF，段落/行内/Setext/表头/任务/代码/窗口边界，原始与逐字符映射、零预算停步和六字节恢复；980 层引用中的百万字正文后连续 100 个空条目按新增前缀计费，后续百万字长行及千次单字追加不重分类或回扫。修改前 5 失败/1 通过，修复后 6 项通过；仅同步一条既有引用空白测试的空列表预期。官方夹具、生成器和评分投影保持。
- 验收：debug/release 各 **1232 项**、doc **75 项（72 + 3）**全部通过；check/clippy/fmt、Block quotes 官方夹具复核与 diff 检查通过。命令 `nix develop -c cargo nextest run --workspace --no-fail-fast` 和 `nix develop -c bash /tmp/mdtui3-goal-run/empty-item-validate.sh`；日志 `/tmp/mdtui3-goal-run/empty-item-validation.log`、`empty-item-debug.log`、`empty-item-release.log`，修复前与专项见 `empty-item-repro.log` / `empty-item-targeted.log`。范围核对 `empty-item-scope-check.log` 确认仅 5 个已有文件及 1 个新测试变化，其余 299 个基线文件 SHA-256 保持；README/TODO/DONE 仅追加，独立差异 `empty-item-unit.patch`（均在同目录）。


## 2026-11-04 CommonMark Lists 标准样例评分交付

- 从标准样例覆盖待办拆出完整 CommonMark 0.31.2 Lists 章节（301–326，共 26 例）；新增离线生成器、固定 SHA-256/章节成员检查和 `--check`，官方 Markdown/HTML 逐字保留。评分为 **17 支持、3 刻意差异、6 未支持**，未把完整列表/容器或宽泛标准覆盖勾选完成。
- 304 沿用任意有序编号可中断段落的方言；308/309 的 HTML 注释保持普通文字。310–313 的变缩进列表分组、318 的容器围栏无前缀空行及 320 的引用末尾空白传播缺口已写入 TODO，312 还保留 lazy 前导空白差异。本单元交付评分基础设施，不改这些解析行为。
- 投影从公开容器 ID/父子关系、首 Item 编号、确切父 List 的 Tight/Loose、叶块与显示片段产生测试 HTML；不接收样例编号、不重解析、不 trim、不合并独立 List、不补代码内容。独立观察与官方期望分开，差异变化要求复核。空条目与隐藏定义验证核心叶块分类；未评分结构和样式显式拒绝。只供短测试，未宣称投影本身满足窗口级复杂度或祖先预算。
- 新增六项回归：完整章节报告、730 切点、704 逐字符前缀及各前缀 EOF，连续原始/显示/逐字符切片映射、零预算停步与六字节共享预算恢复，独立编号/标记分组/嵌套松紧/空条目/隐藏定义/代码空白和拒绝边界。核心解析及既有测试未修改，无新增运行时依赖、unsafe 或解析状态；累计追加 O(n log n)、空间 O(n) 与预算契约保持。
- 最终项目 devShell 验收通过：debug/release 各 **1238 项**、doc **75 项（72 + 3）**，`cargo check --workspace --all-targets`、clippy `-D warnings`、fmt、夹具 `--check` 与 `git diff --check` 全部通过。命令 `nix develop -c bash /tmp/mdtui3-goal-run/lists-validate.sh`；总日志 `/tmp/mdtui3-goal-run/lists-validation.log`，全套见 `lists-debug.log` / `lists-release.log`，专项评分见 `lists-targeted.log`，来源校验见 `lists-provenance.log`。范围核对 `lists-scope-check.log` 确认只追加三份文档并新增本章测试/夹具/生成器，其余 302 个前序文件 SHA-256 保持；独立差异 `lists-unit.patch`（均在同目录）。


## 2026-11-04 引用末尾空白的列表间距边界交付

- 完成 TODO 中 Lists 320 的容器缺口：Quote 关闭时截断引用内部末尾空白，包括引用内嵌套列表的尾部；引用外的兄弟条目或直接块不再因此使父列表 Loose。引用外部的真实空行、未跨 Quote 的嵌套列表尾部仍按既有规则确认 Loose，EOF 关闭本身不确认。
- 核心仅在已有 Quote 出栈分支清除 `trailing_blank`，无新增扫描、持久状态、运行时依赖或 unsafe。每个容器仍只关闭一次，无旧正文、历史条目或保留祖先回扫，累计解析 O(n log n)、空间 O(n) 与共享预算契约保持。源码、叶块/容器身份、编号、显示映射、折叠意愿及实际 Loose 通知保持。
- 官方 Lists 320 对照原始 HTML 通过，全章为 **18 支持、3 刻意差异、5 未支持**；夹具、生成器和评分投影不改，只更新独立分类及计数。310–313 不同缩进、318 围栏无前缀空行、完整容器及宽泛标准覆盖继续待办。README/TODO 更新当前规则与剩余范围，历史 DONE 记录保持。
- 新增 `tests/quote_tail.rs` 六项回归：640 组标记/引用层数/空白/行尾/后继模型，跨引用的嵌套列表和引用外空行反例，UTF-8 全切点/逐前缀/EOF，原始及显示映射、零预算停步与六字节恢复，身份/关闭/Loose 通知与折叠意愿，980 层引用、百万字正文及千次长行追加工作量。修改前 5 失败/1 通过，修复后专项 6 项通过；官方全章继续验证 730 切点、704 前缀及预算。
- 首轮全套 1241 通过/3 失败，均为旧的跨 Quote 空白传播预期；更新脚注及列表尾部测试，并同步既有引用空白测试的一条预期。相关 44 项通过。示例 `list_tail_blanks` 补上引用外的真实空白行，以保留父列表 Loose 的演示。修复前日志 `quote-tail-repro.log`，专项 `quote-tail-targeted-final.log`，首轮全套 `quote-tail-debug-initial.log`（均在 `/tmp/mdtui3-goal-run/`）。
- 最终验收通过：debug/release 各 **1244 项**、doc **75 项（72 + 3）**，check/clippy `-D warnings`/fmt、官方夹具 `--check`、diff 检查及 release 示例运行全部通过。命令 `nix develop -c bash /tmp/mdtui3-goal-run/quote-tail-validate.sh`；总日志 `/tmp/mdtui3-goal-run/quote-tail-validation.log`。范围核对 `quote-tail-scope-check.log` 确认仅 10 个预期文件变化、其余 299 个前序文件 SHA-256 保持，官方夹具/生成器/评分投影和历史 DONE 不变；独立差异 `quote-tail-unit.patch`（均在同目录）。


## 2026-11-04 列表内代码围栏跨省略缩进空行交付

本轮交付 TODO 中 Lists 318 暴露的围栏空行缺口。已开启的反引号/波浪线围栏，在完整分类窗口内接受省略全部或部分列表缩进的 space/tab 空行，所有已有引用标记仍须匹配。不足缩进全部隐藏，换行作为同一代码块的字面正文保留，不登记列表分隔；足够缩进和多余空白沿用原有部分 tab 映射及 opener 去缩进。保持 BlockId、容器身份、源码包络、CODE 样式、折叠与通知；缺失引用/非空路径、兄弟、数学、缩进代码及分类窗口方言保持。

实现只给既有容器选项传入代码围栏标志，在空白保留分支复用引用深度索引和缓存空白终点提前返回；不新增持久状态、依赖、unsafe 或旧正文/历史容器/保留祖先回扫。匹配成本按实际前缀和关闭节点计费，无前缀深列表空行不逐层遍历；累计 O(n log n)、空间 O(n) 与共享预算契约保持。

`tests/fence_blanks.rs` 六项新回归覆盖 1440 组独立物理列宽模型、同一代码块的字面正文、缺失引用/非空/控制空白/数学/外部间距反例、UTF-8 全切点/逐前缀/EOF、源码映射/零预算/六字节恢复、稳定身份与折叠通知。480 层列表（960 个容器）的千次空行保持 arena 容量且无祖先线性扫描，百万字正文与千次单字追加无容器/行内回扫。修复前 4 项失败、2 项通过；初始测试编译中的视图类型名按实际 FoldView API 修正。首轮全套仅两条旧关闭/Loose 预期失败，按新语义更新后相关 35 项与最终全套通过。

官方 CommonMark 0.31.2 Lists 318 对照原始 HTML 通过，全章为 **19 支持、3 刻意差异、4 未支持**。官方 Markdown/HTML 夹具、生成器与评分投影不改，仅更新独立观察分类和计数；原全章 730 切点/704 前缀及预算验收继续通过。310–313 不同标记缩进、完整容器和宽泛标准覆盖仍未完成；README/TODO 已同步，不勾选宽泛待办。

验收：`nix develop -c cargo check --all-targets`、`cargo nextest run --no-fail-fast`（1250 项）、release 同命令（1250 项）、`cargo test --doc`（75 项）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、Lists 离线夹具校验及 `git diff --check` 全部通过。命令在项目 devShell 内执行，汇总日志 `/tmp/mdtui3-goal-run/fence-blank-validation.log`，首轮复现 `fence-blank-repro.log`，专项 `fence-blank-targeted-final.log`，最终全套 `fence-blank-debug-final.log` / `fence-blank-release.log`，范围校验及独立差异见同目录 `fence-blank-scope-check.log` / `fence-blank-unit.patch`。


## 2026-11-04 同型兄弟列表标记的不同缩进交付

本轮闭环 TODO 的 Lists 310–313 列表拆分缺口。已有列表祖先匹配后，新标记缩进至多三列且不足旧 Item 正文缩进时，同型 bullet / 有序分隔符开启兄弟 Item，保留 List ID 和松紧；达到正文缩进则按嵌套处理。每个新 Item 更新正文缩进及叶块标记缩进，部分 tab 路径采用相同规则；任务、编号、后代关闭、空白传播和一次 Loose 通知保持。不同标记、分隔线/Setext、缺失引用、四列代码及分类窗口仍遵守原有边界。

实现只替换三个匹配路径的常数条件并更新叶块缩进，移除不再使用的活动列表标记缩进字段；复用缓存列宽和已有正文缩进，无新增扫描、持久状态、运行时依赖、unsafe 或旧正文/历史容器/保留祖先回扫。累计追加 O(n log n)、空间 O(n) 和共享预算契约保持。

官方 CommonMark 0.31.2 Lists 310、311、313 对照原始 HTML 通过；312 列表结构已修复，仅保留既有 lazy 前导空白刻意差异。完整章节现为 **22 支持、4 刻意差异、0 未支持**；官方夹具、生成器和评分投影不改，只复核独立分类/观察及计数。README/TODO 已同步；仅关闭本章已识别缺口的窄子项，完整容器和宽泛标准覆盖仍未完成。

新增 `tests/marker_indent.rs` 六项回归，覆盖 1920 组标记/缩进/作用域/空白/行尾模型，部分 tab、嵌套/分隔线/四列代码边界，UTF-8 全切点/逐前缀/EOF、连续源码映射、零预算和六字节恢复，条目身份与关闭/Loose 通知。480 层列表、千个兄弟条目、百万字正文及千次单字追加的计数检查通过；官方全章 730 切点/704 前缀及预算检查继续通过。修复前 5 项失败/1 项通过；专项中引用空行补上显式前缀、引用 tab 恰好达到正文缩进的预期按独立列宽修正后，12 项专项通过。首轮全套 1254 通过/2 失败，均为旧的不同缩进部分 tab 字面预期；更新这两条断言后最终全套通过。

验收：项目 devShell 内 `cargo check --all-targets`、debug/release `cargo nextest run --no-fail-fast` 各 **1256 项**、`cargo test --doc` **75 项（72 + 3）**、clippy `--all-targets -- -D warnings`、fmt、官方夹具 `--check` 及 diff 检查全部通过。复跑脚本 `/tmp/mdtui3-goal-run/marker-indent-validate.sh`；汇总 `marker-indent-validation.log`、修复前 `marker-indent-repro.log`、最终全套 `marker-indent-debug-final.log` / `marker-indent-release.log` 均在同目录。范围校验 `marker-indent-scope-check.log` 确认仅 7 个已有文件和 1 个新增测试变化，其余 303 个基线文件 SHA-256 保持，包括官方夹具、生成器和评分投影；独立差异 `marker-indent-unit.patch`。


## 2026-11-04 CommonMark List items 标准样例评分交付

从标准语法覆盖待办拆出 CommonMark 0.31.2 完整 List items 章节（253–300，共 48 例）。新增离线生成器及官方 Markdown/HTML 夹具，固定原始 JSON 的 SHA-256 和完整章节成员；`--check` 检查生成结果，独立 JSON/夹具对照确认全部原文相同，追加空白后的来源也被摘要校验拒绝。既有官方夹具与生成器全部复核通过。

评分为 **41 支持、3 刻意差异、4 未支持**。264/289 保留空行终止缩进代码的既有方言，291 保留 lazy 前导空白。278/279 的空条目续行缩进、280 的空条目跨空行、285 的空标记中断段落是尚未实现的完整容器规则，已独立写入 TODO；本单元交付可复查的评分，不改解析行为，不勾选完整容器或宽泛标准覆盖总项。

测试投影按公开容器身份/父子关系、首条目编号、父 List 松紧、叶块、标题索引和显示片段输出本章的窄范围 HTML；标题校验 BlockId、级别、源码和作用域。投影不接收样例编号，不重解析、不 trim、不合并独立列表或补代码空行；当前差异观察与官方期望分离，未评分的任务、样式和对象显式拒绝。只用于短样例测试，不将祖先遍历投影视为窗口级排版接口。

新增六项回归：完整章节报告、1541 个切点、1493 个逐字符前缀及各前缀 EOF、连续原始/显示/逐字符映射、零预算停步和共享六字节预算恢复，以及编号、分组、嵌套松紧、空条目、隐藏定义、标题级别、代码空白的敏感性和拒绝边界。所有官方输入均为短 ASCII，长输入/深容器及 Unicode 仍由各自专项覆盖。本单元没有修改核心、已有测试或持久状态，没有新增运行时依赖或 unsafe，累计追加 O(n log n)、空间 O(n) 和共享预算契约保持。

项目 devShell 验收全部通过：debug/release 各 **1262 项**、doc **75 项（72 + 3）**，`cargo check --workspace --all-targets`、clippy `--workspace --all-targets -- -D warnings`、fmt、夹具 `--check` 和 `git diff --check`。专项六项首次正式验收全部通过。复跑脚本 `/tmp/mdtui3-goal-run/list-items-validate.sh`；汇总日志 `list-items-validation.log`，专项 `list-items-targeted.log`，全套 `list-items-debug.log` / `list-items-release.log`，独立来源校验 `list-items-provenance.log`（均在同目录）。范围校验 `list-items-scope-check.log` 确认仅追加 README/TODO/DONE 并新增本章测试/夹具/生成器，其余 308 个基线文件 SHA-256 保持；独立差异 `list-items-unit.patch`。


## 2026-11-04 有界空列表标记的段落中断规则交付

本轮从 TODO 的 List items 空条目缺口拆出 285：完整分类窗口内，列表标记之后只有 space/tab 或行尾时，不能中断同作用域段落。根、显式容器、部分 tab 路径与缺失前缀的 lazy 续行保持原段落和容器身份；同 List 的兄弟空条目、新 Quote、段落已结束后的空条目仍可创建。Setext/分隔线优先，任务标记视为非空候选；任意编号的非空条目中断及窗口外不追认方言保持。源码、前导/末尾空白、CRLF 显示及列表松紧/通知沿用既有规则。

实现仅传递根段落的临时布尔选项，在新列表创建和 lazy 探测处复用已有标记空白终点。没有新增扫描、持久状态、依赖或 unsafe，不回扫旧正文、历史容器或祖先；新增工作为 O(1) 条件，累计解析 O(n log n)、空间 O(n) 与共享预算契约保持。

新增 `tests/empty_interrupt.rs` 六项回归：1440 组标记/作用域/缩进/尾空白/行尾模型，lazy 与部分 tab、兄弟/新作用域/任务/Setext/分隔线/表头/窗口边界，UTF-8 全切点/逐字符前缀/EOF，原始与切片映射、零预算停步和六字节恢复，身份与关闭/Loose 通知。980 层引用中百万字正文后的 100 个空标记按新输入计费，后续百万字长行及千次追加不重分类或回扫容器。

官方 CommonMark 0.31.2 List items 285 对照原始 HTML 通过，完整章节为 **42 支持、3 刻意差异、3 未支持**。官方夹具、生成器和评分投影不改，只更新独立分类及计数；全章 1541 切点、1493 前缀与逐前缀 EOF/映射/预算继续验收。278/279 空条目续行缩进、280 空条目跨空行、完整容器与宽泛标准覆盖仍未完成；README/TODO 已同步，历史 DONE 记录不改。

修复前专项 5 失败/1 通过；新测试的 CRLF、终止尾空白/根缩进和 tab 列宽预期按既有规则校正后通过。首轮全套 1266 通过/2 失败，一条为上述尚未校正的模型期望，另一条是旧 lazy 测试把空 `+` 当作中断；该中断输入改为 `+ next`，空标记由新增测试独立覆盖。日志 `empty-interrupt-repro.log`、`empty-interrupt-debug.log` 和 `empty-interrupt-targeted-final.log` 均在 `/tmp/mdtui3-goal-run/`。

最终项目 devShell 验收全部通过：debug/release 各 **1268 项**、doc **75 项（72 + 3）**，`cargo check --workspace --all-targets`、clippy `--workspace --all-targets -- -D warnings`、fmt、官方夹具 `--check` 和 `git diff --check`。专项 26 项通过。复跑命令 `nix develop -c bash /tmp/mdtui3-goal-run/empty-interrupt-validate.sh`，总日志 `empty-interrupt-validation.log`，全套 `empty-interrupt-debug-final.log` / `empty-interrupt-release.log`。范围校验 `empty-interrupt-scope-check.log` 确认只改 7 个已有文件并新增 1 个测试，其余 307 个基线文件 SHA-256 保持，包括官方夹具/生成器；评分投影及历史 DONE 保持。独立差异 `empty-interrupt-unit.patch`（日志及补丁均在 `/tmp/mdtui3-goal-run/`）。


## 2026-11-04 有界空列表标记的续行缩进归一交付

本轮只完成 List items 278/279 暴露的有界空标记续行缩进，不完成空条目跨空行规则、完整容器或宽泛标准覆盖。

- 方言：完整分类窗口内，合法标记之后仅 space/tab 或行尾，续行缩进统一为标记前缩进 + 标记宽度 + 一列分隔；无尾空白补隐含分隔列，任意尾空白不增加正文缩进。新条目、兄弟、引用及部分 tab 祖先路径共用该规则，首行保持 Blank 与完整源码、编号/身份。
- 实现：仅修改 `Prefix::marker` 两个完整空标记返回分支，复用已有空白扫描终点；窗口外候选不追认，未完成输入和 EOF 沿用有界分类。非空正文/任务、Setext/分隔线优先级、空标记不能中断段落、列表松紧和通知保持。没有新增扫描、持久状态、旧正文/历史容器/祖先回扫、依赖或 unsafe，累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。
- 验证：`tests/empty_indent.rs` 六项回归，1536 组标记/缩进/作用域/尾空白/行尾模型及各自不足一列反例；代码围栏/缩进代码、部分 tab 映射、新引用/已有路径、任务与关闭/Loose 通知、窗口边界、UTF-8 全切点/逐前缀/逐前缀 EOF、原始/显示源码映射、零预算停步与六字节恢复；980 层引用、百万字历史正文、100 个空兄弟及百万字正文后的千次追加计数。
- 标准评分：CommonMark 0.31.2 List items 278/279 通过原始官方 HTML，全章 44 支持、3 刻意差异、1 未支持；280 仍保留空条目跨空行缺口，独立观察仅去掉先前错误多留的一列空格。官方夹具、离线生成器、评分投影不变；原有 1541 切点/1493 前缀及预算回归通过。
- 回归处理：专项修复前 5 失败/1 通过。新测试最初将 `Code` 当作有 `fenced` 字段，编译后修正为现有公开类型；部分 tab 的嵌套测试起始正文改为标题，以避开已有 Setext/段落中断优先级。首轮全套 1272 通过/2 失败，两个旧测试各补足空标记要求的一列续行缩进，保持原来的作用域/松紧检查目的；相关 36 项通过。最终 debug/release 各 1274 项通过。
- 剩余：280 空条目跨空行终止、264/289 缩进代码空行兼容扩展、291 lazy 前导空白、完整容器及宽泛标准覆盖保持 TODO；README/TODO 已同步当前评分与范围。
- 验收命令：项目 devShell 中 `cargo check --all-targets`、`cargo nextest run --no-fail-fast`、`cargo nextest run --release --no-fail-fast`、`cargo test --doc`（75 项）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check` 全通过；离线 48 例夹具校验和 `git diff --check` 通过。完整摘要见 `/tmp/mdtui3-goal-run/empty-indent-validation.log`，首次复现、最终专项及 debug/release 原始日志同目录。


## 2026-11-04 有界空列表条目的空行终止交付

本轮只完成 List items 280 暴露的空条目跨空行缺口：完整有界的无任务空条目，后续遇到完整有界空行后不再吸收第一段正文。同型兄弟按既有缩进规则保留 List 并一次确认 Loose，其余退出 Item/List 后按父作用域重新分类；空行仍暂归旧条目，EOF 关闭不确认 Loose。任务、已有正文/定义/子容器、缺失引用与分类窗口外不追认的方言保持。部分 tab 祖先原子匹配，剩余列参与父作用域的正文/代码分类和映射。

实现复用活动条目的 `has_content` / `pending_blank`、延迟空行深度与任务属性，在三个既有匹配路径消费条目缩进之前作常数判断；部分 tab 结构探测增加“空条目结束”的成功路径。没有新增持久状态、扫描、运行时依赖或 unsafe，不回扫历史正文/容器/保留祖先；每层匹配仍按新输入前缀、关闭按一次出栈计费，累计追加 O(n log n)、空间 O(n) 和共享预算保持。

新增 `tests/empty_blank.rs` 六项回归：1536 组作用域/标记/缩进/空白模型、父作用域重新分类、兄弟身份与关闭/Loose 通知、任务/已有子容器/窗口边界/EOF、部分 tab、UTF-8 全切点/逐字符前缀及各前缀 EOF、连续原始/显示源码映射、零预算停步与六字节恢复。980 层引用下的百万字历史正文、千次空行、深层空条目退出、百万字长行和千次追加计数通过。相关 30 项专项通过；官方章原有 1541 切点/1493 前缀和预算回归继续通过。

官方 CommonMark 0.31.2 List items 280 匹配原始 HTML，全章为 **45 支持、3 刻意差异、0 未支持**。官方夹具、生成器及评分投影不变，仅更新独立分类/计数并移除不再使用的 Unsupported 分支；17 份 CommonMark 夹具离线校验通过。README/TODO 已同步；只勾选本章列明的 278/279、280、285 空条目缺口，264/289 缩进代码空行、291 lazy 前导空白、完整容器和宽泛标准覆盖仍待办。

修复前专项 4 失败/1 通过，实施后全部通过，再补第六项边界测试通过。首轮全套 1278 通过/1 失败，旧空行模型仍预期空条目范围包含后续正文；保留输入及松紧检查，更新该模板的列表结束位置。前序 empty_indent 的同一缺口断言同步为根作用域，其余旧测试不变。

最终项目 devShell 验收通过：debug/release 各 **1280 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、官方夹具 `--check` 和 `git diff --check`。复跑脚本 `/tmp/mdtui3-goal-run/empty-blank-validate.sh`；汇总 `empty-blank-validation.log`，首次复现 `empty-blank-repro.log`，最终专项 `empty-blank-targeted-final.log`，全套 `empty-blank-debug-final.log` / `empty-blank-release.log`，夹具 `empty-blank-fixtures.log`（均在同目录）。范围核对 `empty-blank-scope-check.log` 确认仅 7 个已有文件及 1 个新测试变化，其余 309 个基线文件 SHA-256 保持；官方夹具/生成器、评分投影与历史 DONE 内容保持。独立差异 `empty-blank-unit.patch`。


## 2026-11-04 CommonMark Link reference definitions 标准样例评分交付

本轮只完成 TODO 的标准样例覆盖子项：固定 [CommonMark 0.31.2 Link reference definitions](https://spec.commonmark.org/0.31.2/#link-reference-definitions) 完整章节 192–218，共 27 例。新增离线生成器、原始夹具和六项评分回归，核心解析及前序测试保持；无新增运行时依赖、unsafe 或解析状态，累计追加 O(n log n)、空间 O(n) 与共享预算契约不变。

- 来源：官方 Markdown/HTML 逐字保留，夹具注明作者、CC BY-SA 4.0 与来源。`scripts/generate_commonmark_reference_definitions.py` 固定整个 spec.json 的 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20` 和本章成员，支持离线 `--check`。独立 HTTPS 下载、独立提取的 27 组编号/输入/HTML 全等及篡改来源拒绝通过，见 `/tmp/mdtui3-goal-run/reference-definitions-provenance.log`。
- 分类：**13 支持、7 刻意差异、7 未支持**。支持 192、200、202–207、210、214–216、218；197/199/201/209/211/212 保留未定义引用隐藏方括号方言，213 保留定义可中断段落方言；193/195/196/198/208/217 缺多行定义，194 缺定义标签转义 `]` 处理。差异观察与官方期望独立保存，变化时强制审查，README/TODO 已记录缺口，宽泛标准覆盖及完整引用定义未勾选。
- 投影：只读公开块/标题索引、根引用容器、显示片段和引用链接，按核心游标解码目标/title 后做 URI/HTML 输出编码。未定义链接保留公开显示标签；不重解析、不 trim、不补括号或修复定义。保留代码字面空白与内部换行，非代码块只省略一个物理终止 LF，Setext 下划线之前的正文 LF 也按此规则处理；实体 LF 保留。结构范围有断言，不作为通用 HTML 渲染器。
- 验证：833 个 UTF-8 切点、806 个逐字符前缀及逐前缀 EOF 对照完整语义快照；原始/显示/切片源码覆盖，零预算停步、六字节显示恢复，链接元数据六字节/无限预算一致。额外验证晚定义前后链接身份/标签范围、首定义优先、原始元数据不被解码覆盖，以及空白/标题/代码/实体 LF/空引用容器的投影敏感性与未评分结构拒绝。专项六项全部通过，见 `/tmp/mdtui3-goal-run/reference-definitions-targeted.log`。

最终项目 devShell 验收全部通过：debug/release 各 **1286 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、固定夹具 `--check` 和 `git diff --check`。复跑命令 `nix develop -c bash /tmp/mdtui3-goal-run/reference-definitions-validate.sh`，汇总日志 `reference-definitions-validation.log`，全套原始日志 `reference-definitions-debug.log` / `reference-definitions-release.log` / `reference-definitions-doc.log`。范围核对 `reference-definitions-scope-check.log` 确认仅向 README/TODO/DONE 追加内容并新增本章测试、夹具及生成器，其余 **314 个**前序文件 SHA-256 保持；独立补丁 `reference-definitions-unit.patch`（上述日志及补丁均在 `/tmp/mdtui3-goal-run/`）。


## 2026-11-04 有界引用定义标签内的转义右括号交付

本轮只修复 Link reference definitions 194 的定义标签扫描：完整分类窗口内的单物理行定义按反斜线奇偶寻找首个未转义 `]`，奇数个反斜线使其留在标签，偶数个使其关闭标签。原始标签拼写继续作为大小写/空白归一的输入，不解码、不改变符号匹配；保留全局首定义优先、晚定义通知、稳定链接/图片身份和目标/title 原始源码区间。

实现仅替换 `block::definition` 的既有有界扫描，增加局部转义布尔值；没有新增持久状态、第二次扫描、运行时依赖或 unsafe，不回扫旧正文、引用或祖先。扫描最多消耗现有分类窗口，仍计入既有分类工作；追加累计 O(n log n)、空间 O(n) 及共享预算契约保持。失败目标/title、缺失冒号、未完成行等待、脚注排除、标签字节上限和分类窗口方言不变。

新增 `tests/escaped_definition_labels.rs` 六项回归：1536 组独立转义奇偶/根与容器/行尾模型，全 UTF-8 切点、逐前缀及 EOF 对照，待定/失败/不透明代码与数学/窗口边界，shortcut/collapsed/图片共享引用、晚定义身份/失效与首定义优先，原始/显示/切片源码映射和零预算停步/六字节恢复，元数据六字节与无限预算对照。980 层引用下百万字历史正文、百万字失败长行的千次追加，以及两万旧引用的晚定义工作量通过；没有随旧正文或引用数增长的重扫。

官方 CommonMark 194 使用未修改的 Markdown/HTML 通过，本章变为 **14 支持、7 刻意差异、6 未支持**。仅更新独立分类与汇总，官方夹具、生成器和公开语义评分投影保持；该章原有 833 切点/806 前缀及 EOF、映射与预算回归通过。README/TODO 已同步；跨行定义、完整失败回退及宽泛兼容性仍待办。本轮额外确认 full 引用第二个标签中的转义仍不受支持，例如 `[text][r\]]` 在首个 `]` 截断后缀，未扩展此路径，已有结果纳入回归并记入 TODO。

专项修复前 5 失败/1 通过；修复后官方 194 的旧“未支持”断言主动失败，审查后改为支持。新测试对 full 引用后缀和实体分片映射的预期按既有契约校正，最终相关 22 项全部通过。原始复现 `escaped-definition-repro.log`，首次修复验证 `escaped-definition-targeted-initial.log`，最终专项 `escaped-definition-targeted-final.log`（均在 `/tmp/mdtui3-goal-run/`）。

最终项目 devShell 验收全部通过：debug/release 各 **1292 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、固定章节夹具 `--check` 和 `git diff --check`。复跑脚本 `/tmp/mdtui3-goal-run/escaped-definition-validate.sh`，汇总 `escaped-definition-validation.log`，全套 `escaped-definition-debug.log` / `escaped-definition-release.log` / `escaped-definition-doc.log`。范围核对 `escaped-definition-scope-check.log` 确认仅 5 个已有文件及 1 个新增测试变化，其余 **315 个**基线文件 SHA-256 保持，官方夹具/生成器、评分投影和历史 DONE 内容不变；独立差异 `escaped-definition-unit.patch`（均在同目录）。


## 2026-11-04 有界 full 引用第二标签的转义括号交付

本轮只完成 TODO 已列明的 full 引用后缀转义缺口：`[text][r\]]` 和 `![alt][r\[]` 可匹配原始 `r\]`、`r\[` 标签。第二标签按反斜线奇偶识别首个未转义 `]`，允许转义 `[`，未转义 `[` 仍拒绝候选。单物理行、999 源码字节、原始标签大小写/空白归一、首定义优先和已有失败/未完成回退保持；完整字面回退、多行后缀/定义、字符计数上限和宽泛兼容性继续待办。

`inline::Tail::Label` 仅新增一个候选布尔值，随输入字节更新；闭合时沿用原有有界符号归一，没有新缓冲、重放或旧正文/引用/祖先回扫。语法字符仍取消继承旧标签的晚管道表头探测，保留原取消/恢复顺序，不扩展有语法后缀的表头隔离能力。每个输入字节处理常数次，归一受既有字节上限约束，累计解析 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 及共享预算契约保持。

新增 `tests/full_reference_escapes.rs` 八项回归：768 组独立转义奇偶/根与容器/链接与图片/行尾模型，UTF-8 全切点/逐前缀/EOF，窗口之外实时逐字追加与同一 LinkId 从 Provisional 到 Sealed，失败/未完成/长度上限及 shortcut/collapsed 边界，晚定义身份/范围通知与首定义优先，原始/显示/切片映射、零预算停步与六字节显示/元数据预算对照。另有 32 组长表头全切点（含同引用作用域的合法分隔行和无转义控制组）、普通新表头引用，以及 980 层百万字失败候选后的千次追加、两万历史引用的晚定义工作量。

最初新增七项修复前 6 失败/1 通过；修复后全部通过，随后补第八项真实流式状态检查。旧 `escaped_definition_labels` 仅更新原先明确保留 full 后缀缺口的目标/title 断言，输入、身份和其它检查保持。官方引用定义章仍为 14 支持/7 刻意差异/6 未支持，夹具、生成器和评分投影不改；README/TODO 已准确更新支持范围，不宣称完成 Links/Images 整章评分。首次编译修正新测试误用的预算枚举名，未改产品接口。原始复现 `full-reference-repro.log`，专项首次/最终日志 `full-reference-targeted-initial.log` / `full-reference-targeted-final.log`（均在 `/tmp/mdtui3-goal-run/`）。

最终项目 devShell 验收全部通过：debug/release 各 **1300 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、固定引用定义夹具 `--check` 和 `git diff --check`。表头测试补足同作用域合法分隔行及普通后缀控制组后，release 全套和 debug 专项 20 项均通过。复跑脚本 `/tmp/mdtui3-goal-run/full-reference-validate.sh`，汇总 `full-reference-validation.log`，全套日志 `full-reference-debug.log` / `full-reference-release.log` / `full-reference-doc.log`。范围核对 `full-reference-scope-check.log` 确认仅 5 个已有文件及 1 个新增测试变化，其余 **316 个**基线文件 SHA-256 保持，官方夹具/生成器、评分投影、依赖及历史 DONE 内容不变；独立差异 `full-reference-unit.patch`（均在同目录）。


## 2026-11-04 CommonMark Images 标准样例评分交付

本轮只完成 TODO 的标准覆盖方向中 Images 章节评分：固定 [CommonMark 0.31.2 Images](https://spec.commonmark.org/0.31.2/#images) 完整 572–593 共 22 例，独立报告 **19 支持、1 刻意差异、2 未支持**。官方 Markdown/HTML 逐字保留，离线生成器校验固定 SHA-256 和完整章节成员；重新下载来源后的独立三元组比对、夹具 `--check` 及篡改来源拒绝均通过。

公开语义投影按 `LinkInfo.kind/label/source` 输出图片或链接，空 alt 图片由对象范围保留；alt 取已解码可见标签文字，不输出样式标签，内部空白/换行保持。目标和 title 通过公共解码游标共享六字节预算，保留连续原始源码映射，结果与无限预算一致。标签之外的对象语法必须为 HIDDEN，投影不补括号、不重解析 Markdown、不合成外层图片；对章节外块、样式组合和通用 URI 编码主动拒绝。

新增六项测试覆盖全部官方输入的 **723 个切点、701 个逐字符前缀及逐前缀 EOF**、公开结构/显示/对象快照、原始/显示/切片映射、零预算和六字节恢复，以及 UTF-8/实体/转义、空 alt/空与缺失 title、inline/full/collapsed/shortcut、晚定义稳定身份与首定义优先、属性变化敏感性及拒绝边界。首轮专项 5 通过/1 失败是测试错误假定三连强调必然产生嵌套样式，移除该不属于本章的输入假定后六项通过；未修改核心或既有测试来满足评分。随后加强元数据成功步及结束时的预算上界断言，最终 debug 专项六项再次通过。

574/575 中内嵌图片或链接替换外层图片候选，尚未支持规范要求的外层对象；590 保留未定义引用隐藏方括号方言。README/TODO 仅追加评分范围及具体待办，完整图片/引用失败回退、嵌套候选模型、通用 URI 输出和宽泛标准覆盖仍未完成。本轮未改核心解析、依赖、unsafe 约束或累计 O(n log n)/空间 O(n) 与共享预算契约。

最终项目 devShell 验收全部通过：debug/release 各 **1306 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、固定 Images 夹具 `--check` 和 `git diff --check`。最终预算断言由 debug 专项及 release 全套覆盖。复跑脚本 `/tmp/mdtui3-goal-run/images-validate.sh`，汇总 `images-validation.log`，专项 `images-targeted-final.log`，来源 `images-provenance.log`，全套 `images-debug.log` / `images-release.log` / `images-doc.log`。范围核对 `images-scope-check.log` 确认仅追加 README/TODO/DONE 并新增本章测试、夹具、生成器，其余 **319 个**基线文件 SHA-256 保持，核心、依赖、既有测试/夹具/生成器和历史 DONE 均不变；独立差异 `images-unit.patch`（均在同目录）。

## 2026-11-04 CommonMark Inlines 标准样例评分交付

本轮只完成 TODO 标准覆盖方向的 [CommonMark 0.31.2 Inlines](https://spec.commonmark.org/0.31.2/#inlines) 本节评分：唯一的样例 327 为 **1 支持、0 刻意差异、0 未支持**。Inlines 的 Code spans、Emphasis、Links 等子章节不计入本项，完整行内兼容及宽泛标准覆盖仍未完成。官方 Markdown/HTML 逐字固定，生成器校验 SHA-256 和本节成员；重新下载官方来源后的独立原文比对、夹具 `--check`、篡改来源拒绝均通过。

只读投影由公开段落类型和显示片段生成 HTML，保留代码样式、普通尾文及未匹配反引号；仅省略原始借用片段的物理段末 LF，不裁剪内部空白，不把实体换行当段末，不重解析源码。未评分的块、对象和其它样式主动拒绝；样例编号与输入字符串不控制投影。没有核心修改、运行时依赖或 unsafe，累计解析 O(n log n)、空间 O(n) 及共享预算契约不变。

新增五项回归全部通过：官方输入 **9 个切点、8 个逐字符前缀及逐前缀 EOF**，公开结构/显示快照、pending、连续源码/切片映射、零预算停步与六字节可恢复显示消费；另有 UTF-8、追加后闭合、段落边界、内部空白/换行、实体换行、代码空白规范化、投影属性敏感性及拒绝边界。首轮专项即全部通过，未改既有测试或核心行为来满足评分；完整未闭合代码重放、更完整强调/链接和其它标准章节继续保留原待办。

最终项目 devShell 验收全部通过：debug/release 各 **1311 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、固定 Inlines 夹具 `--check` 和 `git diff --check`。复跑脚本 `/tmp/mdtui3-goal-run/inlines-validate.sh`，汇总 `inlines-validation.log`，专项 `inlines-targeted.log`，来源 `inlines-provenance.log`，全套 `inlines-debug.log` / `inlines-release.log` / `inlines-doc.log`。范围核对 `inlines-scope-check.log` 确认仅追加 README/TODO/DONE 并新增本节测试、夹具、生成器，其余 **322 个**基线文件 SHA-256 保持；独立差异 `inlines-unit.patch`（均在同目录）。

## 2026-11-04 CommonMark Autolinks 官方 HTML 评分交付

本轮只补齐 TODO 标准覆盖方向的 [CommonMark 0.31.2 Autolinks](https://spec.commonmark.org/0.31.2/#autolinks) 官方 HTML 评分：594–612 完整 19 例为 **18 支持、1 刻意差异、0 未支持**。新增官方原文夹具及固定 SHA-256/章节成员的离线生成器，保留旧归一化目标语义夹具及测试。重新下载官方来源的独立 Markdown/HTML 三元组比对、生成器 `--check`、来源篡改拒绝和全部已有章节夹具校验通过。

投影只读公开段落/显示/链接对象，URI/email 的目标与标签同源，email 的 `mailto:` 使用核心 URL 接口；输出层做 HTML 转义及本章需要的反斜线/方括号编码，另验证 UTF-8 与已有百分号序列。它不接受样例编号，不重解析源码、解码地址中的实体或补尖括号，主动拒绝未评分的对象、块、样式及其它 URI 编码。606 保留既有失败候选不重放反斜线转义的方言，独立当前观察与官方期望分离；完整失败回退、通用 URI 输出和宽泛标准覆盖保持待办。

新增六项回归首轮即全部通过：全部 **421 个切点、402 个逐字符前缀及逐前缀 EOF**、公开结构/显示/对象快照与 pending、连续源码及逐字符切片、零预算停步/六字节显示消费、显示与目标游标共用六字节和一次 query/object 的预算。补充 URI/email 原始语义、UTF-8、实体字面、空 URI 目标、未完成/失败候选、相邻自动链接、内部空白/实体换行、投影敏感性及拒绝边界。没有核心修改或新增运行时依赖/unsafe；累计追加 O(n log n)、空间 O(n) 和共享预算契约保持。

最终项目 devShell 验收全部通过：debug/release 各 **1317 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、全部固定来源夹具 `--check` 和 `git diff --check`。复跑脚本 `/tmp/mdtui3-goal-run/autolinks-validate.sh`，汇总 `autolinks-validation.log`，专项 `autolinks-targeted.log`，来源 `autolinks-provenance.log`，全套 `autolinks-debug.log` / `autolinks-release.log`。范围核对 `autolinks-scope-check.log` 确认仅追加 README/TODO/DONE 并新增本章测试、官方夹具、生成器，其余 **325 个**基线文件 SHA-256 保持；独立差异 `autolinks-unit.patch`（均在同目录）。

## 2026-11-04 CommonMark 行内链接目标与 title 子集评分交付

本轮只完成 TODO 标准覆盖方向的 [CommonMark 0.31.2 Links](https://spec.commonmark.org/0.31.2/#links) **482–511 共 30 例**，范围为行内链接目标、title 与分隔规则，报告 **21 支持、6 刻意差异、3 未支持**。Links 的 512–571（标签、嵌套与引用组合）尚未纳入本项；这不是完整章节评分或完整链接兼容性完成。官方 Markdown/HTML 逐字保留；离线生成器校验固定 SHA-256 和子集成员，重新下载官方来源后独立解析夹具三元组比对、篡改拒绝及全部 22 份 CommonMark 生成器 `--check` 通过。

投影仅使用公开块、显示片段、`LinkInfo.kind/label/source` 和目标/title 解码游标；按对象范围输出空标签链接，语法包络内标签以外必须为 HIDDEN，未定义引用沿用核心公开显示结果。元数据共享六字节预算并检查连续源码映射，输出层仅补足本子集所需的空格、双引号、反斜线和 UTF-8 URI 编码，保留既有百分号序列；不把 NBSP 当成 title 分隔符。投影不接收样例编号、不补括号、不重解析或恢复失败候选，主动拒绝未评分结构、样式、图片、已解析引用、自动链接和其它 URI 编码。

六项新测试覆盖全部 **722 个合法 UTF-8 切点、692 个逐字符前缀及逐前缀 EOF**、结构/显示/对象快照与 pending、连续原始/显示/逐字符切片映射、零预算停步、六字节显示消费与有界恢复。另有显示/目标/title 三个游标共用一帧六字节及一次 query/object 的完整消费链、原始 URL/title 源码与解码结果、空标签/目标、空与缺失 title、UTF-8/实体/转义和已有百分号、后续追加不改变旧对象身份、属性变化敏感性与拒绝边界。首次编译修正新增测试的目标枚举名及读取借用结束时点后，六项专项全部通过；未修改核心或旧测试。

488/490/493/497/508/511 保留既有未定义 shortcut 隐藏方括号的方言，493 还保留失败候选不重放转义；491/494 的原始 HTML 不在核心需求范围，并伴随回退差异；510 跨物理行目标/title 后缀尚未支持。独立观察与官方期望分离，结果改变时必须复核分类。README/TODO 仅追加准确范围及剩余任务；跨行后缀需先验证候选/撤销、容器边界及复杂度，完整字面回退、Links 余下评分与宽泛标准覆盖继续待办。本轮没有核心解析、运行时依赖或 unsafe 变化，累计追加 O(n log n)、空间 O(n) 和共享预算契约保持。

最终项目 devShell 验收全部通过：debug/release 各 **1323 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、固定来源夹具校验和 `git diff --check`。复跑命令 `nix develop -c bash /tmp/mdtui3-goal-run/inline-links-validate.sh`；汇总 `inline-links-validation.log`，专项 `inline-links-targeted.log`，来源 `inline-links-provenance.log`，全套 `inline-links-debug.log` / `inline-links-release.log` / `inline-links-doc.log`。范围核对 `inline-links-scope-check.log` 确认仅追加 README/TODO/DONE 并新增子集测试、夹具、生成器，其余 **328 个**基线文件 SHA-256 保持；独立差异 `inline-links-unit.patch`（均在 `/tmp/mdtui3-goal-run/`）。


## 2026-11-04 CommonMark 引用标签匹配子集评分交付

本轮只交付 TODO Links 剩余评分中的 [CommonMark 0.31.2 样例 539–552](https://spec.commonmark.org/0.31.2/#example-539)，引用标签匹配、分隔与定义有效性共 **14 例：6 支持、6 刻意差异、2 未支持**。官方 Markdown/HTML 逐字保存；离线生成器固定 SHA-256 与章节成员。独立重新下载、夹具编号/Markdown/HTML 比对、来源篡改拒绝及全部 **23 份**已有和新增 CommonMark 生成器 `--check` 通过。

539/544/549–552 支持；540 按既有 Unicode 小写规则处理，不做完整 case folding；542/543/545/547/548 保留隐藏引用括号、原始标签拼写和单候选嵌套方言。541 跨物理行定义标签未支持；546 暴露定义标签错误接受未转义 `[`，导致无效定义被隐藏。当前观察与官方期望分离，结果改变时强制复核分类。README/TODO 仅追加本次交付及所暴露缺口；512–538、553–571 仍未评分，完整 Links、多行定义、嵌套图片和完整字面回退继续待办。

投影不接收样例编号，只使用公开块、显示片段、对象包络和目标/title 解码游标；定义块必须确为隐藏，未定义引用按核心显示语义输出。输出层仅做 HTML/URI 编码，不重解析标签、补括号、合成定义或丢弃可见语法。主动拒绝未评分块、样式、图片与自动链接。新增七项测试覆盖 **396 个合法 UTF-8 切点、382 个逐字符前缀及逐前缀 EOF**、结构/显示/对象快照与 pending、连续原始/显示/逐字符切片映射、零预算停步，以及显示/目标/title 三游标共用六字节及一次 query/object 的消费链。

额外状态测试逐字符追加晚定义，验证 Unicode 小写与空白折叠、原始转义拼写、相同归一标签共享符号、不同拼写独立符号、稳定 LinkId/LinkInfo、目标映射到获胜定义源码、首次定义后重复定义不覆盖元数据或修订。初始六项专项通过；补充状态测试时修正新增测试对源码长度接口的调用后，最终七项全部通过。核心、已有测试、运行时依赖及 unsafe 均未变；累计追加 O(n log n)、空间 O(n) 与共享预算契约保持，短样例评分不冒充新的长输入复杂度验证。

最终项目 devShell 验收全部通过：debug/release 各 **1330 项**、doc **75 项（72 + 3）**，`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、固定来源夹具校验和 `git diff --check`。复跑命令 `nix develop -c bash /tmp/mdtui3-goal-run/reference-labels-validate.sh`；汇总 `reference-labels-validation.log`，专项 `reference-labels-targeted-final.log`，来源 `reference-labels-provenance.log`，全套 `reference-labels-debug.log` / `reference-labels-release.log` / `reference-labels-doc.log`。范围核对 `reference-labels-scope-check.log` 确认只追加三份文档及新增测试/夹具/生成器，其余 **331 个**基线文件 SHA-256 保持；独立差异 `reference-labels-unit.patch`（均在 `/tmp/mdtui3-goal-run/`）。


## 2026-11-04 有界引用定义标签的未转义左括号交付

本轮只完成引用标签匹配待办中的未转义 `[` 拒绝规则：完整分类窗口内的单物理行定义跳过初始 opener，沿用原有转义布尔值扫描首个未转义括号；遇到 `[` 即拒绝定义，只有 `]` 才进入原冒号/目标/title 路径。原始标签拼写、转义左右括号与反斜线奇偶、999 字节上限、首定义优先、稳定对象和源码映射保持。目标/title 中的 `[`、脚注、代码/数学、有界分类与多行规则不扩大。

CommonMark 546 的无效定义现在保留正文，官方输入/HTML 与公开语义投影不变；该例仍因未定义引用隐藏部分括号而与官方不同，复核为刻意差异。539–552 子集目前 **6 支持、7 刻意差异、1 未支持**，原评分 6/6/2 留在历史记录。541 跨行定义、540 完整 case folding、完整字面回退/嵌套及宽泛标准覆盖继续待办，README/TODO 同步实际范围。

新增 `definition_openers.rs` 七项回归：768 组容器/缩进、0–7 条反斜线、UTF-8/转义括号及 LF/CRLF/EOF 组合；无效定义正文回退及不解析符号、未完成输入、全部合法切点与逐字符前缀/EOF、表格失败候选、窗口/标签长度和字面上下文；晚定义/首定义、稳定 LinkId/LinkInfo、目标/title 原始映射、零预算停步及六字节显示/元数据消费。已有官方评分继续覆盖显示/目标/title 三游标的共享预算。初始五项修复前 **4 失败/1 通过**，修复后全部通过；补齐映射与复杂度两项后，相关四个测试文件 **28 项通过**。评分两条旧观察随 546 真实变化更新，未改官方 oracle 或投影。

核心仅替换已有有界扫描的终止条件，复用局部布尔值；无新增持久状态、额外扫描或旧正文/引用/祖先回访。980 层/百万字后续千次追加不重复分类或遍历容器；深层有效/无效定义的分类与容器操作计数有界；窗口外百万字失败候选单遍续传；两万旧引用的晚定义更新无旧正文重解析，通知及树访问受限。零运行时依赖、禁止 unsafe、累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。

项目 devShell 最终验收全部通过：debug/release 各 **1337 项**、doc **75 项（72 + 3）**、`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、23 份 CommonMark 夹具校验和 `git diff --check`。复跑命令 `nix develop -c bash /tmp/mdtui3-goal-run/definition-opener-validate.sh`。汇总 `definition-opener-validation.log`，修复前 `definition-opener-repro.log`，专项 `definition-opener-targeted-final.log`，全套 `definition-opener-debug.log` / `definition-opener-release.log` / `definition-opener-doc.log`，夹具 `definition-opener-fixtures.log`。范围核对 `definition-opener-scope-check.log` 确认仅 5 个已有文件和 1 个新测试变化，其余 **332 个**基线文件 SHA-256 保持，官方夹具/生成器、评分投影及历史 DONE 不变；独立差异 `definition-opener-unit.patch`（均在 `/tmp/mdtui3-goal-run/`）。


## 2026-11-04 CommonMark collapsed 与 shortcut 引用子集评分交付

本轮只交付 TODO Links 剩余评分中的 [CommonMark 0.31.2 样例 553–571](https://spec.commonmark.org/0.31.2/#example-553)：collapsed/shortcut 引用及后缀优先级共 **19 例，16 支持、2 刻意差异、1 未支持**。官方 Markdown/HTML 逐字保存；离线生成器固定 SHA-256 与 Links 章节成员，重新下载后独立核对编号和两份原文、篡改拒绝及全部 **24 份** CommonMark 夹具 `--check` 通过。

553–563、565–568、570 匹配官方 HTML；564 的强调跨 shortcut 标签边界列为未支持；569/571 保留未定义 full 引用消费后缀、隐藏括号且不重新解析后缀的既有方言。官方期望与冻结观察相互独立，输出变化强制复核分类。投影只读公开块、对象包络、显示样式和核心解码游标，不重解析、不补括号、不移动链接标签；未评分结构及叠加样式主动拒绝。README/TODO 仅追加本次交付和缺口，512–538、完整强调/链接、失败回退及宽泛标准覆盖继续待办。

新增七项测试覆盖 **610 个合法切点、591 个逐字符前缀及逐前缀 EOF**、结构/显示/对象快照与 pending、连续源码及逐字符切片映射、零预算停步、显示/目标/title 共用六字节及一次 query/object 的恢复消费。额外验证空目标、标签样式与元数据变化、URI 输出编码、相同归一标签共享符号、晚定义仍解析原 full 引用对象、目标映射到获胜定义、稳定 LinkId/LinkInfo 和首定义优先。首轮六项通过，一项新增拒绝用例误假定三连星号一定产生叠加样式；改为明确的内外强调组合后七项全部通过。

本单元仅新增评分设施；核心、既有测试、运行时依赖和 unsafe 均未改，累计追加 O(n log n)、空间 O(n) 与共享预算契约保持。短样例不替代既有深层/百万字复杂度验收。项目 devShell 最终通过 debug/release 各 **1344 项**、doc **75 项（72 + 3）**、`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、24 份夹具校验与 `git diff --check`。

专项命令 `nix develop -c cargo nextest run --test commonmark_reference_forms`；汇总 `reference-forms-validation.log`，专项 `reference-forms-targeted-final.log`，来源 `reference-forms-provenance.log`，全套 `reference-forms-debug.log` / `reference-forms-release.log` / `reference-forms-doc.log`。范围核对 `reference-forms-scope-check.log` 确认只追加三份文档并新增测试/夹具/生成器，其余 **335 个**基线文件 SHA-256 保持；独立差异 `reference-forms-unit.patch`（均在 `/tmp/mdtui3-goal-run/`）。


## 2026-11-04 CommonMark 链接标签与嵌套组合子集评分交付

本单元落实 TODO 的 Links 剩余评分中 **512–538** 标签与嵌套组合子集，新增 `commonmark_link_labels.rs`、同名官方夹具与 `scripts/generate_commonmark_link_labels.py`。27 例独立评分为 **15 支持 / 4 刻意差异 / 8 未支持**。四个已交付子集覆盖 Links 482–571 全部 90 个官方样例，但完整标准兼容性仍未完成；本次不改变核心解析、运行时依赖或既有测试。

来源为 [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/#example-512)（John MacFarlane，CC BY-SA 4.0）的官方 `spec.json`，固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。Markdown/HTML 逐字保留；生成器只提取章节成员，无解析器依赖。独立重新下载、编号/成员与原文比对、篡改拒绝和全部 25 份 CommonMark 夹具校验通过。

| 分类 | 样例 | 观察与边界 |
| --- | --- | --- |
| 支持 | 514–516、518–519、522、525–527、529–530、532–533、537–538 | 标签转义/嵌套样式、内层链接优先、代码/自动链接屏蔽和 full 引用与官方相符 |
| 刻意差异 | 513、523 | 未定义引用隐藏括号，保留当前字面回退方言 |
| 刻意差异 | 524、536 | 原始 HTML 字面处理，不屏蔽其中的引用识别；HTML 不在需求范围 |
| 未支持 | 512、528 | 新的未转义括号覆盖外层候选，丢失应有的外层链接 |
| 未支持 | 517、520、531 | 图片/链接内层候选覆盖外层对象，完整嵌套模型尚未实现 |
| 未支持 | 521、534、535 | 强调跨越直接/full 引用标签边界，链接优先级尚未实现 |

评分投影不接收样例编号或预期结果，只消费公开块、LinkInfo/目标、显示样式与目标/title 解码游标。覆盖对象完整源码包络（含隐藏语法）的样式才可包住整个对象，故 519/533 不因拆分强调标签产生伪差异；标签内 em 包住 strong/code 按状态转换保留。其它歧义/反向样式嵌套及未评分结构明确拒绝。投影不重解析 Markdown、不补括号、移动定界符、trim 空白或构造不存在的外层图片/链接；HTML/URI 转义只是输出编码。独立观察单独冻结，后续改变必须审查分类，不能修改官方期望迁就实现。

七项新测试覆盖：

- 全部 27 例及 **814 个切点、787 个逐字符前缀/逐前缀 EOF**，校验结构/显示/对象快照、pending 与元数据；每次完成解析按独立三类结果验收。
- 源码全覆盖、逐字符切片映射、零预算停步、六字节显示读取，以及显示/目标/title 三游标共享六字节预算与可前进性；额外覆盖 UTF-8、实体、转义、空标签/空目标/title、换行和未完成后缀。
- 晚定义逐字追加保留对象/标签/目标身份；首定义优先，定义源码映射正确；528 内层未定义引用后续可独立解析，但不重新构造丢失外层链接。
- 强调包住链接/图片与标签内强调的区别、目标/title/对象种类/内容变化敏感性及投影拒绝边界。首轮新拒绝用例错误假定三连星号产生叠加样式，改用直接歧义状态断言后七项通过，官方评分无变化。

实现只增加离线测试及生成器，不增加核心状态、扫描或旧正文/引用/祖先回访，累计追加 O(n log n)、核心空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算契约保持。本次短样例不替代已有深层/百万字和操作计数回归。嵌套候选与强调作用域修复、跨行后缀/定义、完整失败回退、通用输出渲染及宽泛兼容性继续待办。

验收（项目 devShell）：`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check` 全通过；debug/release `cargo nextest run --no-fail-fast` **各 1351 项**通过，`cargo test --doc` **75 项**通过（72 普通 + 3 compile-fail）。专项七项、25 份夹具和独立来源校验通过；四份 Links 夹具编号并集精确覆盖 482–571，无重叠或缺号。README/TODO/DONE 仅追加，本次仅新增三份评分文件，其余基线文件保持；范围和差异检查见 `/tmp/mdtui3-goal-run/link-labels-scope-check.log`、独立差异 `link-labels-unit.patch`，专项 `link-labels-targeted-final.log`，来源 `link-labels-provenance.log`，汇总 `link-labels-validation.log`（均在同目录）。


## 2026-11-04 CommonMark 基础单分隔符强调子集评分交付

本单元落实 TODO 的强调兼容性与固定标准样例评分，范围仅为 **Emphasis and strong emphasis 350–373**，新增 `commonmark_emphasis_single.rs` 测试/夹具和 `scripts/generate_commonmark_emphasis_single.py`。24 例为 **14 支持 / 8 刻意差异 / 2 未支持**；374–481、完整单分隔符规则与加粗/强调兼容性仍待办。

来源为 [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/#example-350)（John MacFarlane，CC BY-SA 4.0）的官方 `spec.json`，固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。官方 Markdown/HTML（含 NBSP、Unicode 及换行）逐字保留；独立重新下载并使用独立夹具读取器核对编号/章节成员与原文，篡改输入拒绝及全部 26 份 CommonMark 夹具校验通过。

| 分类 | 样例 | 当前语义与边界 |
| --- | --- | --- |
| 支持 | 350–351、355–358、360–361、364–367、370–371 | 基础配对、ASCII 词内下划线限制、空白和不匹配分隔符与官方相符 |
| 刻意差异 | 352、359、363、368、372 | 简化 flanking 不执行完整的标点邻接规则 |
| 刻意差异 | 353、354、362 | ASCII 空白/字母数字判定，非 ASCII 空白和符号视作普通字节；354 同时保留美元数学候选的优先级 |
| 未支持 | 369、373 | 贪心配对只强调两侧括号，内层 `foo` 未获强调；公开样式位也不表示同类嵌套深度 |

独立观察冻结当前实际输出，改变后必须审查分类；投影不接收样例编号或期望，只读公开块和显示样式，不重解析 Markdown、不推导分隔符树、不补标记或 trim。它仅投影公开的单层强调位，不能用于声称任意同类嵌套结构；其它块、对象和未评分样式主动拒绝。段末借用源码 LF 为结构，其余空白和实体产生的 LF 保留，硬换行按公开样式输出。

六项新测试覆盖全部样例及 **286 切点、262 逐字符前缀/逐前缀 EOF**，比较结构/显示快照、pending、源码与逐字符切片映射；六字节显示游标的文本/样式与无限预算对照，零预算和共享预算耗尽不推进另一游标；另验证 UTF-8、空白/实体、跨行与空行边界、失败/未完成分隔符、分类窗口外晚闭合前后块身份及投影敏感性/拒绝范围。初次编译中的 Block 身份访问改为公开 `block_at` 后六项通过；未修改核心或既有测试预期。

本单元只增加离线评分设施与文档，不增加核心状态、扫描或历史正文/引用/祖先回访；零运行时依赖、禁止 unsafe、累计追加 O(n log n)、核心空间 O(n) 和共享预算契约保持。短样例评分不替代已有深层/百万字与操作计数回归。同类嵌套、Unicode/标点 flanking、部分 run 消耗、链接强调边界、通用渲染和宽泛兼容性继续待办。

验收（项目 devShell）：`cargo check --all-targets`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check` 通过；debug/release `cargo nextest run --no-fail-fast` **各 1357 项**通过，`cargo test --doc` **75 项**通过（72 普通 + 3 compile-fail）。来源记录 `emphasis-provenance.log`，专项 `emphasis-targeted.log`，全套 `emphasis-debug.log` / `emphasis-release.log` / `emphasis-doc.log`，汇总 `emphasis-validation.log`；最终共享预算样式断言由 debug/release 全套覆盖。README/TODO/DONE 仅追加，范围核对和独立差异见 `emphasis-scope-check.log`、`emphasis-unit.patch`（均在 `/tmp/mdtui3-goal-run/`）。

## 2026-11-04 CommonMark 单下划线收尾与基础加粗子集评分交付

本单元交付 CommonMark 0.31.2 Emphasis and strong emphasis 的 **374–389 共 16 例**评分：374–377 为单下划线收尾，378–389 为基础加粗；独立报告 **11 支持、4 刻意差异、1 未支持**。350–373 的已有评分保持，两者合计覆盖 350–389；390–481、完整强调与宽泛标准覆盖继续保留 TODO。

新增 `scripts/generate_commonmark_emphasis_strong.py`、`tests/fixtures/commonmark_emphasis_strong.rs` 和 `tests/commonmark_emphasis_strong.rs`。官方输入/HTML 来自 <https://spec.commonmark.org/0.31.2/spec.json>，固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`，保留 John MacFarlane / CC BY-SA 4.0 归属、原章节及行号。重新下载独立比对编号、Markdown/HTML 和摘要，篡改拒绝与全部 27 份夹具离线检查通过。

评分投影只读公开段落/显示片段及 EMPHASIS/STRONG 位，按样式变化输出标签；不接收样例编号或预期、不扫描 Markdown 定界符、不补标记、不合成嵌套树，混合强调/加粗嵌套明确拒绝。375/388 保留 ASCII 字母数字词内判断，380/385 保留简化标点边界。389 全部正文已有加粗，但 API 没有同类嵌套深度，无法输出官方双层 strong，归未支持。首轮对 389 的独立观察误估为内层无样式，导致两项新测试失败；核对分隔符开闭与实际公开显示后修正观察及原因，官方夹具与投影未为此改动。核心修复及完整嵌套语义另行设计。

六项新测试全部通过：230 个合法 UTF-8 切点、214 个逐字符前缀及逐前缀 EOF 的快照/pending 等价；源码连续覆盖与逐字符切片；六字节共享预算的显示及逐字符样式与无限预算对照、零预算和第二游标停步、前进至 Complete；超过分类窗口后的 `**`/`__` 晚闭合保持块身份；未闭合 EOF、实体、空白、换行、可见标记、输出敏感性与越界拒绝。运行 `nix develop -c cargo nextest run --test commonmark_emphasis_strong --no-fail-fast`；夹具使用 `python3 scripts/generate_commonmark_emphasis_strong.py /path/to/spec.json --check`。

没有核心、运行时依赖、旧测试或已有夹具改动；累计追加 O(n log n)、空间 O(n)、每字节扫描及每个 opener 出入栈的既有契约保持。本轮短样例评分不单独证明长输入复杂度，全套仍覆盖既有大输入回归。README/TODO/DONE 只追加实际交付及剩余范围，不勾选宽泛强调或标准兼容性任务。

最终验收：`cargo check --all-targets`、debug/release `cargo nextest run --no-fail-fast` 各 **1363 项**、`cargo test --doc` **75 项**、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、27 份官方夹具及 `git diff --check` 均通过（Cargo 命令在本项目 `nix develop` 中执行）。汇总 `/tmp/mdtui3-goal-run/strong-validation.log`；专项 `strong-targeted-final.log`，首次失败 `strong-targeted.log`，来源 `strong-provenance.log`，全套 `strong-debug.log` / `strong-release.log` / `strong-doc.log`，范围核对 `strong-scope-check.log` 和独立差异 `strong-unit.patch` 均位于同一目录。

## 2026-11-04 CommonMark 加粗收尾与混合强调子集评分交付

本单元完成 TODO 中尚未评分的 CommonMark 0.31.2 Emphasis and strong emphasis **390–405 共 16 例**，新增 `scripts/generate_commonmark_emphasis_strong_close.py`、`tests/fixtures/commonmark_emphasis_strong_close.rs` 和 `tests/commonmark_emphasis_strong_close.rs`。评分为 **13 支持、3 刻意差异、0 未支持**：392/398 的标点边界和 401 的非 ASCII 词内下划线沿用已声明方言；其余与官方 HTML 相符。三份强调夹具编号并集恰为 350–405，406–481、同类嵌套深度、完整强调和宽泛标准覆盖继续待办。

来源为 [CommonMark 0.31.2 官方数据](https://spec.commonmark.org/0.31.2/spec.json)（John MacFarlane，CC BY-SA 4.0），固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。Markdown/HTML（含空白、换行、Unicode）逐字保留，记录来源行号。重新下载后与既有来源比对，并通过独立夹具读取器核对编号/章节/原文；篡改源输入拒绝，全部 28 份 CommonMark 夹具校验通过。

评分不接收样例编号或预期，不扫描源码分隔符、不补标记或裁剪正文；仅依据公开样式变化确定先出现的外层样式，支持本子集两种可辨混合嵌套顺序，同时开启多个样式和交叉结束主动拒绝。直接链接要求完整对象范围具有统一强调样式、标签外语法隐藏、非空标签且对象位于同一段；目标/title 复用公开解码游标，输出层借用已有元数据适配器。图片、引用链接、链接内部样式变化及其它未评分结构不纳入投影。已有同类嵌套深度缺口不因本子集匹配而关闭。

七项回归覆盖 297 个合法 UTF-8 切点、281 个逐字符前缀及每个前缀 EOF 的快照/pending 等价；源码连续覆盖、逐字符切片、六字节显示读取的文本和样式与无限预算对照、零预算与第二游标共享耗尽；超过分类窗口后加粗晚闭合保持块身份，混合嵌套两种顺序下外层晚闭合保持块/链接身份及目标，显示与目标共用六字节预算。还检查实体、空白/换行、未闭合 EOF、可见标记、投影敏感性与歧义拒绝。初次编译的 Style 位或改为读取公开混合样式；首轮新测试三项失败源于直接链接保留暂存引用符号，改为检查实际 LinkTarget 后通过，官方夹具和独立分类未更改。

本次只增加离线评分设施及文档，没有核心、运行时依赖或旧测试改动；累计追加 O(n log n)、核心空间 O(n)、零运行时依赖、禁止 unsafe 及共享预算契约保持。短样例不单独证明大输入复杂度，全套回归保留既有深层/百万字验收。README/TODO/DONE 只追加本轮交付及剩余范围。

最终验收（项目 devShell）：`cargo check --all-targets`、debug/release `cargo nextest run --no-fail-fast` **各 1370 项**、`cargo test --doc` **75 项**（72 普通 + 3 compile-fail）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check` 以及 28 份夹具与 `git diff --check` 均通过。Clippy 指出的新测试 `Copy` 值多余 `clone` 已删除，最终两配置各七项专项及 check/clippy/fmt 复验通过；该处无行为变化，未重复全套。其余 347 个基线文件 SHA-256 保持，三份文档仅追加。汇总 `/tmp/mdtui3-goal-run/strong-close-validation.log`；专项 `strong-close-targeted-final.log` / `strong-close-targeted-release.log`，来源 `strong-close-provenance.log`，全套 `strong-close-debug.log` / `strong-close-release.log` / `strong-close-doc.log`，范围核对 `strong-close-scope-check.log` 和独立差异 `strong-close-unit.patch` 位于同目录。


## 2026-11-04 CommonMark 嵌套强调与分隔符部分消耗子集评分交付

本单元完成 TODO 中尚未评分的 CommonMark 0.31.2 Emphasis and strong emphasis **406–416 共 11 例**，新增 `scripts/generate_commonmark_emphasis_nesting.py`、`tests/fixtures/commonmark_emphasis_nesting.rs` 和 `tests/commonmark_emphasis_nesting.rs`。评分为 **4 支持、0 刻意差异、7 未支持**：406/410/411/412 匹配官方；407 的同类嵌套在公开样式中丢失深度；408/409 要求双连分隔符部分消耗；413–416 要求三连分隔符混合开闭。后七例记为兼容性缺口，不能以既有字面输出冒充支持。417–481、完整强调与宽泛标准覆盖继续待办。

来源为 [CommonMark 0.31.2 官方数据](https://spec.commonmark.org/0.31.2/spec.json)（John MacFarlane，CC BY-SA 4.0），固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。Markdown/HTML（含空白与换行）逐字保留并记录来源行号；生成器只提取固定章节/编号，不运行核心解析器。投影独立读取公开段落/显示片段与样式先后变化，不接收样例编号或预期，不扫描 Markdown 分隔符、补标记、推导同类嵌套树；同时开启多个样式和交叉结束、链接及其它未评分结构主动拒绝。

六项新回归覆盖 171 个合法切点、160 个逐字符前缀及每个前缀 EOF 的快照/pending 等价；源码连续覆盖、逐字符切片和样式一致性、六字节共享预算及零预算停步/第二游标耗尽；超过分类窗口的 Unicode 长正文晚闭合保持块身份，四种混合/同类/未配对组合逐字符追加与整段输入一致；另验证 407 的嵌套与扁平输入公开输出相同但不能匹配官方双层 em，以及实体、空白/换行、可见标记、输出敏感性和歧义拒绝。专项命令 `nix develop -c cargo nextest run --test commonmark_emphasis_nesting --no-fail-fast`；夹具命令 `python3 scripts/generate_commonmark_emphasis_nesting.py /path/to/spec.json --check`。

核心、运行时依赖、旧测试和已有夹具均保持。新增离线测试不改变累计追加 O(n log n)、空间 O(n) 或共享预算契约；短样例评分不单独证明长输入复杂度，全套仍运行既有大输入回归。README/TODO/DONE 仅追加实际交付和剩余范围。

最终验收：`cargo check --all-targets`、debug/release `cargo nextest run --no-fail-fast` 各 **1376 项**、`cargo test --doc` **75 项**、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、29 份官方夹具校验及 `git diff --check` 全部通过（Cargo 命令在项目 `nix develop` 中执行）。独立夹具读取器核对四份强调夹具编号并集恰为 350–416，官方原文逐字一致；新下载匹配固定哈希，篡改来源被拒绝。范围核对确认其余 350 个基线文件 SHA-256 保持，三份已有文档仅追加。汇总 `/tmp/mdtui3-goal-run/emphasis-nesting-validation.log`；同目录保留 `emphasis-nesting-targeted.log`、`emphasis-nesting-provenance.log`、`emphasis-nesting-debug.log` / `emphasis-nesting-release.log` / `emphasis-nesting-doc.log`、`emphasis-nesting-scope-check.log` 与独立差异 `emphasis-nesting-unit.patch`。


## 2026-11-04 CommonMark 长分隔符与嵌套加粗子集评分交付

本单元完成 TODO 中尚未评分的 CommonMark 0.31.2 Emphasis and strong emphasis **417–430 共 14 例**，新增 `scripts/generate_commonmark_emphasis_runs.py`、`tests/fixtures/commonmark_emphasis_runs.rs` 和 `tests/commonmark_emphasis_runs.rs`。评分为 **7 支持、0 刻意差异、7 未支持**：420–424、428/429 匹配官方；417/426/427/430 缺少长分隔符部分消耗，418/425 缺少公开同类嵌套深度，419 的链接正文丢失强调而隐藏包络保留外层强调。431–481、完整强调与宽泛标准兼容性继续待办。

来源为 [CommonMark 0.31.2 官方数据](https://spec.commonmark.org/0.31.2/spec.json)（John MacFarlane，CC BY-SA 4.0），固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。Markdown/HTML（含空白与换行）逐字保留并附来源行号；生成器只提取固定章节/编号，不运行核心解析器。重新下载、独立夹具读取器和篡改拒绝验证来源；五份强调夹具编号并集恰为 350–430，共 81 例。

投影只读取公开段落、显示样式变化和链接元数据，不接收样例编号/预期，不扫描分隔符、补标记、移动样式边界或合成嵌套树。直接链接要求完整对象范围具有统一样式。419 保留拒绝投影，单独断言可见文本、样式、源码范围、链接身份/目标及隐藏包络样式，按未支持计数；不把任意 panic 本身当作缺口证据。425 的嵌套与扁平正文呈现相同公开 strong 位，独立验证不能因此生成第二层 strong。其它结构、同时开启多个样式及交叉结束继续拒绝。

七项新回归覆盖 302 个合法切点、288 个逐字符前缀及每个前缀 EOF 的快照/pending 等价；源码连续覆盖、逐字符切片和逐字符样式一致性、六字节显示预算及零预算/第二游标共享耗尽；超过分类窗口的 Unicode 长正文混合/同类/未配对组合逐字符晚闭合保持块身份，混合强调外层晚闭合保持块/链接身份和目标，显示与目标共用六字节预算。另覆盖实体、空白/换行、可见标记、投影敏感性与歧义拒绝。专项命令 `nix develop -c cargo nextest run --test commonmark_emphasis_runs --no-fail-fast`；夹具命令 `python3 scripts/generate_commonmark_emphasis_runs.py /path/to/spec.json --check`。

核心、运行时依赖、旧测试和已有夹具保持；累计追加 O(n log n)、空间 O(n)、禁止 unsafe 和共享预算契约不变。短样例不单独证明大输入复杂度，全套回归保留既有深层/百万字验收。README/TODO/DONE 仅追加本轮交付及剩余范围。

最终验收（项目 devShell）：`cargo check --all-targets`、debug/release `cargo nextest run` 各 **1383 项**、`cargo test --doc` **75 项**（72 普通 + 3 compile-fail）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、30 份官方夹具校验及 `git diff --check` 全部通过。七项专项首次验收通过；观察阶段确认 419 的非统一链接样式后采用明确拒绝及精确公开语义断言，官方原文与投影未为该样例放宽。范围核对确认其余 353 个基线文件 SHA-256 保持，三份已有文档仅追加。汇总 `/tmp/mdtui3-goal-run/emphasis-runs-validation.log`；同目录保留 `emphasis-runs-targeted.log`、`emphasis-runs-provenance.log`、`emphasis-runs-debug.log` / `emphasis-runs-release.log` / `emphasis-runs-doc.log`、`emphasis-runs-scope-check.log` 与独立差异 `emphasis-runs-unit.patch`。


## 2026-11-04 CommonMark 混合强调收尾与字面分隔符子集评分交付

本单元完成 TODO 中尚未评分的 CommonMark 0.31.2 Emphasis and strong emphasis **431–444 共 14 例**，新增 `scripts/generate_commonmark_emphasis_partial.py`、`tests/fixtures/commonmark_emphasis_partial.rs` 和 `tests/commonmark_emphasis_partial.rs`。评分为 **9 支持、0 刻意差异、5 未支持**：433–441 匹配官方；431 的三连 closer、442–444 的不等长分隔符需要部分消耗，432 的公开 strong 位未保留重复嵌套深度。445–481、完整强调及宽泛标准兼容性继续待办。

来源为 [CommonMark 0.31.2 官方数据](https://spec.commonmark.org/0.31.2/spec.json)（John MacFarlane，CC BY-SA 4.0），固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。官方 Markdown/HTML 与空白逐字保留，生成器只提取固定章节和编号，不运行解析器。重新下载、独立夹具读取器、篡改拒绝及六份强调夹具编号并集 350–444 共 95 例核对通过。

投影只读取公开段落、对象范围、显示样式变化和链接元数据。链接前后隐藏包络须有一致外层样式，标签正文不能丢失外层样式，标签内部样式置于 anchor 内部，退出链接时先关闭内部样式。433 的核心语义已足以表达官方 HTML；旧投影的统一标签样式限制在本单元独立评分器中放宽，已有评分器与测试保持。432 的嵌套和扁平输入得到相同公开样式，独立测试明确不能据此生成第二层 strong；同时开始的歧义样式、交叉结束、419 的链接外层样式丢失及其它未评分结构继续拒绝。投影不接收样例编号/预期、不扫描源码分隔符、不补标记或重建嵌套树。

七项新回归覆盖 224 个字符切点、210 个逐字符前缀及每个前缀 EOF 的快照/pending 等价；源码连续覆盖、逐字符切片和样式一致性、六字节共享显示预算及零预算/第二游标停步；超过分类窗口的 Unicode 长正文混合/同类/不等长分隔符组合晚闭合保持块身份。两种混合样式外层晚闭合前后保留块/链接身份、目标与标签内部强调，显示和目标共用六字节预算。另覆盖实体、空白/换行、可见标记、投影输出敏感性与拒绝边界。专项命令 `nix develop -c cargo nextest run --test commonmark_emphasis_partial --no-fail-fast`；夹具命令 `python3 scripts/generate_commonmark_emphasis_partial.py /path/to/spec.json --check`。

核心、依赖、旧测试与已有夹具保持，三份文档仅追加；累计追加 O(n log n)、空间 O(n)、禁止 unsafe 和共享预算契约不变。短样例评分不独立证明长输入复杂度，全套仍运行既有大输入回归。

最终验收（项目 devShell）：`cargo check --all-targets`、debug/release `cargo nextest run --no-fail-fast` 各 **1390 项**、`cargo test --doc` **75 项**（72 普通 + 3 compile-fail）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、31 份官方夹具校验及 `git diff --check` 全部通过。专项七项两次通过；第二次补充不等长分隔符长正文与链接内部样式的晚闭合/共享预算断言，随后全套覆盖最终代码。观察阶段发现 433 是旧投影限制而非核心缺口，按公开范围扩展本单元投影后判为支持；官方预期未改。范围核对确认其余 356 个基线文件 SHA-256 保持，三份文档仅追加。汇总 `/tmp/mdtui3-goal-run/emphasis-partial-validation.log`；同目录保留 `emphasis-partial-targeted-final.log`、`emphasis-partial-provenance.log`、`emphasis-partial-debug.log` / `emphasis-partial-release.log` / `emphasis-partial-doc.log`、`emphasis-partial-scope-check.log` 和独立差异 `emphasis-partial-unit.patch`。


## 2026-11-04 CommonMark 不等长强调与转义分隔符子集评分交付

本单元完成 TODO 中尚未评分的 CommonMark 0.31.2 Emphasis and strong emphasis **445–459 共 15 例**，新增 `scripts/generate_commonmark_emphasis_unequal.py`、`tests/fixtures/commonmark_emphasis_unequal.rs` 和 `tests/commonmark_emphasis_unequal.rs`。评分为 **6 支持、0 刻意差异、9 未支持**：448–453 的字面下划线串、转义下划线及强调中的字面星号匹配官方；445–447 和 454–459 保留整串字面输出，需要 opener/closer 部分消耗，明确作为未支持。460–481、完整强调与宽泛标准兼容性继续待办。

来源为 [CommonMark 0.31.2 官方数据](https://spec.commonmark.org/0.31.2/spec.json)（John MacFarlane，CC BY-SA 4.0），固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。官方 Markdown/HTML 和空白逐字保留并记录来源行号，生成器只提取固定章节与编号。重新下载后以独立读取器核对七份强调夹具并集 350–459 共 110 例，篡改来源拒绝，全部 32 份官方夹具校验通过。

投影只读取公开段落、显示片段和样式变化；不接收样例编号/预期，不扫描源码分隔符、补标记或推导同类嵌套树。混合样式顺序只由公开先后变化决定，同时开始和交叉结束、链接及其它未评分结构主动拒绝。独立观察和原因与官方预期分离，不用字面输出冒充兼容。

六项新回归覆盖 148 个合法切点、133 个逐字符前缀及每个前缀 EOF 的快照/pending 一致性；源码连续覆盖、逐字符切片与样式、六字节共享预算、零预算/第二游标停步和完成状态；449/452 直接验证转义 payload 原始位置、样式及反斜线和强调语法隐藏，450/453 验证强调中的字面星号，另验证被转义 closer 的 EOF 回退。超过分类窗口的 Unicode 正文对九种不等长组合及两种转义晚闭合逐字符追加，保留块身份并与整段输入比较。还覆盖实体、空白/换行、可见标记、投影敏感性和拒绝边界。专项命令 `nix develop -c cargo nextest run --test commonmark_emphasis_unequal --no-fail-fast`；夹具命令 `python3 scripts/generate_commonmark_emphasis_unequal.py /path/to/spec.json --check`。

核心、依赖、旧测试和已有夹具保持；README/TODO/DONE 仅追加。累计追加 O(n log n)、空间 O(n)、禁止 unsafe 和共享预算契约不变；本次短样例评分不独立证明长输入复杂度，全套仍覆盖既有大输入回归。

最终验收（项目 devShell）：`cargo check --all-targets`、debug/release `cargo nextest run --no-fail-fast` 各 **1396 项**、`cargo test --doc` **75 项**（72 普通 + 3 compile-fail）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、32 份官方夹具校验及 `git diff --check` 全部通过。专项六项首次通过；其余 359 个基线文件 SHA-256 保持，三份文档仅追加。汇总 `/tmp/mdtui3-goal-run/emphasis-unequal-validation.log`；同目录保留 `emphasis-unequal-targeted.log`、`emphasis-unequal-provenance.log`、`emphasis-unequal-debug.log` / `emphasis-unequal-release.log` / `emphasis-unequal-doc.log`、`emphasis-unequal-scope-check.log` 和独立差异 `emphasis-unequal-unit.patch`。


## 2026-11-04 CommonMark 嵌套强调收尾与行内优先级子集评分交付

本单元完成 TODO 中尚未评分的 CommonMark 0.31.2 Emphasis and strong emphasis **460–481 共 22 例**，新增 `scripts/generate_commonmark_emphasis_final.py`、`tests/fixtures/commonmark_emphasis_final.rs` 和 `tests/commonmark_emphasis_final.rs`。评分为 **10 支持、0 刻意差异、12 未支持**：460/462、469–472、478–481 匹配官方；461/463 缺少同类嵌套的公开深度，464–468 缺少分隔符部分消耗，473/474 强调跨越链接标签，475–477 的原始 HTML 在需求范围外。八份强调夹具现精确覆盖 350–481 全部 132 例，合计 **74 支持、15 刻意差异、43 未支持**；此前的剩余样例评分范围在此补齐，完整强调和宽泛标准兼容性仍未完成。

来源为 [CommonMark 0.31.2 官方数据](https://spec.commonmark.org/0.31.2/spec.json)（John MacFarlane，CC BY-SA 4.0），固定 SHA-256 `d431b29d97b6f73e69d547109cf5081578fac931e72afe95639ebe766c1b2a20`。官方 Markdown/HTML 和空白原样保留，生成器只提取固定章节与编号，不运行解析器。重新下载后用独立夹具读取器逐字核对本子集，检查八份夹具编号并集恰为 350–481，来源篡改拒绝且全部 33 份官方夹具校验通过。

投影仅消费公开段落、显示样式、链接对象范围和预算元数据；代码样式加入同一有序样式转换，先有外层强调、后有代码的 478/479 可直接表达。链接前后隐藏包络须一致、可见标签不能丢失外层样式；473/474 明确拒绝投影，并单独锁定全部显示片段、样式、源码/标签/目标区间与直接链接元数据，计为未支持，不合成修复后的 HTML。同时开启多个样式、交叉结束、代码内链接和其它未评分结构主动拒绝；不扫描源码分隔符、补标记、移动强调边界或重建同类嵌套。

六项新回归覆盖 356 个合法切点、334 个逐字符前缀及逐前缀 EOF 的快照/pending 一致性；源码连续覆盖、逐字符切片、样式与链接 ID 映射、六字节共享显示预算、零预算/第二游标停步及完成状态；链接目标/title 通过已有预算解码器与无限预算结果对照。478/479 精确检查代码中的字面星号/下划线与源码样式，480/481 检查自动链接标签/目标中原始分隔符和隐藏尖括号。超过分类窗口的 Unicode 长正文逐字符晚闭合代码、自动链接、三连和同类嵌套，保持块/链接身份。投影另有实体、空白/换行、可见标记敏感性和拒绝边界验证。专项命令 `nix develop -c cargo nextest run --test commonmark_emphasis_final`；夹具命令 `python3 scripts/generate_commonmark_emphasis_final.py /path/to/spec.json --check`。

核心、依赖、旧测试与已有夹具保持，README/TODO/DONE 仅追加；累计追加 O(n log n)、空间 O(n)、禁止 unsafe 和共享预算契约不变。本次评分不单独证明长输入复杂度，全套继续覆盖既有大输入回归。

最终验收（项目 devShell）：`cargo check --all-targets`、debug/release `cargo nextest run --all-targets` 各 **1402 项**、`cargo test --doc` **75 项**（72 普通 + 3 compile-fail）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、33 份官方夹具校验及 `git diff --check` 全部通过。首次编译将新测试错误使用的 Style 位或改为公开 `union` API 后，六项专项及全套通过，官方预期和核心未改。范围核对确认其余 362 个基线文件 SHA-256 保持，三份文档仅追加。汇总 `/tmp/mdtui3-goal-run/emphasis-final-validation.log`；同目录保留 `emphasis-final-targeted-final.log`、`emphasis-final-provenance.log`、`emphasis-final-debug.log` / `emphasis-final-release.log` / `emphasis-final-doc.log`、`emphasis-final-scope-check.log` 和独立差异 `emphasis-final-unit.patch`。

## 2026-11-04 GFM 删除线章节评分交付

本单元推进 TODO「继续扩充标准语法样例覆盖」，完成 GFM 0.29-gfm Strikethrough (extension) **491–493 完整章节三例**，新增 `scripts/generate_gfm_strikethrough.py`、`tests/fixtures/gfm_strikethrough.rs` 和 `tests/gfm_strikethrough.rs`。评分 **2 支持、1 刻意差异、0 未支持**：492 的空行切断候选和 493 的三连波浪号字面行为匹配官方；491 双波浪号已支持，单波浪号按既有仅 `~~` 方言保留字面，整例计为刻意差异。更完整的删除线和宽泛标准兼容性继续待办。

来源为 [GFM 0.29-gfm 官方规范 HTML](https://github.github.com/gfm/#strikethrough-extension-)（GitHub，基于 John MacFarlane 的 CommonMark；CC BY-SA 4.0），完整来源 SHA-256 `b153d814fdfc8624bb6da7449162c1cd707a637f7d1c1b636eb44b9cf63fa220`，与既有表格/任务列表夹具一致。生成器只读取固定章节并解码网页代码块展示标记，逐字保留 Markdown/HTML 与空白，记录 HTML 行号和锚点，不调用核心解析器。重新下载官方来源、独立正则读取器逐字比较 Rust 夹具、章节编号核对与篡改拒绝均通过。

投影仅消费公开段落、显示文本及 STRIKE 位；只省略借用源码的物理段落末尾 LF，保留实体产生的换行与内部空白，按样式变化输出 del，不扫描源码分隔符、补标记或推导同类嵌套。其它块、对象、强调/加粗/代码/数学主动拒绝。独立敏感性测试验证正文、空白、标记变化不会被投影吞掉。

六项新回归覆盖 91 个合法切点、88 个逐字符前缀及逐前缀 EOF 的完整快照/pending 等价；源码连续覆盖、逐字切片与样式映射、六字节显示预算、零预算/第二游标共享耗尽与完成状态；转义波浪号的可见 payload 和隐藏标记原始位置；超过分类窗口的 Unicode 正文在单/双/三连、长度失配及转义组合下逐字符晚闭合，保持块身份。官方三例均为 ASCII；Unicode 和其它变体仅用于专项验证，不混入官方评分。专项命令 `nix develop -c cargo nextest run --test gfm_strikethrough --no-fail-fast`；离线夹具核验命令 `python3 scripts/generate_gfm_strikethrough.py /path/to/gfm.html --check`。

核心、依赖、既有测试/夹具保持，README/TODO/DONE 仅追加。累计追加 O(n log n)、空间 O(n)、禁止 unsafe 和共享预算契约不变；本组三例不单独证明大输入复杂度，全套继续覆盖既有深层/百万字回归。

最终验收（项目 devShell）：`cargo check --all-targets`、debug/release `cargo nextest run --all-targets --no-fail-fast` 各 **1408 项**、`cargo test --doc` **75 项**（72 普通 + 3 compile-fail）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、36 份 CommonMark/GFM 官方夹具校验及 `git diff --check` 全部通过。六项专项首次通过。范围核对确认其余 365 个基线文件 SHA-256 保持，三份既有文档仅追加。汇总 `/tmp/mdtui3-goal-run/strike-validation.log`；同目录保留 `strike-targeted.log`、`strike-provenance.log`、`strike-debug.log` / `strike-release.log` / `strike-doc.log`、`strike-scope-check.log` 与独立差异 `strike-unit.patch`。

## 2026-11-04 GFM 裸自动链接扩展章节评分交付

本单元推进 TODO「继续扩充标准语法样例覆盖」，完成 GFM 0.29-gfm Autolinks (extension) **622–635 完整章节 14 例**离线评分，新增 `scripts/generate_gfm_autolinks_extension.py`、`tests/fixtures/gfm_autolinks_extension.rs` 和 `tests/gfm_autolinks_extension.rs`。评分为 **0 支持、14 刻意差异、0 未支持**：所有样例沿用已有「不处理 GFM 裸 URL 自动链接」方言，不生成裸地址链接对象。`www.`、HTTP(S)、email、`mailto:`/`xmpp:` 以及地址末尾标点、括号、实体边界都未实现 GFM 扩展规则；「刻意差异」只记录已明确的范围选择，不等同功能支持。尖括号自动链接章节保持独立，宽泛标准覆盖仍待办。

来源为 [GFM 0.29-gfm 官方规范 HTML](https://github.github.com/gfm/#autolinks-extension-)（GitHub，基于 John MacFarlane 的 CommonMark；CC BY-SA 4.0）。完整来源 SHA-256 `b153d814fdfc8624bb6da7449162c1cd707a637f7d1c1b636eb44b9cf63fa220` 与既有 GFM 夹具一致；离线生成器只解码网页代码块展示标记，保留每个 Markdown/HTML 字符、空白、HTML 行号和锚点。重新下载官方来源，独立正则读取器与 Rust 夹具逐字比较、章节成员和篡改拒绝核验通过。

测试将当前方言的 HTML 观察作为独立常量保存，与官方 HTML 分离；投影不接收编号或预期，只读取公开段落、显示文本和样式，执行 HTML 输出转义，仅省略借用源码的物理段落终止 LF。不会识别地址、合成链接、trim、解码 Markdown 或改写官方结果。实际链接、容器、其它块与强调/代码/数学等样式主动拒绝；附加敏感性测试保留 Unicode、空格/tab、实体换行和反斜线显示语义。

六项回归覆盖 **945 个合法切点、931 个逐字符前缀及逐前缀 EOF** 的完整快照/pending 等价；源码连续覆盖、逐字切片/样式映射、六字节显示预算、零预算与第二游标共享耗尽、免费完成状态；长 Unicode 裸地址超过分类窗口后的逐字符追加及稳定块身份；裸地址与尖括号 URI/email 的独立对照，核对原始标签范围、显示映射和目标。官方输入均为 ASCII，Unicode 和其它变体只作附加回归，不混入官方评分。专项命令 `nix develop -c cargo nextest run --test gfm_autolinks_extension --no-fail-fast`；来源核验命令 `python3 scripts/generate_gfm_autolinks_extension.py /path/to/gfm.html --check`。

核心、依赖、已有测试/夹具保持，README/TODO/DONE 仅追加；累计追加 O(n log n)、空间 O(n)、禁止 unsafe 与共享预算契约保持。评分测试自身不证明大输入复杂度，全套继续覆盖既有深层/百万字回归。

最终验收（项目 devShell）：`cargo check --all-targets`、debug/release `cargo nextest run --all-targets --no-fail-fast` 各 **1414 项**、`cargo test --doc` **75 项**（72 普通 + 3 compile-fail）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --all -- --check`、37 份 CommonMark/GFM 夹具核验及 `git diff --check` 全部通过。六项专项首次通过。范围核对确认其余 **368 个基线文件 SHA-256 保持**，三份既有文档仅追加。汇总 `/tmp/mdtui3-goal-run/bare-autolinks-validation.log`；同目录保留 `bare-autolinks-targeted.log`、`bare-autolinks-provenance.log`、`bare-autolinks-debug.log` / `bare-autolinks-release.log` / `bare-autolinks-doc.log`、`bare-autolinks-scope-check.log` 和独立差异 `bare-autolinks-unit.patch`。

## 2026-11-04 部分 tab 路径异类标记兄弟列表交付

本轮实现有界分类窗口内、部分 tab 完整祖先路径之后的非空异类标记兄弟列表。标记缩进至多三列且低于旧 Item 正文缩进时，bullet 字符、bullet/ordered 类型或 ordered 分隔符变化会关闭旧 List/Item 及后代，并创建同父新 List/Item；祖先、已发布块及链接身份不变。同类标记继续保留旧 List，达到正文缩进仍走嵌套规则。完整短行上的异类空标记及不完整路径保持原方言，完整容器规则、分类窗口扩展与宽泛标准兼容性仍未完成。

两处只读探测复用已缓存的缩进和 marker 末端，将候选交还既有结构分类器；没有新增持久状态或扫描。匹配消耗显式前缀/正列数，实际关闭按节点数计费，不回访旧正文或保留祖先；累计追加 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 和共享预算契约保持。源码 tab/标记前缀隐藏，任务/正文继续保留原始映射；松紧和关闭/新增通知沿用现有路径。

验收：

- 新增 `crates/mdstream-core/tests/mixed_sibling_tabs.rs` 六项回归，15,660 组独立列宽展开模型覆盖不同标记、缩进、祖先、正文和 LF/CRLF/EOF；另有空行/后代模型、空标记/分隔线/缺失路径/围栏/数学优先级反例。
- 全 UTF-8 切点、逐前缀及每个前缀 EOF、分类窗口边界、祖先/旧块/链接 ID、精确后代关闭与新列表范围、任务/编号/松紧通知通过；原 `sibling_tabs` 仅更新一条本次扩展的异类标记预期，其余断言保持。
- tile 边界、连续源码覆盖、逐字符切片、零预算停步、共享六字节预算耗尽和游标过期通过；千层百万字正文后千次追加不增加分类/容器工作，4096 次列表类型切换的累计操作数受限。
- 专项 12 项、debug/release 各 1420 项、doc 75 项，以及 `cargo check --workspace --all-targets`、clippy `-D warnings`、fmt、37 份官方夹具生成器 `--check`、diff 检查通过。官方夹具、生成器、评分投影和统计未改。

证据保存在 `/tmp/mdtui3-goal-run/mixed-sibling-*`：修复前 `repro.log` 复现异类标记留在正文，`targeted-final.log` 记录专项通过，`debug.log` / `release.log` / `doc.log` 记录全套结果。首轮扩展模型暴露异类空标记边界，收窄为非空准入后通过；身份测试中祖先开放状态的预期改用实际公开契约 `Growing`。`scope-check.log` 与 `unit.patch` 对照本单元开始时基线，三份文档仅追加；完整兼容性待办保持。

## 2026-11-04 部分 tab 路径异类空标记交付

本轮补齐部分 tab 完整祖先路径后的异类空标记。候选缩进至多三列且低于旧 Item 正文缩进时，不再要求异类标记具有非空正文；完整短行交给已有结构分类器，保持空标记不中断段落的方言。已有正文时允许 lazy 续行并保留旧 List/Item，空行后则可关闭旧列表并开启同父空列表，首定义/编号/任务及松紧逻辑没有新增分支。缺失引用、不完整祖先、超出分类窗口的候选和完整容器兼容性继续保留。

修复前 `- parent\n    - first\n\t+\n` 的末行显示原始 tab，与等列宽空格输入不同。去掉两处只读探测的非空限制，并在 lazy 回退保留剩余虚拟列后，末行显示两个空格和 `+`，虚拟空格映射到原始单字节 tab。扩展模型进一步发现已匹配引用祖先的块类型丢失，已用现有局部引用深度恢复。没有新增持久状态、扫描或历史回访；各次路径匹配消费正列数或显式前缀，关闭按实际节点计费，累计追加 O(n log n)、空间 O(n)、零运行时依赖及禁止 unsafe 保持。

新增 `empty_mixed_sibling_tabs.rs` 七项回归：

- 25,056 组独立列宽模型覆盖混合标记、祖先/引用/缩进、空白后缀、空行与 LF/CRLF/EOF；另有 75 组直接复现和后代/优先级边界。
- UTF-8 全切点、逐字符前缀及逐前缀 EOF、窗口边界、游标过期；旧块/链接/祖先 ID、空列表新增和精确关闭范围、编号与 Tight 元数据。
- tab 剩余列、连续源码覆盖、逐字符切片、tile 边界、零预算停步和两个游标共享六字节预算；千层百万字正文追加不回扫历史，4096 次异类空列表切换受操作计数上界约束。

原 `mixed_sibling_tabs` 仅更新两条被本项扩展的空标记回退观察和相邻说明。首轮专项模型发现引用块类型问题并修复；引用祖先模型的空行改为带显式引用前缀，以保证对照双方测试同一完整祖先路径，不把缺失引用后的字面 tab 当作已消费缩进。修复前与中间失败日志保留，官方夹具/生成器/评分统计不变。

最终验收（项目 devShell）：专项 19 项通过；`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1427 项**、`cargo test --doc --workspace` **75 项**、clippy `-D warnings`、fmt、37 份官方夹具 `--check` 和 `git diff --check` 全部通过。范围核验确认 **370 个基线文件 SHA-256 保持**，仅 5 个已有文件和 1 个新测试变化，README/TODO/DONE 仅追加。日志与独立差异保存为 `/tmp/mdtui3-goal-run/empty-mixed-*`：`repro.log`、`targeted-final.log`、`check.log`、`debug.log`、`release.log`、`doc.log`、`clippy.log`、`fmt.log`、`fixtures.log`、`scope-check.log`、`unit.patch`，汇总为 `validation.log`。

## 2026-11-04 完整部分 tab 路径表头与表格正文行交付

完成 TODO 中容器新叶块剩余类型的表头窄子项，并接通同作用域表格正文行：已有完整容器路径消费部分 tab 后，只要现有叶块分类为非 lazy 正文，就沿用普通路径的表头/行识别。确认单元格时裁掉前导空白；失败、未确认 EOF 保留原 tab 的剩余列映射。早管道、长行单遍摘要及可隔离的旧数学/代码/强调/链接候选沿用已有确认/恢复模型，原始源码、容器和已发布对象身份保持。

核心仅修改 `lib.rs` 的一处 `table_text` 准入条件；不增字段、扫描、依赖或 unsafe，也不重扫历史正文、表格行或保留祖先。表头每字节仍只做一次摘要，确认最多按既有规则消费当前单元格，失败一次恢复；追加累计 O(n log n)、空间 O(n)，查询与显示读取继续共享预算。候选确认和 EOF 仍只承诺累计复杂度，不提供单次硬时延。

新增 `tests/partial_table_tabs.rs` 七项回归：1,536 组物理列宽与空格路径对照（含成功、失败和非部分 tab 对照）、短样例全部 UTF-8 切点/逐前缀及 EOF、五种继承候选长表头的成功/失败/EOF 全切点、原始切片/tile 映射与双游标共享六字节预算、稳定 LinkId/TableId/BlockId 与列宽通知。980 层引用加列表的百万字表头/行追加证明正文增长不增加分类或容器遍历，4,096 行历史后只计新增行。七份既有测试仅更新本子项取代的排除断言及关联语义，lazy、不透明候选、Setext、缩进代码等其它边界继续断言。

专项复跑：`nix develop -c cargo nextest run --test partial_table_tabs --test closed_link_headers --test closed_math_headers --test link_suffix_headers --test math_tabs --no-fail-fast`，54 项通过。首次全套发现四处旧的部分 tab 表头排除断言，审查后更新并增加长候选模型验证；新模型还区分了失败分隔行成为待定表头的既有回退，不把该边界扩入本次实现。

剩余：分隔行自身的部分 tab 准入、待定表头下一行的探测、缺失祖先/不完整路径、分类窗口扩展及完整容器/表格兼容性继续待办。失败分隔行若成为新表头，其下一条部分 tab 行仍可能保留原始 tab；四列代码及其它叶块优先级保持。本项不把宽泛标准支持标为完成。

最终验收：debug/release 各 **1,434 项通过**，文档测试 **75 项通过**；`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`git diff --check` 全通过。33 份 CommonMark 夹具离线来源校验通过，GFM 夹具/生成器保持基线哈希不变。证据在 `/tmp/mdtui3-goal-run/partial-table-{validation,debug,release,doc,fixtures,scope-check}.log`，本轮独立差异 `partial-table-unit.patch`；README/TODO/DONE 仅追加，11 个已有文件与 1 个新测试变化，其余 365 个基线文件 SHA-256 保持。


## 2026-11-04 待定表头后部分 tab 普通文本交付

- 范围：已有容器路径完整匹配、剩余缩进至多三列且分类为非空普通文本时，待定表头下一行使用现有部分 tab 路径。短 `-|-`、`:-|-:`、`|-|-|` 可按列数确认；非分隔文本或列数不符回退，保留单字节 tab 的虚拟空格映射，失败行含管道时可继续成为新表头。普通新段落、已确认表格正文和此前叶块规则保持。
- 实现：仅删除 `Containers::partial_prefix` 普通文本分支对待定表头的准入排除，复用现有路径匹配、分隔行确认、候选回退和显示映射。没有新增扫描/状态或旧正文、旧行、保留祖先回访，累计追加和空间上界不变。最初尝试调整调用处的文本资格会干扰四列代码优先级，回归检出后改为上述局部分支；旧代码优先级测试原样通过。
- 验证：新增 `pending_table_tabs.rs` 六项测试，1,152 组独立物理列展开模型（引用/列表混合、六层列表、四种列偏移、成功/失败及不同列数），逐 UTF-8 切点、实时前缀和逐前缀 EOF；确认/回退/空白 EOF 的源码、单字符切片、tile 和共享六字节预算；失败行成为新表头后的对齐、旧链接/容器/表格块身份及通知；980 层引用加列表的百万字表头逐字追加不再增加路径/分类工作，后续短分隔确认保持对象身份。修复前六项中四项失败，最终六项及相关旧测试共 37 项通过。
- 边界：只有普通文本分隔行在此扩展；带新容器标记的 `- | -`、超出分类窗口的部分 tab 长分隔行探测、不完整路径/lazy、空白及其它叶块优先级不在本项扩大。纯 tab 空行仍保留原始拼写；待定表头中尚未发布的短行内对象仍延迟发布。完整容器/表格与宽泛标准兼容性继续待办。
- 四份旧测试仅更新由本项取代的分隔行排除和回退显示断言；运行日志、基线范围核对及本轮独立补丁保存在 `/tmp/mdtui3-goal-run/pending-table-*`。

最终验收：debug/release 各 **1,440 项通过**，文档测试 **75 项通过**；`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` 和 `git diff --check` 通过。8 个已有文件和 1 个新测试变化，README/TODO/DONE 仅追加，其余 369 个基线文件 SHA-256 保持（包含所有官方夹具及生成器）。日志 `pending-table-validation.log`，独立差异 `pending-table-unit.patch`，范围 `pending-table-scope-check.log`，修复前 `pending-table-repro.log`，专项 `pending-table-targeted-final.log`，全套 `pending-table-debug.log` / `pending-table-release.log` / `pending-table-doc.log`，均在 `/tmp/mdtui3-goal-run/`。


## 2026-11-04 部分 tab 路径标记形态短表格分隔行交付

- 范围：完整已有容器路径后的部分 tab 剩余缩进至多三列，短行 `- | -`、`- | -:`、`- | :-` 可在待定表头的确切作用域确认表格，优先于新嵌套列表。无效或列数不符的标记仍沿用结构分类回退；兄弟条目改变 Item 身份，不能确认旧表头。分隔语法及缩进隐藏，已发布的块/容器/链接身份保持。
- 实现：删除 `Containers::line` 部分路径探测中对合法表格分隔行的提前排除，复用下游已有的确切作用域表格优先级与确认路径。无新字段、依赖、unsafe 或扫描；减少一次有界分隔行探测，保留每行新前缀消费和开闭计费，不回访旧正文、旧行或保留祖先。累计追加 O(n log n)、空间 O(n)，不宣称单次 append 或 EOF 的硬时延。
- 验证：新增 `marker_delimiter_tabs.rs` 六项回归；1,536 组物理列展开模型包含混合引用/有序/无序列表、六层列表、四种缩进偏移、合法对齐和无效/列数不符标记。短样例 UTF-8 全切点、逐前缀与 EOF，CRLF 和无换行确认；失败表头成为新表头后的稳定 BlockId/LinkId/容器和通知；原始源码连续覆盖、逐字符切片/tile、零预算停步与双游标共享六字节预算；980 层引用加列表的百万字表头逐字增长不增加容器/分类工作，晚确认计数只受新前缀长度约束。
- 修复前六项中四项失败、两项通过；修复后六项通过，补充边界后与 pending_table_tabs/sibling_tabs 共 18 项通过。旧测试只更新一条被本扩展取代的排除断言，官方夹具、生成器与评分保持。
- 剩余：超出分类窗口的部分 tab 长分隔行、缺失祖先/lazy、空白及其它尚未接通的容器路径继续待办；四列代码和 Setext/兄弟优先级保持。完整容器/表格与宽泛标准兼容性未标完成。

流式专项共 13 个样例、241 个 UTF-8 合法切点、228 个逐前缀及 EOF 对照。首次全套的测试均通过，clippy 指出删除条件后留下的可合并 if；合并等价条件后重新验收，保留首次 clippy 日志 `marker-delimiter-clippy-first.log`。

最终验收（项目 devShell）：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,446 项通过**、`cargo test --doc --workspace` **75 项通过**、clippy `-D warnings`、fmt 和 `git diff --check` 全部通过。5 个已有文件与 1 个新测试变化，其余 **373 个基线文件 SHA-256 保持**，README/TODO/DONE 仅追加。全部官方夹具和生成器保持不变。汇总 `/tmp/mdtui3-goal-run/marker-delimiter-validation.log`，本轮独立补丁 `marker-delimiter-unit.patch`，范围核对 `marker-delimiter-scope-check.log`，修复前 `marker-delimiter-repro.log`，专项 `marker-delimiter-targeted-final.log`，最终全套 `marker-delimiter-debug.log` / `marker-delimiter-release.log` / `marker-delimiter-doc.log`。


## 2026-11-04 完整部分 tab 路径长表格分隔行交付

- 范围：补齐最近短分隔行单元保留的长分隔行探测。原字节边界路径匹配失败后，允许完整已有容器路径消费部分 tab；剩余缩进至多三列，必须保持表头确切作用域。确认支持很长的首列、尾列、对齐冒号和可选外侧管道；四列缩进、缺失祖先、兄弟或新嵌套容器保持排除。原字节边界探测的既有空白方言优先保留，不把这次规则外推到所有缩进形式。
- 实现：只改 `Containers::delimiter_prefix`，追加一次可分割 tab 的只读全路径匹配，检查剩余列并计入既有容器字节/步骤计数。两个路径都只读取当前有界物理前缀；失败可沿原分类回退，不突变活动栈。沿用 `DelimiterCandidate` 的单遍长尾消费、成功确认及失败有界前缀回放/一次尾部消费，拒绝后不重新探测同一行。无新增持久状态、依赖、unsafe 或历史正文/表格行回扫；累计追加 O(n log n)、空间 O(n)，append/finish 仍是摊还界限。
- 验证：新增 `long_delimiter_tabs.rs` 六项测试。960 组独立物理列展开对照包括引用/有序/无序列表、六层列表、四种起始偏移、长首列/尾列/双列、非法字符和列数不符；UTF-8 全切点、逐字符实时前缀，分类窗口附近及尾部 EOF；确认/迟失败/无换行 EOF 的块与容器身份、链接目标、通知和后续表格恢复。源码连续覆盖、逐字符切片/tile、tab 单字节映射、零预算不前进与双游标共享六字节预算全部覆盖。
- 复杂度：980 层引用加列表的百万字候选分 1000 次追加，每次只增加新增候选字节，分类、行内、容器和标题工作计数不变；成功不再走祖先，失败仅回退原前缀并消费延迟正文一次。另验收失败后的 1000 次追加不再增加候选/分类/容器工作。
- 修复前六项中四项失败、两项通过。实现后新测试一项将表头换行预期漏写，经核对改为保留换行；最终六项及相关专项共 43 项通过。三份旧测试只更新被本项取代的长分隔行排除断言和相关命名；官方夹具、生成器及评分统计保持。
- 剩余：缺失祖先/lazy、一般空白及其它分类窗口/容器规则仍按原 TODO 推进；完整容器/表格和宽泛标准兼容性未标完成。

流式专项五个长样例覆盖 5,235 个 UTF-8 合法切点和 5,230 个实时前缀；EOF 对照选择分类窗口两侧及各样例尾部，不宣称逐前缀 EOF 全覆盖。范围核验确认其余 372 个基线文件 SHA-256 保持，7 个已有文件与 1 个新测试变化，README/TODO/DONE 仅追加。

最终验收（项目 devShell）：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,452 项通过**、`cargo test --doc --workspace` **75 项通过**、clippy `-D warnings`、fmt 和 `git diff --check` 全部通过。日志汇总 `/tmp/mdtui3-goal-run/long-tab-validation.log`，独立差异 `long-tab-unit.patch`，范围核对 `long-tab-scope-check.log`，修复前 `long-tab-repro.log`，专项 `long-tab-targeted-final.log`（43 项通过），全套 `long-tab-debug.log` / `long-tab-release.log` / `long-tab-doc.log`。


## 2026-11-04 完整部分 tab 路径短空白行交付

- 范围：补齐完整部分 tab 路径上的有界完整空白叶块。例如 `- > a\n\t> \n\t> b\n` 中间行现在是相同引用作用域的 Blank，而非 lazy 普通段落；保留原容器/块/链接身份。全部祖先匹配后才准入，尚未收尾/超出分类窗口空白、缺失祖先和活动围栏仍沿用原路径。
- 实现：`Containers::partial_prefix` 的已有分类结果允许 `Line::Blank`，复用原有 `observe_payload`、块分类、表头回退和源码映射。引用内部空行不直接使外层列表变松；直接 Item 空行只在后续正文确认分隔时发出一次 loosen 通知。未增加扫描、持久状态或历史正文/祖先回访，累计追加 O(n log n)、空间 O(n) 和预算约束不变。
- 验收：`tests/partial_blank_tabs.rs` 七项新回归，1,728 组独立物理列模型覆盖引用前缀、有序/无序多层列表、内层引用/条目和尾部空格；比较显示、块种类/作用域、容器身份/父链/深度及列表松紧。九个样例覆盖 UTF-8 全切点、逐字符实时前缀及每个前缀 EOF，包括 CRLF、未完成空白、缺失引用、空条目和代码/数学围栏。
- 映射与预算：源码连续覆盖、逐字符切片、跨 tile 显示一致；部分 tab 的两列替换归属一个原始源码字节。零预算不前进，双游标共享六字节预算，输出与完整显示相同。
- 复杂度：970 层外部引用加 List/Item/内层 Quote 保留空行作用域；后续百万字分 1000 次追加，容器/分类工作计数不增加，行内消费恰好增加新增字节数。4,096 轮空行/正文后再追加空行只消耗固定容器工作，不遍历历史叶块。原复现测试修复前失败（Paragraph 与 Blank 不符），修复后六项专项首次全部通过。
- 剩余：缺失祖先/lazy、尚未收尾/长空白及其它分类窗口/容器规则继续按原 TODO 推进；完整容器、表格和宽泛标准兼容性未标完成。依赖、官方夹具/生成器保持；九份旧测试只更新被本项取代的空白 tab 映射断言，引用空白读取器对替换仍核验原 tab 单字节归属和独立物理列数。

补充边界测试锁定：未收尾短空白仍在 pending 中、finish 后才准入；缺失引用关闭原 Quote；首次新建引用保留原空白规则；超出分类窗口的长空白不重新解释为 Blank。九个短样例合计 145 个切点、136 个实时前缀及逐前缀 EOF。首次 debug 全套 11 项失败均为本扩展替代的原始空白映射断言，逐条更新后相关 79 项通过，补充边界后新专项 7 项通过；补充测试首次误把 pending Span 当字符串比较，改为公开 text(span) 后编译通过。首次日志保留为 `partial-blank-debug-first.log`。

最终验收（项目 devShell）：`cargo check --all-targets`、debug/release `cargo nextest run --no-fail-fast` 各 **1,459 项通过**、`cargo test --doc` **75 项通过**、clippy `-D warnings`、fmt 和 `git diff --check` 全部通过。13 个已有文件与 1 个新测试变化，其余 **367 个基线文件 SHA-256 保持**，README/TODO/DONE 仅追加。汇总日志 `/tmp/mdtui3-goal-run/partial-blank-validation.log`，独立差异 `partial-blank-unit.patch`，范围核对 `partial-blank-scope-check.log`，修复前 `partial-blank-repro.log`，专项 `partial-blank-targeted.log` / `partial-blank-targeted-final.log`。


## 2026-11-04 新引用 tab 分隔短空白行交付

- 范围：新引用标记自身的 tab 分隔现在支持完整短空白行。`>\t\n` 从原始 tab 显示改为两个空格加换行，保留原 Quote/Blank 结构；只消费一列分隔，其余列归属于同一 tab 源码字节。尾部其它 space/tab 保持原始显示，空白内容不受三列叶块缩进上限限制。
- 实现：`Prefix::quote_leaf` 使用既有 `spaces()` 缓存的空白末端，完整且余下全为空白时接受部分 tab；后续 Blank 分类、通知、列表松紧及显示映射均沿用原路径。无新增状态、扫描、依赖或 unsafe，不回访历史正文/祖先，累计追加 O(n log n)、空间 O(n) 和共享预算保持。
- 验收：新 `new_quote_blank_tabs.rs` 七项测试，包括 1,344 组独立物理列模型、UTF-8 全切点/逐字符前缀及逐前缀 EOF、稳定块/链接/容器身份、列表松紧和待定表头边界；源码完整覆盖、单字符切片/tile 显示一致、原始 tab 单字节替换、零预算不前进及双游标共享六字节预算。深层与百万字后续追加验证不重访容器路径，4,096 轮历史后新空行只做固定工作。
- 边界：未收尾短行仍为 pending，EOF 可确认；分类窗口外长空白和活动代码/数学围栏沿用原规则。不完整路径、其它新叶块/窗口扩展及完整容器/表格和宽泛兼容性继续待办。
- 修复前专项 4 失败/3 通过，修复后七项通过。首次编译把 `block_at` 返回的 BlockId 当成带 id 字段的对象，已纠正为直接比较公开 ID；未改核心接口。

补充验证：10 个流式样例覆盖 120 个合法切点与 110 个逐字符前缀/EOF；连续 1–100 层 `>\t` 新引用验证线性工作计数，968 层外引用加最终新引用的样例实际消费部分 tab。首次全套 3 项旧方言断言失败，审查后只更新 `partial_blank_tabs`、`quote_prose_tabs`、`quote_tab_code` 的对应预期，相关专项共 31 项通过。其余标准样例、官方原文和评分不变。

最终验收（项目 devShell）：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,466 项通过**、`cargo test --doc --workspace` **75 项通过**、clippy `-D warnings`、fmt 和 `git diff --check` 全部通过。7 个已有文件与 1 个新测试变化，其余 **374 个基线文件 SHA-256 保持**，README/TODO/DONE 仅追加。汇总 `/tmp/mdtui3-goal-run/new-quote-blank-validation.log`，独立差异 `new-quote-blank-unit.patch`，范围核对 `new-quote-blank-scope-check.log`，修复前 `new-quote-blank-repro.log`，专项 `new-quote-blank-targeted-final.log`（31 项通过），首次全套 `new-quote-blank-debug-first.log` 与最终 debug/release/doc 日志均在同目录。


## 2026-11-04 有序列表段落中断编号交付

- 范围：同段落、同确切作用域的新有序列表只有数值 1 可以中断。`text\n14. body` 保留编号为正文；顶层、显式引用/条目及完整部分 tab 路径一致。前导零按数值处理；合法编号仍限 1–9 位。空标记中断规则保持。
- 边界：同类兄弟条目已先替换 Item，不受新列表限制；新 Quote/Item、新段落、空行或标题之后仍可从任意合法编号开始。缺失祖先的 lazy 中断行为保持，例如 `> text\n14. body` 仍会退出引用并创建列表。其它分类窗口和完整容器匹配规则继续待办。
- 实现：仅扩展 `Containers::line` 的已有段落中断门槛，使用 `Marker.number` 与现有段落/容器身份。每个已解析标记增加常量比较，无新增扫描、持久状态、依赖或 unsafe，不回访旧正文或祖先；累计追加 O(n log n)、空间 O(n)，append/finish 保持摊还界限。
- 验证：新增 `ordered_interrupt.rs` 六项测试，756 组编号/点号或右括号/空格与 tab/换行与 EOF/作用域模型；逐切点、逐字符实时前缀及前缀 EOF 对照；保留块/容器/链接身份、跨行强调、待定表头失败回退；源码完整覆盖、逐字符切片/tile、tab 单字节映射、零预算不前进及双游标共享六字节预算。970 层引用中百万字分 1000 次追加不增加分类/容器工作，行内消费恰为新增字节；4096 行历史后新行容器工作保持固定。
- 评分：CommonMark 304 匹配官方结果，Lists 301–326 更新为 23 支持、3 刻意差异、0 未支持。只更新独立分类与统计，官方输入/HTML、生成器和投影不变；固定 SHA-256 来源的夹具校验通过。原空标记专项的一条任意编号中断断言随本项修正。
- 修复前专项 2 通过/4 失败，修复后六项全部通过。原有宽泛 TODO 保持未完成。

补充验收：10 个流式样例共 186 个 UTF-8 合法切点、176 个实时前缀及逐前缀 EOF。首次 debug 全套 3 项失败来自被本规则取代的嵌套起始假设；缩进模型增加“非 1 嵌套标记保留为正文”的分支，两项兄弟替换测试以 `1)` 建立待替换的子列表，仍验证旧块/链接/父容器身份和关闭范围。相关专项 37 项全部通过。首次全套日志 `ordered-interrupt-debug-first.log` / `ordered-interrupt-release-first.log` 保留；修正测试时一次辅助读取漏传 Span 参数，补齐后编译和专项通过。

最终验收（项目 devShell）：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,472 项通过**、`cargo test --doc --workspace` **75 项通过**、clippy `-D warnings`、fmt、固定 Lists 夹具校验及 `git diff --check` 全部通过。9 个已有文件与 1 个新测试变化，其余 **373 个基线文件 SHA-256 保持**，README/TODO/DONE 仅追加。汇总 `/tmp/mdtui3-goal-run/ordered-interrupt-validation.log`，独立差异 `ordered-interrupt-unit.patch`，范围核对 `ordered-interrupt-scope-check.log`，修复前 `ordered-interrupt-repro.log`，专项 `ordered-interrupt-targeted-final.log`（37 项通过），最终全套 `ordered-interrupt-debug.log` / `ordered-interrupt-release.log` / `ordered-interrupt-doc.log`。


## 2026-11-04 lazy 有序标记段落续行交付

- 范围：已有最内层非空段落发生祖先前缀失配时，合法非 1 有序标记不再中断 lazy 续行；编号、分隔符及正文保持原作用域并参与原行内解析。例如 `> text\n14. body` 保留引用，`> > text\n> 2) body` 保留内层引用，`> - text\n14. body` 在省略外层引用时保留原条目。
- 实现与复杂度：`Prefix::lazy_text` 复用 `marker()` 返回的编号，仅增加常量比较。相同类型兄弟已在匹配活动路径时处理；异类兄弟候选用一个当前行布尔值传递给 lazy 判定，保持原切换优先级；编号 1、无序标记、空行与非段落块边界保持。无新增扫描、持久状态、依赖或 unsafe，不重访旧正文或保留祖先；累计追加 O(n log n)、空间 O(n) 和共享读取预算保持。
- 验收：新增 `lazy_ordered_interrupt.rs` 七项回归；756 组缺失或部分祖先路径、编号/分隔符/空格或 tab/换行或 EOF 模型；UTF-8 全切点、逐字符实时前缀及逐前缀 EOF；稳定块/容器/链接身份、跨行强调、表头失败回退；源码覆盖、逐字符切片/tile、原始 tab 映射、零预算不前进与双游标共享六字节预算。窗口外长空白在编号确定后不重新判定；1/100/970 层引用各 1000 条短 lazy 行验证工作量不随缺失祖先深度增长；970 层百万字分 1000 次追加不增加分类或容器工作，4096 行历史后新行工作固定。
- 边界：规则仅在已有 lazy 段落资格成立时应用，不给空引用、标题、围栏、定义、已确认表格或空行后的内容增加资格。完整路径沿用上一项实现；不完整 tab 路径仍显示原始 tab，未扩展虚拟列表示。合法编号仍最多九位，前导零按数值判断；空标记与分类窗口方言保持。完整列表/容器及宽泛标准兼容性仍待办。
- 修复前六项测试 5 失败/1 通过。修复后审查新测试的 tab 映射预期：不完整路径沿用原始 tab，而非完整路径的虚拟列替换；修正该断言后新增七项与相关原测试共 27 项通过。旧 `ordered_interrupt` 的缺失引用断言改为保留原作用域，`container_lazy` 的中断样例从 `2.` 改用仍会中断的 `1.`。首次全套 6 项失败暴露异类兄弟切换被错误当成 lazy 的范围问题；补充当前行兄弟标志后，这六项原断言及官方样例保持不变，相关专项 64 项全部通过。

补充分块验收：12 个流式样例覆盖 246 个 UTF-8 合法切点、234 个实时前缀及逐前缀 EOF。首次全套日志保存在 `lazy-ordered-debug-first.log` / `lazy-ordered-release-first.log`，最终验收重新运行于修正后的实现。

最终验收（项目 devShell）：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,479 项通过**、`cargo test --doc --workspace` **75 项通过**、clippy `-D warnings`、fmt、固定 Lists 夹具校验和 `git diff --check` 全部通过。6 个已有文件与 1 个新测试变化，其余 **377 个基线文件 SHA-256 保持**，README/TODO/DONE 仅追加；核心依赖、官方夹具和评分保持。汇总 `/tmp/mdtui3-goal-run/lazy-ordered-validation.log`，独立差异 `lazy-ordered-unit.patch`，范围核对 `lazy-ordered-scope-check.log`，修复前 `lazy-ordered-repro.log`，专项 `lazy-ordered-targeted-final.log`（64 项通过），最终全套 `lazy-ordered-debug.log` / `lazy-ordered-release.log` / `lazy-ordered-doc.log`。


## 2026-11-04 缺失引用前缀的部分 tab 正文映射交付

- 范围：`> > first\n>\ttext` 和 `- > first\n\ttext` 的部分 tab 已匹配前缀不再因后续缺失引用而丢弃；显示剩余两个空格，保持原最内层容器、块/链接身份和跨行强调、代码、链接标签语义。支持不同 tab stop、混合列表/引用、非 1 有序标记、LF/CRLF 与 EOF。
- 实现：`Containers::partial_prefix` 在首次缺失引用处复用既有 `Prefix::lazy_text`，仅已有非空正文且此前确实拆分过 tab 时接受；返回 lazy 标志和已消费前缀的虚拟列，后续使用已有行内/显示映射。额外判定只扫描当前有界剩余正文，不访问省略的祖先或历史正文；无新增持久状态、依赖或 unsafe，累计追加 O(n log n)、空间 O(n) 与共享预算保持。
- 边界：缺失列表缩进、待定表头、非段落、空白及 lazy 判定拒绝的标题/围栏/定义/中断标记等继续走原分类器，保留其原有结构与显示方言；lazy 正文不会启动表格。短行未收尾时仍 pending，长行按原分类窗口消费。完整容器/表格及其它分类窗口规则仍待办。
- 验收：新增 `lazy_quote_tabs.rs` 七项回归，1,440 组独立物理列空格模型；UTF-8 全切点、逐字符实时前缀及逐前缀 EOF；稳定身份/通知、跨行行内语义和表格排除；原源码覆盖、单字符切片/tile 与原 tab 单字节替换、零预算不前进及双游标共享六字节预算。1/100/970 层缺失引用各 1000 条短行只计两帧路径工作；各百万字后续追加不增加分类/容器工作；4096 行历史后新行仍只计两帧。
- 修复前七项 5 失败/2 通过，其中一项新反例误以为 tab 后标题会退出引用，独立读取基线后修正为锁定原方言；其余失败复现映射及路径工作缺口。修复后七项通过，旧 `prose_tabs` 与 `lazy_ordered_interrupt` 中被本项替代的原始 tab 显示断言按新映射更新。

补充分块与回归验收：13 个流式样例覆盖 250 个 UTF-8 合法切点、237 个实时前缀及逐前缀 EOF。首次全套 1,482 通过/4 失败，均为本项取代的原始 tab 预期；审查后仅更新 `empty_interrupt`、`item_tab_code`、`math_tabs` 与 `new_paragraph_tabs` 的相应映射断言。行内数学额外锁定原始内容保留 tab、语义 payload 返回两空格；额外未消费的 tab 仍原样保留。相关专项共 55 项通过，首次日志 `lazy-tab-debug-first.log` 与修复前 `lazy-tab-repro.log` 保留。

最终验收（项目 devShell）：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,486 项通过**、`cargo test --doc --workspace` **75 项通过**、clippy `-D warnings`、fmt、固定 Tabs 夹具及 `git diff --check` 全部通过。10 个已有文件与 1 个新测试变化，其余 **374 个基线文件 SHA-256 保持**，README/TODO/DONE 仅追加；依赖、官方夹具和评分不变。汇总 `/tmp/mdtui3-goal-run/lazy-tab-validation.log`，独立差异 `lazy-tab-unit.patch`，范围核对 `lazy-tab-scope-check.log`，专项 `lazy-tab-targeted-final.log`（55 项通过），最终全套 `lazy-tab-debug.log` / `lazy-tab-release.log` / `lazy-tab-doc.log`。


## 2026-11-04 缺失列表缩进的部分 tab 正文映射交付

- 范围：已有非空段落的部分 tab 前缀可在首次缺失列表缩进处作为 lazy 正文提交；`-  - first\n\ttext` 保留一个虚拟空格，`>  - first\n>\ttext` 保留两个，`- - - first\n\ttext` 中已消费完的 tab 隐藏。原最内层身份、跨行强调/代码/链接/数学与原始源码读取保持，未消费的后续 tab 不改写。
- 实现与复杂度：`Containers::partial_prefix` 移除仅允许缺失 Quote 的限制，复用当前有界剩余正文的 `Prefix::lazy_text`。同类/异类兄弟标记已在前面的分支交给结构分类器；空行后结束的空条目沿用既有分支。探测在首次失配处停止，不访问省略祖先或历史正文，无新持久状态、依赖或 unsafe；累计追加 O(n log n)、空间 O(n) 与共享读取预算保持，不承诺单次 append/finish 硬时延。
- 边界：待定表头、非段落、空白和被现有 lazy 判定拒绝的候选走原分类器，保留其既有身份与显示方言；这不意味着这些候选一定关闭旧容器。lazy 行不启动表格。短行等待完整换行或 EOF，长行沿用分类窗口；完整容器/表格和其它分类窗口规则仍待办。
- 验收：新增 `lazy_item_tabs.rs` 七项回归；1,728 组独立物理列展开模型覆盖祖先引用、不同外层列表缩进、不同省略深度、Unicode/行内标记和 LF/CRLF/EOF。13 个流式样例覆盖 262 个 UTF-8 切点、249 个实时前缀及逐前缀 EOF。稳定块/容器/链接身份和通知、跨行行内语义与表格排除；源码完整覆盖、单字符切片/tile、原 tab 单字节替换、零预算不前进及双游标共享六字节预算均通过。1/100/480 层缺失列表（最深 962 个 List/Item 容器）各 1000 条短行只计两帧；各百万字后续追加不增加分类/容器工作；4,096 行历史后新行仍只计两帧。
- 修复前复验：定稿七项在原实现上 5 失败/2 通过，失败复现显示映射和路径工作缺口；边界与分块测试通过。初版新测试对待定表头及 tab 后标题的退出身份假设不准确，按原分类器改为锁定不新增替换映射；定稿已在修复前实现上复核。修复后七项全部通过。日志 `lazy-item-repro-final.log`、`lazy-item-targeted-final.log` 保存在 `/tmp/mdtui3-goal-run/`。

全套回归审查：首轮最终为 1,488 通过/5 失败，均为新范围替代的旧显示或工作量断言。仅更新 `lazy_quote_tabs` 的缺失 Item 映射、`mixed_sibling_tabs` / `empty_mixed_sibling_tabs` / `sibling_tabs` 的畸形标记正文映射，以及 `prose_tabs` 的路径计数（每行两次匹配加首次失配，共三帧，不再重复回退遍历）。相关 38 项回归通过；第二轮仍有一项遗漏的 `sibling_tabs` 原始 tab 断言，补齐后再做最终全套验收。首次和第二次日志分别为 `lazy-item-debug-first.log` / `lazy-item-debug-second.log`；没有修改其它结构优先级或官方样例评分。

最终验收（项目 devShell）：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,493 项通过**、`cargo test --doc --workspace` **75 项通过**、clippy `-D warnings`、fmt、固定 Tabs 夹具与 `git diff --check` 全部通过。9 个已有文件与 1 个新测试变化，其余 **376 个基线文件 SHA-256 保持**；README/TODO/DONE 仅追加，依赖、官方夹具和评分保持。汇总 `/tmp/mdtui3-goal-run/lazy-item-validation.log`，独立差异 `lazy-item-unit.patch`，范围核对 `lazy-item-scope-check.log`，最终全套 `lazy-item-debug.log` / `lazy-item-release.log` / `lazy-item-doc.log`。


## 2026-11-04 待定表头失败后的部分 tab lazy 映射交付

- 范围：待定表头之后，部分 tab 已消费前缀在首次缺失引用或列表缩进处可按既有 lazy 条件保留；`- > a|b\n\ttext` 的续行显示两个空格，`-  - a|b\n\ttext` 显示一个空格，原字节读取、最内层容器、块/链接身份及跨行强调保持。候选表头只回退一次，lazy 行既不能确认表格也不能开启新表头。
- 实现与复杂度：`Containers::partial_prefix` 使用既有同作用域 `paragraph` 资格接纳待定表头，复用 `Prefix::lazy_text` 和显示映射；仅修改一处布尔条件，不改变完整路径上的代码准入。匹配在首次失配处停止，不遍历省略祖先，不新增扫描、持久状态、依赖或 unsafe；累计追加 O(n log n)、空间 O(n) 与共享读取预算保持，单次 append/finish 仍无硬时延承诺。
- 边界：同类/异类兄弟及编号标记优先级保持；完整路径仍可确认表格，四列缩进仍开启代码。空白、无活动段落及 lazy 判定拒绝的标题/围栏/定义等候选保持原分类器方言。未收尾短行等待换行或 EOF，长行沿用分类窗口；完整容器/表格兼容性继续待办。
- 验收：新增 `header_lazy_tabs.rs` 六项回归；2,160 组独立物理列模型覆盖引用祖先、列表缩进、缺失 Quote/Item/混合后缀、Unicode 和 LF/CRLF/EOF；12 个流式样例覆盖 206 个 UTF-8 切点、194 个实时前缀及逐前缀 EOF。块/容器/链接身份、通知与跨行强调、表格拒绝、原 tab 单字节映射、源码完整覆盖、单字符切片/tile、零预算不前进和双游标共享六字节预算通过。1/100/480 层缺失列表的首次探测只计两帧；百万字后续追加不增加分类/容器工作；4,096 轮表头失败历史后新行仍只计两帧。
- 失败复现：修复前六项 5 失败/1 通过，失败覆盖显示映射和额外路径工作；初版新测试误把非 1 有序兄弟标记当作 lazy 正文，按既有兄弟优先级改为正文样本，并在边界测试保留该编号的兄弟身份断言。修复前日志 `header-lazy-repro.log` 和修复后专项 `header-lazy-targeted-final.log` 保存在 `/tmp/mdtui3-goal-run/`。

全套回归审查：首轮 1,497 通过/2 失败，仅 `lazy_item_tabs` 与 `lazy_quote_tabs` 原先排除待定表头映射的断言被本项替代；将这两例更新为明确的虚拟空格预期，保留其它非段落边界断言。相关专项 26 项全部通过，首次全套日志 `header-lazy-debug-first.log` 与专项 `header-lazy-related.log` 保留。

最终验收（项目 devShell）：`cargo check --workspace --all-targets`、debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,499 项通过**、`cargo test --doc --workspace` **75 项通过**、clippy `-D warnings`、fmt、固定 Tabs 夹具与 `git diff --check` 全部通过。6 个已有文件与 1 个新测试变化，其余 **380 个基线文件 SHA-256 保持**；README/TODO/DONE 仅追加，依赖、官方夹具和评分保持。汇总 `/tmp/mdtui3-goal-run/header-lazy-validation.log`，独立差异 `header-lazy-unit.patch`，范围核对 `header-lazy-scope-check.log`，最终全套 `header-lazy-debug.log` / `header-lazy-release.log` / `header-lazy-doc.log`。


## 2026-11-04 引用链接定义段落边界交付

完成 TODO 中引用定义段落中断缺口的同作用域子项。`block::classify_indented` 复用已有段落资格，只有非段落位置才能分类成引用链接定义；连续正文及待定表头中的候选按现有行内规则消费。首行、空行后、新容器/兄弟条目、连续定义及已确认表格仍可建立定义；缺失祖先/lazy 与脚注中断方言保持。无新增扫描或持久状态，不访问旧正文、旧引用或祖先；原累计追加 O(n log n)、空间 O(n) 和预算契约不变。

新增 `tests/definition_interrupt.rs` 六项回归：576 组独立作用域/空行/缩进/行尾模型，216 个 UTF-8 切点、206 个逐字符前缀及每个前缀 EOF，原始/显示/标量切片/tile 映射、零预算与共享六字节恢复，待定表头、部分 tab、晚闭合强调与晚定义通知/身份，970 层引用及百万字追加、4096 行历史的工作计数。段落折叠旧测试增加定义形态正文的开放增长/空行封闭断言；旧定义测试按新资格补齐段落边界，保留原元数据、全局首定义、预算及工作量约束。

CommonMark 213 已不再建立定义，但未定义引用隐藏括号，继续归刻意差异；217 的跨行 title 未支持回退为段落，后续 `baz` 定义也因新边界规则不生效，继续归未支持。两项独立观察与说明已审查，官方夹具、生成器和评分投影保持；章节仍为 14 支持/7 刻意差异/6 未支持。更完整定义、缺失祖先/lazy、字面回退及完整容器/宽泛兼容性不标完成。

验收记录：修复前新六项 5 失败/1 通过；实现后新六项通过。首次全套的 21 项失败定位为旧中断假设，逐项更新测试边界/断言；相关测试随后暴露 217 的连带观察变化，审查后专项 59 项通过。最终全套结果与命令见 `/tmp/mdtui3-goal-run/definition-interrupt-validation.log`；首次全套、相关测试和最终专项日志同目录保留，前缀为 `definition-interrupt-`。

最终验收通过：debug/release 各 1505 项、doc 75 项、`container_links` release 示例及 check/clippy/fmt/引用定义官方夹具/diff 检查。README 旧可执行示例与对应 example 补一个空行边界；README 除此一行修正外仅追加，TODO/DONE 仅追加。首次 doc 74 通过/1 失败的记录保留于 `definition-interrupt-doc-first.log`，修正后全部通过。独立差异 `definition-interrupt-unit.patch` 与范围校验 `definition-interrupt-scope-check.log` 同在 `/tmp/mdtui3-goal-run/`：18 个已有文件和 1 个新回归文件变化，其余 369 个基线文件 SHA-256 保持。


## 2026-11-10 缺失祖先/lazy 的引用定义段落边界交付

完成文末同作用域定义边界交付留下的有界 lazy 子项。最小反例 `> seed\n[r]: /url\n` 原先关闭 Quote 并注册定义；已有非空段落省略部分或全部祖先前缀时，现在将候选作为原作用域正文消费，待定表头回退为正文，lazy 行仍不确认或新建表格。部分 tab 路径复用已消费前缀和原 tab 单字节的虚拟列映射。

模型与局部证明：`Containers::line` 的 lazy 分支要求已有最内层段落；`partial_prefix` 的失配探测要求已有正文或同作用域段落/表头资格。两者在首次失配处共用 `Prefix::lazy_text`，旧实现只放行标签无效的 Definition 候选，有效候选错误落入关闭路径。现在放行该分类结果，由既有 `commit_line` 的 lazy → Text 分支阻止定义注册；保留分类器的非段落模式，避免顺带改变 Setext/分隔线判定。标记、空白、脚注、围栏等其它分支和调用资格保持；无活动段落、空行后、新作用域、连续定义及已确认表格仍可建立真正定义。改动不增加扫描或持久状态，移除原标签复查，不访问旧正文、旧引用或缺失祖先；已有累计解析 O(n log n)、空间 O(n)、零运行时依赖、禁止 unsafe 与共享预算契约保持。

复用 `tests/definition_interrupt.rs` 的解析、显示、snapshot 和预算测试设施，新增三项并扩展原有测试：2,592 组独立省略作用域/缩进/行尾模型及部分 tab 反例，17 个流式样本的 375 个合法 UTF-8 切点、390 个字节切点、逐字节输入、358 个逐字符前缀及各前缀 EOF；原始/标量切片/tile 映射、零预算暂停及两个显示游标共享六字节预算；稳定物理块、容器及 LinkId/LinkInfo、晚闭合强调和真正晚定义通知。970 层引用、481 层列表中的百万字追加不增加分类/容器扫描计数，缺失全部前缀的定义候选容器步数最多 2；4,096 轮候选/表头历史后仍验证固定增量和累计工作上界。

先运行反例再改实现：修复前专项 9 项中 5 通过/4 失败；修复后更新四处旧定义中断/tab 映射预期，相关 49 项通过。完整 debug 首轮 1,505 通过/3 失败，均是章节/折叠测试用定义行触发退出的旧假设；补显式容器空行保留原退出、通知与折叠计数断言，相关 75 项全部通过。官方 Markdown/HTML、生成器、评分投影及独立观察均未修改，引用定义章节仍为 14 支持/7 刻意差异/6 未支持。

本轮本地验证均在项目 devShell 完成：`nix develop -c cargo check --all-targets`；`cargo nextest run --workspace --all-targets --no-fail-fast` 与追加 `--release` 各 **1,508 项通过**；`cargo test --doc --workspace` **75 项通过**；`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、`container_lazy`/`container_links` release 示例、固定引用定义夹具核验及 `git diff --check` 全部通过。target 路径由 devShell 内 `cargo metadata --no-deps --format-version 1` 获取。日志在 `/tmp/mdtui3-lazy-definition-{before,targeted-first,targeted-final,debug-first,debug,release,doc}.log`，仅作临时执行证据；持久依据为本记录与测试。README/TODO/DONE 仅追加；未提交或推送，留父会话验收。

本项只修复既有非空段落资格与有界单行定义候选的衔接，不代表完整容器/CommonMark 支持。多行目标/title/标签、未定义引用完整字面回退、分类窗口扩展及其它完整容器/表格规则继续保留 TODO；既有空行、新作用域、已确认表格和脚注等优先级与方言保持。


## 2026-11-10 引用标签完整 Unicode case folding 交付

仅完成 TODO“引用标签匹配样例暴露的剩余范围”中的 CommonMark 0.31.2 例 540 完整 case folding 子项。先核验 ark/Linux、工作目录与 main 领先 origin/main 一个原有提交，保存 `/tmp/mdtui3-casefold-baseline.diff`；读取规划、README、`references.rs`、官方评分/夹具和 rust skill 后固定语义。先新增两项反例，旧实现均失败：540 输出未带链接的 ẞ；扩展等价标签分配了不同符号。再运行独立数据局部模型验证与展开界限，随后实现；修复前证据为 `/tmp/mdtui3-casefold-red.log`，模型证据为 `mdtui3-casefold-model.log`，这些临时日志不替代仓库内回归测试。

从官方主源 `https://www.unicode.org/Public/17.0.0/ucd/CaseFolding.txt` 取得固定 Unicode 17.0.0 原始数据，完整保存于 `data/unicode/17.0.0/CaseFolding.txt`；从 `https://www.unicode.org/license.txt` 保存完整 Unicode License V3。`data/unicode/README.md` 记录来源、取得日期、许可与离线操作，生成器固定两份文件 SHA-256。`scripts/generate_case_folding.py` 仅使用 Python 标准库和本地文件，静态表包含全部 1585 个 C/F 映射，排除 S 和 Turkic T，未列出的标量原值保留；不调用宿主 Unicode 大小写数据库、构建钩子或隐式联网。数据、许可和生成表的篡改拒绝已验证。

核心只替换 `References::intern` 内的小写转换，链接、图片与定义复用同一入口，脚注独立且继续区分大小写。原 `char::is_whitespace` 空白折叠保留；固定 Unicode 版本针对 case folding。支持 ß/ẞ→ss、ﬃ→ffi、三标量希腊文展开、final sigma 和 Cherokee 等完整记录；I→i、İ→i+组合点、ı 保持原值。先检查原始标签最多 999 字节，展开键可超过此限；原始标签、目标/title 源码映射、稳定符号/链接 ID、全局首定义及晚定义通知保持。原始 CRLF/空白计入限额，容器前缀不计入。不增加标签转义/实体解码、NFKC/重音或其它规范化、引用失败回退、多行定义或完整容器规则。

数据模型证明每标量最多输出 3 个标量、6 个 UTF-8 字节，键总字节数至多原标签的 3 倍；固定 1585 项表每个非空白标量最多比较 12 次。新增 `Work.label_scalars`、`label_fold_steps`、`label_key_bytes` 统计实际标量访问、比较与输出，原 `label_bytes` 仍统计原始字节。规范化 O(label bytes)，有界键 BTreeMap 查找/登记 O(log S)，文档整体累计 O(n log n)、空间 O(n)。定义仍只更新当前符号及 first_use 范围，未改历史引用处理路径；零运行时依赖、禁止 unsafe，工具链和主机/用户配置未改。

新增 11 项回归：`references.rs` 的两项单元验证全部 1,112,064 个有效 Unicode 标量（含未列出的恒等映射）及每字节/标量工作上界、超限前零 folding 工作；`tests/reference_case_folding.rs` 八项复用原 snapshot/metadata 支持，独立读取官方数据并采用不同的空白/映射模型，全文覆盖 1585 映射、6360 引用对象及同键首定义。覆盖非 Turkic、不同规范化形式、原始 998/999/1000 字节及收缩/展开、CRLF/Quote 去前缀、所有字节切点和逐字节/逐标量前缀/EOF、原始区间/切片、首定义与晚定义单次通知、稳定身份、980 层/百万字/两万旧引用工作计数。既有标准共享显示/目标/title 六字节预算测试追加 Unicode 链接与图片数据；预算/原始元数据断言保留。

标准评分逐项审查：只将 540 从刻意差异改为支持，539–552 汇总从 6/7/1 变为 **7 支持、6 刻意差异、1 未支持**。539/544/549–552 仍支持；541 仍未支持；542/543/545/546/547/548 的独立观察与刻意差异保持。同步修正两份评分的 `[SS]`/`[ẞ]` 辅助期望，以及晚定义的 ss 键和 /casefold 目标断言；官方 Markdown/HTML、夹具生成器与评分投影均未改。引用形式仍为 16/2/1、引用定义仍为 14/7/6、Images 仍为 19/1/2。本段及 README 新节取代旧“仅 Unicode 小写、不做完整 case folding”和 540 差异/旧汇总的当前描述，历史交付记录保持。

最终验收均在项目 `nix develop` 中完成：`cargo check --workspace --all-targets`；针对性 25 项及新增 CRLF 边界专项通过；完整 debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1519 项通过**；`cargo test --doc --workspace` **75 项通过**；`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`、Unicode 生成数据/许可/静态表与篡改拒绝离线核验、CommonMark 引用标签/引用形式/引用定义/Images 四份官方夹具离线核验、`container_links`/`link_suffix_headers`/`footnotes` release 示例、`git diff --check` 全部通过。日志前缀 `/tmp/mdtui3-casefold-`，独立差异 `mdtui3-casefold-unit.diff`，范围验证 `mdtui3-casefold-scope-check.log`。上一项 11 个文件完整保留：8 个代码/测试字节不变，3 个文档仅在原内容后追加；本项未重置、提交、推送或派生代理。宽泛引用/标准兼容性大类继续待办。


## 2026-11-10 精确单波浪号删除线交付

仅完成 TODO“删除线评分后的剩余范围”中的 GFM 0.29-gfm 例 491 单波浪号子项。先核验 ark/Linux、工作目录与 git 状态，将已有两个验收单元的完整 diff、未跟踪清单/哈希及所有基线文件哈希保存在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-003/`；未提交、推送或修改主机/用户配置，未派生代理。所有 cargo 命令都经本项目 `nix develop`，target 路径从 `cargo metadata` 获取。

实施前模型与累计证明保存在同目录 `model.md`。先加入独立 batch oracle、对象优先级/边界反例和长表头矩阵，`cargo check --workspace --all-targets` 通过，专项红灯 **21 项中 10 项失败**（退出 100，`targeted-red.log`）。`inline.rs` 原先只为双波浪号保存 kind；本项只增加单波浪号独立索引及三处数组长度，复用原有 run 确认、flanking、闭合/弹栈、表头继承及候选处理。独立模型初次转绿检查另暴露生成语料尾部触及既有段落空白隐藏；给组合语料加正文哨兵隔开该无关边界后通过，记录保留在 `targeted-green.log` / `targeted-green-2.log`，核心未因此放宽。

模型采用精确 maximal run：长度 1/2 分开匹配，>=3 全段字面，不做部分 run 消耗；待定 run 在下一个不同字节或 EOF 才确认，EOF 不能开新 opener。保留 ASCII 空白 flanking、能闭合优先且不重开同一 run；闭合时消费目标及其后全部 opener。因此 `~a ~~b~~ c~` 与反向可配对但公开位扁平，`~a ~~b~ c~~` 会丢弃双 opener，反向组合对称；同类贪心及交叉兼容性未改。代码/数学候选内部及其未完成失败内容保持 opaque，链接标签不是 opaque，目标/title 保留既有消费规则；不在本项修复链接强调作用域。表头可隔离配对只存端点/style、消费不同旧 opener，成功丢弃、失败/EOF 提交；bidirectional closer 或弹出本行 opener 继续永久取消本行探测，混合代码/数学/标签也沿用原优先级。

每字节只进入已有扫描一次，每个 run 一次确认，固定六类索引直接定位 opener；每 opener 至多一次 push 和一次 pop/边界丢弃。隔离 pair 由已消费旧 opener 计数约束，成功表头对当前 surviving opener 的前驱下标仅一次重建。每 pair 常数次 O(log n) 范围 tag，累计 O(n log n)、空间 O(n)；没有增长正文回扫、历史对象遍历、栈重放或评分投影补标记，零运行时依赖、禁止 unsafe，查询仍共用原预算。

新增 `tests/single_tilde.rs` 七项及 `tests/support/strike_model.rs`，复用 snapshot、公开源码/显示游标与现有长表头设施。独立 oracle 用有界串上的反向 opener 搜索和最终范围 painting，避开生产 last 索引/区间树；1,728 组组合及显式反例覆盖单双/失配、空白/标点/非 ASCII、转义、未完成/EOF、嵌套/交叉。活动扫描窗口后的 32 组长前缀 run 组合验证到 EOF/下一字节前不提前隐藏；UTF-8 任意字节切分、逐字节前缀/截断标量 EOF 分别在短输入与活动扫描前缀下比较完整快照/pending。另验收跨行、显式/等价/lazy 容器、空行/兄弟/块/表格边界，晚闭合的块/容器/链接/数学身份与 tile 失效、逐标量原始映射、显示/第二游标/解码目标共用六字节预算及零预算停步。`closed_headers.rs` / `open_headers.rs` 扩展单长度与混合长度、局部对象及可隔离/不可隔离矩阵，不削弱旧断言。

压力模型在 970 层引用中先创建 4,096 个交替单/双 opener，再各闭合一次：创建恰 8,192 次 delimiter 操作，闭合与少量局部对象至多 8,198 次。继承 4,096 对、4,096 个历史链接后，分 1,000 次追加百万字仅增加 inline/header 各百万字节；分隔符、分类、祖先、标签、区间写入及 annotation/container arena 均不增长。表头成功/失败各只结算 4,096 对，局部链接/数学和历史链接身份保持。

例 491 只更新独立观察/分类/统计及直接受本项影响的辅助方言断言，当前章节 **3 支持、0 刻意差异、0 未支持**；官方三例 Markdown/HTML、夹具生成器及 `project` 逐字不变。`fixture.log` 记录固定 HTML SHA-256、独立正则读取器的 491–493 原文比较、91 切点/88 前缀、生成器离线 `--check` 与篡改拒绝。原“仅 ~~”/491 刻意差异描述由本项收窄，旧交付不重写；完整 Unicode flanking、同类嵌套深度、完整强调/部分 run 消耗、链接作用域、更广泛容器/GFM 和渲染/UI 待办均保留。

最终验收均退出 0（完整命令、状态、时间见同目录 `validation.jsonl`）：

- `nix develop -c cargo check --workspace --all-targets`（`check-final-2.log`）。
- `nix develop -c cargo nextest run --test single_tilde --test gfm_strikethrough --test closed_headers --test open_headers --test inherited_headers --no-fail-fast`：48 项（`targeted-final.log`）。
- `nix develop -c cargo nextest run --workspace --all-targets --no-fail-fast` / 加 `--release`：各 1,526 项（`debug.log` / `release.log`）。
- `nix develop -c cargo test --doc --workspace`：75 项，72 普通 + 3 compile-fail（`doc.log`）。
- `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`、`nix develop -c cargo fmt --all -- --check`（`clippy.log` / `fmt.log`）。
- `python3 scripts/generate_gfm_strikethrough.py /home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-003/gfm-0.29.html --check`，由独立核验脚本执行（`fixture.log`）。
- `nix develop -c cargo run --release --example container_inline` / `open_headers` / `closed_headers` / `budget`（四份 `example-*.log`）。
- 范围/基线核对与 `git diff --check` 均通过（`scope-final.log` / `diff-final.log`）：389 个其它基线文件与所有既有未跟踪文件保持，三份文档仅追加；本项完整独立差异见证据目录 `task-unit.patch`。


## 2026-11-10 强调加粗 ASCII 侧界与词内下划线交付

本轮只完成现有精确单/双 `*`、`_` 的 ASCII 标点侧界及下划线词内规则。旧实现只检查相邻字节是否非空白，错误地把 `a*"foo"*` 识别为强调，把 `*(*foo)` 的中间星号当作 closer；下划线只在两侧均为 ASCII 字母数字时排除。新增两个反例/模型测试先在旧实现下失败，既有 19 项专项保持通过，退出 100 的原始日志已保存。

方言固定为三类相邻字节：序列端点及 HT/LF/FF/CR/space 是空白，ASCII 标点是标点，其余（含非 ASCII 字节）为普通正文。左侧界要求后继非空白，并在后继为标点时要求前驱为空白或标点；右侧界对称。星号直接使用左右侧界，下划线仅在“左侧界，且（非右侧界或前驱是标点）”时开启，并以对称规则闭合。因此 Cyrillic/中文等普通字节词内下划线也保持字面；这不代表 Unicode 空白/标点分类已完成。最大 run 继续等待后继字节或 EOF，未匹配或三连/更长 run 字面保留，失败不回放；既有代码/数学/转义/链接候选优先级、单/双波浪号独立空白侧界、贪心配对和扁平样式位保持。

实现只替换 `Inline::finish_run` 内的常数次谓词，不增加持久状态、分配或扫描。每个 run 仍计一个既有 delimiter step，每个 opener 至多入栈/出栈一次；标点后继可使继承 closer 成为 close-only，复用已有晚表头配对隔离，成功丢弃、失败/EOF 一次提交。区间样式与引用索引保持累计解析 O(n log n)、空间 O(n)，不回扫增长正文、历史引用或祖先链，不引入运行时依赖或 unsafe；显示/查询及元数据仍共用 Budget，append/finish 继续仅承诺摊还界限。

`tests/support/emphasis_model.rs` 使用三类真值表、反向 opener 查找和最终范围涂色，独立于核心索引栈和树；`tests/ascii_flanking.rs` 七项回归覆盖 153,664 组侧界组合、686 组混合/交叉/转义、UTF-8 任意字节切点和逐前缀/EOF、待定/失败表头、对象优先级及块/容器边界、稳定 BlockId/ContainerId/LinkId/MathId、原始区间与切片映射、六字节共享显示/目标预算、970 层百万字追加、4,096 个旧链接和 16,384 个历史 opener 的累计计数。普通增长不增加 delimiter/祖先/引用工作；原有 `closed_headers` 独立矩阵按新 close-only 判定修正，前轮单波浪号覆盖完整保留。

官方观察逐项审查：352/359/362/363/368/372、375/380/385/388、392/398/401 共 13 例从刻意差异转支持；350–373 当前 **20 支持/2 差异/2 未支持**，374–389 **15/0/1**，390–405 **16/0/0**。369/373 改为包住 `(foo)` 的单层样式，仍缺重复强调深度；419 的链接标签恢复外层强调、包络可投影，但仍缺内层重复深度，保持未支持，417–430 仍为 **7/0/7**。419 的精确源码/标签/包络观察保留并修正样式，另保留真正非均匀链接包络的投影拒绝反例；其余 350–481 观察不变。固定 0.31.2 原始来源摘要、132 例 Markdown/HTML、全部生成器和八个评分投影字节不变，离线 `--check`、独立读取器和篡改拒绝通过。

项目 `nix develop` 内 `cargo check --workspace --all-targets`、专项 nextest **107/107**、完整 debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,533/1,533**、`cargo test --doc --workspace` **75/75**、Clippy `-D warnings`、格式检查与 `git diff --check` 全部通过。release 示例 `closed_headers`、`open_headers`、`container_inline`、`budget` 均退出 0。验证命令、真实退出状态和完整日志在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-004/validation.jsonl`；逐项观察/模型界限见同目录 `design-and-review.txt`，本轮独立差异见 `task-unit.patch`。

本节收窄旧记录中的 ASCII 标点及普通字节词内下划线差异描述，历史交付不改写。完整 Unicode flanking、同类嵌套深度、部分 run 消耗、链接作用域与通用确认/撤销、完整容器和终端排版/UI 继续保留原 TODO。本轮未提交、推送或派生代理，三份项目文档仅追加，已有验收单元保持。


## 2026-11-10 强调加粗 Unicode 侧界交付

本轮仅交付 TODO 中强调/加粗的 Unicode 侧界子项。旧实现只记录相邻字节，`*NBSP a NBSP*` 会隐藏星号，`a*£*b`、`a**😀**b` 也会误配对。先新增反例，在旧核心上通过 `cargo check` 后运行专项 nextest，真实退出 100；完整失败输出保存于本轮 `targeted-red.log`。

沿用 CommonMark 0.31.2 左右侧界及下划线词内规则，将输入改为相邻语义标量：空白为 Unicode Zs 加 TAB/LF/FF/CR，标点为 P/S；NEL、VT、Zl/Zp 保持普通字符，不使用宿主 `char::is_whitespace` 代替规范分类。固定 Unicode 17.0.0，与已验收引用 case folding 的版本一致。官方 `DerivedGeneralCategory.txt` 原样入库，保留来源与 SHA-256，沿用完整 Unicode License V3；`scripts/generate_unicode_categories.py --check` 离线验证数据/许可、357 段有序不交叉分类表及比较上界，构建不生成或下载数据。

局部原型先用独立展开的官方类别模型核对全部 1,112,064 个有效 Unicode 标量，再扩展集成回归。`Inline` 在既有正向字节扫描中记录一个前一标量，序列结束/表头隔离重置、部分 tab 虚拟空格写入空格；不读取旧正文的前驱或跨前缀反向搜索。相邻标量解码至多四字节，每个受支持强调 run 仅分类两侧。静态表二分、每侧至多 12 次分类/比较由 `Work.flanking_steps` 计量；纯正文增长不增加该计数。既有 opener 索引栈、精确 1/2 长度、未完成/失败/EOF、跨行表头可隔离配对及源码显示预算接口保持；累计解析 O(n log n)、空间 O(n)、零运行时依赖和禁止 unsafe 的约束保持。

新增七项集成回归及一项全标量单元测试，复用 snapshot、批量强调模型和显示游标设施。覆盖 114,314 组 Unicode 上下文/混合 run、UTF-8 任意字节切点/逐字节前缀及逐前缀 EOF、显式/lazy/部分 tab 路径、代码/数学/实体和失败候选、长 run 字面行为、迟确认表头成功/失败、BlockId/容器/LinkId/MathId 稳定身份、tile 版本、逐标量切片源码映射和共享六字节预算。970 层及百万字 Unicode 正文增长不回访旧侧界，4096 个历史链接与 16384 个混合 opener 的增长/收尾计数保持线性。

逐项审查 CommonMark 350–481 全部 132 例：仅 353 的 NBSP 和 354 的 £/€ 改变观察，二者现匹配官方 HTML，350–373 更新为 **22 支持 / 0 刻意差异 / 2 未支持**。354 的既有美元数学候选优先级保持；369/373 仍缺同类强调深度，473/474 仍由投影明确拒绝，其余 130 例观察保持。官方 Markdown/HTML、全部八个评分投影和原生成器逐字节保持，既有 ASCII 测试中两项明确 Unicode 边界期望随实际语义更新，批量模型改读独立官方类别。

验证在本项目 `nix develop` 内完成，target 由 `cargo metadata` 确定：`cargo check --workspace --all-targets`、专项 nextest **114/114**、完整 debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,541/1,541**、`cargo test --doc --workspace` **75/75**、Clippy `-D warnings`、fmt、离线 Unicode/132 例强调/GFM 删除线夹具核验及 `git diff --check` 均通过。release 示例 `closed_headers`、`open_headers`、`container_inline`、`budget` 全部退出 0。命令、真实退出状态及完整日志在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-005/validation.jsonl` 及同目录各日志；首次反例、观察审查和审计脚本自身修正的失败记录保留。

本节收窄前述“非 ASCII 字节普通分类/完整 Unicode 侧界待办”及 353/354 旧统计的当前描述，历史记录不改写。删除线仍保留独立 ASCII 空白方言；长 run 部分消耗、同类嵌套深度、链接分隔符作用域/通用确认撤销、完整容器和终端排版/UI 继续属于未完成项。本轮未提交、推送、修改配置或派生代理，项目说明仅追加，既有其它验收单元保持。


## 2026-11-10 已确认强调加粗的同类嵌套深度接口交付

本轮仅完成公开同类嵌套深度这一核心子项。`_foo _bar_ baz_` 与 `**foo **bar** baz**` 已有内外配对，但区间标注按样式位 OR 合并，消费者不能区分一层和两层。先新增真实反例，在旧行为上得到内层深度 1、预期 2，专项 nextest 退出 100，见本轮 `targeted-red.log`。随后用局部独立原型对照 6,000 次重叠区间累加和全部 4,096 个点，核对对数更新/线性存储界限，再实现；没有派生代理或重复此前五个验收单元。

方言保持现有精确单/双 `*`、`_`、Unicode 侧界与贪心确认：未确认外层不增加计数，内层已确认效果不回放、不撤销；长 run、链接/代码/数学优先级、块/容器及晚表头隔离不变。`Mark` 增加两个 `usize`，每次原有 EMPHASIS/STRONG 区间标注贡献一次，按规范覆盖节点分解保证每个点恰好累加一次；根扩容保留原标签，不复制历史深度。没有新增解析栈、配对 arena、正文/旧引用/祖先回扫；每次已确认配对仍为既有 O(log n) 更新，常数次计数运算纳入 `tree_visits` 和原查询遍历，累计 O(n log n)、空间 O(n)。解析生成的深度不超过输入定界符数，使用 `usize` 避免窄计数饱和；零运行时依赖、禁止 unsafe 和共享预算保持。

`RenderRun` / `DisplayRun` 同时公开 `emphasis_depth`、`strong_depth`，run 合并条件包含两者；原位样式、link serial、revision、源码和替换映射保持。普通区间、tile、scratch 和预算游标使用相同标注；六字节共享预算仍限制显示、目标解码和并行游标，查询无需单独访问深度索引。既有标注树随机点/区间模型现在同时校验累加深度，公共 snapshot 设施纳入显示深度，原有分块套件一并覆盖。新增 10 项集成回归包括 2,400 组独立批量范围涂色、混合/失败/未完成配对、任意 UTF-8 字节切点/逐前缀/EOF、根/Quote/Item 表头确认与失败、晚闭合通知和稳定 BlockId/ContainerId/LinkId/MathId、源码切片/tile/替换映射与共享预算、65,536 层及增长期零额外 tree/delimiter 访问、970 层容器百万字追加、4,096 个历史链接和晚定义。首轮压力测试在有界分类器未提交短行时过早读取 LinkId，延长测试准备正文后通过；真实失败及修正后日志均保留，未因此改变实现。

官方输入/HTML、生成器及八份原样式位评分投影、独立观察、分类和汇总逐字节保持。新增严格的单类深度消费者仅使用公开计数，直接匹配原始官方 369/373/389/407 HTML，不扫描 Markdown、补标记或推导缺失树；四例为本接口的独立消费验收，不改写原评分。132 例逐项核对见 `official-review.tsv`；离线生成、Unicode 两表与 GFM 删除线核验及篡改输入拒绝通过。

项目 `nix develop` 中 `cargo check --workspace --all-targets`、专项 nextest **135/135**、完整 debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,551/1,551**、`cargo test --doc --workspace` **75/75**、Clippy `-D warnings`、fmt 与 `git diff --check` 全部通过；release 示例 `emphasis_depth`、`closed_headers`、`open_headers`、`container_inline`、`budget` 均退出 0。完整命令、真实退出状态及日志在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-006/validation.jsonl`；局部原型、设计/边界审查、验收输入哈希、既有修改保留核对及本项独立补丁均保存在同目录。

本项只收窄公开同类深度缺口，计数不表示完整嵌套顺序或相邻配对身份。部分 run 消耗、一般确认/撤销、链接作用域、完整容器和宽泛标准兼容性以及终端绘制/排版/UI 仍保留原未完成项。TODO 仅追加一个具体勾选，README/DONE 仅追加；本轮未提交、推送、reset、清理或修改配置/其它仓库，先前验收修改完整保留。


## 2026-11-11 强调分隔符部分消耗与三倍数规则交付

本单元实现任意长度星号/下划线 run 的部分消耗：先按既有 Unicode 侧界判断，选择最近的兼容 opener，双方剩余量至少二字节则确认 strong，否则确认 emphasis；只消费 opener 末端与 closer 起端。双向 run 按原始长度应用三倍数限制，剩余量不改变原始分类。未完成 run 等待后继标量/EOF；未配对余量保留字面或继续作为 opener，已确认内层保持。单/双波浪号仍精确长度配对。代码/数学/链接、容器/块/单元格和不透明失败候选的原有优先级保持，不扩展链接作用域及一般语义撤销。

每种标记保存六个分类头，键为原 can-close 和原长度模三，分类内资格一致，每次匹配/失败至多六次探测并逐次计入新增 `Work.delimiter_search_steps`。原 opener 一次入/出栈，部分余量原地缩小，被越过的候选只弹一次；每次配对消耗至少一个新 closer 字节。close-only 继承配对复用表头隔离，成功丢弃、失败一次提交；无需复制或回扫增长中的正文、旧引用或祖先。标注复用既有区间写入与公开深度，累计解析 O(n log n)、空间 O(n)，零运行时依赖、禁止 unsafe 和共享整帧预算保持；一次 append/EOF 仍只承诺摊还成本。

先新增 `__foo_ bar_` 等反例，确认旧实现失败。局部原型用 30,000 组随机 run 序列对照反向搜索，验证固定分类头、原长度及部分剩余量模型。复用独立批量侧界/范围涂色模型，新增 7,290 组不等长、双向、Unicode 和交叉组合；七项新集成测试与既有深度/表头套件覆盖 UTF-8 任意字节分块、逐前缀/EOF、CRLF、稳定 BlockId/ContainerId/LinkId/MathId、原始与替换/切片/tile 映射、零/六字节共享预算、表头确认/失败及部分余量、65,536 字节定界串、流式增长中的长 run、失败搜索、970 层引用百万字追加和 4,096 个历史对象。增长期 tree/delimiter/search 计数不变，配对/查询的线性与对数界限有断言。新增两项单类深度消费者回归，连同扩展的既有消费者直接匹配官方 369/373/389/407–409/417/425–427/461/463–466 原始 HTML，不推断混合顺序或修改旧投影。

132 例官方输入/HTML、所有固定夹具/离线生成器/数据保持原样；八份 `project`、`transition` 及链接投影函数逐字节核验相同。28 例变化逐项记录于本单元 `official-review.tsv`：414/415、431/442–444、445–447/454–459 共 15 例由未支持转支持；413/416/430/467/468 由字面观察改为拒绝歧义同时开启，另行锁定公开文本及深度；408/409、417/426/427、464–466 共八例更新观察，仍因旧样式位投影未消费深度而保留未支持。当前八份评分合计 **104 支持、0 刻意差异、28 未支持**，不外推完整 CommonMark 兼容。独立模型、官方固定来源与篡改拒绝日志均保留；首次完整回归暴露的旧字面断言/表头模型/评分观察均按新核心行为修正，未改官方原文或投影算法。

本单元同时整理现行重复待办，保留历史，不把旧评分适配器的限制冒充核心字段缺失。原七处宽泛后续合并为当前“完整强调作用域、确认/撤销及嵌套顺序”，原“不等长强调评分的剩余范围”由有证据的部分消耗交付替换为完成项；原文逐字归档于证据目录 `todo-merged-original.md`，对应关系如下。

| 原后续条目 | 已有实现/证据 | 仍未完成，统一保留 |
| --- | --- | --- |
| 强调剩余评分与分隔符作用域 | 全章评分、ASCII/Unicode 侧界、同类计数 | 完整作用域、确认/撤销、嵌套顺序/身份 |
| 基础加粗评分暴露的剩余范围 | 侧界及 389 等同类深度 | 完整作用域/嵌套表示 |
| 加粗收尾与混合强调评分的剩余范围 | 侧界、同类深度、本单元部分消耗 | 同时开启样式顺序、交叉/链接范围及撤销 |
| 嵌套强调评分暴露的分隔符部分消耗与深度 | 407–409 计数、413–416 部分消耗 | 混合样式顺序及通用作用域/撤销 |
| 长分隔符与嵌套加粗评分暴露的剩余范围 | 长串部分消耗及同类深度 | 419 链接内外作用域、顺序/撤销 |
| 混合收尾与字面分隔符评分的剩余范围 | 431/442–444 配对、432 公开计数 | 完整链接和混合嵌套/撤销 |
| 强调全章评分后的兼容性剩余范围 | 全章评分、同类深度、464–468 消耗 | 473/474 链接作用域、混合顺序/身份；原始 HTML 继续不在需求范围 |
| 不等长强调评分的剩余范围 | 445–459 全部匹配；部分消耗/三倍数规则及累计界限 | 通用嵌套、作用域/撤销归合并项 |

因此现行未完成复选项 **51 → 44**，差额为六处重复合并及一处实质交付，真实未完需求没有移除；其它代码回退、容器、折叠、终端排版、渲染和交互需求保持。README 将八段相互过时的强调评分说明收拢为当前表格，并更新侧界、深度和部分消耗的行为边界；历史交付与此前六单元提交完整保留。

所有 Cargo 命令均在本项目 `nix develop` 中运行，target 来源于 `cargo metadata`。最终 `cargo check --workspace --all-targets`、专项 nextest **103/103**、完整 debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,560/1,560**、`cargo test --doc --workspace` **75/75**、Clippy `--workspace --all-targets -- -D warnings`、fmt、相关离线生成数据/固定夹具及 `git diff --check` 通过。release 示例 `emphasis_runs`、`emphasis_depth`、`closed_headers`、`open_headers`、`container_inline`、`budget` 均退出 0。真实命令、退出状态及日志在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-007/validation.jsonl`，失败的红测与中间回归亦保留；设计、局部原型、官方逐项审查、代码哈希及最终差异证据同目录。本轮未提交、推送、reset、清理、派生代理或修改主机/用户配置、其它工作树及运行器/账本。


## 2026-11-11 已发布标签的未配对分隔符终点交付

本单元完成原 TODO“链接标签嵌套及强调作用域剩余范围”中的例 535 及同一模型下链接/图片标签的右边界：标签发布为对象时，清退内部未配对的强调、加粗和单/双波浪号 opener，保留标签内已确认的样式及外部 opener。例如 `[foo *bar][ref]*` 的链接文字保留 `foo *bar`，外部 `*` 为字面；外层完整强调/加粗仍可包住对象。未定义 shortcut 仍按现有方言发布隐藏括号的引用对象，晚定义保持 LinkId 与原始区间。未闭合/被替换候选、超长标签的未完成/失败显式后缀沿用原不重放规则；超长标签仅在成功显式后缀后结束作用域。

先红后绿：新增公开语义反例在原实现上失败；最终审查补到长继承标签经晚表头回退发布直接对象的路径，先用 close-only 反例复现再补清退调用。仅新增一个栈尾清退方法，复用 candidate.open 与现有 delimiter arena/category heads；每个原 opener 至多入栈/出栈各一次，新增工作计入原 delimiter_steps，不扫描或复制旧标签、正文及栈。每次对象发布只有常数边界检查与实际弹栈；原部分 run 配对由新 closer 字节摊还，表头确认仍只对 surviving opener 一次重建。没有新增持久字段、运行时依赖或 unsafe，累计解析 O(n log n)、空间 O(n) 和整帧共享预算保持。

六项回归复用原 snapshot 支持：**144 组**独立作用域边界产品模型（3 外层 × 6 内层 × 2 对象 × 4 后缀）及已确认样式/部分 run 反例；任意 UTF-8 字节切点、逐字节前缀与 EOF、CRLF、容器和表格；未完成/失败/超长标签；稳定对象/块 ID、原始标签/对象源码区间及晚定义；两显示游标与目标/title 解码共同使用六字节预算；晚表头成功、失败、EOF、长继承标签及七字节分块；970 层容器、百万字增长、4096 历史对象与 4096 opener 的累计工作及分配上界。长正文增长不增加 delimiter/tree 工作，不回访历史对象或祖先。

官方 535 由未支持转支持，512–538 汇总现 **16 支持、4 刻意差异、7 未支持**；其余 26 例独立观察/分类保持。244 对官方 Markdown/HTML、所有相关评分投影、生成器、Unicode/GFM 数据及许可均逐字/摘要核验保持，篡改输入被拒绝。旧删除线测试的 `[~label](/u)~` 从跨界删除线更新为字面 `~label~`，其它对象优先级断言保留；新示例最初漏计原始空白行的一处 LF，已更正并保留失败日志。

仍未交付：外部 opener 提前在标签内部闭合（521/534/564、473/474）、419 现有投影的深度/顺序限制、嵌套候选、完整失败字面回退、通用确认/撤销及其它原 TODO 要求。现行相关宽泛条目已收窄到剩余范围；同时依据此前交付事实纠正 Links 全章评分、540 完整 case folding 和 491 单波浪号仍“未交付”的过时描述及汇总。真实未完成复选项仍 **44**，没有删除或勾选这些未完需求；更新前相关原文留在本单元 todo-original-lines.md，DONE 旧历史及此前所有提交完整保留。

最终验证全部在本项目 `nix develop` 中完成，target 取自 `cargo metadata`：`cargo check --workspace --all-targets`；针对性 nextest **102/102**；完整 debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1,566/1,566**；`cargo test --doc --workspace` **75/75**；Clippy `--workspace --all-targets -- -D warnings`、fmt、相关离线生成数据/固定夹具和 `git diff --check` 全部通过。release 示例 `label_delimiters`、`emphasis_runs`、`container_links`、`closed_link_headers`、`link_suffix_headers`、`budget` 均退出 0。最终代码哈希、设计/累计界限、官方逐项审查、红测和中间失败、真实命令/退出状态及完整日志保存在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-008/`（索引 `commands.tsv`，最终命令日志后缀 `-v3`，离线日志 `fixtures.log`）。本轮未提交、推送、reset、清理、派生代理或改动主机/用户配置、其它工作树及运行器/账本。


## 2026-11-11 未闭合代码候选无串数上限回退交付

本单元完成原 TODO“强调和代码 span”及“CommonMark Code spans 标准样例评分”中超过 32 个异长反引号串的代码配对回退。开放外层仍暂定字面；同长度 closer 到来时优先闭合外层并丢弃后备摘要。外层在 EOF、段落/容器/标题/单元格边界失败时，按源码次序寻找最近同长度后继，贪心恢复不相交代码对；没有匹配的串保持字面。其它行内语法仍不重放，完整失败回退继续待办。晚表头确认恢复前序摘要边界及末段空白摘要，失败保留整个候选，跨新表头的配对不泄漏。

先红后绿：原实现面对 33 串只能恢复 0 对，红测要求 16 对并失败。实现移除 overflow/32 串常量，只保留每串的原始区间、全空白/末字节摘要；收尾排序索引并建立同长度最近后继，再作一次源码序贪心遍历。每字节仍只作常数摘要更新，代码规范化沿用已有常数端点读取，不复制或回扫增长中的旧正文、不遍历祖先或旧对象。每个异长串只记录一次，参与一次排序和常数次线性索引/匹配；r 串排序 O(r log(r + 1)) 比较计入 Work.delimiter_search_steps，其余索引/匹配访问不超过 4r 次计入 delimiter_steps。全部候选累计 O(n log n)、空间 O(n)，核心依旧零运行时依赖、禁止 unsafe，消费继续共用整帧预算。32 串的常数候选空间改为与实际串数成正比的摘要空间；这不是解析单次硬时延保证。

复用原 255 组独立贪心模型，新增 256 组 33–128 串模型及外层闭合对照。11 项专项回归覆盖 31/32/33/1000 串、UTF-8 任意字节切点/逐前缀及 EOF、CRLF/空白规范化、容器虚拟 tab、表头确认/失败/EOF、稳定 BlockId/LinkId 和原始源码映射、过期读取游标及两个显示游标共享六字节预算；64/256/1024/4096 串的重复/全异长度对抗模型计量索引和排序，4096 个全异串实际 16,383 次线性访问、51,873 次比较。百万字/千层容器原回归及正文增长不增匹配/分类/树/祖先工作均通过。内部存储测试确认摘要按串数增长、普通正文不增配、成功外层清空并复用容量。

中间两次针对性失败均保留日志：一次是旧算法的工作量阈值尚未更新；一次是新工作量用例不足分类窗口，首批正文尚未消费，已补足前缀后再测增长。这些修正没有放宽渐近界限或修改评分投影。官方 Code spans 仍为 21 支持/0 刻意差异/1 未支持，Backslash escapes 仍为 11/1/1；33 个 CommonMark 离线生成器核验 587 对原始 Markdown/HTML，Unicode/GFM 数据核验通过，123 份来源/生成器/夹具/评分文件与基线逐字一致，篡改来源被拒绝。观察、分类和汇总没有变化。

TODO 直接修订原完成条目和两处剩余范围描述，README 原章节及摘要同步移除过时 32 串限制；没有新增完成复选项或删除任何真实未完成需求，44 个未完成复选项完整保留。旧 DONE 全部保留，原条目文本另存本单元 todo-original-lines.md。

最终验证全部在本项目 nix develop 环境执行，target 由 cargo metadata 获取：cargo check --workspace --all-targets；针对性 nextest 80/80；完整 debug/release cargo nextest run --workspace --all-targets --no-fail-fast 各 1,570/1,570；cargo test --doc --workspace 75/75；cargo clippy --workspace --all-targets -- -D warnings、cargo fmt --all -- --check、离线生成数据/固定夹具及 git diff --check 全部通过。release 示例 code_fallback、code_headers、closed_code_headers、display、budget、stream 和 syntax_fuzz 的 seed 305419896 / 256 cases 均退出 0。设计/累计界限、红测和中间失败、逐项官方审查、完整命令/退出状态与日志、最后代码哈希保存于 /home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-009/，命令索引 commands.tsv。本轮只实现和验证，未提交、推送、reset、清理、派生代理或改动主机/用户配置、其它工作树、运行器及账本。


## 2026-11-11 已发布标签左边界的强调隔离交付

第 10 单元仅交付单候选 direct/full/collapsed/shortcut 标签的左边界；标签内部 `*`/`_` closer 不消费外部 opener，保留标签内配对、单/双及部分消费、外层包围配对和第 8 单元的右边界。`annotations.rs` 无需撤销或改写标注，核心仍零运行时依赖、禁止 unsafe。

失败模型先固定为：合法有界 shortcut 在 `]` 发布，即便未定义也沿用隐藏括号方言，失败显式后缀不取消已发布对象；不能发布 shortcut 的长/空白标签等待成功显式后缀，失败/EOF 才作为普通正文处理分隔符。未闭合或被新括号/自动链接起始替换的候选同样按普通正文处理；后缀保持既有不重放规则。确认前暂存 run 的区间及前后标量，尚不发布对应样式，因此失败后仍允许普通正文跨括号配对。

原反例 `*[foo*](/uri)` 的外部 opener 在旧实现中已被消费，追加式标注无法撤销。新增红测先观察到外部/标签深度为 1（`red-left-boundary.log`，退出 100），随后核心改为：候选只记录分隔符摘要，成功时固定六个分类头查询排除标签外 opener，失败时普通配对一次，之后沿用原栈尾清退。代码/数学候选继续原路径，波浪号保持原配对方言。晚表头确认时前一行摘要作为原段落处理，当前行摘要交给独立首单元格；失败才发布继承标签。摘要处理及一次移交计入 `Work.delimiter_steps`，每个摘要仅配对一次，栈入/出与部分消费按原 run 摊还；无增长正文/标签回扫或外部栈复制，累计 O(n log n)、空间 O(n)，容量跨候选复用。

十项 `label_delimiters` 回归及一项内部存储回归覆盖：64 组左边界模型（每组分别验证阻断及内外合法配对）和部分消费；原 144 组右边界模型；108 组左边界晚表头；未闭合、失败/未完成后缀、超限标签；全部短例 UTF-8 切点、逐字节前缀/EOF 与长标签 1/7/1024 字节分块；晚定义前后稳定 LinkId/块身份/源码区间与六字节显示/目标/title 共享预算；970 层容器、百万字标签、4096 历史对象、4096 内部 opener、最多 32768 个外部 opener 和大量重复候选的累计计数/存储约束。长正文追加不增加已存摘要数、容量、分隔符搜索或标注树访问。

独立观察后，官方 521/534/564 及同规则的 473/474 匹配原 HTML；523 的新观察为 `<p>*foo bar* baz</p>`，仍属未定义 shortcut 隐藏括号差异。三份报告分别为 512–538 **18 支持/4 刻意差异/5 未支持**、553–571 **17/2/0**、460–481 **12/0/10**；强调全章八份旧投影合计 **106/0/26**。更新观察、分类与汇总，四份相关投影算法逐字核对保持，68 个相关官方 Markdown/HTML 经独立读取器与固定 SHA-256 核验；夹具和生成器未改。原以错误跨界为拒绝条件的投影测试改用真正歧义的样式顺序反例，并锁定已修复例的公开结果，不向投影补语义。

验证均在本项目 `nix develop` 下执行；target 从 `cargo metadata --no-deps --format-version 1` 获取。命令、退出码及完整日志位于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-010`，汇总见 `verification.json`：

- `cargo check --workspace --all-targets`：通过。
- 针对性 `cargo nextest run --test label_delimiters --test commonmark_link_labels --test commonmark_reference_forms --test commonmark_emphasis_final --test commonmark_emphasis_partial --test single_tilde --lib --no-fail-fast`：86/86 通过。
- `cargo nextest run --workspace --all-targets --no-fail-fast` 及对应 `--release`：各 1578/1578 通过。
- `cargo test --doc --workspace`、`cargo doc --workspace --no-deps`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check`：通过。clippy 初次指出单例观察测试的冗余循环，简化后复核通过，原失败日志保留。
- `cargo run --release -p mdstream-core --example label_delimiters` 与 `--example emphasis_runs`：通过。
- 三份相关生成器 `--check`、独立 68 例/投影核对、六项独立观察与三类报告、官方输入/生成器/依赖未变及 `git diff --check`：通过。

README 当前说明和本任务对应 TODO 已同步，只新增此窄项勾选；原三个宽项仍未勾选。419、嵌套链接/图片、多候选、跨行定义/后缀、完整字面失败回退、不可隔离晚表头的一般确认/撤销、混合样式顺序与 UI 保留后续。本单元未提交或推送，由新审查会话验收和父运行器提交。

本单元首次父审修正：未发布的继承标签被新括号或自动链接起始替换时，旧实现以当前 `header_floor` 处理所有暂存摘要，误把旧行内部配对隔离，或将旧行 opener 当作首格本地 opener；双向分隔符还会错误拒绝长表头。两个新增红测均失败（`parent-fix1-red-all-models.log`，退出 100），复现强调丢失、跨段落/表格强调及表头误拒绝。修复只在原摘要处理循环中使用已有旧行前缀长度：先以普通正文边界处理旧行摘要，再用剩余栈长度恢复当前行继承边界；每个摘要仍只处理一次，无额外正文扫描、摘要复制或放宽表头探测。

新增两项公开回归覆盖 288 组新括号/自动链接替换模型，含 `*`/`_` 单双、旧行局部配对、旧行 opener、旧行双向分隔符、当前行局部配对、0/1100 字节填充、表头确认/失败/EOF，以及整段和 1/7 字节分块（含 UTF-8）快照、LinkId 和源码区间。一项内部回归覆盖 18 组 64/1024/4096 规模摘要：旧行摘要单次释放、当前行边界恢复、跨行配对单独暂存、摘要容量复用，释放前后各 100 次正文追加不增加分隔符处理或搜索；同时核对累计工作、标注树访问和空间界限。既有身份、晚定义、原始映射和共享预算回归继续通过。

本次修正后的最终验证取代上方初次验证计数：`cargo check --workspace --all-targets` 通过；原专项另加 `tables`、`partial_table_tabs`、`pending_table_tabs` 后 **111/111** 通过；完整 debug/release `cargo nextest run [--release] --workspace --all-targets --no-fail-fast` 各 **1581/1581** 通过。doc 测试/构建、clippy `-D warnings`、fmt、两个 release 示例、三份离线生成器、独立官方夹具/投影及三类报告复核、依赖/官方输入未变和 diff 检查均通过。仍使用项目 `nix develop` 及 metadata 给出的 target；完整命令、退出码和日志使用同目录 `parent-fix1` 前缀保留，最终哈希与计数更新到 `verification.json`，原验证快照保留为 `parent-fix1-prior-verification.json`。三类报告仍为 **18/4/5、17/2/0、12/0/10**；真实未完成项仍为 **44**，本次未新增交付项、未进入第 11 项、未提交或推送。

## 2026-11-11 折叠增量与版本契约审计交付

- 仅关闭 TODO `494b98378962`「折叠增量与版本」。现有 `FoldChange { version, source }`、单节点/批量版本语义、区间遮蔽和后缀惰性失效已满足要求；旧测量、布局 pass、视口继续位置及后续 checkpoint 验证依赖，无关前缀 ticket 仍可复用。本次无源码/测试改动；README「基础折叠」已有准确说明，保持原文。
- 复核空 EOF、开放/晚确认边界、UTF-8 分块、稳定身份/源码映射、共享预算及百万字/十亿 tile 工作计数。运行 `nix develop -c cargo nextest run -p mdstream-core --test folding --test fold_batch --test containers --test container_folding --test fold_preview --test checkpoints --test layout --test cache_search --no-fail-fast`，103/103 通过；另选现有 `paragraph_folding` 边界失效测试和 `layout::folding_tests` 数组/十亿 tile 模型，4/4 通过。
- 逐条契约映射、基线、Cargo target、完整命令/退出状态/日志保存于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-011/`（`audit.md`、`results.tsv` 及各 `.log`）。文档测试 75/75、文档构建、fmt 及范围/diff 检查通过；按任务单文档审计例外未重跑源码未变的全套。其它折叠与排版 TODO、按键/图标/动画/终端 UI 继续保留。

## 2026-11-11 零高度来源与折叠独立性审计交付

- 仅关闭 TODO `4376f565c4c1`「区分不同来源的零高度」。语法隐藏由 Document 的标注/可绘制索引保留，暂测零值及有效性由基础高度和 `HeightState` 表达，人为折叠由视图选择和独立区间遮蔽表达；`expire` 复活基础零估计时，可见高度仅纳入未遮蔽部分，父折叠移除后仍保留子遮蔽。resize、晚引用和表格列宽变化可重置/过期基础布局，折叠选择不变。本次无源码/测试改动；README「共享预算与增量缓存」「基础折叠」现有说明准确，保持原文。
- `nix develop -c cargo nextest run -p mdstream-core --test estimates --test folding --test container_folding --test container_sections --test layout --no-fail-fast`：79/79 通过。另选 `layout::folding_tests` 两个数组/十亿 tile 模型、`frame` 七项及 `container_definitions::late_definitions_invalidate_dependencies_and_layout_without_changing_fold_state`：10/10 通过。证据覆盖旧零测量后失效、父子交替折叠/展开、空代码及语法隐藏、同 tile 多边界与占位归属、resize/晚引用/列宽增长、UTF-8 分块/EOF、稳定 ID/源码区间、共享预算及百万字隐藏区跳过。
- 文档测试 75/75、文档构建、fmt、release `folding` 示例及范围/diff 检查通过。完整命令、退出状态和日志、逐条状态/模型/复杂度映射及最终改动哈希保存于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-012/`（`commands.jsonl`、`audit.md`、`final.diff`、`final-hashes.json`）。按任务单文档审计例外未重跑源码未变的全套 debug/release/lint；其它折叠边界、开放范围、可见性预算及终端排版/UI TODO 保留。

## 2026-11-11 开放范围随流增长契约审计交付

- 仅关闭 TODO `f4fc97046adb`「开放范围随流增长」。开放容器及内层章节以开放终点索引和共享尾部遮蔽表示，顶层章节与当前未闭合代码等使用常数个增长槽；`FoldView::apply` 返回前新 tile 继承遮蔽，查询要求已消费连续更新。新标题、Setext 晚确认、容器关闭及 EOF 按结构事件修正范围，合并 Update 保留关闭事件与稳定 ID。本次无源码/测试改动；README「基础折叠」「显式容器折叠」「容器内章节独立折叠」现有说明准确，保持原文。
- `nix develop -c cargo nextest run -p mdstream-core --test folding --test container_folding --test container_sections --test containers --test container_headings --test updates --test utf8 --no-fail-fast`：96/96 通过；另选 `layout::folding_tests` 数组/十亿 tile 模型，2/2 通过。千层容器与 500 个开放章节的百万字正文逐字追加只更新共享尾部、无祖先索引写入；1000/500 个真实关闭事件分别计费。4096 个旧候选、UTF-8 分块/EOF、Setext 收缩、原始源码映射、独立区间/高度模型及共享预算均有现有断言覆盖。
- 文档测试 75/75、文档构建、fmt、release `container_folding` / `container_sections` 示例及范围/diff 检查通过。逐句契约/反例/累计复杂度映射、完整命令退出状态和日志、最终范围哈希保存于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-013/`（`audit.md`、`commands.jsonl`、`final.diff`、`final-hashes.json`）。按任务单文档审计例外未重跑源码未变的全套 debug/release/lint；不承诺任意 append 的硬时延，手工范围自动增长/交叉/嵌套、其它可见性预算及实际排版/UI 待办保留。

## 2026-11-11 所有可见性工作纳入预算契约审计交付

- 仅关闭 TODO `835f97a7fd0a`「所有可见性工作纳入预算」。候选发现、可见性查询、逐节点路径展开、边界排版、占位/段尾交付、测量与修正后再查询均在受预算驱动器内按同一传入 Budget 计费并保留继续状态。容器及内层章节查找使用范围索引；前后导航跳到隐藏范围端点，高度查询按权重跳过遮蔽子树，不逐标题或 tile 试探。顶层标题的六级循环没有被用于容器深度；必要展开每个实际节点各收 query/object。本次无源码/测试改动；README「共享预算与增量缓存」「基础折叠」现有说明准确，保持原文。
- 相关既有 nextest 204/204；整帧修正/稀疏索引专项 6/6；祖先游标/缓存专项 7/7 通过。现有断言覆盖 200 层容器和 300 个容器/章节遮蔽者逐步展开、五万隐藏标题正反各四步、百万字隐藏正文零字节交付、千层索引访问界限、接近 isize::MAX 的稀疏视口、零预算/耗尽重入及 UTF-8 分块/EOF、稳定 ID/源码映射。整帧性能模型以同一 Budget 包含高度修正、首查、恢复、消费、测量和第二次查询，逐维核对 used 与实际访问/交付计数；屏外正文增长不增加帧工作。
- 文档测试、文档构建、fmt及范围/diff 检查通过。逐入口计费/继续状态、失败输入、模型和累计复杂度映射、真实命令/退出状态/完整日志及最终范围哈希保存于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-014/audit/`（`audit.md`、`commands.jsonl`、`final.diff`、`final-hashes.json`）。按任务单文档审计例外未重跑源码未变的全套 debug/release/clippy。低层无整帧约束接口仍由调用方计费；不可暂停外部回调、解码/析构及任意 append/finish 不获得硬时延保证。其它边界细化、手工范围扩展、终端排版/UI 等真实待办保留。

## 2026-11-11 引用定义目标紧接下一物理行交付

- 仅完成 TODO `fe3907bae22e` 的窄子项：块起始处合法单行 `[label]:` 后仅水平空白时，紧接下一物理行读取完整目标及该行可选 title。根与既有完整显式容器路径支持缩进/部分 tab、LF/CRLF/EOF、Unicode/转义括号标签、首定义优先和晚引用；保留两行各自的分类窗口与 999 个原始标签字节上限。独立下一行 title、title/标签/行内后缀跨行、完整回退/容器和分类窗口扩展继续待办，原宽 TODO 保留。
- 待决首行只保存 Span/BlockId，延迟行内消费，不发布可撤销 shortcut；管道标签复用 table_header 候选。成功保留 BlockId/container、扩展原始区间并使首行布局失效；失败只消费一次首行，保留表头/Setext/标题/代码优先级及 lazy 方言。真实 SourceFoldRegistration/FoldView.apply 覆盖固定范围排除、开放段落折叠确认/回退、旧测量拒绝；增长目标超窗后不追认，晚定义不遍历历史用法。核心无新运行时依赖/unsafe，无旧正文复制重解析，累计 O(n log n)、空间 O(n)。
- `next_line_definitions.rs` 二十一项回归覆盖原 1440 组目标模型及新增 3465 组容器、3780 组叶块/列宽结构对照、UTF-8 全切点/逐前缀及 EOF、复杂行内首行失败回退、连续候选、稳定身份/源码切片映射、共享六字节显示与 dest/title 元数据预算、百万字失败增长及两万旧引用工作量。独立于 HTML helper 证明下一行 `<my url>` 的原始/解码目标含空格，title 与目标共用微小预算。按父任务明确授权，输出 helper 仅新增通用 U+0020→`%20` 编码；单行/下一行定义和实体解码同规则。两个共享 helper 消费者的旧空格拒绝用例改为明确编码正例，其它 URL/结构拒绝保持，三个 project 函数正文不改；该输出适配器变更列入后续独立审查。
- 独立复审四个新增 Quote/List 反例先红后绿。目标确认前复用既有路径、Quote/marker 和段落中断规则，保留部分 tab 路径探测中首个缺失 frame，防止新容器或兄弟 Item 被旧字节边界匹配遮住；根已有段落的非 1 有序标记、空的不同标记、四列目标缩进及普通标点目标仍按原方言处理。同型空兄弟和非 1 兄弟列表按既有身份规则回退，不另造容器语法。新增扫描计入 Work；25 组对象/更新、81 组工作量模型及扩展 FoldView/映射/分块测试覆盖确认前零容器扫描、确认时计费与累计界限。路径探测只保留借用游标和一个 frame，不复制容器树或原文，不回扫旧正文。
- 官方 198 转支持；193/195 目标自然确认，独立 title 行仍分别为缩进代码/普通正文，原始目标、无 title 及外部可见文本另有核心断言，仍分类未支持。官方输入/HTML/生成器保持原样，独立观察与 README 三类计数更新为 15 支持/7 刻意差异/5 未支持，不用投影补造解析语义。
- 首处容器修正验证：本项目 `nix develop` 下 check、针对性核心 138/138、全部共享 helper 消费者 89/89、完整 debug/release nextest 各 1596/1596、文档测试 75 项、doc、clippy `-D warnings`、fmt 全通过。七组相关离线夹具校验、82 个固定夹具/生成器/manifest 与三个 project 正文不变检查、container_definitions/source_folding/budget release 示例及 diff 检查通过。完整命令/状态/日志、失败到成功证据、模型/复杂度与审查摘要、最终补丁和变更文件哈希在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-015/fix-containers/`；`implementation/`、`completion/` 和原复审记录全部保留。
- 第二处父验收修正：`>\t[r]:` 后的 `>\t ` 仅剩三列缩进，旧目标字节边界却把 ATX、Setext、分隔线和代码围栏误算为至少四列，四个精确红测及列宽矩阵全部先失败。现在同一个拆分 tab 结构游标同时判容器和叶块优先级，目标/title 仍从原字节边界借用源码，不新增持久状态或回扫。3780 组独立 seed/列宽模型另含数学围栏、脚注、外形相近的普通目标及真正四列例外；225 组身份/原始 Span/行尾、90 组确认前零扫描与计费界限，以及扩展分块/逐前缀/EOF 和真实 FoldView 更新全部通过。
- 叶块优先级修正验证：项目 `nix develop` 下 check、针对性核心 159/159、完整 debug/release nextest 各 1603/1603（含全部共享 helper 消费者）、doc 测试 75 项、doc 构建、clippy `-D warnings`、fmt、三个 release 示例、七组离线夹具/官方输入与投影正文不变检查、重编运行原父探针及 diff 检查均通过。命令/退出状态/完整日志、先红后绿、模型/复杂度说明、最终补丁和哈希保存于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-015/fix-leaf-priority/`（`commands.jsonl`、`audit.md`、`final-verification.json`、`fix.patch`、`final.patch`、`final-hashes.json`）。宽 TODO 的 40 条未完成项保持，未开始 task16，未提交或推送，由新独立审查会话验收并由父运行器提交。

- 第三处窄修复（联合表头资格）：先红测复现 `>\t[r|s]:\n>\t  -|-\n` 与 `- [r|s]:\n\t  -|-\n`；等长普通表头对照不形成表格，原字面分隔行检查却误拒绝目标。联合候选现经过既有根/容器表格确认分支，仅在表格不成立时读取目标，直接复用 `partial_leaf`、实际路径及 lazy 资格，不另建表格/容器规则；已观察的容器 payload 不重复记账。普通空格路径和至多三列部分 tab 的表格优先级保留。16380 组根/Quote/List/嵌套、space/tab/物理列宽、Unicode/普通/转义管道标签、合法/非法/不同列数分隔行、同目标行 title、LF/CRLF/EOF 的等长普通表头对照通过；扩展全切点/逐前缀/EOF、稳定 ID/目标与 title 原始区间、真实 FoldView.apply/ticket 失效、共享显示/元数据预算、双组百万字及两万旧引用工作计数。候选固定 Span/BlockId、不发布可撤销 LinkId、逐行分类窗口、累计 O(n log n)/O(n)、零运行时依赖及禁止 unsafe 不变。
- 联合表头资格最终验证：本项目 `nix develop` 下 check、相关专项 163/163、完整 debug/release nextest 各 1607/1607、文档测试 75 项、doc 构建、clippy `-D warnings`、fmt、三个 release 示例、七组离线夹具、82 个官方输入/HTML/生成器/manifest 与三个 project 正文不变检查、原父叶块探针及 diff 检查通过。完整命令/状态/日志、先红后绿、模型与复杂度说明、最终补丁/文件及日志哈希在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-015/fix-header-eligibility/`。本次只修改 task15 的表头资格问题，官方 198 支持及本章 15/7/5、输出 U+0020 通用编码授权边界、原宽 TODO 和 40 条未完成项保持；未开始 task16、未提交或推送，由新独立审查会话验收及父运行器提交。


## 2026-11-11 引用定义独立下一物理行完整 title 交付

对应 TODO `c6c77e9d8146` 的 task16 窄项：块起始处已发布有效目标、且目标行无 title 时，仅接收紧接下一物理行的完整单/双引号或括号 title，含空 title；目标可来自标签同一行或紧接下一行，定义至多三条物理行。根与既有完整显式路径、水平缩进超过三列、partial tab、Unicode、转义/实体原始 Span、LF/CRLF/EOF 保持。

目标确认后定义保持 Growing，单独持有 BlockId 与可选获胜 SymbolId，不保留段落/Setext 或 Inline 资格。成功扩展原块包络并密封，只由首定义更新符号/title/revision 与引用范围；重复定义可消费自身合法 title 而不覆盖胜者，已有 title（包括空 title）不再吸收第二行。失败、空行、坏尾缀、缺失路径/新兄弟、未闭合或超窗只结束候选，目标不撤销，当前行走原普通块流程。待决固定折叠边界停在定义起点，确认/失败/EOF 后解除。

实现复用既有完整容器路径及 Destination 状态机，每条有界 title 行只扫描常数次，新增分类/路径工作真实计数；超窗不重试，不回扫旧正文、引用或祖先历史。首定义目标与 title 分两次发布，元数据修订相应递增；通知沿用当前符号及 first_use 起保守区间。核心零运行时依赖、forbid unsafe、累计 O(n log n)、空间 O(n) 与共享预算保持。

`tests/next_line_titles.rs` 八项回归：2100 组成功模型及独立普通块失败对照；首定义无 title 时重复定义仍不能补写（含空目标）、已有空 title 停止、两/三物理行；UTF-8 任意字节切分、逐标量前缀/EOF；稳定 BlockId/LinkId/ContainerId、原始源码/切片与解码；pending source_fold_limit/固定注册、真实 FoldView.apply/过期 ticket；共享六字节显示/目标/title 微预算；分类窗口边界、970 层百万字失败增长及两万旧引用工作量。既有下一行目标、叶块优先级与联合表头模型保持。原深250层路径工作250→750、重复定义路径3→5是新增有界 title 探测的真实收费，断言保留线性上界。

官方 193、195、217 原投影直接匹配原 HTML，章节为 **18 支持、7 刻意差异、2 未支持**。196/208 原观察及其它章节评分保持；官方夹具、生成器、project、通用 U+0020 输出编码 helper、Cargo/Nix 结构均经哈希审计保持。title 正文本身跨行（196）、标签跨行（208/541）、行内后缀跨行（510）、分类窗口/元数据/999 原始标签字节限制放宽、完整容器及字面失败回退仍待办；原宽 TODO 保持未完成。

验证全部 exit 0（Cargo 均经本项目 `nix develop`，target 取 metadata）：

- `cargo check --workspace --all-targets`；针对性 nextest 的 next_line_titles/next_line_definitions/container_definitions/commonmark_reference_definitions 共 53 项。
- `cargo nextest run --workspace --all-targets --no-fail-fast` 及增加 `--release` 的完整验证，各 **1615/1615**。最后补充所有权反例后再次通过最终全套。
- `cargo test --doc --workspace` 共 **75** 项；`cargo doc --workspace --no-deps`；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo fmt --all -- --check`。
- reference_definitions/reference_labels/reference_forms/inline_links/images/block_quotes/list_items/lists/tabs 九组离线生成器 `--check`；`cargo run --release -p mdstream-core --example` 的 container_definitions、stream、source_folding；官方评分单独报告及保护项审计。

完整命令、退出状态、红测与最终日志、状态/复杂度模型、独立观察审阅、最终补丁及 SHA-256 均在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-016/implementation/`；索引 `commands.jsonl`，最终主证据 `check-final-ownership.log`、`targeted-final.log`、`debug-final.log`、`release-final.log`、`doc-tests.log`、`doc-build.log`、`clippy-final.log`、`fmt-final-check.log`、`official-report.log`、`protected-inputs.json`、`final.patch`、`final.patch.sha256`。只交付本项，未提交/推送，交父会话独立审查。

## 2026-11-11 引用定义跨物理行标签交付

完成块起始处跨物理行定义标签及既有完整显式 Quote/List/嵌套/部分 tab 路径。闭合标签同一行紧接冒号，目标在闭合行或紧接下一行，之后可有独立下一行完整 title。真实去前缀标签内容保留 LF/CRLF、缩进与转义，统一限 999 个原始字节，虚拟列不伪造源字节；完整 Unicode folding/空白归一、首定义优先及目标/title 借用 Span 保持。196 的多行 title 正文、510 的跨行行内后缀、完整容器/字面失败回退和长度/窗口放宽仍保留待办，两个宽 TODO 未勾选。

复用已验收的非 Clone 延迟来源账本和生产回流入口，语法仅追加新行数据，不提前发布需要撤销的 LinkId/后续物理叶块/容器/标题。确认消耗全来源并保留首 BlockId/ContainerId；失败先取走候选，再迭代单次回流。共享表头失败时先归还消费权，保留下一行原有 tab/code/lazy 资格；中间表头 delimiter 到来前回流旧普通来源，由真实表格流程确认。闭合标签行含管道但无目标的三列部分 tab 表格、四列目标例外与普通空格原表格优先级分别有测试。pending 固定 source fold、开放段落注册的确认/回退、真实 FoldView.apply/ticket、合并 Update 和共享六字节显示/元数据预算均核验。

新增 9 项集成测试、3 项私有生产管线对照及 208/541 两项官方直验，另更新 3 处直接相关旧观察：待定首块 Growing、首次回流的 3 个原字节、原先未支持的跨行转义标签。4445 组失败/结构/表格/lazy 普通对照、UTF-8 每切点/逐前缀 EOF/字节解码、999/超限及 LF/CRLF、元数据/原始映射和旧对象身份通过。969 层/500 片段成功输入为 486008 个物理字节、999 个标签字节；失败尾部覆盖深度 0/32/128/970 和百万字，随后 1000 次单字节追加仅增加 1000 inline visits，classification/container 计数不增长。两万旧引用与 4096 历史定义后只更新当前符号和区间，重复定义不覆写赢家。每个新物理字节/有界片段只常数次扫描和至多一次普通回流；累计 O(n log n)、空间 O(n)、零运行时依赖及 forbid unsafe 保持。

官方 208/541 直接匹配未改 HTML：定义章为 19 支持/7 刻意差异/1 未支持，标签子集为 8/6/0，其余冻结观察不变。实际普通管线结构对照：208 从普通 1 Block/1 Link 转为定义及正文 2 Block/0 Link；541 从 3 Block/2 Link 转为 3 Block/1 Link，保留正文链接原始范围，定义源码包络分别为 0..14 和 0..18。官方输入/HTML、全部生成器、评分 project 算法和输出编码 helper 未改，离线固定 SHA-256 校验通过。

本单元证据目录：`/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-017/multiline-labels/`。`MODEL.md` 为先于实现的成功/未完成/失败/回流及累计复杂度模型，`commands.jsonl` 保存真实命令、退出状态、完整日志路径与日志哈希，`RESULT.json` 汇总最终验证，`integrity.json`/`patch.diff` 保存源码哈希、受保护文件/投影核验和含父基础的最终补丁。check、定向 nextest、完整 debug/release nextest、doc/docbuild、clippy -D warnings、fmt、相关离线夹具与示例/diff 检查均见该证据。未提交、推送、reset、清理或修改主机/用户配置，交父会话独立验收及提交。


## 2026-11-11 引用定义根作用域多行 title 正文交付

仅交付 TODO `886b346f94f4` 的根作用域窄项：单/多行 label、目标位于闭合 label 行或紧接下一行，单/双引号及括号 title 可在目标同行或独立下一行开启并跨有界物理行。复用 Destination 的转义/括号语法、真实普通结构/表格资格和共享暂存来源消费权；不扩展容器方言、行内后缀、字面失败回退或原长度/窗口限制。宽 TODO 保持未勾选，明确保留 Quote/List/嵌套容器内多行 title 及去前缀元数据来源读取。

同行未闭合 title 确认前不发布定义或幽灵 LinkId；独立 title 失败/EOF 保留已发布目标，仅将暂存 title 来源迭代回普通流程一次。重复定义不覆盖赢家，已有完整/空 title 不吸收后续行。根 title Span 借用真实原源，保留 LF/CRLF、缩进、实体及转义；title 解码游标按原始双字节映射输出归一 LF。暂存片段上限按 `MAX_LABEL_BYTES + MAX_LINK_SUFFIX_BYTES + 4` 证明，合法 4096 字节/超过 1002 条来源不被标签上限误拒；逐行窗口、4096 title/目标与 suffix 上限各自保留。扫描仅处理新字节与一次失败回流，晚定义/补 title 不遍历旧用法，累计 O(n log n)/O(n)、零运行时依赖和 forbid unsafe 保持。

新 title 集成九项、私有实际普通流程 1512 组对照及 Destination 界限测试覆盖 UTF-8 全切点/逐前缀 EOF、入口/Unicode/转义/坏闭合/坏尾缀/空白/结构与真表格中断、首定义所有权、稳定身份、原始/显示切片、共享零/六字节预算、Growing/Sealed、source_fold_limit、固定/开放 source fold、Update 重建/合并、FoldView.apply 和 ticket 失效。百万字失败尾部后逐字追加 1000 字节仅增加 1000 inline 字节，classified/container 计数不增长；两万旧引用及 4096 条历史定义后追加重复多行 title，inline 不增长、tree_visits 增量 41，所有 LinkInfo 保持。

官方 196 直接匹配原 HTML；定义章 **20/7/0**，引用标签 **8/6/0**、引用形式 **17/2/0**，其余冻结观察保持。官方输入/HTML、生成器、project 算法与元数据输出 helper 均未修改，哈希核验见 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-018/implementation/unchanged-oracles.json`。基线核心两项/官方 196 红测与 CRLF 映射红测均保留；旧 next_line_titles 的两种根跨行成功输入移出失败列表，容器对应输入仍验证基线失败。

全部 Cargo 命令在项目 nix develop 中执行，target 来自 cargo metadata。最终 check --workspace --all-targets、针对性 88 项、完整 debug/release nextest 各 1649 项、75 doc、docbuild（RUSTDOCFLAGS=-Dwarnings）、clippy --workspace --all-targets -- -D warnings、fmt、三份相关固定离线夹具、release stream 示例及 diff 检查通过。命令/状态/完整日志、模型/界限证明、源码哈希和最终补丁位于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-018/implementation/`；未提交或推送。容器内多行 title、510、完整回退/容器与窗口放宽继续待办。

### 2026-11-11 task18 容器普通文本基线修复

独立复审发现新增 `Line::DefinitionTitle` 遗漏原普通容器文本资格，导致缺失前缀的 Quote/List lazy 正文错误发布根定义，以及部分 tab 丢失显示空格映射。先保存两项真实失败，再仅在 `containers.rs` 的已有路径 partial、lazy 和新 Quote partial-tab 三处判定加入该变体。根多行 title 实现保持，容器多行 title 仍未交付，宽 TODO 保留。

新增四项集成回归覆盖两最小反例、嵌套/引号/括号/普通控制、UTF-8 全字节切点/逐前缀 EOF、LinkId/BlockId/ContainerId、tab 原始 replacement、共享零/六字节显示预算，以及百万字历史后 1000 次候选的累计工作计数。复用实际基线与原审查探针，600 组快照无差异；新增 1836 组 Quote/List/嵌套、不同 tab 余量、有/无目标、三种 title 定界的整块/逐字/UTF-8 切点及每前缀 EOF 均与真实基线一致；根 288 组入口/解码与 0/1/6 预算/所有权、180 组真实表格观察保持。无新扫描、缓冲、依赖或 unsafe，累计 O(n log n)/O(n) 不变。

本次生产源码修复是重跑完整验证的理由；check、针对性及完整 debug/release nextest、doc/docbuild、clippy、fmt、离线夹具/示例和 diff 的真实命令、退出状态、完整日志，以及新源码哈希、完整/本次补丁统一保存在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-018/fix-container-regressions/`，以 `commands.jsonl` 和 `validation.json` 为准。原 implementation/review 证据未改；保留父工作树全部原改动，未提交/推送。


## 2026-11-11 引用定义容器多行 title 正文无虚拟余量交付

交付宽待办 815e11602d83 的窄项：既有 Quote/List/嵌套显式路径中，单/多行 label、两种目标入口、目标同行或独立下一行开启 title 均可接续多行正文。仅新增正文续行限定去结构前缀后的 `virtual_spaces == 0`，允许前缀内已完全消耗的 tab；label/目标/开启行原部分 tab 能力保持。正文含部分 tab 余量的语义和映射仍未交付，510、完整字面回退/容器及窗口放宽继续保留宽待办。

原始 title 仍借用真实 Source Span 包络，闭合行仅有定界符时的前缀也包含在内。解析时按序追加结构前缀间隙，获胜定义按 SymbolId O(1) 定位，固定大小解码游标只借用索引；按共享预算逐段跳过，保留语义缩进、实体、转义及 LF/CRLF 原源映射，实体产生的 CR/LF 不再归一。title4096 与 suffix 上限独立按去结构前缀后的真实语法字节计数，逐物理行窗口和来源片段上限保持。定义前缀探测返回目标偏移、准确 raw Span 和虚拟列，避免将部分 tab 余量算作字节或丢掉开启行首个真实字符。

同行开放 title 完整合法前不发布可撤销定义/LinkId；独立 title 失败保留目标，仅回流自己的来源。首定义、已有空 title、版本、稳定 BlockId/LinkId/ContainerId/LinkInfo、增量通知、Growing/Sealed、固定/开放 source fold、真实 FoldView.apply/ticket、Update 重建/merge 保持。只保留当前候选或获胜定义的间隙；重复定义不积累持久垃圾，来源单次迭代回流。没有增长候选/旧正文/祖先回扫，没有评分投影补齐，累计 O(n log n)、空间 O(n)、核心零运行时依赖与 forbid unsafe 保持。

先保存真实红测与 148 份完整普通基线快照，最终逐字对照通过；672 组成功模型、768 组真实普通流程失败对照、UTF-8 所有切点/逐前缀 EOF、0/1/6 字节及 query/object 预算、部分 tab 原始映射、title/suffix/逐行窗口、百万字物理包络/失败尾部、两万旧引用和 4096 条定义/重复定义计数均通过。全部 `Line::DefinitionTitle` 直接消费者已按普通文本、lazy、partial-tab、表头及实际语法角色核对，task18 分类修复保持。

项目 `nix develop` 下 check --workspace --all-targets、针对性 nextest、完整 debug/release nextest --workspace --all-targets --no-fail-fast（各 1664 项）、doc（75 项）、RUSTDOCFLAGS=-D warnings docbuild、clippy -D warnings、fmt --check、stream release 示例及 diff 检查通过。固定 SHA-256 官方来源的 27/14/19 例离线生成器逐字核验通过；定义章 20/7/0、引用标签 8/6/0、collapsed/shortcut 17/2/0 及其它冻结观察保持。官方 input/HTML、生成器、project 算法、输出 helper、依赖清单均未改。完整模型、命令/退出状态、日志、源码哈希、基线探针与完整补丁保存在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-019/implementation/`；本会话未提交或推送，由父运行器复审与提交。

## 2026-11-11 引用定义部分 tab 余量的多行 title 正文交付

- 本单元仅交付 TODO `2f0d65b84f7d` 中完整显式 Quote/List/嵌套路径的 title 正文部分 tab 余量。冻结结构虚拟列、真实源字节、title 解码和普通显示四量模型后，红测复现 `> [r]: /u 'a\n>\tb'` 被额外余量过滤拒绝；最小实现移除真实普通文本资格检查之后的 `virtual_spaces == 0` 否决，沿用任务 19 的来源账本、原始包络、前缀 gap 和预算游标，没有增加分类或持久状态。
- 余量 1/2/3 列仍参与普通结构、Code 和表格优先级；支持原子/部分消费多个 tab。已消费 tab 字节属于已知真实结构 gap，raw title 仍借用 Source 包络，decoded 不产生虚拟空格，真实剩余缩进照旧保留。仅闭合定界符行的末尾 gap、物理 CRLF 原子归一、实体生成 CR/LF 不再次解码、两种 title 所有权、首赢家/已有空 title、普通失败 tab replacement 均保持。title/source gap 游标 O(1) 构造、固定大小状态和共享 0/1/6 字节/query/object 契约不变；无旧正文/祖先回扫、旧树克隆或历史用法遍历，累计 O(n log n)、空间 O(n)、核心零运行时依赖和 `forbid(unsafe_code)` 保持。
- 复用原测试并新增三项测试：4032 组独立零/部分余量模型、3840 组真实普通流程失败对照、逐路径精确 gap/余量和 EOF tab 映射、999/1000 label 字节边界、单/多行 label 与两种 target/title 入口、三种定界符、UTF-8 所有切点/逐字符/每前缀 EOF、稳定 BlockId/LinkId/ContainerId/ReferenceInfo、raw/display 切片、共享微预算、真实 Growing/Sealed/source fold/FoldView/Update merge 与 ticket。旧零余量极端测试继续运行，并对部分余量重复验证 4096 title/逐行窗口、百万字包络和失败长尾、1000 单字节追加、两万旧引用和 4096 条定义；Destination 独立目标/title/suffix 上限保持。重复定义的 gap 只保留首赢家，不累积持久垃圾。
- 外部临时探针保存 1760 份真实基线快照：56 份自然变为本项新增成功，其余 1704 份结构、ID、引用、原始/显示映射逐字相同。参考 demo 版本为 0.31.0，只辅助解释虚拟余量不输出；项目真实缩进契约及固定 0.31.2 官方夹具保持。定义章 **20/7/0**、引用标签 **8/6/0**、collapsed/shortcut **17/2/0**，其它独立观察按全库真实报告保持；官方 Markdown/HTML、生成器、评分 project 和输出 helper 均未改。
- 验证全部通过：项目 `nix develop` 内 `cargo check --workspace --all-targets`；针对性 nextest **29/29**；完整 debug/release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1667/1667**；doc **75/75**、docbuild（`RUSTDOCFLAGS=-D warnings`）、clippy `-D warnings`、fmt；六组固定 SHA-256 离线夹具；`container_definitions`、`container_tabs`、`stream` release 示例及 diff 检查。最终源码与测试冻结后完成全套验证，仓库内没有临时驱动；测试不依赖本机 `/home` 或证据目录，也不自动写期望。
- 证据目录：`/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-020/implementation/`。四量模型 `model.md`、红/绿日志、真实基线与对照 `baseline-comparison.json`、最终命令/退出状态及完整日志 `commands.jsonl`、全套结果 `final-suite-result.json`、最终源码哈希和补丁分别保存；未提交或推送。完整字面失败回退、完整容器、510 跨行行内后缀和原长度/窗口放宽继续保留宽 TODO，本项完成不表示整个 TODO 清空。


## 2026-11-11 根段落旧行内表头资格观察基础交付

仅完成 foundation-design 第 11 节步骤 1：同一 Inline 内核的有限无发布 effects、纯 label 规范化、从实际根段落语义消费开始的 RootEligibility 生命周期及独立 Work。合法引用接受与公开 ID 分离，collapsed/full/invalid/overflow 和 inherited label/code/math/header 分支保持。没有第二 Document/引用表/树、旧栈克隆或正文回扫，没有新增依赖或 unsafe。

原 publisher 继续决定表头资格和全部公开发布结果；容器/lazy/partial-tab、ATX、单元格、代码/数学块/脚注不建立 root observer，定义 ledger 释放前不观察。根源按缩进和 eager 第一 pipe 截点单次交付，确认/取消/Setext/新块/EOF 清理。额外一份当前根行内摘要，累计 O(n log n)、O(n)；root_eligibility_bytes/steps 与原发布计数分列，不遍历历史树/引用。

验证包含真实干净 HEAD 7dafb15 的 1552 份完整公开 Snapshot 摘要（source/style/IDs/maps/references/display/objects，含整段、UTF-8 每字节、逐前缀 EOF）、两 backend 逐事件全状态/heads 差分、合法接受标记故障注入红测、真实 root 事件对照、百万字后 1000 追加、两万历史引用和 code fallback 索引计数。旧压力断言不改，官方输入/HTML/生成器/评分/helper 不改。全套验证命令与退出记录见本单元证据 `commands.jsonl`；证据目录 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-021/foundation-implementation/` 保存模型、红/绿、冻结 rlib/探针、完整验证日志、哈希与 patch。

最终验证：`nix develop -c cargo check --workspace --all-targets`；定向 nextest 235/235；`nix develop -c cargo nextest run --workspace --all-targets --no-fail-fast` 与同参数 `--release` 均 1673/1673；`nix develop -c cargo test --workspace --doc` 两组 72+3；`nix develop -c cargo doc --workspace --no-deps`；`nix develop -c cargo clippy --workspace --all-targets -- -D warnings`；`nix develop -c cargo fmt --all -- --check`，均退出 0。7 项离线生成核验、6 个 release 示例、官方 510 原报告、40 个最终公开快照原始文本 diff 及发布 Work 基线 diff 均通过。百万字普通根正文额外 1,000,005 次观察字节/步骤，原树访问仍为 1；8192 组 fallback 的 90,119 字节额外记录 563,305 步，原发布计数逐字段保持。

本次未启用 Destination 新 LF、未接 RootDeferred、未改原表头决策来源；510 仍为基线未支持，跨行链接后缀与完整失败回退的原宽 TODO 保留。复跑定向：`nix develop -c cargo nextest run --lib --test root_eligibility --test code_fallback --test inherited_headers --no-fail-fast`。


## 2026-11-11 根段落行内链接后缀组件换行交付

实际根作用域普通段落的链接/图片后缀在 `(` 后、完整目标后、完整 title 后三个组件空白中，各支持至多一个 LF/CRLF。目标和直接 title 正文各限单物理行，空目标/裸与尖括号目标/平衡括号/三种 title 定界及原长度语义保持。容器、lazy/partial-tab、ATX、表格单元格、代码、数学和脚注不扩展；空白行、真实块、Setext 与表格仍由原结构流程决定。官方 510 原输入直接匹配原 HTML，Links 482–511 为 **22 支持、6 刻意差异、2 未支持**；512–538 **18/4/5**、539–552 **8/6/0**、553–571 **17/2/0**、Images **19/1/2** 保持。原官方 Markdown/HTML、生成器、评分投影和输出 helper 不变。

复用前置 RootEligibility 基础：真实根表头资格持续来自旧单行 grammar，实际 Inline 的新分隔 LF 设置粘滞标志，后续合格行显式使用 RootDeferred；首 pipe、取消/确认/EOF 的来源游标各消费一次。非根与未分歧的 ordinary/eager/opaque 复用保持。观察 effects 不发布或合并，实际成功才原子升级旧 shortcut LinkId、借用 source/target/title Span，原引用及 first_use 保持。没有正文回扫、第二 publisher、对象回滚、历史引用遍历、运行时依赖或 unsafe；累计 O(n log n)、空间 O(n) 与共享预算保持。

父流程明确选择保留新 root 候选内已消费前缀的 opaque 失败方言。`[a](u\n 'a&amp;'x)\n` 当前显示 `a(u\n'a&amp;'x)\n`，EOF `[a](u\n 'a&amp;` 显示 `a(u\n'a&amp;`；旧单行语法在首 LF 拒绝，分别显示 `a(u\n'a&'x)\n` 和 `a(u\n'a&`。这是新增 opaque 范围，不是原基线行为或正确实体解码。已消费 `&#10;`、反斜线转义与代码/强调定界符同样保留原拼写；拒绝字节普通词法一次，其后实体仍正常解码。失败/EOF 保持旧 ID/source/引用 target/空 title，finish sealing；原 LF/CRLF break 映射在成功隐藏后缀前保留，无丢字符。完整字面失败回退、容器后缀、直接 title 正文跨行和其它宽范围需求继续保留 TODO，仅勾选本次窄项。

`root_link_suffixes` 覆盖 3600 组组件模型、分块/逐字/Unicode 切点与逐前缀 EOF、稳定身份/引用/版本/Update、Source/Display 原始切片、真实显示及解码共享 0/1/6 预算、长度上限、折叠与百万字/两万旧对象 Work。24 份非目标与 48+24 份表格冻结 fixture 哈希保持；foundation corpus 仅 `[a](u\n 't')` 及带终止 LF 两个完整新合法输入有独立语义断言，其余基线不改。原失败日志及红绿 chronology 保留。

本轮独立复审发现：根候选已接纳 LF 后，旧 RootDeferred 标志会泄漏到新 Quote/List 的长行表头探测，`new_container_text` 关闭旧段落并删除观察器后触发 expect panic，无需 pipe。最小修复让模式与保留当前根块共用真实作用域条件，不删除 expect、不跳过源、不捕获 panic，也不改变根资格、SourceOwner 双游标或 first-pipe 路径。六个独立反例在修复前全红，修复后通过。新增 `root_suffix_scope` 的 11 项回归含 1260 份 6605b24 冻结对照：840 份未完/失败完整快照，420 份成功新根前缀独立断言升级后核对全部容器字段/绝对映射/身份。Link/Image、LF/CRLF、1023/1024/1025 字节与长行、Quote/List/嵌套/partial-tab/lazy/真实块/新根、无 pipe/有 pipe、分隔确认/失败、UTF-8 每字节/逐字符/逐前缀 EOF、共享 0/1/6 预算及两万旧对象均覆盖。新容器后追加 1000 字节，publisher/summary 各恰计 1000，旧根观察/label/树访问增量为零。

原开发与收尾证据只读保留在本任务 `implementation/` 和 `close-opaque-boundary/`；其中原 1690 runtime 日志不代表本轮修复后的 runtime。修复后在项目 `nix develop` 内重新验证：`cargo check --workspace --all-targets`；定向 nextest **142/142**；完整 debug 与 release `cargo nextest run --workspace --all-targets --no-fail-fast` 各 **1704/1704**；doc **75**、docbuild、clippy `-D warnings`、fmt；五份离线官方 fixture、五份评分报告、五个 release 示例、1260 份作用域快照原始文本 diff 与源码/旧 fixture/原 DONE 前缀核对，均通过。额外运行的 `container_tables` 示例在第 30 行仍用旧折叠父节点期望而退出 101，6605b24 冻结库运行相同原示例也以相同断言退出 101；源码未改，该既有示例问题另交父流程，不扩大本轮修复范围。target 来自 cargo metadata。命令、退出状态、完整日志、模型/consumer audit、全局 source 哈希与补丁见 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-022/fix-scope-transition/`。未提交、未推送，交父流程复审与提交。


## 2026-11-11 根段落直接链接与图片的多行 title 正文交付

仅实际根普通段落的直接链接/图片 title 正文允许多条 LF/CRLF，复用原三定界、组件分隔、稳定 ID/原引用和首次使用信息；目标正文保持单物理行。`Destination::root_byte` 接入既有 title-body 消费与 CRLF 验证，真实缩进及 closing-only 行的空格保留在原始借用 Span，已有 `LinkTitle`/`CodeInfo` 游标原子归一 CRLF 并保留源码映射。label 999、目标/title 4096、suffix 8256 的原始字节界限不变，行内 streaming 不套用定义逐行窗口。

沿用原 RootEligibility 的旧单行 grammar、分歧标志、真实 root_continuation 限定的 RootDeferred 和 SourceOwner 双游标；本次不修改分类枚举/消费者或作用域基础。完整 suffix 才原子升级原 LinkId/Image；失败/EOF 保留已消费 opaque raw 前缀，拒绝字节普通词法一次，不回扫、重放、回滚或补发公开对象。新正文内实体、反斜线、代码/强调和链接标记的失败显示变化有逐例模型；完整字面回退与容器内后缀/title 正文、目标跨行、HTML 和其它宽需求继续保留，不能视为完整 CommonMark 兼容性。

新增 `tests/root_title_bodies.rs` 17 项覆盖 1440 组独立成功模型、三定界/转义/多正文行与三组件组合、LF/CRLF/单独 CR、实体产生 LF、空白/真实块/Setext/table 优先级、UTF-8 每字节/每切点/逐前缀 EOF、raw metadata/SourceDisplay、CodeInfo/目标/title/显示共享 0/1/6 与 query/object 预算、原 ID/ReferenceInfo/首定义/Update 合并、真实 FoldView/open/fixed/ticket/reader、原长度界限、2000 正文行、千次追加/百万尾及两万旧对象/晚定义真实 Work（含资格观察）。累计 O(n log n)、空间 O(n)、零 runtime 依赖与 forbid unsafe 保持，无每 append 硬时延承诺。

真实 `47d95a8` 基线：新正文前缀进入长 Quote/List/嵌套/partial-tab 等路径的 864 份结构/身份/映射冻结对照保持。新增 120 个晚 pipe 完整输入中，88 个全快照保持，12 个新合法 title、20 个 opaque 失败变化均按精确输入独立断言 metadata/display，全部表格决策仍与原 HEAD 对照。历史 48+24 表格、1260 scope、资格 corpus 和原 fixture 逐字节保持；旧 24 非目标只有 `[a](u 't\nrest')\n` 是新合法输入，独立更新预期，另三个旧 negative 输入 `[r](/url 'a\nb')`、对应图片及 1024 字节段落后的 `[a](url "two\nlines")` 保留输入并明确断言新对象/显示。没有泛化跳过 LF 或批量采用当前输出作 oracle。

官方原 Markdown/HTML、生成器、评分 project/helper 均未改；Links 四子集 22/6/2、18/4/5、8/6/0、17/2/0 和 Images 19/1/2 的独立观察及分类保持。README 当前概览/详细/解码/评分/组件换行节已同步，并增加根 title 正文节；TODO 仅新增本窄项完成标记，宽 TODO 保留。

最终验证全在项目 `nix develop` 内运行：`cargo check --workspace --all-targets`；专项 nextest 219/219；`cargo nextest run --workspace --all-targets --no-fail-fast` debug 1721/1721、release 1721/1721；`cargo test --workspace --doc`；`cargo doc --workspace --no-deps`；`cargo clippy --workspace --all-targets -- -D warnings`；`cargo fmt --all -- --check`。8 份固定 spec 离线生成器 --check、官方报告、container_tables/link_suffix_headers/stream 三示例、工作计数专项和 diff 检查均通过。初始红测/旧 negative 发现与修正、完整命令/退出码/日志、模型/全 source 差异/冻结源码哈希和 patch 均存 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-024/implementation/`，索引 `commands.jsonl`、`validation.json`；未提交或推送，交父流程独立验收。


## 2026-11-11 容器普通段落旧语法资格观察基础交付

仅交付容器普通逻辑段落的无语法变化观察基础。私有 Eligibility 共用现有无发布 Inline 内核，保存首 leaf BlockId 与精确 ContainerId 的唯一活动 key；同 scope 物理续行保留实例，blank、父 scope 返回、兄弟/嵌套、块中断、成功 definition、Setext/table/EOF 沿原生命周期退役。原 root 行为与计费保留，容器 can_isolate/header_crossed 和结构决定仍由真实 publisher 驱动。

三处实际 virtual 生产点和 payload、streaming/eager header、definition fallback、isolate/cancel/confirm/finish 全部镜像；raw cursor 与真实 parsed_end 独立，最后 virtual tab offset 对整组1–3列去重，不猜结构 prefix 或预扫未交付源。SourceOwner/expect 及原 first-cell reuse 保留，observer 不转入 RowParser，不发布/合并 effects 或访问历史引用/公开 ID。新增 Work.container_eligibility_bytes/steps 独立记录实际 raw/virtual 访问与局部状态工作。无新增运行时依赖、unsafe、旧正文/祖先回扫或第二文档，累计 O(n log n) 时间、O(n) 空间。

新增七项测试：6592 份真实 7da6c27 的完整公开快照与全部旧 Work 字段冻结对照；2700 组真实 producer header 路由逐事件核对全部活动语法状态、完整 Destination 状态与 reuse；活动首 leaf/scope 生命周期、definition ledger 零提前观察、fallback 首次消费、virtual-only/整组去重、公开 ParagraphFoldCandidate/原始映射，以及500层/百万字/两万旧引用后1000次单字节追加精确新增1000次观察。闲置 label scratch 仅在无任何消费者时不作为活动语法比较；活动 label 全字节比较。既有 root、Update merge/rebuild、sourceFold/open/fixed/ticket 和共享0/1/6预算测试保持。

验证均在项目 nix develop：check --workspace --all-targets；针对性 nextest 216项；完整 debug/release nextest --workspace --all-targets --no-fail-fast 各1728项；doc75项；cargo doc --workspace --no-deps；clippy --workspace --all-targets -- -D warnings；fmt；7组固定 CommonMark 离线夹具、9个相关 release示例及 git diff --check 全部通过。完整命令/退出状态/日志、模型、全局源码哈希和补丁在 /home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-025/implementation/。静态 oracle 随测试提交候选，不依赖证据目录或自动生成期望。

容器后缀组件 gap、直接 title 正文 LF/CRLF、完整字面失败回退和 prefix 预算反例均未扩展；官方 Markdown/HTML、评分/projection/helper 和历史 fixture 原样保留。TODO 的40项真实未完需求保留。本单元未提交、推送或派生代理，后续 B/C 由父流程另行安排。


## 2026-11-11 容器普通段落后缀组件换行交付

- 仅 B：真实普通容器链接/图片的三个组件 gap 各至多一个 LF/CRLF，target/title 正文保持单物理行；root 多行 title 模式及 cell/ATX/code/math/footnote/definition 边界保持。容器 title 正文 C、完整字面失败回退、完整容器/嵌套图片和其它宽 TODO 保留。
- 独立 component 模式复用原 Destination；现场结构 prefix 单独从 suffix8256 扣除，title4096 计数不减历史 prefix，virtual 为零 metadata 源字节。首 leaf+精确 scope 绑定 candidate/observer/Inline；旧 Eligibility 决定表头资格，Deferred 不提前 feed/isolate 实际解析器，旧 key 下唯一 SourceOwner 释放 raw/virtual 一次，确认 table 使用独立 cell。稳定 LinkId/引用 first_use/revision、失败/EOF 已消费 opaque raw 方言和拒绝字节正常词法保持，不宣称完整 fallback/实体解码成功。
- 18,000 组组件模型；真实 quote/list/nested/lazy/partial-tab、1023/1024/1025/late-pipe、star/underscore/tilde/close-only/code/math/label 角色矩阵；原 observer 分歧后语义独立核对；UTF-8 每切点/逐前缀 EOF；4095/4096/4097 与8256结构前缀反例、稳定元数据/原始映射、共享0/1/6微预算、Update apply/merge与真实Fold/open/fixed/ticket/reader、百万字/两万旧对象/1000追加计数覆盖。
- 原静态 oracle、root fixture 和官方 Markdown/HTML 原字节保留。A corpus 的66个精确完整输入中32个公开 Direct 新成功，34个公开全快照不变而仅工作计数改变；root旧negative仅4个精确容器输入、scope corpus仅60个精确容器新成功独立验证 typed metadata/display，未泛化按LF或前缀跳过。源码累计 O(n log n)/O(n)，无新增依赖/unsafe/旧正文、旧引用或祖先回扫。
- 验证命令均在本项目 nix develop：check --workspace --all-targets；针对性nextest；debug/release各1743项 --workspace --all-targets --no-fail-fast；doc 75项、doc build、clippy -D warnings、fmt、8组离线官方夹具和相关示例。完整命令、退出状态、日志、冻结源码hash及patch：`/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-026/implementation/`。本单元不提交，由父独立审查。

## 2026-11-11 容器普通段落直接链接与图片的多行 title 正文交付

仅 task027 C：已有普通容器逻辑段落的直接 title 接纳多条 LF/CRLF，三定界、转义复位、真实 indent/tab、closing-only 与原始 Span 保持。候选自持有有序结构 gaps，延迟 header 同结果移动；合法 `)` 才一次移动到原稳定 LinkId 平行私有槽，pending/failure/EOF/table 丢弃不污染旧引用或后续候选。全部 complete_link、autolink、新建和原位升级入口维护空槽；Direct decoded reader 选择该槽，Reference 保留 winning definition gaps，Email 无 title。未改 key、SourceOwner、classification、observer/Deferred 协议、官方输入/HTML/评分/helper 或原静态 fixture。

结构 prefix 只由真实消费生产者现场通知一次，title/suffix 各自扣结构源，partial-tab 虚拟列保持原 Work/previous 更新但不送 title 正文。title4096 计真实正文与 CRLF 两字节，target4096/suffix8256 及原窗口保持。每非空正文 prefix 加一次 Work.link_title_gap_steps，摊销 push 与发布 move 不访问旧正文/引用/祖先/gaps；空 Vec 无独立堆分配，私有存储为每 LinkId 一个 Vec header 加 Span capacity。累计 O(n log n)/O(n)，核心零运行时依赖且禁止 unsafe。

新增13项集成测试及1项私有存储测试：132组独立 source/typed metadata/decoded chunk 映射，LF/CRLF/三定界/Link/Image/lazy/partial-tab1–3/真实 indent 与 tab，全部 UTF-8 切点及前缀 EOF；6592份前驱容器 corpus 仅5个完整新合法输入改变，946份角色/EOF和100份窗口/late-pipe全快照与旧 Work 冻结仅按完整源名单扩展独立断言。depth1000 的真实前驱与新实现同为512个结构 Quote、同 Container511；后488对为正文而非结构，独立模型精确保留 decoded/source 映射。保留旧 table/Setext 身份、引用 first_use/revision、原子升级、真实 Update apply/merge/rebuild、Fold/sourceFold/open/fixed/ticket/DisplayCursor stale；原读者借用由 compile_fail 例核验。共享 CodeInfo/title/destination/display 的0/1/6与query/object预算、末尾免费完成、gap内暂停、CRLF/UTF-8原子与实体probe恢复均覆盖。两万旧定义+百万正文后1000单字节追加保持固定新增工作，1000行title成功且gaps只随新prefix增长。

失败域沿用已验收 opaque raw：已消费实体/转义/强调不回放，拒绝字节及后续正常词法一次。完整字面失败回退、target正文换行、嵌套图片、完整引用定义和其它宽需求仍留 TODO；本窄项不代表 TODO 清空。README当前规则和对应TODO同步，DONE只追加。

验证命令、退出状态、完整日志、源码hash与patch保存在 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-027/implementation/`。项目 `nix develop` 内 check --workspace --all-targets、针对性 nextest、完整 debug/release nextest --workspace --all-targets --no-fail-fast、doc test/doc build、clippy -D warnings、fmt、离线fixture及相关示例/diff为最终门禁。父侧 depth/window、gap结束和计费反馈已按真实前驱探针处理；未提交/推送，交父独立验收。

补充全字段计数审计：历史 hash helper 既有排除 container_eligibility 字段的投影保持；另外冻结并检查完整旧 Work（仅排除本次新字段），100份表头 corpus 中8个精确完整输入的 deferred 确认各多一个 observer 步骤、字节不变且公开快照/结构完全保持，单列完整 source 名单；其余新增差异仍是19个直接 title 成功。外部6592+946份全字段前驱对照中，observer计数全部保持。证据40-all-old-work及最终全源码hash对应测试可独立复跑，不整体重录旧 Work/oracle。


## 2026-11-11 原子闭合的失败直接链接后缀恢复交付

仅 task028 implementation：所有实际 Publishing Inline 的 Tail::Destination 失败/EOF，记录 Progress::Pending 实际 raw visits 的不可 Clone Span RLE。先完整验证闭合 entity/escape 与其它语法排除域，合格才第二遍通过原 Effects.entity/hide 恢复；开头括号、当前拒绝字节、结构 prefix/virtual 不录入，不扫描物理包络或普通 lexer，不改变成功 grammar、label 括号、break/空白方言与非目标语法。保持旧 LinkId/Reference/first_use/Sealed、title gaps、key/SourceOwner/observer 协议及 first-cell effect guard。无发布观察器不建 trace；成功、失格、EOF 与 header discard 释放容量。

新增 Work.link_fallback_bytes 计实际恢复读取，link_fallback_steps 计 raw 录入/合并与扫描 span。A 为真实发布 raw visits，录入≤A、恢复字节≤2A、扫描 span≤2A，每条 trace 的 span≤数据字节≤8256；不存在嵌套重放或增长历史扫描。404414字节/2000行/100层 Quote 的物理包络实测4014 inline visits，只恢复4010个raw字节。重叠 [a]( 链的第一个未转义 [ 使整个 trace 失格，不创建内层候选；拒绝点以后的新候选仍由原普通字节路径产生，trace不重叠。累计 O(n log n)/O(n)，核心仍零运行时依赖、禁止 unsafe。

前驱 lib 实际红测11个目标全失败，159个非目标完整 Snapshot 冻结通过，探针/静态oracle生成在外部证据，普通测试只读取portable fixture。独立三定界/Link/Image/行尾/Unicode/实体/转义模型、每UTF-8切点与逐前缀EOF、source/scalar/replacementSpan、先整体验证的反例、0/1/6/query/object及既有Direct/Reference/CodeInfo共享预算、稳定引用/首定义、真实Fold/Update合并/rebuild/open/fixed/ticket/stale覆盖。新增私有RLE/释放/observer及DisplayIndex源序/无重复测试，逐事件仅排除trace元数据继续比较全部活动grammar。真实table确认/取消/首pipe/继承label guard、原窗口/容器/定义/Setext等6592旧corpus与原fixtures保留；旧raw断言只扩展完整合格输入，不按含LF豁免。8256与跨cap半实体、40万字prefix、百万正文/两万旧定义和连续成功/失格计数覆盖。

README当前说明、对应TODO及示例同步；完整普通lexer回退、未解析括号、Tail::Label、其它行内语法、target正文换行、嵌套图片和宽泛标准需求仍保留。官方输入/HTML/评分helper不改，原8例及22/6/2分类保持；本项不等于完整literal fallback或整个TODO完成。

全部命令、退出状态、完整日志、前驱/冻结源码hash和patch保存于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-028/implementation/`（commands.jsonl）。Rust skill与父反馈已按要求核验；最终门禁结果另随本节补记，不提交/推送，由父新审查会话验收。

最终门禁均退出0：check全目标；针对性183/183及最终相关124/124；完整debug与release各1782/1782；doc76、docbuild、clippy全目标-D warnings、fmt --check；8组离线CommonMark夹具（inline_links/images/link_labels/reference_forms/reference_labels/reference_definitions/entities/tabs）、4个相关release示例及diff检查。8个官方失败输入/HTML/真实Snapshot与前驱逐字相同，外部2000/5000/10000层重叠链对照也相同（official-diff-033.log为空）。原所有fixture保持，未新增运行时依赖或改评分/observer协议；最终源码与验证冻结一致，证据索引含真实开发期红测与修正日志，无覆盖日志。


## 2026-11-12 未解析 shortcut 边界与晚定义可见性交付

仅已发布纯 shortcut Link/Image 的真实 `[]`/`!` marker 改为私有 `Mark.conditional_hide: Option<SymbolId>`；原符号 destination 未定义时可见，首个获胜定义后隐藏。永久 HIDDEN 优先，完整 Direct/full/collapsed 继续在原 ID 上永久覆盖旧 gate，不扩展完整引用选择。label 样式/深度、原始 Span、候选 Image metadata、SourceSearch 和 raw summary 保持；公共 Runs/scratch/display/raw LayoutPass 与 next_visible/next_drawable 一致。winner 且 first_use 存在时新增同范围 Visibility，确保 references_affect_size=false 也失效旧 ticket；duplicate/title-only 不新增该历史可见性事件。

旧 rlib 的121份前驱快照逐字节复核；915份扩大 corpus 在实现前冻结，以独立 typed marker/metadata 和逐 scalar 字段比较限制差异。八类 hard LF/CRLF、全空格 code/math 与 base-visible label 证明落实私有查询计数，另有60,000随机 label、UTF-8/EOF、微预算、搜索/摘要/heading/table、Update 合并/重放、策略/缓存/stale、真实固定折叠、百万字追加和两万旧对象回归。旧/新解析 Work 实测逐字节相同；x86_64 Mark 56→72字节、Frame 88→104字节，无 boundary/uses 索引或历史回扫，累计 O(n log n)/O(n) 保持。

原99份 fixture/生成脚本逐字保留，七个评分 project 原字节不变。父反馈要求恢复的旧 rs oracle 与八份完整 txt 派生副本均已归档；仓库恢复原 include_str 回退，仅用精确 case/end/mode 或夹具/索引/完整输入 hash/旧行 hash 的增量表覆盖 shortcut 目标，其余继续验证旧值。普通测试不写期望、不依赖本机路径。

最终 check --workspace --all-targets、针对性 nextest、完整 debug/release nextest --workspace --all-targets --no-fail-fast（各1796通过）、doc（76通过）、doc build、clippy -D warnings、fmt、七项离线夹具和四个相关 release 示例均 exit0；git diff --check 通过。完整命令/退出状态/唯一日志与父反馈处理证据见 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-029/implementation/commands.jsonl`、`consumer-audit.md`、`final-oracle-integrity.json` 和 `final-validation.json`。README/TODO 仅同步本 shortcut 窄项及直接受影响的当前统计；完整 full/collapsed 回选、失败 lexer、HTML、嵌套图片、窗口与其它宽需求继续保留。未提交或推送。


## 2026-11-12 已有 shortcut 对象的未解析完整引用边界交付

仅 `Publishing.complete_link` 入参已有合法 shortcut `candidate.link` 的 full/collapsed Reference 完整升级恢复原 `[]`/`!` 与整个 opaque suffix 的字面边界；当前目标首赢后隐藏。私有 gate owner 改为稳定 LinkId，已解析 a→未解析 b 仍复用原身份，Direct/Email 与永久 HIDDEN 先短路且不读取旧符号。标签的样式/深度/link/raw mapping、原 LinkInfo 源域/first_use/title gaps、通知顺序与 dirty 范围保持；不改 grammar、observer、SourceOwner、ParagraphKey 或评分投影，不新增公共字段、依赖、unsafe、uses 索引或动态 hidden summary。

范围标注和 successor 沿用区间树 O(log n)；已有 shortcut 标签提供 base-visible witness，空/空白/超限/非法 shortcut 标签的完整 full 保持永久隐藏与聚合跳过。累计 O(n log n)、空间 O(n)、原共享预算及 scratch 渐进空间保持。`[][b]`、`![][b]`、空 collapsed、失败 lexer、HTML、嵌套及 full 目标回选均未扩展；宽 TODO 保留。569/571 仍消费 `[bar]` 并选择 bar，不重作下一标签；545 的 opaque suffix 保留原始反斜线。官方输入/HTML/生成器/project 均未改，相关分类仍为 539–552 13/1/0、553–571 17/2/0、Images 20/0/2（支持/刻意差异/未支持）。

新实际前驱小 baseline 为 40 例；524 个 UTF-8 split 与 524 个 prefix EOF 对照、六例四阶段的 208 个独立 typed scalar 真值覆盖 full/collapsed/image/Unicode/escape/opaque。原 root/container 8,144 行与 shortcut 915 行经独立 scalar/结构比较，只接受资格语法 HIDDEN 与自然 run 合并差异；完整 resolved、Direct 与非目标精确快照保持。旧 fixtures（含 task29 大 baseline）、数据、生成器和 task29 增量文件共 106 个原文件保持字节，只加 source/key/oldhash 精确增量。普通测试不写预期、不依赖本机或证据路径；外部冻结/诊断 driver 仅在证据目录。

新增与复用回归覆盖定义先后/重复/title-only、同 ID 三类升级、UTF-8/EOF、999/1000 原标签和 suffix、容器/真实表头确认失败/独立 cell、source mapping、raw/scratch/display、Layout/Fold、search/heading/excerpt/table/anchors、共用 0/1/6 和零 query/object/dependency、marker-only/suffix-only tile 当前 b 依赖、全 false policy 旧 ticket 与 stale 零消耗拒绝。私有节点/gate/link/symbol 计数覆盖真实 999 字节后缀、全空格 code/math/hard LF/CRLF witness、两万不同 owner、百万字追加、空 full 聚合短路及不以 999 为常数的百万字合成区间；晚 winner mutation 不随历史 uses 数线性增长。

本轮新功能验证（全部经项目 nix develop，target 来自 cargo metadata）：check --workspace --all-targets；针对性 nextest 48/48 与私有计数/cache 12/12；完整 debug/release nextest --workspace --all-targets --no-fail-fast 各 1808/1808；doc 76/76、docbuild、clippy -D warnings、fmt 均通过。9 项 CommonMark 与 3 项 GFM 离线生成器 --check、10 份独立分类报告，以及 container_links/link_headers/closed_link_headers/link_suffix_headers/inherited_headers/display 六个 release examples 通过。最终全套前冻结 463 个项目文件（386 个 Rust 文件），全套后只有本 DONE 追加，源码哈希保持。

证据目录：`/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-030/implementation`。`commands.jsonl` 保存真实 argv/退出码/完整日志路径/SHA；`source-frozen.json`、`source-verification.json` 与 `validation.json` 关联源码及最终结果。早期红测/编译/夹具失败保留；初期日志命名冲突的 007/010 明确排除验收，已修复为原子预留，最终全套日志唯一。父设计验收仅作为前驱，本轮日志另证实现。未提交、推送、reset、清理、派生代理或执行下一项。


## 2026-11-12 稳定根起点的手工文档尾折叠范围自动增长

本单实现与验证完成，待父独立复审确认提交。`SourceFoldRegistration::tail` 从 `start < source_fold_limit()` 的 UTF-8 根切口登记至真实 EOF；付费拒绝任何语法折叠包络内起点，手工登记仍不相交。单 live 槽在有效 apply 更新真实 registry、summary、Mask 和 HeightIndex，包含未来分类内容，保留 view-local SourceFoldId、意愿和预览；不修改正文、SourceOwner 或语法父链，不回扫历史/正文/祖先。新增同步工作为固定次数 O(log n) 索引操作，累计 O(n log n)、空间 O(n)，核心依赖与 unsafe 约束保持。

`finish` 自动同 ID 封口；有预算 `seal_tail` 仅对本视图 live ID 接受稳定 EOF，再跑固定两端拓扑校验，内部开放结构可能使其失败。预算耗尽可续，失败不修改视图；成功复用 ID、关闭 live 槽并失效布局依赖，保留 saved choice，随后按固定 EOF 混合 tile 一次刷新。移除清登记、意愿、预览与槽且 ID 不复用；reset 保留注册。开放表格行在 live 根尾内可双向顺序登记，固定/段落校验不放宽。

复用 source folding 的独立区间/高度/布局模型，新增14项测试，覆盖8组 corpus 的 UTF-8 每切点/前缀 EOF、Setext/晚表头/definition 确认与回放/多行 inline/CRLF/pending finish、真实语法父链与原始/显示映射、0/1/6及零 query/object 的共享预算、条件封口、跨视图/错误文档/乱序/merged/replay、批量/预览/导航/锚点、ticket/checkpoint/pass/cursor 失效和前方依赖保留。16/256/4096 历史登记下8次追加各新增 range_visits 恰1，Height/generation/checkpoint 访问受常数乘 log 上界；百万字尾只保留一个 owner 占位，不逐 tile 布局、无 paragraph_end。千层输入的 seal 实际访问一个已闭合 Code 后被首个开放 Container 拒绝（5个付费步骤），不将请求嵌套深度冒充 validator 访问深度。

最终均经项目 `nix develop`：`cargo check --workspace --all-targets`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo fmt --all -- --check` 通过；针对性 nextest 230/230；`cargo nextest run --workspace --all-targets --no-fail-fast` 与 `--release` 各1822/1822；`cargo test --workspace --doc` 76/76，`cargo doc --workspace --no-deps` 通过。13个相关 release 示例通过，包括新增 `source_tail_folding`，以及 source/folding/batch/paragraph/container/auto/rows/budget/display/excerpt/stream。58份既有 fixture 与 HEAD 原字节一致，`git diff --check` 通过。新测试不依赖本机路径或外部证据，不写期望；完整验证前已冻结源码，验证后仅文档同步。

按既有“语法父链 + 手工叠加”语义，已准备勾选“稳定身份与范围查询”供父复审；正式确认仍由父验收提交。宽项“手工范围扩展”保持未勾选：任意未定型/语法宿主内起点、一般手工嵌套、交叉拆分/优先级、重载身份映射及 UI/软换行/媒体仍未完成。


## 2026-11-12 每个视图独立折叠状态完整性审计

逐项审计前驱 `d98b33721f61966219121350777ca52a0887d5f3` 的当前实现，未发现需要修复的运行时缺口；本项只更新 README/TODO 并追加本记录，生产代码、既有测试、fixtures/data/scripts 均保持原字节。已准备将“每个视图独立保存状态”唯一 checkbox 勾选，39→38，待父独立复审、接受并提交后正式确认，其它 38 条未完成要求保持。

当前七族 Heading/Code/Math/Table/Container/TableRows/Source 均保存视图本地选择，实际遮蔽与祖先/Source 叠加独立；父收起再展开不改子选择，Subtree 显式递归改写。覆盖根/容器内标题与代码/数学/表格、Quote/List/Item/任务、固定行号，以及 Source 固定、live 根尾、封口、已封闭/开放顶层与容器段落、自动长度注册。语法默认值只初始化后续 new_* 首次识别，Setext/表格晚确认不漏；改变策略不覆盖既有默认/explicit 选择，双视图策略互不影响。Rows 与手工 Source 初始展开，自动段落只在预算任务准入时收起；段落改型/余文消失撤销旧注册，不把选择移交给新语法。Source 保留最内层包含语法父及原 Subtree/Focus 范围语义，不建几何父树。预览中的显式/批量修改更新 saved choice，reset/resize 保留默认值、注册、选择与预览，退出按当前范围恢复。

本项新运行：复用 35 项判别力测试，debug/release 各 35/35（各跳过 1787，非新 full）；仓库外独立区间联合/意愿模型在 debug/release 各 6/6，覆盖七族 11 候选的全部 2048 种选择、跨族 preview/toggle/reset/Subtree、两视图逐 Unicode 字符新增/晚确认与策略切换、各 Source 注册模式及自动段落准入。一次外部 probe 错把跨两个兄弟块的 Source 父预期为首个 Code，按现有“包含整个包络的最内层语法节点”文档修正为内层 Heading；初始源码、失败/修正日志均保留，未改生产实现或旧 oracle。

2001 轮七族操作在 392 tiles 上，高度/依赖/检查点 arena 的节点数、capacity_bytes 和 allocation_calls 在预热后稳定；失效检查点 payload 实测每缓存位置一份，drop 后全部释放，Measurement 为固定 80 字节 Copy 值，未由视图保存历史。另在仓库外逐字复制源码、仅追加两个只读统计 getter，2001 轮注册/移除、toggle/preview/reset 后各实际 map/set 长度、容器/章节 Vec 长度与容量稳定；结束时注册/收起/预览均为空，稀疏索引分别保留 len/capacity=6/8、3/4。保留容量及每位置旧 checkpoint payload 与逐操作历史快照分别记账；未将逻辑 count 代替实际历史容器审计。

此前完整门禁沿用父已接受的 task031 `parent-acceptance.json`：debug/release 各1822、doc76、13个示例；这些是前驱结果，本项未重复 full/doc/check/clippy/fmt/docbuild。项目 `nix develop` 与 `cargo metadata` 定位真实 target；完整 argv/cwd/退出码/唯一 O_EXCL 日志及 SHA、合同映射、最终源文件冻结与受保护文件校验位于 `/home/ajen/.local/state/mdtui3-todo/jwnujuz2/task-032/implementation/` 的 `audit.md`、`validation.json`。实际生产字节未改，零依赖、forbid unsafe、O(n log n)/O(n)、共享 Budget 及无历史/子树初始化扫描保持。

本条覆盖全部已实现折叠候选；媒体/图片详情、显示行数/字素及对象截断、未支持容器语法、一般手工嵌套/交叉/reload 和 UI 仍由原后续条目约束，没有以新空实现或移走需求清空待办。未提交、推送、派生代理或执行下一项。

## 2026-11-12 GFM 裸自动链接核心交付

交付 `www.`、HTTP(S)、email、`mailto:`/`xmpp:` 的单遍候选、域名及尾部边界，并接入显示、目标读取、视口对象和共享预算。`LinkTarget::Www` 保存原始地址，`link_url` 补 `http://`；原始目标与有预算读取保持真实源码映射。裸候选无长度上限，尖括号自动链接的原 4096 字节上限保留。代码、数学、显式标签及既有容器方言继续规定各自优先级。

URL 在封口时前向验证完整主机名，再处理路径尾标点、括号和实体形后缀；`www.foo` 已满足一个点号要求，末两段域名的下划线不能先裁掉再判有效。email、mailto 与 xmpp 共用强调后的文本节点边界；未配对的 local 下划线保持字面。候选依赖的 opener 只登记一次 owner，配对/丢弃后一次结算；候选外、跨行及显式/lazy 容器的晚闭合不改写已发布目标。受晚表头暂存配对影响的地址等待确认/失败决定，恢复正文与独立单元格分别处理。协议前缀、资源、失败多 @ 恢复均有独立期望和增长计数。

迟确认地址可晚于后方对象分配 ID。`links()` 保留稳定分配顺序，源码顺序由独立 BTreeMap 提供给 `links_in`、`next_drawable` 和 LayoutPass；对象交付的后继查询计入原预算。新 `Work.bare_autolink_bytes` / `bare_eligibility_bytes` 独立记录额外工作，保留旧工作计数与缓存、dirty 范围、continuation 和身份契约。每个字节/摘要/opener/候选常数次处理，有序索引操作 O(log n)，累计解析 O(n log n)、空间 O(n)，零运行时依赖、无 unsafe，不遍历旧引用使用点。

固定 GFM 0.29-gfm 622–635 全部匹配原 HTML，评分 **14/0/0**，保留 945 切点、931 前缀及 EOF。启用 GFM 扩展后 CommonMark 608/611/612 如实记为方言差异，594–612 为 **15/4/0**；官方 Markdown、HTML、生成器及固定来源保持。仓库外编译官方 cmark-gfm `.0` 和 `.13`，规范与实现差异及采用依据见 README；协议以固定 GFM 6.9/633–635 和支持该分支的 `.13` 为依据。63 组基本边界对照、270 组域名模型、混合样式、UTF-8 任意字节分块/逐前缀/EOF、长表头、跨 tile 映射、0/1/6 预算、4096 逆序晚闭合、百万字增长及大量历史对象均纳入回归。

最终代码验证：项目 `nix develop` 下 `cargo check --all-targets`、专项 **63/63**、完整 debug/release `cargo nextest run --no-fail-fast` 各 **1843/1843**、`cargo test --doc` **76/76**，fmt 与 diff 检查通过。15 个 Rust 文件与冻结哈希一致。日志及验收证据见 `/tmp/mdtui3-codex-todo/001-gate-debug.log`、`001-root-gate-release.log`、`001-root-gate-doc.log`、`001-root-gate-accepted.json` 和 `001-gate-source.sha256`；中间版本通过数不充作最终验收。

保留的失败方言：失败 URL 候选只由普通行内机消费一次，不重新发现其内部另一个裸地址；例如 `http://foo_bar@bar.com` 的 email 恢复归入现有完整失败回退待办，相关红测已单独保留供后续任务。完整链接/图片嵌套、其它字面失败回退、分类窗口扩展及原始 HTML不由本项交付。TODO 仅完成裸自动链接核心，未完成要求保持。
