# Clip Studio Paint brushes

Prism can import `.sut` brushes through File › Open, drag-and-drop, or the Brushes panel's
Import Brushes… button. The imported brush appears in a group named after its file and is
selected automatically. The brush library persists it across restarts.

Agents can use the same importer without a document:

```json
{"command":"brush.presets.importSut","params":{"path":"/path/to/brush.sut","select":true}}
```

The command also accepts base64 `data`, an optional `group`, and `replace` (default true).
Capability-restricted MCP/control callers must pass `data`; ambient filesystem `path` reads
are refused by the same policy as ABR imports.
It returns `format`, `group`, `count`, `imported` names and `warnings`. Re-import replaces the
same user group; built-in brushes are preserved. Invalid files leave the library unchanged.

## Supported subset

The reader loads the current named sub-tool's settings from a SQLite rowid database (UTF-8)
and PNG images from its referenced primary materials. It imports size, opacity, flow,
hardness, spacing, thickness, rotation, flips and the pen-pressure size flag. Multiple tips
are used cyclically in reference order. Sampled tips retain black coverage and transparency
in 16-bit masks, and can paint 8/16/32-bit documents.

CSP custom pressure curves, automatic spacing and stabilisation are approximated. Alternate
or random tip ordering, textures, dual brushes, spray, ribbons, taper, wet edges and colour
mixing are not reproduced faithfully. Import warnings explain these limits. This is an
initial compatibility path, not a claim of identical CSP rendering. Only the first named
sub-tool is imported; collections and other material layouts are not supported.

Limits: 64 MiB per SUT/database, 4096 rows per table, 32 b-tree levels, 64 tips,
2500 px per tip edge and 16 million total tip pixels. The brush preset store additionally
limits each persisted group to 16 MiB. Corrupt page links, overflow cycles, missing materials,
unsupported encodings and oversized PNGs produce errors.

## Implementation and validation

The implementation is native Rust and uses the existing Prism PNG codec. It does not execute
SQL or run Python, SQLite native libraries, or CSP2PC at runtime. Format research used
[CSP2PC](https://github.com/Leon-Schoenbrunn/CSP2PC) and the
[public SQLite file-format specification](https://www.sqlite.org/fileformat.html); no converter
code, template brushes or user-owned SUT files are included in the source tree.

Synthetic tests exercise masks, tip order, SQLite overflow pages, corrupt files and cancellation.
A local-only engine test can exercise the initial user-provided oracle without committing it:

```sh
PRISM_TEST_SUT=/path/to/세모네모펜.sut cargo test -p prism-engine local_sut_import -- --ignored
```
