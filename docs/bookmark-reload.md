# 重载后的书签：显式宿主映射设计与可执行原型

2026-11-19；正式输入 `82e229afd04b81b845b882943a8b2fc39182596c`。本设计交付原 TODO `b22e77d:184/199` 留下的“重载后的书签持久化映射另行设计”。决定：**现有 API 足够，采用宿主持久身份 + 单点显式映射 + 新 Bookmark 的组合，不新增核心 API。** 交付的是设计和可执行宿主原型；没有部署磁盘存储、自动 reload、全文 diff 或新 UI。

## 已有边界及选择

`history.rs` 中的 Bookmark / FoldAnchor 属于一个 Document。`Bookmark::navigation(new_document, ...)` 必须继续返回 `WrongDocument`，即使新旧内容完全相同。`target()` 中的 HeadingId / BlockId / TableId 等只是捕获时的文档内元数据，既不能跨重载寻找同名对象，也不能当作持久身份。恢复必须通过 `Bookmark::new(new_document, mapped_source, saved_viewport_offset)` 重新捕获合法的新身份和目标元数据。

`SourceFoldSnapshot` / `SourceFoldRestore` 已解决手工折叠的显式宿主恢复：调用方保证宿主对应及相对切口有意义，核心检查新宿主类型、范围、UTF-8 和注册规则；它不推断内容对应关系。这里复用这个责任边界和既有 Bookmark / FoldNavigation 能力。折叠快照保存范围与意愿，书签保存一个精确点和视口偏移，不能用恢复手工折叠代替书签设计，也不应为一个点虚构 SourceFold 注册。

| 方案 | 前提和决定 |
| --- | --- |
| 相同内容版本，直接沿用 source | 宿主可靠证明字节版本相同后，可显式给出恒等点映射。只比较路径、长度、标题或 Document 内 ID 不足以证明；本原型仍走单点映射流程。 |
| 稳定宿主对象键 + 相对字节偏移 | 若原对象到新对象的对应和相对切口仍有意义，宿主换算为新绝对 source；复用 SourceFoldRestore 的前提。对象内部编辑后必须另有偏移映射证据。 |
| 编辑器变更日志 / 既有 source-map / 版本系统映射 | 选定的通用接入边界。宿主对请求的旧点提供唯一的新点，或明确给出未完成/无法映射结果。核心不负责产生这个证据。 |
| 自动全文 diff、最近标题或同名标题搜索 | 本次不采用。成本、歧义及语义未由原文约定；不能把猜测包装成精确恢复。若产品以后提供近似跳转，必须另行标明近似，由宿主决定是否接受。 |

## 最小合同

持久记录只含 `(resource_key, content_version, source_byte_offset, viewport_offset)`。资源键表示宿主中的原资源，不仅是可能复用的路径；内容版本是该资源的确切字节快照身份。可以使用宿主已有的不可复用版本令牌或可靠指纹。计算、验证指纹的读取成本属于宿主，不能在恢复构造器内免费扫描全文。重命名是否仍为同一资源由宿主明确决定。

示例的 `Version { resource: u64, content: u64 }` 是可信宿主令牌，**不是计算出来的 hash**。同一版本不能指代不同字节；相同长度、相同 source 的改文也必须换版本。原型不能识别宿主谎报或误绑定的同版本内容，这与 SourceFoldRestore 无法识别同类型同长度错误宿主的限制一致。生产宿主必须用自己的加载/快照机制建立可信绑定。

`Binding` 临时绑定资源版本到当前 `DocumentId + revision`，不持久化。捕获时校验旧 Bookmark 的 owner 和旧绑定，再提取精确 source 与 offset；不能拿另一个 Document 的书签贴到当前资源键上。恢复时校验资源、映射的旧版本/旧点、新版本以及目标 Document。绑定不保存 Document、renderer、折叠树或正文。

持久层的整数编码、溢出/解码验证和文件格式不在本次范围内。原型用 usize 表示内存中的 source；实际从持久数据解码时宿主须检查它能转换为目标平台 usize。保存的是 UTF-8 字节位置，遵循当前 Bookmark 的 `is_char_boundary` 策略，允许空输入 0 和 EOF；不额外要求 EGC、词或标题边界。

## 单条恢复流程与错误归属

1. 选定一条 Saved，建立新 Document 的可信 Binding，申请这一个旧点的宿主映射任务。构造只复制固定大小元数据和借用任务，不扫描书签历史、映射表或正文。
2. 每次推进先用现有 `view.layout_parts_mut(document)` 只校验视图 owner/已同步状态，不修改取得的索引；错误视图在消费映射步骤或 Budget 前被拒绝。同一 Document 的具体 view 在创建导航时才绑定。随后在宿主预算下推进 mapper。结果必须同时绑定旧资源/版本/点和新资源/版本。只有唯一、明确的 Exact(new_source) 能进入下一步；映射尚未完成不等于目标不存在。
3. 创建新 Bookmark，再用其 `navigation` 创建 FoldNavigation。新的 source 越界（含 usize::MAX）或落在 UTF-8 续字节上，沿既有 API 返回 `InvalidRange`，不夹取到最近合法位置。source 已经改变或原字符已删除时，只有宿主另有明确的点对应才能报 Exact，否则失败；不擅自选删除区间附近、下一个同名标题或新 EOF。
4. FoldNavigation 按共享 Budget 展开必要遮蔽路径，最终返回精确新 source 和估计 top。只有成功返回目标才允许宿主提交该历史槽位或跳转结果。预算不足、mapper 未完成/失败、取消和所有错误都不提交游标。已展开折叠在取消后保留，沿用核心语义。
5. renderer 在自己的预算/继续位置中定位实际 source_row，再应用 `bookmark.viewport_top(source_row, viewport_rows)` 和 `Navigator::reconcile_source`。这一步不属于持久映射，导航的估计 top 不能当作已测得的实际行。要求实际画面定位成功才提交的宿主，应把第 4 步结果保留为 pending，等 renderer 成功后再提交；本原型的完成边界是现有核心的“取得精确 source 与估计行”。

| 情况 | 结果 / 责任 |
| --- | --- |
| 插入或删除发生在目标前，宿主有唯一 source-map | Exact，新 Bookmark 指向映射后的真实字节，不沿用旧绝对数值。 |
| 改文、删除、无映射、多个候选 | 宿主分别报告 Changed / Deleted / Missing / Ambiguous；错误不伪装成核心 WrongDocument。已有 Exact 也不能仅凭类型/长度证明内容对应。 |
| 映射工作未完成或尚无结果 | Pending；保留单点继续位置，不发布 Bookmark/游标，也不把“未检查”当成 Missing。 |
| 新资源不同，映射版本或旧点不符 | 宿主层 WrongResource / WrongMapping；构造失败，不开始映射。 |
| 捕获或推进时 Document owner 错误、绑定 revision 过期 | Core(WrongDocument / StaleContinuation)，在扣费前拒绝。旧 Bookmark 直接调用新 Document 的 navigation 仍是原生 WrongDocument。 |
| 新 source 越界 / 非 UTF-8 边界 | 创建新 Bookmark 时沿用核心 InvalidRange；这次已付创建步骤的费用，不发布成功结果。 |
| 视图属于别的 Document / 尚未同步当前 update | 推进的前置校验沿用 WrongDocument / OutOfOrderUpdate；不扣预算、不消费映射，也不修改视图；修正视图后可继续同一个任务。 |
| 导航中视图错误或被 reset，文档 append/finish 等变化 | 沿用 WrongView / StaleContinuation 等核心错误。原型绑定检查在预算前；导航也保留现有 view/version 检查。 |

mapper 的终态必须是单点的权威、唯一结论，而不是未消歧候选列表。原型的 `MapStep::Exact` 表示宿主已经完成这种判断。它不会通过读完候选流来替宿主证明唯一性；错误的 Exact 可以生成语法合法但语义错误的书签，这属于违反宿主合同。

## 预算、继续和取消

原型文件为 `examples/support/bookmark_reload.rs`，只被 example / 集成测试编译。`PointMap` 是预先提供的单点步骤脚本，每条 Working/终态的消费是固定操作，先收费再推进游标。它验证调度合同，**不实现自动 mapper**；准备脚本、读编辑日志、查版本库、指纹计算或加载文档的费用不在 core Budget 内。真实宿主若无可暂停 mapper，须在独立任务中运行并保留 Pending；不可暂停外部回调不能因为被 Budget 包围就获得时间上界。

| 步骤 | 原型计费与继续位置 |
| --- | --- |
| 消费一条宿主映射步骤 | 1 query + 1 object + 1 recovery；保存脚本位置，Working 后可暂停。脚本为空或耗尽而无终态时继续 Pending，不收费也不捏造结果。 |
| 创建新 Bookmark 与导航任务 | 另付 1 query + 1 object + 1 recovery；Bookmark 通过已有索引查找，O(log B + log H)，不读正文；不是所有索引操作的 CPU 硬时延保证。 |
| FoldNavigation | 直接使用同一 Budget 的剩余额度；每步 1 query + 1 object，recovery/bytes/tiles 为 0。分步解析 action、检查可见性/展开一层、取得估计行。 |

各阶段共享同一个传入 Budget，没有内部 unlimited。任一所需维度为 0 时该步原子不前移；导航能在 recovery 为 0 时继续。示例有一个 Working、一个 Exact、一层遮蔽，在每帧 1q/1o/1r 下共 7 帧：2 映射、1 创建、4 导航。恢复任务空间 O(1)，不含宿主提前准备的映射证据。构造为 O(1)，捕获一个已有 Bookmark 也不读取正文；创建原 Bookmark 本身仍有既有索引成本。

取消就是 drop：映射/创建阶段不会改变视图或历史；导航已打开的路径不回滚。终态映射错误被保留，重复调用不消费预算。原型脚本不可变；宿主后来补全证据时重建这一条点任务，不复制整个历史。真实可续 mapper 可直接保留自己的继续状态，仍须保证已消费证据不被悄悄改写。

append 后原 Document 内已有 Bookmark 仍可按左亲和恢复，但 pending 映射任务绑定的目标版本已经改变，必须重新确认 map.to 与 Binding 并重建单点任务。首次 finish 也改变 revision；若字节相同，内容令牌可不变，但临时绑定仍须刷新。resize/reset 不改变持久 source/offset，却使已启动的导航失效，须按新视口重建导航；不需要重新生成内容映射证据。测试覆盖这些重启，不声称旧 continuation 自动穿越变更。

## 历史及 viewport_offset

跨重载的宿主历史可保留 Saved 记录和逻辑槽位，**按用户请求映射一条**，避免每次 reload 重建/复制全部历史。原型以 `active_slot` 演示这个提交边界，没有引入新的历史容器。现有 NavigationHistory 仍只接受一个 Document 的 Bookmark，不能往旧实例 push 新 Document 的条目；本次不修改其内部 entries、identity 或 position。需要在新文档继续使用核心历史的宿主可以在成功后从新 Bookmark 开始新实例；跨版本前进/后退由宿主的稀疏记录按需恢复。

`viewport_offset` 是捕获时**目标源码行在视口顶端下方的行数**，不是目标内容内部行/图片像素/表格行偏移，也不是绝对全局行数。持久化保留原数值，布局重定位后再按新 viewport_rows 限制；例如保存 9、实际 source_row=12、新视口 3 行时 viewport_top=10，视口 2 行时为 11。视口 0 行限制为零偏移，文档开头使用饱和减法；末尾由 reconcile 限制。示例/测试中这个实际行数是明确手给的输入，未运行 renderer，也未把它说成实测终端位置。

## 运行及验证

在项目 devShell 中复跑；最多 2 jobs / 2 test threads：

```sh
nix develop -c env CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo check -p mdstream-core --example bookmark_reload --test bookmark_reload
nix develop -c env CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo nextest run -p mdstream-core --test bookmark_reload --test-threads 2
nix develop -c env CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo run --release -p mdstream-core --example bookmark_reload
```

`tests/fixtures/bookmark_reload/` 冻结原始 Markdown 和手写 oracle.tsv。旧中文 source=20；插入 `# Intro\npreface\n` 后为 36，删除前一节后为 7，相同内容和同位置其它段落改文后仍为 20。测试用新 source 读取真实文本、检查新 Document owner 与新 HeadingId，重复 `# Same` 不按名称猜位置。旧 source=7 的 alpha 被同长度 other 替换时，Changed 明确失败。

9 项新回归覆盖上述 oracle、旧 Bookmark/历史 owner 拒绝、宿主资源/版本/旧点错误、Missing/Ambiguous/Changed/Deleted、无结果及稍后补全、越界/UTF-8、每个预算维度为零、微预算继续、同一个预算跨阶段、取消前后游标与折叠、错误 Document/view、append/finish/reset、EOF/空文档和偏移缩放。既有 history、fold_navigation、source_fold_overlays 的针对性回归用于确认原边界。没有修改生产源码、Cargo manifests、hooks 或 rustfmt 配置，核心仍为零 runtime 依赖，没有新增 unsafe。

本轮 raw 日志、命令及实际 exit、源码输入/冻结哈希和独立 oracle 保存在 `/tmp/mdtui3-codex-todo/042-prototypes/`，机器结果为 `/tmp/mdtui3-codex-todo/042-result.json`。未验证真实外部 mapper、磁盘格式/崩溃恢复、实际终端行布局或自动 reload；这些不是本次原文设计交付的实现承诺。生产核心原有文档内合同保持。
