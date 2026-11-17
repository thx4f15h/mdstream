# 当前标题 slug / fragment owner

生产入口是 `HeadingSlugSession::new(&mut Document)`，或 UTF-8 adapter 的 `heading_slug_session()`。订阅后把每个连续宿主 `Arc<Update>` 交给 observe；中间 Update 可以先 merge。文档身份、base/revision、订阅 owner 和首尾 effect sequence 先校验，observe 本身只排队。实例不借用 Document，持续输入不会重建长期状态。

推进分为付费 effect/heading 元数据、source normalization、canonical metadata 投影、受影响标题、源序 greedy collision 和回收。当前 dirty 区间按真实 Heading.content 剪裁，不遍历正文；尚未确认的 Setext 原文先缓存 raw/code 前缀摘要，确认时按当前结构读取 metadata。annotation 使用一次 root-to-leaf 查询，避免公开 Runs 的相邻段合并在一次查询中遍历任意多旧节点。条件 style 和 link guard 复用正式 resolver；没有第二个 Markdown parser，也不把未来数学屏幕布局当 slug 文本。

更新和查询共用 Budget。mutation lane 只拥有核心图和固定输入，不跨帧借用 Document；安全 Rust Future 保存每次 await 的继续位置。新 revision 或取消不 drop 暂停中的图，先续到安全边界，再应用当前事实。查询返回前所有必要事件/标题/collision 已与当前 revision 对齐；维护中返回进度，当前 get 拒绝读取。原 `HeadingAnchors` 和 `FragmentLookup` 的 stale 行为没有变化。

分页 raw/code 规范化存储与 source-coordinate 投影保留当前聚合；共享 rope 表达 base，名称只另存 usize 后缀。压缩 radix 为已知 append 保存真实旧 endpoint 证明，新 suffix 从 base endpoint 开始。独立竞争标题不会使普通逐字增长反复比较旧前缀。数字候选和按 ordinal 的成员使用有摘要的 AVL，vacancy/displacement 只唤醒最早受影响 source priority。结果逐次等价于从第一标题开始的全局 greedy cold 分配，literal `foo-1` 等也占完整名称。

普通 heading envelope 增长只使新增内容 dirty，不使旧内容失效。Run/Pair 的实际 delimiter spans 分别更新；条件 scope 复用同一正式 Recovery 的 last-event、opener 和 forest，只继续 append-only 事件，并按新增/被新外层包围的 pair 范围更新 counts。当前 marker 的 revision/source 有序索引报告实际新隐藏端点，每个 start 只有一份当前记录；不保留每 revision 的历史列表。真正 binding/旧事件改写/rollback 保留显式 scope 恢复，成本仍在 Budget。新增 rebind_current_slugs 可保留该 memo 同时拒绝旧 prepared continuation；原严格 rebind_display_shape 合同保持。TUI 通过一个条件标题 lease 保留当前 canonical 状态，并在真实失效或退出时逐项退休。

长期 CurrentFragment 保存原始输入 Arc、固定 percent/UTF-8 状态和一个不占名称的 endpoint lease。即使已知不存在，尾部 malformed percent/UTF-8 仍被完整检验。重复读取重新取当前 owner；没有旧 target 副本或别名。Drop 仅排队一个取消 ID，advance/retire 按预算归还 lease、合并未引用路径并释放分页缓冲。Session.retire 支付剩余 Future、请求、索引、source 页、依赖图和 Update 内的 child graph 后才可进行轻量析构。直接 Drop 一个尚未 retire 的 session 仍是 Rust 容器的整体析构，调用方不能把它放在有帧延迟要求的路径中；真实 TUI 的退出和请求取消已接入付费路径。

原始 UTF-8 输入只 normalize 一次，生成实体缓存一次；新定义发布仍不遍历旧使用点。标题相关 scope 的引用依赖发现属于维护 Budget，任意 changed-base 的精确比较 X、真实 base 变化 D 和全局改名 P 都可能产生跨帧工作。输入 parser 的累计 O(n log n)/空间 O(n) 保持。维护采用标准有序容器，预算计量一次有序查询/更新及所有显式 rope/trie/AVL/metadata/比较步骤；每个有序容器步骤本身有 logarithmic 索引成本。此文不声称一般动态字符串的普遍 D/P-only 界或单次 append 硬延迟。

常驻存储为当前 source 规范化页、metadata 投影、当前 base/claims/数字组、当前相关 link/scope 依赖、活跃 query lease 和待消费事件；不按 revision 保存完整数组或长 slug。常驻空间 O(N + H + Q + backlog)，Q 为活跃请求的实际总输入/解码/端点规模，backlog 由调用方背压。旧 rope/map/tree 的退役逐节点/页计费。诊断 `SlugSessionStats` 包含暂停 lane 的固定页容量、逻辑 map payload、目录、canonical resolver 和事件/child Update；`CurrentFragment::capacity_bytes` 另报调用方持有的 Arc 输入与 token，TUI 的 fragment_capacity_bytes 报激活 buffer/guard/request。标准 BTreeMap 私有 slack/header、allocator 元数据及已有窗口 renderer/refiner 的共享存储不虚构为精确 malloc 峰值，规模 probe 另报告实际进程 RSS。诊断会遍历队列和 resolver scope，不应放入帧热路径。

成功使用内部 fragment 后，live 窗口对原排版结果全空白的链接标题保留可见标签，防止可导航目的地成为空行；混合标题、静态格式化及未启用 fragment 的原 Reading 合同保持。

TUI 以 `LinkDestinationTask` 复用核心 destination 解码器，固定保存实体探测，不因 revision 重扫已有前缀；解码后移动 String 描述符到 Arc，percent 只解码一次。当前 LinkId 的实际 Active、原始目标来源和 session owner 都通过后才进入 refinement。owner 改变或 resize 使旧 refiner 结果失效；导航前再次查询当前 endpoint。成功序号让 Enter 与 render poll 共用一次 history 提交；历史一直保存源码位置，不跟改名重新解析。目录 current/selection 仍由 HeadingIndex 和 source 导航决定。FoldNavigation::Reveal 接缝保持，完整正文 FoldView UI 不在 025 中冒称完成。

可复验命令（均在本 worktree 的 nix develop）：

```sh
cargo nextest run -p mdstream-core --test slug_session
cargo nextest run -p mdstream-core --lib -E 'test(slug_session)'
cargo nextest run -p mdtui-tui --test fragments --test fragments_pty
MDSTREAM_SLUG_SCALE=1000000 cargo nextest run --release -p mdstream-core --lib -E 'test(native_independent_competitor_growth_preserves_root_counterexample)' --success-output immediate
cargo run -r -p mdstream-core --example slug_maintenance -- growth 1000000
cargo run -r -p mdstream-core --example slug_maintenance -- history 20000
cargo run -r -p mdstream-core --example slug_maintenance -- cascade 40000
cargo run -r -p mdstream-core --example slug_maintenance -- setext 1000000
cargo run -r -p mdstream-core --example slug_maintenance -- queries 1000000
```

`source_bytes` 是实际首次 source normalization；`generated_input_bytes` 单列实体生成输入；`normalized_bytes`/copied_bytes、compared_bytes、各 rope/trie/numeric/projection 步骤、changed_bases、renamed、metadata/effect/heading reads 与退休分别计数，canonical_style_work 另报共享条件样式恢复的真实事件/索引工作。probe 同时记录 parser 的真实索引工作、进程 CPU ticks、RSS 和容量；Linux CPU tick 频率由环境的 `getconf CLK_TCK` 取得。测试 oracle 独立批量读取正式显示语义并重做字符串规范化及全局名称集合，不复用维护索引。
