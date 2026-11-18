# Native terminal math

The core discovers math and supplies typed, prefix-free payload chunks with
root-source mappings. The TUI never reparses Markdown to find equations. The
ordinary DisplayScanner, TileLayoutSink, viewport, static output and Reading
paths consume these objects. `LayoutSink::math_object` receives current native
metadata; TUI-owned tiles deliver their already prepared native object records.
No TeX parsing, glyph expansion or source lookup runs inside that callback.

The finite language is concatenation/groups; `frac`, `dfrac`, `tfrac`; `^`/`_`;
`sqrt`; the Greek, symbols and operators in symbols.rs; `left`/`right` round,
square, vertical and invisible fences; rectangular `matrix`, `pmatrix`, `bmatrix`
and `aligned`. ASCII math whitespace is ignored, except explicit spacing
commands. Indexed roots, malformed structure and ragged matrices give a visible
`[invalid]` object. Unknown control words remain inert literal lexical atoms;
their groups still group. There are no macros, packages, fonts, file execution,
TeX programming or claims of arbitrary LaTeX compatibility.

For example, `A \(\frac{a}{b}\) Z` renders three rows:

```text
   a
A ─── Z
   b
```

Inline boxes wrap as units and align their baselines. Standalone equations own
their intrinsic height, including an empty object's single row. Quote/list
prefixes and heading roles use the existing TUI policy. Table rows take the
maximum cell height and align first baselines. The continued non-buffering
width measurement uses intrinsic math metrics from the same cache; normal
column width/pan policy still decides the visible columns. Narrow windows clip geometry and retain
whole glyph/source atoms; a glyph wider than its requested rectangle has a
visible ellipsis with the original source. Source refinement also projects the
horizontal coordinate, so a search hit can reveal clipped mathematical ink.

## Preparation, queries and ownership

`MathEngine::new` only validates fixed metadata. Construction and zero-budget
advance allocate no object-sized input, token, AST, layout or output buffers.
`advance` retains a lexer, flat AST and intrinsic layout across genuine prefix
appends. `PendingInput` keeps incomplete tokens/groups; only real payload EOF
seals syntax. `new_at_source_cut` instead retains a pending root-source boundary,
including when that boundary equals the current growing payload end.

`SharedMath` is the document's cache shared by scanner, viewport and Reading.
It binds engines to object epoch and preparation context. Native Update deltas
retire withdrawn/replaced owners without walking all objects; prefix appends
keep their parser. The cache retains the normal object and one frozen
source-cut context per object. These two consumers can progress concurrently
(screen/table precision and Reading's frozen prefix). A different frozen cut retires the prior frozen
owner under Budget; it does not evict the normal owner. Both share a native
epoch liveness token, so withdrawal also invalidates pinned retired payloads.
Every task validates document/object/version and its actual engine owner before
using cached execution state or spending budget. A same-document/object/version
engine with a coincident local layout counter cannot take over paint, locator,
ink or query continuation. Old immutable Cell payload snapshots can stay pinned
across a context change; native withdrawal invalidates their readers.

`paint(Rect)` and `SharedMath::query` use X/Y child indexes and fixed-page DFS
frames. They retain only the requested output rectangle, not a complete object
bitmap. `MathView.cells` has a paged row-major index. `locate_source` seeks a
source-order primitive and pays for its parent projection; fractions need not
have monotone screen/source order. All locations and selections remain root
source spans, never screen-column arithmetic. Empty boxes are excluded from
child window indexes without conflating zero-column ink with empty layout.
A multi-character operator such as `\sin` has one source atom. Its selected
atom copies the full projection even through a one-column viewport.

`MathView::focus_at` supplies an opaque `MathFocus` for an actual projected
cell. It carries the same engine/snapshot identity as paint and ink, its whole
root-source atom, and intrinsic row/column coordinates. `SharedMath::set_focus`
checks the owner and native generation before spending one query/object to
retain that source focus. This disambiguates different glyphs or fence rows
sharing one command span without deriving source offsets from screen columns.
Reading retains the accepted hit's focus; repeat searches, source refinement,
table placement and width restoration use the same native source anchor.

`MathTextCursor` streams row-major cells, geometry spaces and structural
newlines. Its output window advances by 256 columns with one extra column for
the shared policy's complete two-column glyph. It does not retain a whole wide
row or object bitmap. UTF-8 payload readers separately pay for the actual bytes.
Both rendered search and single-object atom copying use this projection.

All potentially object-sized engine records use fixed 64-record pages: input,
post-TeX bytes/glyphs, nodes, links, parser frames, boxes, placements, matrix cells,
prefix summaries, paint stacks and output descriptors. A full page allocates a
new page and never copies an old one. BTree page/index entries cost O(log N),
including the standard library's fixed-node splits; these are not unit CPU
instructions. Paint, query output and replaced engines retire under the same
caller's Budget. Held screen/clipboard payloads delay their data's retirement;
fair continued preparation still progresses. Direct engine/cache teardown is
an explicit non-frame boundary.

The post-TeX provider uses the accepted shared `shape_unicode` GCB/width state
machines and their Unicode provenance, not another math width policy. Ordinary
groups and ignored whitespace are transparent to a display flow; 2D operands
and matrix cells establish boundaries. Generated Greek can join a following
combining mark, and ZWJ/RI sequences retain complete clusters. Controls are
filtered with a boundary rather than attached to an unrelated preceding glyph.
No unfinished cluster is accumulated in a growing String. Cell/diff/wire/copy
readers consume paged UTF-8 under budget. The public `NeedShape` enum variant is
retained for the low-level guard/API; complete `advance` supplies the shared
post-TeX policy rather than returning raw TeX as a substitute.

Stable-width tail joins update the existing glyph/source metadata. A join that
changes display width pays to retire and rebuild intrinsic layout from retained
AST. Its real query cost can therefore be O(A log A) for A retained AST records;
it is not another parse of Markdown or TeX input. Repeated context/geometry
changes are separate queries and are not claimed to have one cumulative input
bound. Core delimiter input retains its separate O(N log N)/O(N) guarantee.

## Actual UI and Reading

The scanner splits at the core's indexed object boundaries before ordinary
source shaping. First preparation decodes/parses the object's committed input;
subsequent rectangle queries reuse it. Viewport geometry chooses the actual
object-relative row window after baseline/table placement. Source-continuation
tiles use the same HeightIndex zero-height tickets as the accepted native
cluster path. Owner heights, fold coverage, End and source jumps remain separate
facts; a late close, withdrawal, native rollback, fold/width change or checkpoint
invalidates/rebinds the corresponding generation.

End can query a tall object's bottom without materializing its preceding rows.
An object ending in the physical tail tile does not replace the later prose
with repeated old equation rows. Ctrl+End uses the typed payload prefix cut and
never manufactures EOF at an incomplete TeX command.

TUI search indexes post-TeX mathematical rows, including intrinsic horizontal
ink outside the current clip. It streams indexed row queries, UTF-8 and geometry
spaces under Budget, retains the existing at-most-256-byte query-prefix
matcher, and maps matches back to complete source atoms. The native DisplayCursor supplies all ordinary logical bytes, including
clipped heading/code/table text and complete entity replacements. It keeps
the existing native search contract for terminal-only image placeholders.
Only the current indexed math object is replaced with intrinsic row-major
text; advancing past its source boundary keeps no visited-object history. A width change clears a mathematical glyph hit while preserving its source
view; n/N can search explicitly again. Ordinary logical hits keep their
existing width-independent behavior even when the document also contains math. It does not match hidden command spelling such
as `frac`; core DisplaySearch remains the separate raw-TeX display interface.
Rendering and visible copy use the runtime's existing FoldView. Projection
precedes context and TeX preparation; a fully hidden object is not decoded.
A source fold inside an equation is a semantic cut: only the visible prefix
starting at its native opener is parsed, with PendingInput rather than an
invented EOF. A suffix whose opener is hidden does not start a second equation.
The fold placeholder remains visible, and reopening restores the full object.
Search explicitly includes hidden targets, like native DisplaySearch, and its
source jump reveals the owner. Its native source cursor and bounded math
windows own neither a second FoldView nor height/checkpoint measurements of
the runtime view.

A selection confined to one fully visible math object streams its selected atoms through
bounded row/column windows, including all glyphs sharing a command source.
Mixed content selections use the existing viewport projection and continued
row windows. Visible copy keeps the canonical Reading separator policy: structural rows remain distinct, ordinary padding is
normalized. Raw copy returns the exact selected original TeX. The existing
64 KiB clipboard limit is an explicit clipboard contract, not a TeX fallback or
parser size limit.

## Costs and evidence

Input source bytes, TeX bytes/tokens/nodes/edges, shaping scalars/joins, layout
edges, paint steps, actual ink output bytes and callback counts are different
quantities. `MathQuery::source_work` counts newly decoded typed-source bytes;
a cached window does not report the whole object's source envelope as reread
input. Native shared shape initialization is separately budgeted/reported.
Window/source seeking is O(log N) plus actual ancestry, intersecting primitives
and output; deep ancestry and large mathematical output remain real work.
Projection search cost includes every scanned output byte and the indexed
window/ancestry work. Two-dimensional output can exceed input size (for example
many nested radical rules); no O(input) bound is claimed for all output.
The engine's indexed paint bound is separate from full UI preparation.
Source cuts, canonical native style/context recovery and the shared source
shape index also consume the caller's budget. An unchanged normal-engine
`source_work` counter does not imply zero bytes for an End or UI request;
examples report the whole parent budget as well as that cache delta.
Optional scan/search tasks use fixed heap descriptors rather than enlarging
all no-math runtime values on the test or UI stack.

`StorageWork.capacity_bytes` reports actual record-page reservations.
Directory/index payload fields exclude BTree headers, slack and allocator
metadata. `MathView.cells.index_payload_bytes` reports the output ordering index
separately. Examples also print actual Linux RSS/HWM and elapsed/max frame time;
allocator and OS scheduling do not have a hard latency guarantee.

```sh
nix develop -c cargo nextest run -p mdtui-tui --test math_engine --test math_ui --test math_pty
nix develop -c cargo run --release -p mdtui-tui --example math_engine_scale -- wide 1000000
nix develop -c cargo run --release -p mdtui-tui --example math_engine_scale -- cluster 1000000
nix develop -c cargo run --release -p mdtui-tui --example math_engine_scale -- deep 10000
nix develop -c cargo run --release -p mdtui-tui --example math_ui_scale -- 1000000
```

Use the task's external heavy-gate lock when running large/full validations on
the shared machine. The finite models, official primary sources and historical
observations remain archived; current math semantics are not scored against a
math-free cmark oracle. No parent TODO94 or independent footnote/body-folding UI
completion follows from this feature.
