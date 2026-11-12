# TUI 静态规格符合性矩阵

本文由旧实验 P62 矩阵接入，当前 TODO 优先；图片可见、Alignment、目标配额及字素窗口边界的新增证据见 [终端集成说明](tui-integration.md)。本文是 ANSI/TUI 渲染行为的审计表，不替代 `docs/tui-render-baseline.md` 或 `docs/render-style-spec.reference.md`。TUI 的 oracle 是 cell/Screen（`tests/static_render.rs`），`--once` 的 oracle 是原始字节 golden（`tests/once_ansi.rs` 和 `tests/golden/`）；两者不能互相替代。

测试命令（均不访问网络）：

```bash
nix develop -c cargo nextest run -p mdtui-tui
nix develop -c cargo nextest run -p mdtui-tui --test compliance_matrix
```

## §0 总则

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 普通行尾 CRLF，最后一行也 CRLF | Screen 用行坐标，不携带行尾 | `render_once_ansi` 对普通行/空行/末行写 CRLF | `tests/once_ansi.rs::once_clear_prefix_is_emitted_once_for_nonempty_content`、`once_table_uses_internal_lf_and_block_final_crlf`；golden `once_headings.ansi`、`once_paragraphs.ansi` | TUI 与 once 行寻址不同；once 符合 |
| 表格内部 LF，表格块末尾 CRLF | 表格是 cell 行，无 LF | 根据 table `LineSource` 写内部 LF、末行 CRLF | `tests/once_ansi.rs::once_table_uses_internal_lf_and_block_final_crlf`；`tests/golden/once_tables.ansi`、`once_overwide.ansi` | 符合 |
| 空/纯空白零字节 | 空文档为空白网格 | `RenderStats.rows == 0` 时不写 clear/content | `tests/once_ansi.rs::once_empty_and_whitespace_are_zero_bytes`；`tests/viewport.rs::empty_document_has_an_empty_ready_frame`；golden `once_empty.ansi`、`once_whitespace.ansi` | once 符合；TUI 按空白网格处理，差异已记录 |
| 非空全屏先 `ESC[2J ESC[H` 且一次 | 首帧生命周期独立处理，不把 CRLF 带入交互帧 | `ONCE_CLEAR` 仅写一次 | `tests/once_ansi.rs::once_clear_prefix_is_emitted_once_for_nonempty_content`；`tests/frame_runtime.rs::first_frame_clears_once_and_resize_width_frames_are_diff_only`；`tests/flicker.rs` | once 符合；TUI 正常帧不清屏 |
| 唯一内置主题、无语法高亮 | `StyleRole` 映射到固定 Theme | `theme` 产生固定 SGR | `crates/mdtui-tui/src/theme.rs::theme_constants_match_reference`；`tests/golden/once_inline.ansi`、`once_code.ansi` | 符合；TUI/once 共用 Theme |

## §1 颜色

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 标题、加粗、斜体、代码、链接、列表/任务、分隔线、表格框线、删除线、元信息、代码背景 RGB 固定 | 各 `StyleRole` 由 `Screen` cell 保存并按 Theme 解析 | `tests/golden/once_{headings,inline,links,code,tables,overwide}.ansi` 固定精确 RGB 字节 | `theme::tests::theme_constants_match_reference`、`headings_h1_to_h6_emit_exact_sequences`、`inline_roles_emit_exact_sequences`、`code_and_link_emit_exact_sequences`；`fixtures/spec/{headings,inline,links,code,tables}.md`；`tests/static_render.rs::all_fixtures_have_stable_cell_snapshots_at_all_target_widths` | 无差异 |

## §2 通用编码

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 24-bit 前景/背景、独立 SGR、固定属性顺序、无 39/49、弱显/反显不出现 | Theme 的 `ResolvedStyle` 按固定顺序编码 | `MONOKAI.encode_segment` 逐段自包含；恰好保留 reset | `theme::tests::inline_roles_emit_exact_sequences`、`forbidden_sgr_sequences_are_absent`、`adjacent_segments_reset_independently`；`tests/once_ansi.rs::once_adjacent_equal_styles_keep_independent_resets` | 无差异 |
| 样式段自包含，相邻段不省 reset | TUI 的 diff writer 可合并相邻 cell；cell 样式仍可验证 | once 按 cell style/link/source 分组，每段 `start+text+reset` | `tests/golden/once_inline.ansi`、`once_links.ansi`；`tests/once_ansi.rs::once_raw_goldens_cover_all_required_blocks_and_overflow`、`once_adjacent_equal_styles_keep_independent_resets` | once 符合；TUI reset 分段由 P41 增量写出器负责，不能要求与 once 字节相同 |
| OSC8 开在样式前、闭在 reset 后；每行独立闭合 | Screen 保留 `LinkId`，交互 writer 独立编码 | `encode_link` 固定 OSC8 顺序；空目标只样式 | `render::link::tests::http_and_https_emit_osc8_outside_self_contained_sgr`、`email_uses_raw_destination_while_link_url_adds_mailto`、`adjacent_links_open_and_close_independently`；`tests/golden/once_links.ansi` | 无差异 |

## §3 元素样式

### §3.1 标题 H1–H6

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| H1–H6 色/粗体/下划线变体；单行不折行；内联文字按拼合规则；普通链接文字省略、图片保留；Setext 等价 | heading renderer 使用统一 heading StyleRole，超宽在正文区裁剪 | unclipped heading 保留完整字节，CRLF 结束 | `fixtures/spec/headings.md`；`render::heading::tests::specification_example_merges_base_then_reversed_emphasis_blocks`、`all_levels_use_uniform_heading_style_without_inline_roles`、`setext_multiline_text_keeps_images_and_drops_link_text`；`tests/once_ansi.rs::once_raw_goldens_cover_all_required_blocks_and_overflow`；golden `once_headings.ansi` | TUI 裁剪超宽、once 溢出，差异已记录 |

### §3.2 段落

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 软换行空格、硬换行消失、连续空格/tab/NBSP/emoji 保留、首尾裁剪、贪心折行 | paragraph renderer 按 content width 产生 cell | once 按 `--width` 折行，保留原始 tab/NBSP 字节，CRLF | `fixtures/spec/paragraphs.md`；`render::paragraph::tests::soft_break_becomes_space_and_hard_break_disappears`、`trim_ascii_edges_preserve_internal_spaces_and_nbsp`、`greedy_wrap_preserves_spaces_and_can_start_a_line_with_space`；golden `once_paragraphs.ansi`、`once_flow.ansi` | 无实现差异；TUI 写 cell，once 写原始行尾 |

### §3.3 加粗/斜体/删除线

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 粗体/斜体/删除线颜色与标志；嵌套外层前景决定；词中生效；每段 reset | StyleRole 位并集，按 P14 固定前景优先级 | 同一 StyleRole 经 Theme 编码 | `fixtures/spec/inline.md`；`theme::tests::nested_emphasis_roles_and_bytes_are_identical_at_fixed_priority`、`strong_or_emphasis_with_strike_uses_emphasis_foreground`；golden `once_inline.ansi` | 已知差异：核心 `Style` 位并集无法恢复任意嵌套外层前景，采用 P14 固定优先级 |

### §3.4 行内代码

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 代码前景/背景，反引号消失，外层属性保留，双反引号等同 | CODE StyleRole + Theme | 同一 cell 样式编码 | `fixtures/spec/inline.md`；`render::input::tests::entities_and_code_whitespace_are_not_renormalized`、`theme::tests::code_keeps_outer_flags_in_sgr_order`；golden `once_inline.ansi` | 无差异 |

### §3.5 链接

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 链接样式、空目标、标题忽略、引用/行内一致、邮件裸地址、表格无 OSC8、`--no-links` 只关 OSC8 | Screen 保留 LinkId/样式；普通表格链接省略 ID，图片例外保留 | `encode_link` 输出有界 OSC8；表格图片可输出 OSC8 | `fixtures/spec/links.md`；`render::link::tests::email_uses_raw_destination_while_link_url_adds_mailto`、`render::table::tests::inline_styles_survive_but_table_links_have_no_link_id`；`tests/once_ansi.rs::once_table_links_keep_style_without_osc8`、`once_no_links_removes_osc8_but_keeps_link_style`；golden `once_links.ansi`、`once_tables.ansi` | 无实现差异；参考工具当前对表格链接的 OSC8 实际行为与本规格/P61 要求不一致，矩阵按规格结论记录 |

### §3.6 图片

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 当前 TODO 要求图片可见（覆盖旧参考不可见规则） | alt/`[image]`、源码映射与 LinkId；所有块类型 | 与 TUI 同一可见语义及有界 OSC8 | `fixtures/spec/images.md`；`workspace_integration`；`once_images.ansi`、`once_images_use_clickable_alt_and_an_empty_alt_placeholder` | 当前合同与旧参考的明确差异 |

### §3.7 引用块

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| `│ ` 前缀、只一层、段落才有前缀、引用内其他块无前缀、空引用行消失 | quote paragraph renderer 写前缀；标题/代码/表格/列表不写 | 前缀是普通字节，普通行 CRLF | `fixtures/spec/quotes.md`；`render::paragraph::tests::quote_prefix_is_single_plain_two_cells_and_body_starts_after_it`、`nested_quote_still_uses_one_prefix`、`wrapped_quote_rewrites_prefix_on_every_line`；golden `once_flow.ansi` | 无实现差异 |

### §3.8 列表

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 无序/有序/任务标记、缩进公式、空项消失、多行拼合、按词折行 | list renderer 按 cell 和 content width 写标记/缩进 | once flow 折行，表格外普通行 CRLF | `fixtures/spec/lists.md`；`render::list::tests::nested_bullet_ordered_and_task_formulas_match_reference`、`ordered_markers_preserve_source_start_numbers`、`word_wrap_collapses_whitespace_and_drops_separator_at_breaks`；golden `once_flow.ansi` | 无实现差异 |

### §3.9 代码块

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| info label、两空格/CODE、空行两个带样式空格、CRLF 归一、三种 fence、未闭合、长行不折行 | code renderer 在正文宽裁剪 | unclipped code 行保留完整内容；行尾 CRLF | `fixtures/spec/code.md`；`render::code::tests::backtick_tilde_and_indented_code_render_identically`、`crlf_is_normalized_and_never_written_as_a_cell`、`info_label_uses_full_trimmed_info_and_meta_style`、`long_lines_clip_whole_graphemes_at_widths_one_two_20_80_120`；golden `once_code.ansi`、`once_overwide.ansi` | TUI 裁剪超宽、once 溢出，差异已记录 |

### §3.10 分隔线

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| `─` 填满宽度、灰色、三种语法一致、引用内无前缀 | thematic renderer 写 cell | 同一 RULE 样式经 once 编码 | `fixtures/spec/hr.md`；`render::thematic::tests::top_level_three_syntaxes_render_the_same_rule`、`quoted_three_syntaxes_render_without_quote_prefix`、`widths_zero_one_two_and_80_fill_content_area`；`tests/static_render.rs` | 无实现差异；没有单独的 once rule golden，属于矩阵待补证据而非行为缺口 |

### §3.11 表格

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 顶/底/表头分隔/内容竖线、列宽、按 Alignment 对齐、padding 样式、表内链接无 OSC8、无表体无底边、内部 LF/块末 CRLF | table renderer 按左/中/右对齐，只写内容区可见列，单元格单行裁剪 | once 使用全表宽度、不裁剪，按规格写 LF/CRLF | `fixtures/spec/tables.md`、`fixtures/spec/overwide.md`；`render::table::tests::two_by_two_grid_is_exact_and_border_styles_are_separate`、`header_only_table_has_three_rows_without_bottom`、`inline_styles_survive_but_table_links_have_no_link_id`、`overwide_table_keeps_visible_column_prefix_and_never_wraps`；`tests/once_ansi.rs::once_table_links_keep_style_without_osc8`、`once_table_uses_internal_lf_and_block_final_crlf`；golden `once_tables.ansi`、`once_overwide.ansi` | 当前 Alignment 高于旧参考的统一左对齐；TUI 可见前缀裁剪与 once 全宽是双路径差异 |

### §3.12 标题/列表文字拼合

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 普通文字/代码/换行先拼接，强调段倒序追加，普通链接省略、图片保留 | `compose_heading_text`/list item composition 共用规则 | once heading/list 复用同一组合结果 | `fixtures/spec/headings.md`、`fixtures/spec/lists.md`；`render::heading::tests::specification_example_merges_base_then_reversed_emphasis_blocks`、`render::list::tests::item_composition_keeps_image_identity_and_reorders_emphasis`；`tests/once_ansi.rs` | 无差异 |

### §3.13 脚注/HTML/其他

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 有定义脚注隐藏、无定义保留；行内 HTML 清理；HTML 块安全处理；未定义脚注/实体等 | visibility/HTML helper 过滤，不 panic | 同样过滤后输出；HTML 块安全跳过而非崩溃 | `fixtures/spec/footnotes.md`、`fixtures/spec/html.md`；`render::visibility::tests::resolved_footnote_markers_disappear_but_missing_ones_stay`、`inline_html_tags_and_comments_disappear_without_touching_text`、`recognized_html_blocks_are_skipped_and_following_paragraph_is_not`、`fixture_sources_clean_without_panicking_or_overflowing`；`tests/static_render.rs` | 已知差异：参考实现 HTML 块崩溃，本 TUI 安全跳过 |

## §4 块间空白

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 顶层块间距表、首块无前导空行、末块无尾随空行、引用/列表紧凑、代码/分隔线/列表紧贴 | group spacing 写空白 cell 行 | spacing 写裸 CRLF，最后一行也闭合 | `fixtures/spec/spacing.md`；`tests/static_render.rs::spacing_fixture_stats_match_p25_oracle`、`render::document::tests::spacing_matrix_is_exhaustive`、`spacing_fixture_matches_expected_rows_and_blank_sources`；golden `once_flow.ansi`、`once_headings.ansi` | 无实现差异；行尾表现按 §0 |

## §5 折行规则

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 段落逐字符贪心、列表按词、CJK/宽字符、超长词硬断；标题/代码/表格不折行 | 段落/列表/引用按 content width；标题/代码/表格不折行但裁剪到正文区 | once 段落/列表/引用按 `--width`，标题/代码/表格不裁剪 | `fixtures/spec/{paragraphs,lists,tables,overwide}.md`；`render::paragraph::tests::cjk_and_emoji_wrap_whole_clusters`、`render::list::tests::long_words_hard_break_and_wide_graphemes_stay_whole`、`render::table::tests::overwide_table_keeps_visible_column_prefix_and_never_wraps`；golden `once_paragraphs.ansi`、`once_flow.ansi`、`once_overwide.ansi` | TUI/once 宽度路径差异已记录 |

## §6 特殊字符显示宽度

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| `•│─═` 等 1 格、CJK/emoji/任务标记 2 格、NBSP 1 格、tab 段落 1 格且字节原样 | P11 width table 写 cell/continuation | once 不做 cell 裁剪；折行/表格测量复用同一 width table，输出保留原始 tab/NBSP 字节 | `fixtures/spec/paragraphs.md`、`fixtures/spec/tables.md`；`render::text::tests::reference_width_table_is_explicit`、`cjk_emoji_padding_and_wide_continuations_are_styled`；golden `once_paragraphs.ansi`、`once_tables.ansi`；`tests/static_render.rs` | 无实现差异；TUI 用 continuation cell，once 保留原始字符字节 |

## §9 不可见内容

| 规格要求 | TUI cell 行为 | `--once` 原始字节 | fixture / 测试 / 命令 | 结论 |
| --- | --- | --- | --- | --- |
| 除已按当前 TODO 改为可见的图片外，脚注定义/有定义引用、链接/引用定义行、表格超列、空列表项、空引用行、HTML 标签不产生可见内容；HTML 块不复制参考崩溃 | visibility/块分组过滤，零 cell；HTML 块返回零行 | once 同样过滤，空/空白输出零字节；HTML 块安全跳过 | `fixtures/spec/{images,footnotes,html}.md`；`render::visibility::tests::images_keep_labels_placeholders_and_surrounding_spaces`、`definition_blocks_are_structurally_hidden`、`inline_html_tags_and_comments_disappear_without_touching_text`、`recognized_html_blocks_are_skipped_and_following_paragraph_is_not`；`tests/once_ansi.rs::once_empty_and_whitespace_are_zero_bytes` | 无可见输出差异；HTML 崩溃差异按安全策略记录 |

## 已知差异与审计结论

| 主题 | TUI 行为 | `--once` / 参考行为 | 结论 |
| --- | --- | --- | --- |
| HTML 块 | `spec/html.md` 安全跳过，不 panic | 参考实现会崩溃；本 TUI 明确选择安全跳过 | 安全差异，不复制崩溃 |
| ratatui §8 | 本矩阵不把 ratatui 局限作为 ANSI 降级授权 | ANSI 后端仍使用完整样式/OSC8/underline/表头规则 | 不作为 ANSI 实现差异的豁免 |
| TUI 行模型 | cell 网格、无 CRLF；正常帧 diff/定位、首帧/恢复另走生命周期 | once 才写 `ESC[2J ESC[H`、CRLF、表格内部 LF | 正常帧出现 `2J`/裸 `H` 是回归；两套 oracle 不互换 |
| reset 分段 | P41 增量 writer 可合并相邻同视觉 cell | once 每个语义样式段自包含并 reset，相邻同样式也 reset | 不要求 TUI/once 字节相等，只要求各自协议 |
| 嵌套强调外层前景 | 核心 Style 是位并集，采用 P14 固定前景优先级 | once 复用同一 StyleRole/Theme | 已知规格差异，fixture `inline.md` |
| MATH | 固定策略：默认前景、保留外层属性；不猜测嵌套语法 | once 复用同一 Theme | 已知固定策略，theme tests 覆盖 |
| 超宽不折行块 | 标题/代码/表格裁剪到正文内容区宽 | once 保留完整溢出字节 | 明确双路径差异；`overwide.md` + `once_overwide.ansi` |
| 表格内链接 | 普通链接仅样式；图片保留对象和 OSC8 | once 与 TUI 的对象策略相同 | 图片遵守当前 TODO，普通链接沿用旧参考 |
| `--no-links` | 只关闭 OSC8，保留链接样式/内部焦点 | once 只关闭 OSC8，样式仍保留 | 无差异 |
| 空输入/纯空白 | TUI 是空白网格 | once 零字节，不写 clear | 明确差异，由 §0 记录 |

## 矩阵自检

`crates/mdtui-tui/tests/compliance_matrix.rs` 无网络地校验：上述 §0–§6/§9 标题及 §3.1–§3.13 标题存在、列出的 fixture/golden 文件存在、关键矩阵测试名可在 `tests/once_ansi.rs` 和 `tests/static_render.rs` 中检索，以及差异关键词存在。它只做文档/文件/名称审计，不重复渲染实现。

当前 Unicode 窗口边缘的实体/隐藏标记、超限 Local 占位、稀疏表格分配、裸 URL 长目标及逐帧 OSC8 配额，另由 `tests/workspace_integration.rs` 验收。表格巨大 cell 可恢复精测、宽度上下限和横向滚动另由 `tests/table_policy.rs` / `tests/table_policy_end.rs` 的模型、预算及实际交互验收；目录筛选、折叠和搜索仍未完成，不能由本静态矩阵推导完成。
