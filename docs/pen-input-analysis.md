# Windows pen input investigation

Investigation date: 2026-10-14. This report separates binary observations from
engineering choices and unresolved behavior. It is not a claim of Photoshop
source recovery or brush equivalence.

## Authorization and implementation boundary

The user explicitly authorized a read-only local REA/Ghidra investigation of
their installed Photoshop executable for Windows pen input and brush behavior.
This is a narrowly scoped exception to the usual behavior-only investigation
rule for this task. It does not authorize copying proprietary implementation,
shaders, profiles, assets, or decompiler output into Prism. Prism's implementation
uses its own Rust code and public Windows input contracts. No proprietary binary
or pseudocode belongs in source control.

## Target identity and tools

| Item | Observed value |
| --- | --- |
| Selected executable | `C:\Program Files\Adobe\Adobe Photoshop 2026\Photoshop.exe` |
| File version | `27.7 (20260514.r.11 fa4476d)` |
| Product version | `27.7` |
| Size | 267,588,080 bytes |
| SHA-256 | `ec2758175e5da6500525bfed74bacfc26c1ca4f81e256646572f2c871166fbb4` |
| REA package | `rea-agents@6.4.0`, published Windows x64 native bundle |
| Node | `24.21.0`, portable Windows x64 |
| Ghidra | `12.1.4 PUBLIC`, Windows x64 |
| JDK | Eclipse Temurin `21.0.12.1+1`, full Windows x64 JDK |

The analysis tools live in the ignored `target/rea-tools` directory. They are
not dependencies or build steps of Prism. Node and Ghidra archive SHA-256 values
were matched against their publishers' release metadata; the JDK archive matched
Adoptium's published checksum. REA was installed through npm at the exact version
above with optional dependencies and package install scripts disabled.

REA's source instructions were read from the user-selected checkout at
`E:\0.Project\other\rea`; the published package supplies its compiled CLI and
matching Windows native addon. No MCP registration or global toolchain settings
were changed.

### Local launcher accommodation

The initial AppContainer run could not complete REA's native path-handle checks.
The authorized execution outside that sandbox then admitted the target and verified
an immutable snapshot against the same SHA-256. This is an executor permission
change, not permission elevation of Photoshop, which was never executed.

Ghidra initially exited before analysis: the user's `cmd` AutoRun emitted a
missing `doskey_aliases.txt` diagnostic under the private runtime's user-home
directory. Ghidra's `FOR /F` VM-options loop captured that diagnostic as a JVM
class name. Only the isolated Ghidra `support/launch.bat` was adjusted to accept
VM-option output lines beginning with `-`; its original remains alongside it as
`launch.bat.original`. The user's registry, shell configuration and Photoshop
installation were not edited.

## Evidence and current limits

Local full REA responses, requests and logs are retained under
`target/rea-tools/{evidence,requests,runtime}`. Raw-byte scans are explicitly
identified separately from REA Evidence. Addresses and file offsets refer only
to the exact target digest above. String presence alone does not establish a
runtime call, selected tablet backend, pressure formula, or brush algorithm.

### Confirmed independent PE observations

`evidence/pe-input.json` records an independent parse of the executable's PE
import table and selected literal byte sequences. It repeats the target hash
and maps file offsets through section headers; it is not REA Evidence.

| Import from `USER32.dll` | Import-address-table VA | Name file offset |
| --- | --- | --- |
| `GetPointerPenInfoHistory` | `0x14a793278` | `0x0d516ad2` |
| `GetPointerPenInfo` | `0x14a7931f8` | `0x0d516aa4` |
| `GetRawPointerDeviceData` | `0x14a793200` | `0x0d516ab8` |
| `GetPointerDeviceRects` | `0x14a7931f0` | `0x0d516a8c` |
| `GetPointerDeviceProperties` | `0x14a7931e8` | `0x0d516a6e` |

The executable also contains `WinTab32.dll` at file offset `0x0c1d2998`,
`WTPacketsGet` at `0x0c1d2f88`, and `WTPacketsPeek` at `0x0c1d2fb8`.
`UseSystemStylus` appears at `0x0c193880`; a diagnostic mentioning
`DrainPointerMessages` occurs at `0x0c1a4035`.

Two `FF 15` RIP-relative call-encoding candidates point to the
`GetPointerPenInfoHistory` import slot at VAs `0x148b75b93` and
`0x148b76c0f`. The focused Ghidra pass below validated both as actual decoded
indirect `CALL` instructions.

The PE exception directory places those candidates inside runtime-function
ranges `[0x148b75a36, 0x148b75d05)` and `[0x148b769aa, 0x148b76c4b)` respectively
(table file offsets `0x0df14970`, `0x0df14a18`). A runtime-function record can
describe an exception-handling fragment; it is not by itself proof of an
original source-level function boundary.

The initial default Ghidra analysis spent more than ten minutes in
`Disassemble Entry Points`. It was stopped through its owned JVM and REA's
provider cleanup, and the temporary analysis project was removed. A focused
retry applied both runtime-function seeds successfully (seed digest
`33a8c751034db6465fedbc9e23506099da699ced7ba54e298ca1f8d3bbba42b7`). However,
the published REA 6.4.0 package does not implement the checkout's documented
`REA_GHIDRA_DISABLED_ANALYZERS` setting: it was ignored, so broad analysis resumed.
That attempt was stopped and `close_binary` returned successfully. This release
boundary is recorded rather than presenting the setting as effective.

The fallback invokes the same Ghidra installation directly with `-readOnly`,
`-deleteProject`, and `-noanalysis`. A local Java script disassembles/decompiles
only the two runtime-function starts. Its retained output is explicitly direct
Ghidra observation, not REA Evidence, and does not cover the executable's
remaining code. The input is a separate executable copy whose SHA-256 matches
the original; this avoids resolving libraries beside the installed executable.
An initial direct import from the installation directory was stopped during
library metadata resolution. No target code is executed.

### Confirmed focused Ghidra observations

The final isolated-copy import completed within five minutes. The log reports
zero additional files loaded, successful execution of `PenInputFocused.java`,
and discard of the import under `-readOnly`. Both selected fragments report
`DECOMPILE COMPLETE: true`. The shell launcher nevertheless returned exit code
1; success here refers specifically to those retained script outputs, not a
clean full-program analysis run.

Evidence: `target/rea-tools/evidence/ghidra-focused-functions.txt`, SHA-256
`ebdaa7436723ae1e1921bbba76915d3cdb3f8903407bad99bf5d99e93e50624e`;
launch/script logs use the `ghidra-direct-snapshot` prefix in that directory.
This is direct Ghidra output without a REA Evidence ID. It remains local and
ignored; no assembly or decompiler body is copied into Prism source.

- At `0x148b75b93` and `0x148b76c0f`, decoded calls target the imported
  `GetPointerPenInfoHistory` slot. These establish real static call sites, not
  merely API-name literals.
- On successful return, both paths start at the returned count minus one and
  walk the history array backwards with a `0x78`-byte stride. The loops call a
  conversion helper for each entry (`0x148b74120` or `0x148b73e90`) before
  copying the converted record into an internal queue and incrementing its
  count. Loop anchors are `0x148b75be0`–`0x148b75cf8` and
  `0x148b76c60`–`0x148b76d78`.
- The second fragment also calls the imported `GetRawPointerDeviceData` at
  `0x148b76a9d`. The semantic meaning of each requested raw property was not
  followed into the conversion helpers.

Microsoft documents that
[GetPointerPenInfoHistory returns newest entries first](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerpeninfohistory).
**Inference from that contract and the decoded loops:** these Photoshop paths
process the recovered history in chronological order and retain per-entry
records instead of reducing the batch to one latest sample. The inspected code
does not establish the contents of every converted field or the active path on
the user's device. The seeded starts are unwind fragments with unrecovered
incoming register state, so generated function signatures are not reliable API
reconstructions.

**Inference:** the Windows executable contains support for pointer-history input
and WinTab-related handling. This supports investigating history preservation
in Prism. It does not establish which backend Photoshop selected on this user's
device, how it normalizes pressure, or whether a given brush uses that path.

**Unresolved:** start-pressure initialization, normalization curve, tail
interpolation, stationary dab scheduling, brush-size filtering, and the user's
actual hardware event trace. The bounded static investigation is complete at
the scope above; these questions remain open. The original executable's SHA-256
was verified unchanged after the investigation.

| Investigation question | Status and evidence boundary |
| --- | --- |
| Does this build contain Windows pointer-history support? | Answered statically: two decoded calls, chronological per-entry loops and queue insertion. Selected runtime path remains unobserved. |
| Does it contain WinTab support? | Partially answered: API/library literals present; resolution and invocation unobserved. |
| What pressure curve does it use? | Unresolved; imports do not reveal normalization or brush dynamics. |
| How does it start and end a stroke? | Unresolved; the supplied screenshots show outcomes, not events. |
| How does a stationary pressure increase produce dabs? | Unresolved; requires the relevant code path or physical input observations. |
| Can Prism's fixes claim Photoshop algorithm parity? | No; validate Prism's own input preservation and renderer behavior independently. |

## Comparison protocol

Use a hard round brush with matching diameter, hardness, spacing, minimum
diameter, pressure-size control, opacity, flow and smoothing. Record document
zoom and pixel depth alongside these settings. Compare a light initial touch,
stationary pressure increase, progressively released tail, quick tap, rapid
successive strokes and a prolonged stationary contact on the same physical pen.
The user's supplied images establish the reported visible difference, but do
not establish matched settings or identical input samples.

A separate read-only Photoshop descriptor query captured the current brush:
25 px, 100% hardness, 5% spacing, 0% minimum diameter, pressure size control and
size override enabled, 100% opacity/flow, opacity pressure disabled, smoothing
enabled at 0%, and pressure smoothing disabled. The exact descriptor and the
matching Prism comparison settings are recorded in
[pen-input-validation.md](pen-input-validation.md). These are current settings,
not proof that the earlier screenshots used them. The query changed no settings
or documents.

Scripted Photoshop paths and Prism synthetic strokes are deterministic rendering
checks; they are not measurements of physical pen packets. Synthetic replay can
prove that Prism is invariant to frame batching and preserves pressure events.
It cannot prove that Photoshop received identical physical samples. Actual pen
tests and exact Photoshop pressure/taper algorithm recovery remain separate
acceptance work.
