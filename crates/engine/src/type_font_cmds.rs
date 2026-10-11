//! Local font registration and persisted Type-picker favorites.
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

static REVISION: AtomicU64 = AtomicU64::new(0);
/// Largest accepted local font, including font collections.
pub const MAX_FONT_BYTES: usize = 64 * 1024 * 1024;

/// Invalidate family menus after registering a local font.
pub fn revision() -> u64 {
    REVISION.load(Ordering::Relaxed)
}

fn bad(cmd: &str, message: &str) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: message.into() }
}

fn import(_: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "type.importFont";
    let data = p.get("data").and_then(Value::as_str).ok_or_else(|| bad(CMD, "give base64 font `data` (TTF/OTF/TTC/OTC)"))?;
    if data.len() > MAX_FONT_BYTES.saturating_mul(4).div_ceil(3).saturating_add(4) {
        return Err(bad(CMD, "font exceeds 64 MiB"));
    }
    let bytes = prism_paint::tile::b64_decode(data).ok_or_else(|| bad(CMD, "invalid base64 font data"))?;
    let faces = prism_text::fonts::face_count(&bytes);
    if bytes.len() > MAX_FONT_BYTES || !(1..=256).contains(&faces) {
        return Err(bad(CMD, "invalid or oversized font file (at most 256 faces)"));
    }
    let mut engine = prism_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let families = engine.fonts.register_font_data(bytes);
    if families.is_empty() {
        return Err(bad(CMD, "font contains no usable families"));
    }
    REVISION.fetch_add(1, Ordering::Relaxed);
    Ok(json!({"families": families, "scope": "process"}))
}

fn favorite(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "type.favoriteFont";
    let family = p
        .get("family")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= 1024)
        .ok_or_else(|| bad(CMD, "give a nonempty `family` (at most 1024 bytes)"))?;
    let selected = p.get("favorite").and_then(Value::as_bool).ok_or_else(|| bad(CMD, "give `favorite`: true or false"))?;
    let mut names = s.prefs().type_.favorite_fonts.clone();
    names.retain(|name| name != family);
    if selected {
        if names.len() >= 512 {
            return Err(bad(CMD, "at most 512 favorite families"));
        }
        names.push(family.to_string());
    }
    s.edit_prefs(|p| p.type_.favorite_fonts = names);
    Ok(json!({"family": family, "favorite": selected}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "type.importFont",
            label: "Load Local Font",
            menu: &[],
            shortcut: None,
            params: r#"{"data":base64 TTF/OTF/TTC/OTC, at most 64 MiB} → families; available until Prism exits"#,
            enabled: |_| Ok(()),
            journal: false,
            run: import,
        },
        CommandSpec {
            id: "type.favoriteFont",
            label: "Favorite Font",
            menu: &[],
            shortcut: None,
            params: r#"{"family":str,"favorite":bool} → persisted Type-picker favorite"#,
            enabled: |_| Ok(()),
            journal: false,
            run: favorite,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favorites_survive_restart_and_are_idempotent() {
        let mut s = Session::new();
        for _ in 0..2 {
            s.execute("type.favoriteFont", json!({"family":"Inter", "favorite":true})).unwrap();
        }
        let mut restored = Session::new();
        restored.load_prefs_json(&s.prefs_to_json()).unwrap();
        assert_eq!(restored.prefs().type_.favorite_fonts, ["Inter"]);
        restored.execute("type.favoriteFont", json!({"family":"Inter", "favorite":false})).unwrap();
        assert!(restored.prefs().type_.favorite_fonts.is_empty());
    }

    #[test]
    fn font_commands_reject_bad_input_without_changing_preferences() {
        let mut s = Session::new();
        let before = s.prefs_to_json();
        for p in [json!({}), json!({"family":false,"favorite":true}), json!({"family":"Inter","favorite":"yes"})] {
            assert!(s.execute("type.favoriteFont", p).is_err());
        }
        for p in [json!({}), json!({"data":42}), json!({"data":"!!!"}), json!({"data":"YWJj"})] {
            assert!(s.execute("type.importFont", p).is_err());
        }
        assert!(s.execute("prefs.set", json!({"path":"type.favoriteFonts", "value":[42]})).is_err());
        assert_eq!(s.prefs_to_json(), before);
    }

    #[test]
    fn local_font_registration_is_visible_to_font_queries() {
        let mut s = Session::new();
        let previous = revision();
        let data = prism_paint::tile::b64_encode(prism_text::fonts::INTER_REGULAR);
        let r = s.execute("type.importFont", json!({"data": data})).unwrap();
        assert!(r["families"].as_array().unwrap().iter().any(|f| f == "Inter"));
        assert!(revision() > previous);
        assert!(!s.execute("type.fonts", json!({"family":"Inter"})).unwrap()["faces"].as_array().unwrap().is_empty());
    }
}
