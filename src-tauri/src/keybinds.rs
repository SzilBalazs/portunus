//! Chord grammar for user keybinds - the backend half of the shared contract
//! (frontend twin: `src/keybinds/chord.ts`, the runtime dispatch authority).
//!
//! Canonical form: `ctrl+alt+shift+<key>` - modifiers in that fixed order,
//! one key token, lowercase. The backend only uses this to clamp untrusted
//! extension-shipped shortcuts (result actions, manifest `default_shortcut`);
//! user-authored chords in config.toml pass through unvalidated.

/// Raw-input cap for untrusted sources; the longest canonical chord
/// ("ctrl+alt+shift+bracketleft") is 26 bytes.
const MAX_CHORD_BYTES: usize = 32;

/// Named key tokens that produce text when typed (with or without shift) -
/// bindable only behind ctrl/alt.
const PRINTABLE_NAMED: [&str; 12] = [
    "space", "comma", "period", "slash", "semicolon", "quote", "backquote", "minus", "equal",
    "bracketleft", "bracketright", "backslash",
];

/// Structural/navigation keys: parseable (so a reserved chord is reported as
/// reserved, not as a typo) but never bindable, with any modifiers.
const NAV_KEYS: [&str; 11] = [
    "escape", "backspace", "delete", "arrowup", "arrowdown", "arrowleft", "arrowright", "home",
    "end", "pageup", "pagedown",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Canonical key token: `a-z`, `0-9`, `f1`-`f12`, or a named key.
    pub key: String,
}

/// Parses one chord string, mapping aliases (`,` → comma, `control` → ctrl,
/// mixed case, any modifier order) onto the canonical grammar. None for
/// anything outside it - unknown tokens, duplicate/meta modifiers,
/// modifier-only chords, oversized or non-ASCII input.
pub fn parse_chord(s: &str) -> Option<Chord> {
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_CHORD_BYTES || !s.is_ascii() {
        return None;
    }
    let tokens: Vec<String> = s.split('+').map(|t| t.trim().to_ascii_lowercase()).collect();
    let (key_token, mods) = tokens.split_last()?;
    let (mut ctrl, mut alt, mut shift) = (false, false, false);
    for m in mods {
        let slot = match m.as_str() {
            "ctrl" | "control" => &mut ctrl,
            "alt" | "option" => &mut alt,
            "shift" => &mut shift,
            // meta/super/cmd are never part of the grammar.
            _ => return None,
        };
        if *slot {
            return None; // duplicate modifier
        }
        *slot = true;
    }
    let key = canonical_key(key_token)?;
    Some(Chord { ctrl, alt, shift, key })
}

fn canonical_key(token: &str) -> Option<String> {
    let mapped = match token {
        "," => "comma",
        "." => "period",
        "/" => "slash",
        ";" => "semicolon",
        "'" => "quote",
        "`" => "backquote",
        "-" => "minus",
        "=" => "equal",
        "[" => "bracketleft",
        "]" => "bracketright",
        "\\" => "backslash",
        "return" => "enter",
        "esc" => "escape",
        "up" => "arrowup",
        "down" => "arrowdown",
        "left" => "arrowleft",
        "right" => "arrowright",
        other => other,
    };
    let valid = (mapped.len() == 1
        && (mapped.as_bytes()[0].is_ascii_lowercase() || mapped.as_bytes()[0].is_ascii_digit()))
        || matches!(mapped, "enter" | "tab")
        || PRINTABLE_NAMED.contains(&mapped)
        || NAV_KEYS.contains(&mapped)
        || is_fkey(mapped);
    valid.then(|| mapped.to_string())
}

fn is_fkey(key: &str) -> bool {
    key.strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=12).contains(&n) && key.len() == if n < 10 { 2 } else { 3 })
}

/// Whether a chord is reserved for structural launcher behavior and can never
/// be bound (recorder, host clamp, and runtime dispatch all reject these).
pub fn is_reserved(c: &Chord) -> bool {
    // Escape/Backspace/arrows/nav keys stay structural with any modifiers.
    if NAV_KEYS.contains(&c.key.as_str()) {
        return true;
    }
    // Unmodified or shift-only printables would swallow typing.
    if !c.ctrl && !c.alt && is_printable(&c.key) {
        return true;
    }
    // Plain Enter (activate) and plain Tab (Contents toggle) - modified
    // variants (ctrl/alt/shift+enter, ctrl+tab) are bindable.
    if (c.key == "enter" || c.key == "tab") && !c.ctrl && !c.alt && !c.shift {
        return true;
    }
    // Alt+digit jumps to a result row.
    if c.alt && c.key.len() == 1 && c.key.as_bytes()[0].is_ascii_digit() {
        return true;
    }
    false
}

fn is_printable(key: &str) -> bool {
    if key.len() == 1 {
        let b = key.as_bytes()[0];
        return b.is_ascii_lowercase() || b.is_ascii_digit();
    }
    PRINTABLE_NAMED.contains(&key)
}

/// Parses, rejects reserved chords, and re-serializes to canonical form -
/// the one-call clamp for untrusted (extension-shipped) shortcut strings.
pub fn canonical(s: &str) -> Option<String> {
    let c = parse_chord(s)?;
    if is_reserved(&c) {
        return None;
    }
    let mut out = String::new();
    if c.ctrl {
        out.push_str("ctrl+");
    }
    if c.alt {
        out.push_str("alt+");
    }
    if c.shift {
        out.push_str("shift+");
    }
    out.push_str(&c.key);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_canonicalizes() {
        assert_eq!(canonical("ctrl+q").as_deref(), Some("ctrl+q"));
        assert_eq!(canonical("ctrl+shift+comma").as_deref(), Some("ctrl+shift+comma"));
        assert_eq!(canonical("alt+enter").as_deref(), Some("alt+enter"));
        assert_eq!(canonical("f5").as_deref(), Some("f5"));
        assert_eq!(canonical("ctrl+2").as_deref(), Some("ctrl+2"));
        assert_eq!(canonical("shift+enter").as_deref(), Some("shift+enter"));
        assert_eq!(canonical("ctrl+tab").as_deref(), Some("ctrl+tab"));
    }

    #[test]
    fn maps_aliases() {
        assert_eq!(canonical("ctrl+,").as_deref(), Some("ctrl+comma"));
        assert_eq!(canonical("ctrl+/").as_deref(), Some("ctrl+slash"));
        assert_eq!(canonical("control+option+x").as_deref(), Some("ctrl+alt+x"));
        assert_eq!(canonical("alt+return").as_deref(), Some("alt+enter"));
        assert_eq!(canonical("ctrl+[").as_deref(), Some("ctrl+bracketleft"));
    }

    #[test]
    fn normalizes_modifier_order_and_case() {
        assert_eq!(canonical("shift+ctrl+a").as_deref(), Some("ctrl+shift+a"));
        assert_eq!(canonical("Ctrl+Q").as_deref(), Some("ctrl+q"));
        assert_eq!(canonical("ALT+SHIFT+CTRL+F2").as_deref(), Some("ctrl+alt+shift+f2"));
        assert_eq!(canonical("  ctrl + p ").as_deref(), Some("ctrl+p"));
    }

    #[test]
    fn rejects_bare_and_shift_only_printables() {
        for s in ["q", "7", "shift+q", "shift+7", "comma", "shift+comma", "space", "shift+space"] {
            assert_eq!(canonical(s), None, "{s} must be reserved");
        }
    }

    #[test]
    fn rejects_nav_keys_with_any_modifiers() {
        for s in [
            "escape", "ctrl+escape", "backspace", "ctrl+backspace", "delete", "arrowup",
            "ctrl+arrowleft", "up", "alt+down", "home", "end", "pageup", "shift+pagedown",
        ] {
            assert_eq!(canonical(s), None, "{s} must be reserved");
        }
    }

    #[test]
    fn rejects_plain_enter_alt_digit_plain_tab() {
        assert_eq!(canonical("enter"), None);
        assert_eq!(canonical("tab"), None);
        assert_eq!(canonical("alt+1"), None);
        assert_eq!(canonical("alt+shift+9"), None);
        assert_eq!(canonical("ctrl+alt+2"), None);
    }

    #[test]
    fn rejects_meta_and_malformed() {
        for s in [
            "meta+k", "super+x", "cmd+c", "win+e", // no meta ever
            "", "ctrl", "ctrl+shift", "ctrl+", "+q", "ctrl++", // modifier-only / empty tokens
            "ctrl+ctrl+q", // duplicate modifier
            "ctrl+f13", "ctrl+f0", "ctrl+ä", "ctrl+qq", // outside the key grammar
        ] {
            assert_eq!(canonical(s), None, "{s:?} must be rejected");
        }
        // Oversized input is rejected before parsing.
        assert_eq!(canonical(&format!("ctrl+{}", "a".repeat(40))), None);
    }

    #[test]
    fn parse_exposes_reserved_chords() {
        // Reserved chords still parse - the recorder distinguishes "reserved"
        // from "not a chord".
        let c = parse_chord("enter").expect("plain enter parses");
        assert!(is_reserved(&c));
        let c = parse_chord("ctrl+q").expect("ctrl+q parses");
        assert!(!is_reserved(&c));
    }
}
