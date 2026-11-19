# 冷 Viewport V01：调查与只读范围原型

本阶段交付原始负例、原生只读范围 metadata 游标及测试专用 SharedShape 原型。**原冷 Viewport 缺口仍未解决**，没有启用冷 owner 的 epoch、范围覆盖或增量更新机制，也没有改变公开 Viewport/BodyRenderer 接口。原 TODO 的窗口、预算及“不因文档变大引入全文扫描”约束保持；本记录不增加初始化豁免。

## 保留的入口与使用范围

`Document::display_shape_range_seed(source)` 返回绑定 DocumentId/revision 的 `DisplayShapeRangeSeed`，以同一 Budget 读取当前范围的 annotation、replacement、space、相交 link 和 footnote 元数据。标注树只下降相交子树，保留覆盖范围的祖先 tag；replacement/space 使用已有有序索引，link/footnote 使用已有范围及 source-order 游标，不从 arena ID 0 枚举历史。`work().steps` 计成功的原子 metadata 扣费调用：本游标、annotation 和 footnote 的一次 1q/1o 计一步；原生 LinkWindow 每步只扣 query 或 object 之一，因此该委托的 deltaqueries + deltaobjects 同样是步数，不是双计 1q/1o。独立驱动 LinkWindow 的回归核对这一口径；steps 不是 Budget 各维之和或 CPU 时间。

原始字节解码为 0；工作量随**请求的元数据范围**增长，传入整篇范围仍然请求整篇元数据。

Mark 片段裁剪到请求范围，内部字节树边界可能切在 UTF-8 标量内部，消费者沿用原 seed 的原生 scalar 下界映射。replacement 和对象保留完整相交 extent，可能在窗口前开始。Candidate 保守覆盖请求范围，样式仍须通过已有原生 style 索引求值。空合法范围在身份检查后无工作完成；错误文档、过期 revision 和无效 UTF-8 边界拒绝。

`DisplaySource::first_html_in` 返回相交的输入阶段 HTML 分类 span，包括窗口前开始的 opener；`Document::display_shape_html_qualified` 付费读取原始 opener 的当前 annotation/replacement 上下文。这是当前元数据检查，不重扫 opener 到窗口之间的正文。输入索引的维护、查询及 shape 容量分别计数，不把逻辑采用字节数当作实际解码，也不把这些计数称为完整进程 RSS 或 CPU 硬界。

该游标只提供**当前只读状态**，不提供 annotation rollback 基线。它不能替代完整 ShapeSeed 初始化后消费 Update 的合同。`crates/mdtui-tui/src/render/shape_range_prototype.rs` 仅由 `cfg(test)` 编译，采用实际 SharedShape、DisplaySource 和现有 ShapeQuery/ShapeWindow。实验只允许完成后读取那个范围，不支持重复/重叠采用、任意后续范围或 renderer 跨 revision 更新。没有第二 parser、输入索引或 renderer；生产 SharedLinks、ShapeSeed、Reader、epoch 与更新路径保持正式 base 行为。

## 原始反例与撤回的候选

正式 base 为 `f7920d0907327a71d0cbf49b8bae2aaf40a6709e`。原输入 800014B、100000 列，40×12、min3/max8、pan99998；每帧 1 query / 1 object / 6 bytes / 1 tile / 1 recovery / 1 dependency，200000 帧 ceiling。原代码、输入、断言和 SHA 均保留。

| 实测阶段 | 结果 | Budget.bytes | 旧 source 解码 | annotation seed 节点 | CP |
| --- | --- | ---: | ---: | ---: | ---: |
| 正式 base，注册开 | 200000 帧 PendingScan，退出 101 | 800022 | 800014 | 66638 | 0 |
| 正式 base，注册关 | 200000 帧 PendingScan，退出 101 | 800022 | 800014 | 66146 | 0 |
| indexed-only 候选，注册开 | 200000 帧 PendingScan，退出 101 | 8 | 0 | 199972 | 0 |

原输入阶段的 DisplaySource 已解码 800014B；冷普通 seed 再解码同一旧 source，再采用全局当前语义元数据。原负例中 paint 与列源码测量未开始。Pending 是实测状态，不是死循环证明，也不是 PASS。早期033的“baseline/new”均是其同一 worktree 的注册关/开；本阶段的正式 base 重放与其历史测量分开。

未闭环的冷 owner 接入曾在 44 帧、14B 到达 Ready、旧 source 解码 0、CP1，但原字面测试仍在 `table_rows_prepared <= 1` 失败：当前该统计包含 4 个终端装饰/表头行。它没有通过全部原断言，且 epoch/覆盖/rollback 生命周期未完成验证。接入补丁和完整源码已先归档，随后撤回，**不包含在本阶段生产交付中**。实际 FrameRuntime 输入 owner 的既有033成功对照独立保留，不用于证明低层冷入口成功，也不撤销033验收。

原测试在第一个冷输入失败，第二巨型隐藏行输入未在该原测试中执行；两份原始输入仍保留。最终034原组合审计没有完成，本阶段不等待或替代该审计。

## 定向验证与复跑

核心回归覆盖空/零/微预算、无效边界、foreign/stale、范围越过 annotation capacity、祖先 tag 裁剪与完整 native seed 对照、窗口内起读的 link/image/code/footnote、原始 opener 在窗口前的 HTML，以及真实 inherited-scope/late-pipe rollback 和 finish 后的游标拒绝。rollback 检查针对只读游标的失效与当前状态重读，不代表增量 renderer 更新已闭环。

TUI 测试专用原型覆盖大段历史后的 6 类尾部语义，以及从 bold、链接 label、image alt、code、HTML 和 footnote 内部开始的 8 类窗口，比较字素、文字、绝对来源和样式。范围采用使用 1q/1o/6B 微预算；后续 ShapeQuery/ShapeWindow 的 oracle 比较使用 unlimited 预算，只证明该范围的语义一致，不证明整帧微预算完成或输出时延。比较对象是已有完整 seed owner；fenced code 项只证明与既有 shape oracle 一致，不构成独立的 code renderer/UI 正确性证明。

在项目 devShell 复跑：

```sh
nix develop -c env CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo nextest run -p mdstream-core --lib --test display_shape_range --test display_shape_seed --test-threads 2 -E 'binary(display_shape_range) | binary(display_shape_seed) | test(shape_range_outside_annotation_capacity)'
nix develop -c env CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo nextest run -p mdtui-tui --lib --test shape_ownership --test-threads 2 -E 'test(range_adoption_prototype::) | binary(shape_ownership)'
```

最终 debug/release 各通过 16 项：核心范围 10 项（新增 8 项、既有 seed 2 项），TUI 范围 6 项（测试专用原型 3 项、既有 ownership 3 项）。core 类型检查、core/TUI 范围 strict Clippy、无警告 rustdoc 与 fmt 检查通过；没有运行全仓或真实 CLI/UI 验收。独立 LinkWindow 计数回归加入后重跑核心 10 项，TUI/核心生产源码与先前两档 TUI 6 项通过时逐字节相同。

加 `--release` 检查 release。本次所有重命令经过共享 heavy-gate，最多 2 build jobs / 2 test threads；没有执行原026内存组或全 workspace 门禁，没有新增依赖或 unsafe，没有改 hook/rustfmt/全局配置，也没有 commit/push。

完整源码归档、初版编译失败、错误 fixture 假设及更正、原始负例、撤回候选、命令/exit 和最终冻结保存在 `/tmp/mdtui3-codex-todo/043-prototypes/`，最终阶段结果为 `/tmp/mdtui3-codex-todo/043-wrapup-result.json`。后续任何冷入口实现仍需证明未知范围 summary、同 owner 的追加/rollback、finish、resize/CP、列 pan 及原例；本阶段不继续实现这些机制或声明它们通过。
