# eframe 0.36.2 native viewport patch

This is the unchanged crates.io eframe 0.36.2 source except for the immediate
wgpu viewport output path in `src/native/wgpu_integration.rs`.
That path now forwards screenshot requests, delivers requested clipboard events,
and installs custom cursors through the existing event-loop-aware API, matching
the regular redraw path. It also exposes a safe weak window lookup in `src/lib.rs`
for parenting native file dialogs to their originating document viewport.
On macOS, mouse motion that the focused window does not take (the pointer is outside
it) repaints the root viewport while more than one viewport exists, so an unfocused
window under the pointer can follow it (the OS sends mouse-moved events to the key
window only).
No new unsafe code is introduced.

Source: https://crates.io/crates/eframe/0.36.2
Upstream: https://github.com/emilk/egui
License: MIT OR Apache-2.0 (LICENSE-MIT and LICENSE-APACHE alongside).
Remove this patch when the corresponding upstream fix is available.
