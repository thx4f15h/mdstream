# URI 与图片基础阶段（030）

本阶段在正式 `f7920d0907327a71d0cbf49b8bae2aaf40a6709e`（含 041/042）上收束现有实现。完整需求仍包括通用当前 URI 输出和真实图片 UI；本交付只提供下面的基础 API，不代表该完整功能完成。用户已要求本阶段结束后收尾，不继续下一阶段。

## 当前用户行为与接缝

CLI 默认 `--images=off`，继续显示可点击 alt，空 alt 为 `[image]`。显式 `--images=auto` 或 `--images=kitty` 只通过现有单 writer 查询 Kitty ACK 和 cell 像素几何；无支持或无几何时不会上传，不能依靠 TERM 假定支持。`kitty` 也不绕过验证。`--once` 不接受该参数，其既有 ANSI 合同不变。

CLI 尚未把当前可见对象生产者连接到 Resources、Scene 和 FrameRuntime 的图片行带。它不会因文档包含图片而下载、解码或上传图片，也尚未显示图片进度/错误状态。独立组件确实支持真实协议绘制，真实终端证据的边界是组件，不是自动正文图片 CLI。

原始 `link_destination`/metadata 解码、parser 预算、官方 HTML 和既有 ANSI goldens 保持。当前所选 link/image 与正文身份的最终语义接缝尚未集成；既有 OSC 8、Open 和评分投影尚未改用新的通用序列化器。标题/列表既有 formatter 对普通链接的省略仍是独立方言。

## 序列化与 worker URI 合同

核心公开 `UriEncoder<'a>`、`UriChunk`、`UriWork`。构造绑定固定标准 `Cow<'a,str>`；借用 `&str` 或移动 `String` 不复制输入，没有可变 provider 回调。读取和 `%HH` lookahead 先支付真实 Budget，输出逐个 ASCII atom 返回；0 预算不读字节，1q/1o/6B 可跨多次调用推进。`UriChunk.input` 是已解码输入的字节范围，可能只覆盖一个 UTF-8 字节，不能当成 Document source span。

序列化器保留字母数字、`-._~`、URI reserved ASCII 和已有合法 `%HH` 的大小写；其余字节编码为大写 `%XX`。Unicode 按 UTF-8 字节编码，空白/control/反斜线编码，坏 `%` 编为 `%25`。它不解析 scheme、不解码 entity 或 Markdown escape、不选择对象、不解析相对路径。调用者必须从真实语义/metadata 解码接口取得输入和来源；本阶段未接通这个调用者。

worker `Task::Uri` 先复用核心序列化，再由实际 URL 库解析或对显式 URL/native 文件 base 做 join。不猜 cwd；支持 Unix 非 UTF-8 文件路径 base。目标为空、decoded control、认证信息或远程 file host 时拒绝；file 解码后的路径也拒绝 control。允许 `http`、`https`、本地 `file`、`mailto`，保留普通 URI fragment；其它 scheme（含 `data`、`javascript`、`ftp`）拒绝。`Task::ImageUri` 只允许 http/https/file，拒绝纯 fragment，下载身份去 fragment。相对、base 和 dot segments 由 URL 库在 worker 内处理；entity 不在此层重新解释。raw API 不变。

Resources 的优先 `UriRequest` 只产生 `UriReady`，不会自己 Open 或写 OSC 8。它可以取消低优先级图片工作并先付费 reap 唯一进程槽，再启动 URI 工作；不会同时运行两个 codec/HTTP worker。

## 资源、协议与所有权

- `image::worker::Process` 使用一个受限子进程执行 HTTP/URL/codec。实际 PNG（含 palette/transparency、16-bit）和 baseline/progressive JPEG，JPEG EXIF orientation、RGBA8、最近邻等比 containment、不放大、透明补齐 terminal cells。这里只取静态默认图，不提供动画或色彩管理保真承诺。
- 仅常规本地文件或直接 200 HTTP/HTTPS；无 redirect/proxy，identity body。FIFO 非阻塞打开后拒绝。encoded ≤8 MiB，尺寸各 ≤16384、declared pixels ≤16 Mi，display canvas ≤1 Mi pixels/4 MiB，单 URI ≤192 KiB；超限明确失败，不截断成伪图片。HTTP 全程 5s、单次读取 500ms；父进程仅在实际 WouldBlock 时开始 10s idle watchdog。
- 第三方单次解码/URL/HTTP 调用不可合作抢占，成本不冒充一个 UI Budget object。Linux worker 有 512 MiB 地址空间及 4s CPU rlimit；父端 nonblocking IPC、取消 kill 后分帧 try_wait/retire，q 不 join codec。未返回报告的被杀 worker 单独计数，不把其 CPU/网络成本说成零。
- `Resources` 全局队列8，输入 current+retiring ≤1 MiB，待交付 URI ≤2 MiB。固定24个 retirement descriptor；每个队列准入先保留2个，使每个已接受请求都可取消。拒绝返回原 owner，0/micro Budget 和取消继续位置均受测。`Resolved` 必须显式 `approve` 后才能 load；未来宿主仍须做真正 current-owner 校验。
- `Cache` 当前64个 generation ID、32 MiB byte admission（包括 receiving/retiring 预留）。key 比较分帧且重验 generation/hash/length，LRU 提交重验 touch；有 pin 时不 evict，无每 revision 历史。当前、pin 子集、retiring、页目录、worker receiving 及其容量分别报告，不能把这些子集相加当总 RSS。
- `Session` 真正发送 Kitty direct RGBA、匹配 upload ACK、place/crop/reuse/hide/free。64 image /256 placement descriptors、16 MiB terminal pixel reservation、单 packet 有界 staging；prepared/raw/base64/wire bytes 分别计数。ACK deadline 从完整 ST 发出后计时，错误 ACK/超时有状态，缺失图像最多一次合并重传。真实 terminal framebuffer 已验证，fallback 不作为协议绘制证据。
- `Scene` 从已提交 native frame 的 `BandLine` 建立有预算的有限矩形；Capture 绑定实际不可变 `Rc<ViewportFrame>`，不复制全部文本或 URL。scene nonce、resource generation、resize paid clear 阻止旧/外国 session 的继续任务覆盖新场景；空/离屏 scene 释放不再需要的 GPU pin。调用者仍必须提供可信当前帧和资源身份。
- tty 输入只有一个 nonblocking owner，分离 Kitty ACK/设备应答和键盘/鼠标；保留 Esc/Ctrl/Alt/Shift/Unicode/resize。输出由 021-aware FrameRuntime arbiter 统一：图片控制不切开未完成 EGC、OSC 8、OSC 52 或 synchronized transaction。短写/EAGAIN 返回 host poll，不忙循环等待 ACK。

依赖仅加在 TUI：image 0.25.10（PNG/JPEG）、ureq 3.5.2（rustls）、url 2.5.8、base64 0.23.1、termion 4.0.6；实际版本、来源与 license 从锁定 crate 源及 primary 文档核验，记录在原型 evidence/dependencies.json 和 production/codec-formats/primary-library-inputs.json。核心仍零 runtime 依赖并禁止 unsafe；TUI 自有库与 binary 同样禁止 unsafe。

## 原生布局基础与限制

`image::layout` 的稳定对象 key 为 Root(DocumentId) 或 Note(BodyKey) + LinkId + source start，最多256个 current bands。AnchorQuery 从真实 Shape 找 alt 尾 EGC；BandQuery 尊重同一 FoldView 与 source cut。FrameRuntime 的 publish/withdraw 复用真实 root/active native note renderer，withdraw 比较完整旧 band，不能撤销后来同位置的新资源。

行带进入既有 Sink/HeightIndex/checkpoint，不另造 renderer；表格同一行按各 cell stack 的最大高度，边框保持。宽高变化保留 source-object + intrinsic-row 位置，缩短时 clamp；移除后用原 source refiner 回到 caption。只准备 viewport 交集，不逐行建立离屏整图；end_tile 只记录 descriptor，之后逐 row handle 付费 adoption。真实 End 包含 raster 行。图片覆盖 cells 连接真实对象 source envelope 和 click hit，装饰边框无来源；Reading y 跳过 raster 行，Y 保留精确原文。

这些基础由实际 Root/BodyKey Note、表格、fold/source cut、CP/window、End、移除/resize、click/copy 测试调用。尚未连接自动 late-binding/current producer，因此不能声称完整 UI 已处理所有 late definition、流追加/截断、EOF、viewport/fold 变化下的资源审批、状态和退役。

## 验证与保留的失败

原 phase1 设计/17 probes/真实 Kitty pixel oracle、root 原始 red 与独立 green 保留在 `/tmp/mdtui3-codex-todo/030-prototypes`。原不可变 AsRef panic、cache generation false hit、retirement 第13次取消 panic 均有原样独立 witness；生产修正复用 Cow、ResourceId generation 与有界准入，不删弱 oracle。

本阶段最终源码输入、准确命令/exit、最新 debug/release 定向验证、真实 private PTY/Kitty 及 reused evidence 的逐文件相同/不同范围由 `wrapup/` 记录；机器交接 `/tmp/mdtui3-codex-todo/030-wrapup-result.json` 区分阶段可审查与完整 feature 未完成。早期编译、lint、observer shadowing、错误测试 credit/fixture 假设和 ENOSPC 失败全部保留，失败本身不计 PASS。没有改写官方输入、golden 或用当前输出重生成期望。

未测/未交付：自动 current URI/OSC8/Open，完整 CLI 自动图片 UI，跨终端协议全集、全部图片格式、动画、色彩管理、网络 sandbox/重定向策略、所有机器 ABI 或第三方调用硬实时。现有资源上限不是完整网络沙箱；当前组件验证也不替代这些未来范围。本阶段结束即交 root 审查并统一整理单个阶段提交，不继续推进。
