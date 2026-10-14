# UI localisation and live language switching

Prism uses Rust catalogs in the native egui shell. English source strings are lookup keys. UTF-8 TSV catalogs supply plain,
contextual, command-ID and plural translations; missing entries fall back to English.

## Supported languages

Prism speaks **English and Korean** only.

| Language | Preference code | Integer plural forms |
|---|---|---|
| English | `en` | one / other |
| 한국어 | `ko` | one form |

The Korean catalog covers the current menu labels, `tl!` literals, blend mode names,
and generated preference labels. Tests enforce that coverage. This does not include every
engine error or status message: those remain English, as do automation command IDs and
user-provided document, layer and preset names.

## Switch without restarting

Open **Edit > Preferences > Interface > Language** and choose a language. The Preferences
dialog previews its title, controls and buttons in the chosen language. **Cancel** discards
the preview, **Apply** commits it without closing the dialog, and **OK** commits and closes.
The rest of the interface follows the committed preference on its next draw. Existing
documents, undo history and tools remain available throughout a language change.

The existing preference store saves the selection for future launches. `auto` follows the
native system locale; an unsupported code follows the same fallback. Regional tags such as
`ko-KR` and `en-GB` resolve to Korean or English. Any other system language (Japanese, French, …)
is skipped, and with none of the two in the list the UI uses English. A preference saved by an
older version for a removed language (`ja`, `fr`, …) is treated the same way.

### First launch and system language

With no saved preference, `interface.language` defaults to `auto`, so the first rendered
window uses the first supported system UI language. An older preferences file without this
field also uses Auto. If detection fails or none of the preferred languages is registered,
the UI uses English (`en`). A saved manual language choice takes precedence on later launches;
startup never overwrites it or writes the detected language into the preference file.

The native-only, pinned `sys-locale` dependency provides safe Rust access to the platform APIs:

- **Windows:** `GetUserPreferredUILanguages`, in the user's preferred order. This uses UI
  languages rather than the Region setting for dates and numbers; they can differ.
- **macOS:** `CFLocaleCopyPreferredLanguages`, in preferred order, without starting `defaults`
  or depending on a particular `.plist` representation.
- **FreeBSD:** the platform's standard locale environment, including `LANGUAGE` lists.

`PRISM_LOCALE` is an optional single-tag override for reproducible launches; an unsupported
override falls back to English. Empty overrides use the system. Unix locale variables do not
override Windows/macOS UI-language preferences. Tags are read once per process (at most 64,
128 bytes each) and matched against the supported language registry. Changes to the
OS language list itself take effect at the next launch.

API implementation and licensing: [sys-locale](https://github.com/1Password/sys-locale).

Agents use the existing `prefs.set` command through CLI, MCP or the desktop control channel:

```json
{"method":"engine.execute","params":{"command":"prefs.set","params":{"path":"interface.language","value":"ko"}}}
```

Control requests are processed before synchronising the drawing language, so a request
updates the labels in the same rendered frame. Drawing language is local to the UI thread;
a scoped preview restores the previous language even if drawing unwinds. Menu and dialog
identity continues to use command IDs, independent of translated labels.

This is hot switching among the bundled catalogs. Catalog files are embedded with
`include_str!`; editing a TSV file requires a rebuild. External catalog watching and
file-resource hot reload are not implemented.

## Fonts

The native lazy CJK font loader puts Hangul first when the UI is Korean. In English it keeps the
system's CJK fallback order, so Japanese and Chinese document names and text still render.
Switching languages rebuilds the font definitions and resets the existing loader, so fonts
removed from the definitions can be registered again. Idle frames keep their font cache.

Font files are not added to this repository. Native rendering uses installed system fonts
and the optional craft-fonts build input; see [Fonts](development.md#fonts-craft-fonts).
Switching catalogs does not supply missing fonts.

## Maintain the Korean catalog

1. Edit `crates/ui-egui/src/i18n/ko.tsv`, following its header.
2. Translate the meaning of the English source keys. Preserve `{name}` placeholders, escaped
   characters and the trailing `…` on dialog commands. Use `@id` to disambiguate a command
   label and `@plural` for count-sensitive text. Keep product names such as Prism.
3. Every `tl!` literal and menu string needs a Korean entry: the catalog and shell tests fail
   otherwise. Then run:

```sh
cargo test -p prism-ui-egui --locked
cargo clippy -p prism-ui-egui --all-targets --locked -- -D warnings
cargo xtask i18n-coverage
cargo xtask layers
```

Render representative dialogs with the offscreen `snapshot` example, including a sequence
of language changes in one process. Inspect the PNGs for missing glyphs and truncated labels.
Translations are original work or reuse the open Prism contributions credited in
[ATTRIBUTION.md](../ATTRIBUTION.md); no proprietary localisation resources are used.
