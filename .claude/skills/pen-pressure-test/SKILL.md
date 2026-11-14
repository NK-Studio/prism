---
name: pen-pressure-test
description: Test pen pressure, tilt and stroke timing in the real Prism app without a physical tablet, by injecting synthetic Windows Ink pen strokes (prism-tablet example `pen_inject`) and reading them back with `ui.penTrace`. Use whenever a change touches brushes, pressure curves, stabilizers, SUT/ABR brush import, pen input or stroke rendering and needs a pressure check ("필압 테스트", "펜 테스트", "pressure test", "check the stroke with pressure"), or to feed the same pressure stroke into CLIP STUDIO PAINT or Photoshop for comparison. Windows only.
---

# Pen pressure test (synthetic pen)

Unit tests and `ui.pointer` replays skip the native pen path (Windows Ink packets, winit's
emulated mouse/touch, per-frame delivery), so their results can differ from a real pen. The
`pen_inject` example injects OS-level pen packets (`InjectSyntheticPointerInput`, PT_PEN) with
pressure, tilt, rotation and timing. They take exactly the path a physical pen takes. Use it for
every pressure check instead of assuming the user will test by hand. Background and the measured
verification: `docs/pen-input-validation.md` › Synthetic pen injection.

Reply to the maintainer in Korean; commands, paths and ids stay as they are.

## Ground rules

- Injection **moves the real cursor and focuses the target window**. Tell the user one line
  before you run it ("커서가 잠깐 움직입니다"), and don't run it while they're typing elsewhere.
- Use your own target dir: `CARGO_TARGET_DIR=target/agent-pen`.
- Keep the control token, traces and screenshots in the scratchpad, never in the repo.
- Always check results from data (`ui.penTrace`, `document.inspect` history), then look at a
  screenshot. Never report "pressure works" from the injector's exit code alone.

## 1. Start Prism with the control channel

Build the app with your change (`cargo build --profile fast -p prism`, or `--release`). Then run it
in the background:

```sh
<target>/prism.exe --control 7979 --control-token-file "$S/control.token" \
  --automation-read-root "$S" --automation-write-root "$S"
```

`$S` is a scratchpad folder. Talk to it with a small JSON-lines client (auth first). For example,
save this as `$S/ctl.py` and run it with `python -I "$S/ctl.py" '[[method, params], …]' [out.json]`:

```python
import json, socket, sys, pathlib
sys.stdout.reconfigure(encoding="utf-8")  # Korean/Japanese sub tool names break the cp949 console
here = pathlib.Path(__file__).parent
token = (here / "control.token").read_text().strip()
s = socket.create_connection(("127.0.0.1", 7979), timeout=60)
f = s.makefile("rw", encoding="utf-8")
f.write(json.dumps({"id": "auth", "method": "auth", "params": {"token": token}}) + "\n"); f.flush(); f.readline()
calls = json.loads(sys.argv[1])
for i, (method, params) in enumerate(calls):
    f.write(json.dumps({"id": i, "method": method, "params": params}) + "\n"); f.flush()
    line = f.readline()
    if len(sys.argv) > 2 and i == len(calls) - 1:
        (here / sys.argv[2]).write_text(line, encoding="utf-8"); print(method, "->", sys.argv[2])
    else:
        print(method, "->", line[:400])
```

## 2. Prepare the document and brush

```json
[["engine.execute",{"command":"file.new","params":{"width":1600,"height":900,"mode":"rgb","depth":8,"background":"white"}}],
 ["ui.pointer",{"tool":"brush","events":[{"kind":"cancel"}]}],
 ["ui.penTrace",{"enabled":true,"clear":true}]]
```

Set the brush or preset under test (brush commands, `ui.set`, or import the SUT/ABR) **before**
injecting. Record the settings you used.

## 3. Inject

```sh
cargo run -p prism-tablet --example pen_inject -- --window Prism --list   # confirm target + client size
cargo run -p prism-tablet --example pen_inject -- --window Prism          # default 0→1→0 pressure ramp
cargo run -p prism-tablet --example pen_inject -- --window Prism --ramp X,Y,LEN --hover N
cargo run -p prism-tablet --example pen_inject -- --window Prism --file trace.json --origin X,Y --scale S
```

- Coordinates are **client pixels** of the target window (physical pixels; the tool is DPI aware).
  Read the canvas position from a screenshot to aim inside the document.
- `--file` takes `ui.penTrace` output or `[x,y,pressure]` / `{x,y,pressure,time?,tiltX?,tiltY?,rotation?}`
  samples, mapped as `origin + point * scale`. For a trace in document coordinates, use the
  canvas origin and zoom. A real-pen trace the user once recorded can be replayed this way.
- `--hover N` (default 8 × 16 ms) shortens the lead-in to model quick successive strokes.
- `--dry-run` prints the frames without injecting.
- The **first stroke right after the window is brought forward** has lost its `down` before and
  committed nothing. Inject one warm-up stroke (or click the window) first, and report it if a
  measured stroke misses its `down`.

## 4. Verify

1. `[["ui.penTrace",{"enabled":false}]]` → save the result. Expect one `down`, the moves and one
   `up` per stroke, `source: "windowsInk"`, `pressureValid: true`, and pressures that follow the
   injected profile (ramp: 0 → ~1 → 0).
2. `[["engine.execute",{"command":"document.inspect","params":{}}]]` → one history step per stroke
   (e.g. "Brush Tool").
3. `[["ui.screenshot",{"path":"shot.png"}]]` → crop or downscale and **look at it**: taper, width
   change, opacity, start/end shape.
4. Report numbers (event counts, pressure range, history) and what the image shows. Say plainly
   what was not checked.

## Comparing with CLIP STUDIO PAINT / Photoshop

The same stroke can drive another app as an observation oracle (clean-room rules in AGENTS.md
apply: compare outputs only). Drive the other app's UI with **cua-driver** (load the `cua-driver`
skill), not computer-use. Ask the user before changing its preferences or tools, and leave them as
found. Keep oracle renders in the scratchpad or `target/` (never committed).

CLIP STUDIO PAINT, as done on 2026-11-14 (`docs/pen-input-validation.md` › G-pen comparison):

1. **Tablet service.** This machine uses **Wintab**, which ignores synthetic pens: the stroke draws
   at constant width as if from a mouse. With the user's OK, open Preferences (`Ctrl+K`) › Tablet ›
   **Tablet PC** › OK. It applies without a restart. Switch back to Wintab afterwards and confirm
   `Tablet` in a *copy* of `%APPDATA%/CELSYSUserData/CELSYS/CLIPStudioPaintVer1_5_0/Preference/Config.sqlite`
   matches the copy taken before (`ContextKind 1000`).
2. **Canvas.** `Ctrl+N` (foreground). Its number fields accept only `type_text` with `delay_ms`
   after a double-click; separate `press_key` digits leak to the main window as tool shortcuts.
3. **Brush.** Pick the same sub tool in both apps. Prism: `ui.cspTool {"tool":"pen","subTool":"G펜"}`
   loads CSP's own sub tool; then match the size the user's CSP shows (`ui.set {brushSize}`).
4. **Same physical scale.** Prism `ui.set {"zoom": cspZoom / pixelsPerPoint}` (`ui.inspect` › `window`).
   Find each canvas origin from a native-resolution screenshot (the white rectangle), then inject the
   same doc-coordinate file with `--origin <client x,y> --scale <zoom>` into each app. Inject one
   warm-up stroke first.
5. **Measure.** Prism: `app.save` a PNG (exact doc pixels). CSP: set View 100 % (`Ctrl+Alt+0`) and
   take a native screenshot; CSP's clipboard holds only a private format, so `Ctrl+C` gives no image,
   and its 89.9 % display is resampled. Compare coverage-weighted width per column (sum of
   `(255-G)/(255-G_ink)` down the column), extent and peak, and calibrate with a constant-width
   stroke.
6. **Isolate dab vs line.** A gap in line width can come from the dab itself or from how
   overlapping dabs build up. Measure one dab too: on Wintab a one-sample `--file` is a full-size
   mouse click, so no preference change is needed.
7. **cua-driver pixel coordinates** are relative to that window's *most recent* screenshot. After
   a native-resolution capture (`max_image_dimension: 0`), take a scaled read again before
   clicking by pixels from a scaled image. A mismatched click selected a different tool once.
8. **Restore.** Close the test canvas with Do Not Save, reselect the user's sub tool, set the tablet
   service back.

## Cleanup

Stop the trace (`enabled:false`). Close test documents without saving only if you opened them, or
leave the app open and say so. Delete `target/agent-pen` when finished if disk is tight.
