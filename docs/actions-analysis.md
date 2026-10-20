# Photoshop Actions investigation

Investigation date: 2026-10-18. The user explicitly requested read-only REA
analysis of the installed Photoshop Actions subsystem, followed by an independent
Prism implementation with Clip Studio Paint actions connected through an adapter.
This extends the earlier brush investigation only to the requested Actions scope.
No Photoshop process was launched or attached, and no account data was read.
Proprietary code, decompilation, presets and assets are not implementation inputs.

## Target and evidence boundary

| Item | Observed value |
| --- | --- |
| Executable | `C:\Program Files\Adobe\Adobe Photoshop 2026\Photoshop.exe` |
| File version | `27.7 (20260514.r.11 fa4476d)` |
| Size | 267,588,080 bytes |
| SHA-256 before and after | `ec2758175e5da6500525bfed74bacfc26c1ca4f81e256646572f2c871166fbb4` |
| Analysis tools | Existing local REA 6.4.0, Node 24.21.0, Ghidra 12.1.4, Temurin JDK 21.0.12.1+1 |
| Local evidence | Gitignored `target/rea-tools/photoshop-actions/` |

REA's `inspect_pe_resources` completed: **790 resources**, **1,586 examined
directory entries**, complete inventory within its stated scope. Evidence ID:
`ev_5e8622c0fd8c2f9ec8ced547ca82176e5bc220688b409d9a19815a4e361a59a0`.
The retained response is `inspect_pe_resources.json`; its limitations say that
non-icon payloads remain opaque. This establishes resource identity/ranges,
**not Actions execution semantics**.

REA `open_binary` identified the same x86-64 PE and digest. The subsequent
literal `search_strings("ActionSet")` returned `provider_timeout`, with a
60,000 ms operation timeout. The provider required broader Ghidra startup before
answering the focused query. That query supplied no successful string or code
evidence; it was not retried. The owned analysis JVM was selected for bounded
cleanup, and `close_binary` completed successfully. The installed executable's
digest remained unchanged (`identity.json`). No full-program decompilation or
runtime state-machine reconstruction is claimed.

The independent static observations below are intentionally **not labelled REA
Evidence**. `scan.mjs` records literal file offsets and PE section-mapped virtual
addresses in `static-strings.json`; `rtti.mjs` decodes selected MSVC x64 RTTI
records in `rtti.json`. Both operate on the same hashed executable. These local
scripts/results remain ignored and are not shipped with Prism.

## Static observations

The selected RTTI complete-object locators have signature 1 and a self RVA
matching their file-to-image mapping. Each points to its named type descriptor
and a bounded base-class array:

| Selected type | Complete-object locator VA | Recorded base sequence |
| --- | --- | --- |
| `TActionSet` | `0x14c661b48` | `TActionSet`, `TActionNode`, `TOutlineTree` |
| `TAction` | `0x14c661b20` | `TAction`, `TActionNode`, `TOutlineTree` |
| `TActionCommand` | `0x14c661af8` | `TActionCommand`, `TActionNode`, `TOutlineTree` |
| `TActionNode` | `0x14c661ad0` | `TActionNode`, `TOutlineTree` |

**Observation:** the image encodes separate set, action and command types with
shared tree-node ancestry. **Inference:** a tree-oriented model is consistent
with the documented Actions hierarchy. RTTI alone does not establish allowed
child relationships, persisted fields, object layouts or playback algorithms.

Selected literal locations corroborate distinct controls. The table describes
their meaning without reproducing the complete proprietary string catalog.

| File offset | Literal role |
| --- | --- |
| `0x0a9dccb8` | Edit a recorded parameter value |
| `0x0a9dccf0` | Show a dialog during action playback |
| `0x0a9dcd40` | Toggle a playback step |
| `0x0c094ca0` | Warning for changing all command states in a set |
| `0x0c094db0` | Separate warning for changing all dialog states in a set |
| `0x0c011338` | Insert Stop menu item |
| `0x0bfe17c8` | Continue label for action playback |
| `0x0c1d5a20`, `0x0c1d5b10` | Separate creation controls for sets and actions |
| `0x0c1d5b80`, `0x0c1d5bf8` | Stop playback/recording and play the selection |
| `0x0a9f5a10`, `0x0a9f5a30`, `0x0a9f5a50` | Accelerated, stepwise and timed-pause playback tokens |

The independent `stop-strings.json` scan also records node move/duplicate/delete
type names. String presence establishes neither executed code paths nor whether
the classic or modern panel is selected in this installation. The executable
contains both classic and modern Actions panel labels, including newer preview
and recommendation features. Those features are outside this investigation.

## Public behavior contract

Adobe's documentation was checked on the investigation date. It supplies the
behavior contract; local binary observations corroborate that this installed
build contains the corresponding structure and control labels.

- An action records a sequence of commands. New actions have a set, and recording
  can include tools when enabled. See [Record an action](https://helpx.adobe.com/photoshop/desktop/automate-tasks/create-record-actions/record-an-action.html).
- Sets organize actions; commands may be duplicated and reordered. Playback can
  begin at a selected command, or execute only that command. The accelerated,
  stepwise and timed modes control redraw/delay, rather than deleting commands.
  `.atn` is Photoshop's action-set file extension. See [Play and manage actions](https://helpx.adobe.com/photoshop/using/playing-actions.html).
- A command's checkmark excludes it from playback while keeping it in the action.
  See [Exclude commands](https://helpx.adobe.com/photoshop/desktop/automate-tasks/create-record-actions/exclude-commands-from-an-action.html).
- A modal control pauses before applying the selected command so its settings can
  be adjusted, then playback continues. This is distinct from whether the command
  is enabled. See [Change settings during playback](https://helpx.adobe.com/photoshop/desktop/automate-tasks/create-record-actions/change-settings-when-playing-an-action.html).
- An inserted stop displays a message and permits manual work before Play resumes.
  A Continue button may be included when immediate continuation is appropriate.
  A stop can be inserted while recording or afterwards. See [Insert a stop](https://helpx.adobe.com/photoshop/desktop/automate-tasks/create-record-actions/insert-a-stop.html).

## Independent Prism design implications

These are engineering choices informed by the behavior contract, not recovered
Photoshop implementation details:

1. Use a common set/action/step model. A step needs independent inclusion and
   dialog flags; a stop needs an explicit message and continuation policy.
   Existing flat Prism action files need defaults/migration rather than data loss.
2. Keep pending playback position separate from recording state and persistent
   action data. A disabled step is retained and skipped. A dialog request suspends
   before applying that step; a stop resumes after the stop. Headless clients must
   receive an actionable suspended result rather than silently bypassing input.
3. Keep command authorization on every resumed or nested command. Stop/cancel,
   command failure and unsupported import are distinct outcomes. A source action
   is not a sandbox exemption.
4. Treat CSP LAF as an import adapter into that model. Preserve set/action names,
   step inclusion, settings-dialog intent and source command identity. A CSP step
   expanded into several Prism commands needs a shared source-step relationship,
   so disabling/editing it cannot silently leave a partial conversion running.
5. Preserve unsupported source operations with an explicit status, or reject
   their execution before editing the document. Never replace an unsupported
   operation with a plausible but different command, and never silently drop a
   disabled source step merely because it is currently unchecked.

The prior `actions.importLaf` implementation was an adapter to Prism's existing
generic Actions recorder/player, not evidence that the whole Actions core was
originally designed around CSP. Its source-format details and real-file findings
remain in [SUT/CSP analysis](sut-analysis.md).

## Coverage and unknowns

| Question | Result |
| --- | --- |
| Separate set/action/command concepts in this build? | Answered statically by RTTI; documented hierarchy corroborates meaning. |
| Step enable and dialog controls separate? | Answered as public behavior; separate installed labels corroborate it. Internal flags remain unknown. |
| Stop and resume expected behavior? | Answered by public documentation; stop/continue literals corroborate availability. Runtime cursor mechanics remain unobserved. |
| Exact playback scheduling, undo grouping, modal cancellation internals? | Unresolved; no successful function-level REA result or runtime trace. |
| Full `.atn` parser/export compatibility? | Not investigated or implied by this report. A Photoshop-style action model alone does not provide `.atn` compatibility. |
| Pixel-equivalent execution of arbitrary Photoshop/CSP actions? | Not established. Each Prism command mapping needs independent validation. |

Product implementation and tests must state their supported subset separately.
This report records a bounded static investigation and public behavioral contract,
not a claim of complete Photoshop Actions parity.
