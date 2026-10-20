# Actions

Prism uses the classic Photoshop Actions workflow as its primary interaction model:
sets contain actions, and actions contain ordered commands with independent execution
and settings-dialog controls. Photoshop `.atn` and CLIP STUDIO `.laf` files import
into this same model.
The evidence and limitations of the installed Photoshop investigation are in
[Actions analysis](actions-analysis.md).

## Using the panel

Open Window > Actions. Create an action with its name and set, perform edits, and
stop recording. The Record button appends to an existing selected action. Expand an
action to select or drag steps; Play starts at the selected step, or the beginning
when its action header is selected. The first step checkbox controls execution; the
second asks for settings before that step runs. Double-click a step to edit its
recorded scalar settings. Nested/array settings are preserved; this form currently
does not edit them. Parameter choices are validated by the command on execution.

Right-click an action to change its set, duplicate it, insert a stop or assign a
function key. Right-click a step to duplicate it or insert a stop after it. Moving
an action onto an action in another set moves it into that set. Sets are derived
from their members, so an empty set is not retained. Action names remain globally
unique for compatibility with existing shortcuts, nested calls and droplets.

An inserted stop displays a message, then leaves the document available for manual
work. Play/Resume continues after it. Allow Continue adds immediate continuation to
the notice. A settings dialog pauses before the command and its changed values
apply to this execution; the recorded values remain unchanged. Canceling the dialog
leaves playback paused. Cancel Action Playback discards the remaining continuation.
Definitions cannot be changed while paused; document edits are allowed. The panel
keeps Play/Stop/Record visible while its rows scroll, including in a short dock.
While paused, Play resumes and Stop cancels; hover the paused label to read the message.

Nested calls retain their parent continuation across pauses. A command failure stops
playback and keeps completed edits. Every command is authorized again when resumed.
Playback has a 16-frame nesting limit and 100000-step execution budget per run.
Interactive playback while recording another action is rejected at its pause point;
ordinary nested playback records a named action call. Script wrappers have no action
continuation protocol: an interactive child stops their execution with an error.
Inside an action, call another action directly instead of through a script wrapper.
The initiating authorization policy remains attached to a paused action and is
combined with the policy of the request that resumes it.

## Automation and persistence

- `actions.record {name,set}` creates and records; `{action}` appends.
- `actions.list {}` reports names, sets, counts, recording and paused state.
- `actions.get {action}` returns `items` with all flags/provenance, plus `steps`
  for batch/droplets. Disabled steps are omitted from the batch plan; interactive
  steps retain an explicit guard that fails rather than silently bypassing input.
- `actions.configure {action,step,enabled,dialog,params}` edits a step;
  `{action,set}` changes its set. Parameters replace the recorded object.
- `actions.move`, `actions.delete`, `actions.duplicate` edit actions or steps.
- `actions.insertStop {action,after?,message,allowContinue}` inserts a pause.
- `actions.play {action,from?}` can return `paused` instead of completion.
  `actions.resume {params?}` continues; `actions.cancel {}` discards it.
- `actions.importLaf {path|data,replace?}` imports a supported CSP action set without
  executing it. Restricted callers use inline `data`.
- `actions.importAtn {path|data,replace?}` imports Photoshop version-16 ATN sets.
  The response reports each action's `ready` state, `unsupportedEnabled` count,
  and a `blockedReason` when a direct or nested imported step prevents playback.
  `ready` means no known unsupported step, not that the current document satisfies
  every command's requirements. Restricted callers use inline base64 `data`.

The desktop control channel accepts `ui.set {"actions":{"selected":0,
"selected_step":null,"expanded":[true],"collapsed_sets":[]}}` for panel view
state. Definition edits and playback still use engine commands. Dialogs use the
existing `ui.dialog.set` / `ui.dialog.confirm` protocol.

`actions.json` now writes version 2 with structured steps. Version 1 command pairs
remain readable and default to enabled, without dialogs, in Default Actions.
Recording and paused playback are transient and do not survive restart. Existing
action names, shortcut keys and batch command-pair input remain supported.

## Importing Photoshop files

Use File > Open or drop an `.atn` file into Prism. The Actions panel opens with
the imported sets. Importing reads definitions only; it does not run the actions.
Select an action and press Play to use the supported steps on the active document.
If a step is unsupported, its original name and reason appear in the panel.
Disable that step with its execution checkbox or replace it before playback.
Skipping a step can change the action's result.

The importer translates supported targets and parameter combinations for Invert,
Desaturate, Threshold, Brightness/Contrast, Gaussian Blur, High Pass, Median,
Motion Blur, Unsharp Mask, Feather, Find Edges, Solarize, basic clipboard commands,
selection All/None/Inverse, basic color-mode conversions, new/duplicate/delete
current layers, Flatten/Merge Visible, basic fills, Stop, and named calls to
actions in the same imported file. Unsupported targets, extra parameters or
units remain visibly unsupported; they are never silently omitted.

Enabled/dialog flags, source names, event IDs, raw descriptors and shortcut/color
metadata survive saving the library. Imported shortcuts are not assigned
automatically. Duplicate names in different sets are qualified as `Set / Action`,
and nested calls follow that mapping. A name collision with an existing action
fails atomically unless `replace:true` replaces that action in the same set.
Empty sets are omitted. The command limits source files to 4 MiB and the saved
library to 16 MiB; the low-level parser has its own bounded 64 MiB limit.

Known unsupported enabled steps, including direct nested action calls, prevent
Play from changing the document. `actions.get` also emits a blocking batch plan
for these actions. Other runtime failures still keep earlier completed edits.
Paths, scripts, external resources and plug-ins inside descriptors are retained
as data and are not resolved or executed. The investigation and measured local
structural coverage are documented in [ATN analysis](atn-analysis.md).

## Compatibility limits

This is independently implemented behavior, not Photoshop code or a complete
Photoshop clone. Full Photoshop event coverage, `.atn` export, empty sets, button mode, timed playback,
conditional actions and editing complex command parameters in the Actions form
are not implemented. LAF supports the verified transparency/clear-outside/deselect
subset; other commands fail the entire import with an explicit error. CSP and
Photoshop pixel equivalence is not claimed.
