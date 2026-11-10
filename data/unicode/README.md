# Fixed reference-label case folding data

`17.0.0/CaseFolding.txt` is the unmodified Unicode 17.0.0 UCD file from
<https://www.unicode.org/Public/17.0.0/ucd/CaseFolding.txt>, retrieved 2026-11-10.
`LICENSE.txt` is the complete Unicode License V3 copyright/permission notice from
<https://www.unicode.org/license.txt>, retrieved the same day. Both files and their
SHA-256 digests are pinned by `scripts/generate_case_folding.py`.

Regenerate with `python3 scripts/generate_case_folding.py > crates/mdstream-core/src/case_folding.rs`.
Verify offline with `python3 scripts/generate_case_folding.py --check`. The script
uses only the Python standard library and local files; builds/tests never run a
generator or download data. It does not use the host Python/Rust Unicode version.
The derived Rust table is distributed with the accompanying Unicode License V3.

The table contains all 1585 C/F records (default full folding), excludes S/T,
and leaves every absent scalar unchanged. A scalar expands to at most 3 scalars
or 6 UTF-8 bytes; output/input byte ratio is at most 3. Outputs are already folded.
Whitespace folding still uses the existing Rust `char::is_whitespace` contract;
the fixed version applies to case folding. No normalization or escape/entity
decoding is added. Cherokee folding can produce uppercase keys.

For a label of B raw bytes and U scalars, validation rejects B > 999 before
folding. Each scalar is visited once, with at most 12 table comparisons, and
the normalized key has at most 3B bytes (including collapsed spaces). The
fixed-size table makes normalization O(B). The existing bounded-key BTreeMap
adds O(log S) per intern; overall parsing stays cumulative O(n log n), space O(n).
Definition updates touch one symbol and its first-use range, never old references.
