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
        (here / sys.argv[2]).write_text(line); print(method, "->", sys.argv[2])
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
apply: compare outputs only). For CLIP STUDIO, File › Preferences › Tablet must be **TabletPC**
(Windows Ink); Wintab does not see synthetic pens. Match canvas size, zoom, brush size and
settings. Inject with `--window "CLIP STUDIO"` and the same `--ramp` / `--file` mapped to that
canvas. Ask the user before changing CLIP STUDIO or Photoshop preferences or tools, and leave them
as found. Keep oracle renders under `target/` (never committed).

## Cleanup

Stop the trace (`enabled:false`). Close test documents without saving only if you opened them, or
leave the app open and say so. Delete `target/agent-pen` when finished if disk is tight.
