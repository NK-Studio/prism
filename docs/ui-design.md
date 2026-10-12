# UI design system

## Themes

| Theme | Intent |
|---|---|
| Pro (Dark) | Photoshop-style Spectrum dark: flat charcoal panels (#323232), dark tab strips, Spectrum blue accent (#378ef0), pill buttons, checkboxes, compact 12 px type |
| **Pro Medium** (default) | Photoshop’s second brightness level: medium-gray chrome and dark canvas |
| Studio | Dark studio style: near-black, rounded cards, pill tabs, violet accent, toggles |
| Studio Light | Studio on light surfaces |

Saved Classic settings migrate to Pro Medium; Classic is no longer offered in the menus or preferences.

Switch themes with the sun icon, Window → Theme, or `ui.set {"theme":"studioLight"}` over the control channel.

## Rules

- **Colours and radii come from tokens.** Read them with `Tokens::get(ctx)`; never hard-code a colour in a widget.
- **Shared widgets live in `widgets.rs`:** `card` (panel group; Pro renders a Photoshop tab strip), `value_field`, `slider`/`slider_row`, `toggle`/`checkbox`, `primary_button`/`secondary_button`, `dropdown` (with a chevron icon), `hairline`/`vline`.
- **Icons:** Lucide SVGs in `assets/icons` (ISC licence), embedded via `icon_data.rs`. Regenerate that file when you add icons.
- **Fonts:** Inter (UI) and JetBrains Mono (numbers), both OFL. Named families `medium` and `semibold` are available via `theme::medium()` and `theme::semibold()`.
- **Photoshop layout grammar (Pro):**
  - Essentials dock order: Color | Swatches, then Properties | Adjustments, then Layers | Channels | Paths (Layers fills the remaining height).
  - Options-bar labels end with a colon ("Size:").
  - Document tabs read "name @ 12.5% (RGB/8)".
  - Toolbar tool groups carry a corner triangle.
- **Verify every visual change** with `ui.screenshot`, at several window sizes and in every theme.

## Adding icons

```sh
curl -sfL -o assets/icons/<name>.svg https://raw.githubusercontent.com/lucide-icons/lucide/main/icons/<name>.svg
# regenerate the embedded table
{ printf '%s\n' '//! Lucide icons (ISC licence), embedded and tinted at runtime.' '' 'pub static ICONS: &[(&str, &[u8])] = &['; \
  for f in assets/icons/*.svg; do n=$(basename $f .svg); echo "    (\"$n\", include_bytes!(\"../../../assets/icons/$n.svg\")),"; done; echo '];'; } \
  > crates/ui-egui/src/icon_data.rs
```

## Interaction models (match Photoshop CC)

| Feature | Module | Behaviour |
|---|---|---|
| Type tool | `type_tool.rs` | Click: point text with the placeholder "Lorem Ipsum" selected. Drag: paragraph box. Inline caret and selection drawn from the text engine layout. ⌥/⌘ word and line navigation, ↩ newline, ⌘↩ or Esc commits, a click outside commits. One history step per session (`coalesce`). A new layer is named after its text; an empty one is deleted. |
| Free Transform | `transform_tool.rs` | ⌘T. Corners scale proportionally (⇧ frees them); edges scale one axis; ⌥ scales about the reference point; ⌘-corner distorts; dragging outside rotates (⇧ snaps to 15°); dragging inside moves. Preview = document without the moving pixels + a textured 24×24 mesh. ↩ or a double-click commits via `edit.transform {rect, quad}`. |
| Quick layer pick | `quick_pick.rs` | macOS: ⌘⌥⌃-click with any tool selects the topmost visible layer with pixels under the pointer (`layer.pickAt`) without switching tools; the drag and release are swallowed. Windows have no third modifier distinct from the ⌃⌥ brush resize, so the gesture is a no-op there (use the Move tool's Auto-Select). A failed pick is a status-bar error. |
| App name and About | `panels.rs`, `about_ui.rs`, `dialogs.rs` | The main title bar centres Prism / 프리즘 on the window, including the macOS traffic-light inset. About is a single light rounded card with the current owner-supplied Prism PNG logo, NK Studio identity, a localized editorial introduction, an expanded copyright/tagline footer and actual package version (starting at 1.0.0); no tabs, contributor/model lists or source/license links. Legacy tab fields have no effect. Keyboard/control-channel closing and separate System Info remain available. The lower-right NK Studio wordmark uses installed Pretendard Bold (700) through the Rust type renderer, cached per font revision and DPI. No font files are copied or committed. |
| Move alignment | `panels.rs` | All themes expose left/centre/right and top/centre/bottom alignment icons. With one active layer and a pixel selection (including ⌘A / Ctrl+A), alignment uses the selection bounds; without a selection it uses the canvas. Several selected layers align to their combined bounds. The ellipsis menu exposes alignment and distribution commands. Each alignment is one undo step and preserves the selection. |
| Move to type editing | `canvas.rs`, `type_tool.rs` | Double-click visible text to switch to Type (Vertical Type for vertical text), select every character and type a replacement. This works with Auto-Select off and Show Transform Controls on. Hidden text, fully locked text, and text covered by opaque layers are ignored. A single click retains the Move tool. Cancel restores the original text. |
| Move cursor | `tool_cursor.rs`, `cursor_pack.rs`, `canvas.rs` | Idle: original arrow with a small four-way badge; moving a layer: ordinary arrow. Bitmap artwork is cached at the viewport DPI, with a canvas fallback. Precise cursors, guides, transform handles and temporary Hand retain priority. |
| Layer hover | `layer_row_ui.rs`, `theme.rs` | Unselected rows show `layer_hover` blue and `layer_hover_text` over both thumbnail and name. Selected rows keep their existing style; the labelled eye column retains its colour. The cursor is a pointing hand. Hover leaves document selection and history unchanged. |
| Document windows | `doc_windows.rs`, `canvas.rs` | Drag a tab 24 pt outside its strip or use Float in Window to detach. Drop its native title bar or tab on the main strip to return at the insertion indicator, or use Move to Main Tabs. A valid return target gets a blue workspace border; the source native window fades to 55% opacity, including OS caption controls. A detach in progress shows an empty frame using the current theme’s chrome colour. Esc cancels drags and restores position and opacity. Detached documents share data and Undo with the Session and retain their own camera. Main panels follow the active document window. Save/Discard/Cancel guards closing; Cancel or failed Save keeps the window. New Window for Document remains an additional view. Floating windows stay above the main editor while Prism is foreground and use ordinary desktop stacking when another app is active. During Save/Discard/Cancel, all floating canvases temporarily use ordinary stacking and the prompt host is restored/focused; this prevents a main-window quit prompt from being hidden behind a detached canvas. The normal floating level returns after the prompt is resolved. |
| Color panel | `color_panel_ui.rs` | The Color tab menu selects Hue Cube (default), Brightness Cube, Color Wheel or HSB Sliders in every theme. Brightness Cube varies hue/saturation in the field and brightness on the vertical strip. The wheel varies hue on the ring and saturation/brightness in the fixed triangle; drags remain on the component chosen on press. Both cubes stretch vertically to fill the resized panel viewport. Cubes and Color Wheel omit HEX/RGB input rows; the wheel fills the space below its HSB header. HSB Sliders retains its HEX/RGB readout. Both chips can be edited, hue/saturation survive black, and tool-colour edits use `tools.setColors`. The selected mode is saved with the workspace and exposed as `colorPanel.mode`. |
| Layers rows | `panels.rs` | Shape thumbnails keep their image preview and show a neutral grey vector-path badge at the lower right; rasterization removes it and Undo restores it. The badge takes no separate input. Delete is disabled when the target would remove every root layer or any fully/partially locked content (including inherited group locks and locked descendants); the footer greys its trash icon, ignores hover/click/drop and follows the same engine precondition as menus and keyboard commands. Adding a layer, unlocking, changing the selection or Undo immediately updates enablement. Double-click: on the name renames in place; on the Background makes it a normal layer; on an adjustment or fill thumbnail opens its Properties; on a Smart Object thumbnail opens its contents (Edit Contents); anywhere else on the row opens Layer Style. |
| Layer masks | `panels.rs` | Clicking the mask thumbnail targets the mask (corner-bracket frame; the tab reads "Layer, Layer Mask/8"). Brush, eraser (paints background colour), gradient and bucket then send `"target": "mask"`. Adjustment and fill layers target their mask automatically. |
| Levels / Curves | `tone.rs` | Histogram of the image *below* the adjustment. Curves: click to add a point, drag out to delete. Every change is a coalesced `layer.setAdjustment`, so one drag = one undo step and the canvas updates at full resolution on the GPU. |
| Rectangle corners | `shape_corners_ui.rs` | Rectangle, Path Selection and Direct Selection show four circular widgets inside the selected live rectangle. Drag adjusts all corner radii; Alt/Option at the press adjusts one corner. A live compositor preview preserves fill/stroke, release applies one `shape.edit`, Enter commits and Esc restores the original. Sharp corners can be rounded; radii stop at adjacent-side limits. Hover shows a diagonal resize cursor and a radius badge. |

While editing type, hold Ctrl (Windows) or Command (macOS) for an oriented transform
frame. Drag inside to move, a corner to scale (Shift keeps proportions; Alt/Option scales about
the reference point), a middle handle to skew, or just outside the frame to rotate (Shift snaps
to 15°). Drag the reference point to move the rotation centre. These Type gestures follow
[Adobe's type guide](https://helpx.adobe.com/photoshop/using/creating-type.html), independently
of the Free Transform preference: Ctrl/Cmd never distorts a Type corner.

Without Ctrl/Cmd, paragraph handles resize the container and reflow its text. Temporary
transforms keep its logical dimensions and all character/paragraph styles. A drag begun with
Ctrl/Cmd owns the pointer until button release, even if the modifier is released first. Its
preview does not change the document; release applies one coalesced `type.edit`, and typing
continues in the same layer. Escape during a drag cancels that preview; focus loss or changes
to its source also discard it. Holding the modifier, moving the reference point, or returning
the pointer to its start creates no history entry. Ctrl/Cmd+T while typing still toggles the
Character panel.

Where the font lacks a symbol (e.g. ∠ ↦ ▔), draw it with the painter or use a Lucide icon; never ship
missing-glyph boxes. Check every new panel with the offscreen snapshot tool (`docs/development.md`).

## Title bar

The app's top bar (`panels::title_bar`) contains the menus, localized Prism app name and workspace controls. Detached document windows use OS decorations with their document title, plus an internal tab for returning to the main strip, canvas, rulers, scrollbars and status bar.

- **Windows and FreeBSD:** the window has no OS decorations, so there is one bar, not two stacked. The top bar is the title bar (`titlebar.rs`): Minimize, Maximize/Restore and Close (red on hover, `caption_close` tokens) sit flush in the window's top-right corner, the free gap between the menus and the controls drags the window and a double-click there maximizes it, and invisible 5 pt edges (12 pt corners) resize it. Close runs File › Exit, so unsaved documents are asked about first.
- **macOS:** the traffic lights sit over the integrated title strip (`integrated_titlebar`).
- **Narrow windows** drop controls that are also in a menu before anything overlaps: the theme toggle, then search (Edit › Search), then the workspace switcher narrows (Window › Workspace).

## Menus

`menu_catalog.rs` holds Photoshop's menu tree (standard command names, order, separators, default shortcuts). Items whose id matches an engine or UI command are live; others render disabled until implemented. Give new commands the catalogue's id (for example `image.imageSize`) and they light up in the right place automatically.

Menus never run off the window: the menu bar's menus and submenus scroll with arrows (`menu_nav::level`), and long right-click menus (Layers, canvas tools, Channels, Paths, document tabs) wrap their rows in `widgets::menu_scroll`, so they move up to fit and scroll only when taller than the visible window.

## Automation for visual checks

Use `ui.click {x,y}`, `ui.move`, `ui.key` and `ui.type` (synthetic input in screen points) to open menus, popups and context menus, then `ui.screenshot`.

## Preferences

Preferences has **Apply**, **OK** and **Cancel**. Apply saves the edited sections and keeps the
dialog open; it is disabled when the values match the saved preferences. OK saves and closes.
Cancel discards only changes made since the last successful Apply. A failed Apply leaves the
draft open for correction. Settings marked for the next launch still require a restart.
On Windows and FreeBSD, System Title Bar displays “Applies at next launch.” beneath its checkbox,
including before editing and after Apply. Saving this preference does not restart the app or
change the current window's decorations; the next launch reads the saved choice.

## High DPI and 4K displays

Edit → Preferences → Interface → UI Scale applies immediately. Auto follows the operating
system's display scale, including fractional scales such as 125%, 150% and 175%. Only when the system reports no scaling (100%) on
a 4K or larger monitor does Auto use 200%, so text and controls remain readable. Detection uses the current monitor,
including portrait displays, and updates when the window moves between monitors. If monitor
size is unavailable, Auto follows system DPI. The fixed choices (75% to 300%) set an absolute UI
scale, allowing large 4K displays to use smaller controls when desired. Canvas zoom shortcuts
continue to control the document independently of UI scaling.

Interface → UI Font Size changes interface text independently of UI Scale and canvas zoom.
Tiny, Small (the default), Medium and Large use 10/12, 1, 14/12 and 16/12 of the theme's original
font sizes, preserving the relative sizes of headings, captions and numeric fields. Apply or OK
updates text immediately; the choice persists across launches and theme changes. CJK fallback
fonts use the same size, including fonts loaded after the preference changes.

## Localisation

Strings in code stay English and are the default lookup keys. `crates/ui-egui/src/i18n` maps them
to display text at render time from the Korean catalog (`i18n/ko.tsv`; its header documents the
format). Command ids, menu paths used for logic, the control channel, the CLI and MCP always use
the English ids and labels.

Prism speaks English (`en`, the source language) and Korean (`ko`, one plural form). The Korean
catalog must cover every menu string and `tl!` literal (`complete_menus`; the tests enforce it).
Other system languages show English.

- `tr(lang, s)` plain strings; `tr_ctx` when one English word needs different translations;
  `tr_id(lang, command_id, label)` for menu items (keyed by command id, English label as the
  fallback); `trn(lang, n, one, other)` for plurals; `fmt` fills `{name}` placeholders, which
  translators may reorder.
- The language is Preferences › Interface › Language (`interface.language`: `auto` or a language
  code; `auto` follows the system locale, an unknown code falls back to `auto`).
- Language changes apply without a restart; Preferences previews the chosen language until
  Apply or OK commits it. Cancel restores the committed language. Regional preference tags
  resolve through the same locale matcher. See [`localization.md`](localization.md).
- To add a language: add `<code>.tsv` and one row in `i18n::LANGUAGES` (code, native name, catalog,
  plural rule). The dropdown, locale matching and the catalog tests (well-formed, no duplicates,
  placeholders and ellipses agree, command ids exist) pick it up. Set `complete_menus` once every
  menu string is translated; a test then enforces it. A language with more plural forms lists them
  all in `@plural` entries (Russian: `one|few|many`); the row's plural rule picks the form.
- Translations are clean-room: written from the meaning of the English text in ordinary vocabulary,
  never from another product's localisation resources.

Localised so far: menus, the command palette, dialogs and panels (literals wrapped in `tl!("…")`;
widgets such as `checkbox`, `slider_row`, `dropdown` and the buttons translate their labels
themselves). A test fails when a `tl!` literal, a menu string, a blend mode name, brush section name or a generated
preference label has no entry in a language marked `complete_menus`. Not translated: status-bar
messages and errors (they stay English, also for agents), names that are user data (layers, styles,
documents), strings assembled with `format!` that were not converted to `fmt`/`trn`. Not done yet:
right-to-left layout,
locale-aware number and date formats. Native builds read `LANG`/`LC_*`, the macOS preferred languages and the Windows user locale.

Camera Raw has explicit coverage for all ten available languages, including Korean and
Simplified Chinese. Its contextual `cameraRaw` entries distinguish tonal regions,
colour-band names and font-weight Light. The title uses a `{layer}` placeholder; user layer
names and the universal RGB/Lab channel symbols stay unchanged. Collapse IDs use English
source keys, so switching languages preserves open sections. Mixer tabs wrap within the
panel width. Catalog coverage and actual shell language-switch tests enforce these rules.

Native CJK font fallback follows the selected UI script and resets its cache when switching
languages.

Optional personal cursor artwork is loaded as described in [Development](development.md#optional-local-cursor-pack). Standard tool images can be replaced while dynamic brush outlines, precise settings and temporary tool priority remain intact. Artwork geometry includes transparent padding and its original hotspot; Retina pixels must never be interpreted as points.
