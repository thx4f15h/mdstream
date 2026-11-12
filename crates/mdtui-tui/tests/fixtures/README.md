# TUI 静态 fixture 语料

本目录是 P06 的静态语料清单，只提供 Markdown 输入和确定性放大说明，不包含 cell 快照、ANSI golden 或渲染实现。`tests/fixtures_manifest.rs` 校验文件存在性、UTF-8、非空、README 覆盖、规格章节标注以及遗漏/重名。

| Fixture | 规格 | 用途与预期重点 | TUI / `--once` 差异关注 |
| --- | --- | --- | --- |
| `spec/headings.md` | §3.1, §3.12, §5 | H1–H6、多行 Setext、ATX 后独立段落、标题内链接/图片丢弃、强调文字按拼合规则重排、引用内标题无引用前缀。 | TUI 对超宽标题按内容区宽裁剪；`--once` 保留 CRLF 和完整超宽字节。 |
| `spec/paragraphs.md` | §3.2, §5, §6 | 软换行、行尾两空格/反斜杠硬换行、连续空格、真实 tab、NBSP、emoji、CJK、首尾空白和长段落折行。 | TUI 用 cell 行坐标折行；`--once` 逐字节保留 tab/NBSP 并以 CRLF 结束。 |
| `spec/inline.md` | §3.3, §3.4, §3.13 | 两种嵌套顺序、双反引号、词中强调、粗体/斜体/删除线内代码、代码内多空格/tab、实体 `&amp;`、转义。 | TUI 受核心位并集限制使用固定前景优先级；`--once` 仍按已记录差异核对字节。 |
| `spec/links.md` | §3.5, §5 | 行内/引用式/折叠/快捷、自动链接、邮件、空目标、标题忽略、同段多链接、窄宽折行、定义行隐藏。 | TUI 可点击范围按 cell；`--once` 关注 OSC 8 字节，`--no-links` 仅关闭 OSC 8。 |
| `spec/images.md` | §3.6, §9 | 独占段落图片、混排图片、空 alt、标题图片、多个混排图片。 | TUI 不创建可见 cell；`--once` 同样不输出图片，仅保留混排两侧空格。 |
| `spec/quotes.md` | §3.7, §4, §5 | 嵌套引用、引用内标题/列表/代码/表格/分隔线、空引用行、普通段落与 `[!NOTE]`/`[!WARNING]`。 | TUI 只有引用段落行带 `│ `；`--once` 关注前缀字节、块间距和 CRLF。 |
| `spec/lists.md` | §3.8, §3.12, §4, §5 | `-`/`+`/`*` 归一、0/3/10 起始、任务标记、深嵌套缩进、松紧列表、空项、多行项、链接/图片丢弃与按词折行。 | TUI 按内容区宽和动态缩进生成 cell；`--once` 关注标记颜色、续行、块间无空行。 |
| `spec/code.md` | §3.9, §4, §5 | 反引号/波浪线/缩进代码、完整 info 串、无语言、空代码行、超长行、只有 info 的空围栏、未闭合围栏及前后段落。 | TUI 裁剪超宽且不折行；`--once` 保留 `[info]`、两空格前缀和超宽原始字节。 |
| `spec/overwide.md` | §3.1, §3.9, §3.11, §5 | 超宽标题、超宽代码行、20 列表格、CJK/emoji 单元格及前后段落。 | TUI 按 `content_width` 裁剪，侧栏开关会改变宽度；`--once` 溢出并可能由终端自动折行。 |
| `spec/hr.md` | §3.10, §4 | `---`/`***`/`___` 三种分隔线及引用内分隔线。 | TUI 生成内容区宽度的 `─` cell；`--once` 为灰色完整 `─` 行和 CRLF。 |
| `spec/tables.md` | §3.11, §3.5, §5, §6 | 对齐标记无视觉差异、无表体、空单元格、缺列、超列丢弃、CJK、内联样式、表格链接、列数不一致回退段落。 | TUI 只分配内容区可见列；`--once` 使用全表最大宽度，表格内 LF/块末 CRLF。 |
| `spec/spacing.md` | §4 | 段落↔段落、段落↔列表、列表↔列表、代码/分隔线↔列表以及标题、引用、表格、代码、分隔线的通用间距组合。 | TUI 用空 cell 行表达间距；`--once` 用裸 CRLF 表达块间空行。 |
| `spec/footnotes.md` | §3.13, §9 | 有定义脚注的多次引用、命名脚注、定义续行、无定义脚注保持可见。 | TUI 有定义引用和定义块不占 cell；`--once` 同样不可见，未定义引用按原文输出。 |
| `spec/html.md` | §3.13, §9 | 行内标签、属性标签、自闭合标签、HTML 注释、HTML 块和后续段落。 | TUI 与 `--once` 都必须安全处理且不复制参考崩溃；具体跳过/原文策略留待 P16/P62。 |
| `links/anchors.md` | §3.1, §3.5 | 重复 slug、百分号编码、大小写、`#`、畸形转义、反斜杠/实体 destination、可命中的 `#a\_b`/`#&#x61;` 与解码用例。 | TUI 流式期间延后片段链接；`--once` 不负责交互跳转，只保留链接字节语义。 |
| `stream/large.md` | §3.2, §3.11, §5 | 小种子：开头标题、长段落、表格行、末尾标题；用于按 README 命令确定性扩展到 8/64 MiB。 | TUI 首屏/滚动只处理可见窗口；`--once` 用于规模字节基线，但不提交大文件。 |
| `stream/hidden-blocks.md` | §3.9, §4, §9 | 小种子：空 info 围栏夹在长段落和列表之间，覆盖 `LayoutPass` 跳过与 TUI 独占 tile。 | TUI 必须预扫描隐藏块保留标签/间距；`--once` 关注空围栏语言标签和块间距。 |
| `sidebar/many-headings.md` | §3.1, §5 | 小种子：10 个左右标题，含重复 slug、深层级和超长标题；用于确定性扩展到 10k 标题。 | TUI 侧栏只读取可见标题；`--once` 不构建侧栏，但可校验标题字节。 |
| `sidebar/huge-setext.md` | §3.1, §5 | 单个超长 Setext 标题种子；用于确定性扩展到长物理行并测试侧栏读取预算/一次性锚点构建。 | TUI 侧栏预算截断且不缓存全文；`--once` 保留完整标题字节和 CRLF。 |

`spec/paragraphs.md` 包含真实行尾双空格硬换行；同目录 `.gitattributes` 只对该文件的 `blank-at-eol` 检查关闭，不影响其他 fixture。

## 确定性放大

以下命令只使用 shell/awk 和种子中的固定文本，不访问网络、不使用随机数，也不把生成物提交到仓库。所有生成文件放到 `/tmp/mdtui-fixtures/`，重复执行得到相同内容。

```bash
mkdir -p /tmp/mdtui-fixtures

# 8 MiB 快测种子；64 MiB 夜间把 scale 改为 8。
LC_ALL=C awk -v scale=1 'BEGIN {
  print "# Large stream fixture start\n";
  for (i = 0; i < 50000 * scale; i++) printf "Deterministic paragraph line %07d with fixed words for bounded layout and scrolling checks.\n", i;
  print "\n| key | value |";
  print "| --- | --- |";
  for (i = 0; i < 65000 * scale; i++) printf "| row-%07d | deterministic value %07d |\n", i, i;
  print "\n# Large stream fixture end";
}' > /tmp/mdtui-fixtures/large-8m.md

LC_ALL=C awk 'BEGIN {
  for (i = 0; i < 2000; i++) {
    printf "Paragraph before hidden block %05d with fixed text.\n\n```rust\n```\n\n- list item %05d\n\n```python\n```\n\n", i, i;
  }
}' > /tmp/mdtui-fixtures/hidden-blocks-stream.md

LC_ALL=C awk 'BEGIN {
  for (i = 1; i <= 10000; i++) printf "## Section %05d\n\nBody %05d.\n\n", i, i;
  print "## Repeated heading\n\n## Repeated heading\n";
}' > /tmp/mdtui-fixtures/many-headings.md

LC_ALL=C awk 'BEGIN {
  for (i = 0; i < 8192; i++) printf "word-%05d-", i;
  printf "\n";
  for (i = 0; i < 8192 * 11; i++) printf "=";
  printf "\n";
}' > /tmp/mdtui-fixtures/huge-setext.md

LC_ALL=C wc -c /tmp/mdtui-fixtures/large-8m.md
```

规模测试应在需要时消费 `/tmp/mdtui-fixtures/` 的生成物；仓库只保留可读的小种子和本说明。
