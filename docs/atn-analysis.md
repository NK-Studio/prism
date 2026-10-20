# Photoshop ATN investigation

Investigation date: 2026-10-18. The maintainer requested direct `.atn` import
and explicitly authorized local read-only REA investigation of the installed
Photoshop Actions reader. This report extends [Actions research](actions-analysis.md).
No Photoshop process was launched or attached, and no account data was read.
Implementation is independent: proprietary code, pseudocode and presets remain
in ignored local research output and are not copied into product code or fixtures.

## Identity and evidence boundaries

| Item | Observation |
| --- | --- |
| Target | `C:\Program Files\Adobe\Adobe Photoshop 2026\Photoshop.exe` |
| File version | `27.7 (20260514.r.11 fa4476d)` |
| Size | 267,588,080 bytes |
| SHA-256 | `ec2758175e5da6500525bfed74bacfc26c1ca4f81e256646572f2c871166fbb4` |
| Tools | REA 6.4.0, Node 24.21.0, Ghidra 12.1.4, Temurin JDK 21.0.12.1+1 |
| Local output | Gitignored `target/rea-tools/photoshop-atn/` |

The executable digest matched before and after the REA session (`identity.json`),
and the direct-analysis copy matched the original (`direct-identity.json`).
The installed ATN files were also hashed and rechecked unchanged
(`corpus.json`, `source-integrity.json`). These local files identify evidence;
they are not checked-in test fixtures.

REA `open_binary` succeeded and identified the matching x86-64 PE. A new,
specific request, `procedure_pseudo_code` for `0x149b4d550`, returned
`provider_timeout` after its 60,000 ms provider limit. It supplied no function
Evidence. `close_binary` completed successfully. The previous Actions
`search_strings` timeout was not repeated. Request/result JSON and the tool
schema are retained locally. No successful REA function Evidence ID is claimed.

The following executable observations used direct Ghidra as an explicitly
labelled fallback. They are **not REA Evidence**. Ghidra imported a hashed local
copy with `-readOnly -noanalysis -deleteProject`; only selected entry points were
requested. Raw decompilation stays ignored. The report contains observations and
addresses, not proprietary implementation text.

## Reader investigation

An independent PE string scan found the Actions-load error, newer-file-version
error, `ActionsLoad`, and the `TReadActionDescriptor` RTTI name. RIP-relative
reference candidates were matched to PE exception-table function ranges
(`reader-strings.json`, `xref-candidates.json`). String presence alone does not
establish a parser or semantics.

Direct Ghidra completed all three initial function requests (`focused.txt`):

| Address | Bounded observation | Interpretation |
| --- | --- | --- |
| `0x1410fddf0` | References the Actions-load error and passes `0x1410fdf10` as a callback; invokes `0x1410f5cc0` on success | Actions load orchestration candidate |
| `0x1410ff130` | References the same error and callback; passes the `8BAC` file-type constant | Stronger corroboration of Actions-file loading |
| `0x149b4d550` | Selects the newer-action-file-version diagnostic for a specific error state | Version-error handler; not the byte reader |
| `0x1410fdf10` | Reads an integer through `0x146ee2780`, checks a lower bound and a helper-derived upper bound, then delegates to `0x1467f96b0`; its rejection error matches the version handler | Load callback with version-gate candidate |
| `0x1410f5cc0` | Traverses loaded entries and calls collection-integration/update operations | Post-load integration candidate |
| `0x146ee2780` | Reads four bytes from a stream and conditionally reverses byte order | Integer input helper used by the callback |
| `0x1467f42c0` | Returns 16 normally; returns 18 for the observed callback argument under another internal state | Version upper-bound helper; state meaning unknown |
| `0x1467f96b0` | Repeats record loading while stream position is below its end; uses different loading paths below version 10 and from version 10 onward | Actions collection reader, corroborating consecutive records after one version header |

The two callback/integration requests and final three helper requests also
completed (`reader-focused.txt`, `format-focused.txt`): **eight functions total**,
all with successful decompilation. All three direct-analysis processes exited
normally. Function names assigned by Ghidra are synthetic; some automatically
applied library names are unreliable and were not used to infer semantics.

Taken together, the callback accepts a version value above 5 and at most the
helper's upper bound (16 or conditionally 18). This is a static observation of
the inspected path, not tested compatibility with every intervening version.
The state enabling 18 and the version-17/18 layouts were not investigated.
The reader's stream-end loop independently supports the installed Essentials
multi-set observation. Its virtual per-record readers were not traced further.
Full-program analysis and complete reader reconstruction are outside this
bounded investigation; no implementation was transcribed from these functions.

## Public contract

Adobe documents a big-endian version-16 Actions format containing set, action
and step records. Step state includes enabled, dialog and dialog-options bytes;
events can be string IDs or four-byte IDs. A signed descriptor-presence marker
precedes typed descriptor data. The public table describes one set per file.
This is a format contract, not a guarantee that Prism can execute every command.
See [Adobe Photoshop File Formats Specification, Actions](https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/).

Adobe's current action-descriptor documentation distinguishes command, target
reference and parameters. References can select by class, ID, index, name or
enumeration, and reference chains run from specific to containing objects.
Its scripting dialog modes distinguish silent execution, conditional UI and
displayed UI. No equivalence between those strings and every ATN numeric
dialog-options byte was established here. See
[Adobe BatchPlay details](https://developer.adobe.com/photoshop/uxp/2022/ps-reference/media/batchplay).

## Independent installed-file observations

The installed `Presets\Actions` directory contains nine files. The separate
parser investigation also included two files in `Required`. An independent
Python decoder and the new Rust parser both consumed all 11 files exactly:
**17 sets, 181 actions, 1,673 steps**. Evidence is in `corpus.json`,
`parser-local-tests.log` and `source-integrity.json`; these figures measure
structural decoding, not Photoshop-equivalent execution.

| Installed file | Sets | Actions | Steps |
| --- | ---: | ---: | ---: |
| `Required\Default Actions.atn` | 1 | 12 | 160 |
| `Required\Essentials.atn` | 7 | 37 | 165 |
| `Presets\Actions\Commands.atn` | 1 | 22 | 23 |
| `Presets\Actions\Frames.atn` | 1 | 14 | 307 |
| `Presets\Actions\Image Effects.atn` | 1 | 19 | 160 |
| `Presets\Actions\LAB - Black & White Technique.atn` | 1 | 1 | 16 |
| `Presets\Actions\Production.atn` | 1 | 17 | 30 |
| `Presets\Actions\Stars Trails.atn` | 1 | 1 | 89 |
| `Presets\Actions\Text Effects.atn` | 1 | 17 | 236 |
| `Presets\Actions\Textures.atn` | 1 | 26 | 351 |
| `Presets\Actions\Video Actions.atn` | 1 | 15 | 136 |

An initial independent JavaScript probe consumed eight of the nine Presets files
exactly and stopped on an unhandled `ObAr` descriptor value in `Frames.atn`.
The parser investigation subsequently resolved the observed `ObAr` and `UnFl`
structures and corroborated all files. This distinction preserves the history
of what was observed versus what the initial probe supported.

Observed details relevant to an independent implementation:

- `Essentials.atn` has one version header followed by seven consecutive sets.
  The public one-set description is insufficient for this installed file.
- Set/action names can contain localization keys with a fallback after `=`.
  Preserve the original source string while displaying its fallback.
- The observed Unicode lengths include a terminating UTF-16 null. Step events
  predominantly use `TEXT` string IDs. Their dictionary names and descriptor
  class IDs are separate fields; they are not interchangeable event aliases.
- After the descriptor-present marker, observed ATN descriptors begin directly
  with their descriptor structure, without an additional version-16 prefix.
- Reference `Idnt` and `indx` entries in the installed files carry class data
  before their integer. A PSD descriptor helper with a different assumption
  cannot be reused without explicit ATN handling.
- Typed values, nested descriptors, lists, references, units and opaque payloads
  occur. Preserve type distinctions and reject unknown unframed value types;
  guessing a skip length can desynchronize the remainder of the file.

## Execution implications and unknowns

Decoded event names alone do not establish a safe Prism command mapping.
Concrete observations from `Commands.atn` demonstrate the ambiguity:

| Event | Observed descriptor target | Consequence |
| --- | --- | --- |
| `flip`, `rotateEventEnum` | Active layer reference | Do not reinterpret these as whole-image transforms |
| `clearEvent` | History-state property of the target document | Do not reinterpret as deleting selected pixels |
| `make` | Snapshot class | Do not reinterpret as a new layer |
| `select` | Menu-item enumeration | Do not reinterpret as layer selection |

The installed LAB action contains `stop` with a message and `Cntn=true`;
those encode useful Stop/Allow Continue data independently of its dialog flag.
An `imageSize` step has no descriptor, dialog byte zero and dialog-options byte
one. Its missing parameters must not be silently invented. The complete meaning
of numeric dialog options remains unverified.

References to Photoshop object IDs, files, plug-ins, scripts, external presets
or environment-specific resources require separate resolution. Their complete
portability and execution semantics are unknown. Preserve unsupported commands
and source descriptors visibly rather than substituting guessed behavior.
Import should construct editable action data without executing it, resolving
external paths or contacting Photoshop services.

No Photoshop runtime playback oracle was used for these files. Structural
coverage is broader than supported execution: unsupported operations, class
targets, units, extra descriptor keys or missing required parameters must stay
unsupported until independently implemented and tested. File layouts beyond the
observed version 16, all future descriptor types, and compatibility with
arbitrary third-party ATN files remain unknown, despite the executable's
conditional higher version gate.
