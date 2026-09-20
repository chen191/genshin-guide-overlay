use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

pub const DEFAULT_PLAY_LABEL: &str = "F8";
pub const DEFAULT_SEEK_BACKWARD_LABEL: &str = "Ctrl+Alt+Left";
pub const DEFAULT_SEEK_FORWARD_LABEL: &str = "Ctrl+Alt+Right";
pub const DEFAULT_VISIBILITY_LABEL: &str = "Ctrl+Alt+O";
pub const DEFAULT_LOCK_LABEL: &str = "Ctrl+Alt+L";
pub const OPACITY_UP_LABEL: &str = "Ctrl+Alt+Up";
pub const OPACITY_DOWN_LABEL: &str = "Ctrl+Alt+Down";

fn parse_shortcut(label: &str) -> Option<Shortcut> {
    let mut tokens = Vec::new();
    for raw in label.split('+') {
        let token = raw.trim();
        if token.is_empty() {
            return None;
        }
        tokens.push(match token.to_ascii_lowercase().as_str() {
            "win" | "windows" => "Super".to_string(),
            _ => token.to_string(),
        });
    }
    tokens.join("+").parse().ok()
}

fn display_key(key: Code) -> String {
    let raw = key.to_string();
    if let Some(letter) = raw.strip_prefix("Key").filter(|value| value.len() == 1) {
        return letter.to_string();
    }
    if let Some(digit) = raw.strip_prefix("Digit").filter(|value| value.len() == 1) {
        return digit.to_string();
    }
    if let Some(direction) = raw.strip_prefix("Arrow") {
        return direction.to_string();
    }
    if let Some(numpad) = raw.strip_prefix("Numpad") {
        return format!("Num{numpad}");
    }
    match raw.as_str() {
        "Backquote" => "`".to_string(),
        "Backslash" => "\\".to_string(),
        "BracketLeft" => "[".to_string(),
        "BracketRight" => "]".to_string(),
        "Comma" => ",".to_string(),
        "Equal" => "=".to_string(),
        "Minus" => "-".to_string(),
        "Period" => ".".to_string(),
        "Quote" => "'".to_string(),
        "Semicolon" => ";".to_string(),
        "Slash" => "/".to_string(),
        _ => raw,
    }
}

fn display_shortcut(shortcut: Shortcut) -> String {
    let mut parts = Vec::new();
    if shortcut.mods.contains(Modifiers::CONTROL) {
        parts.push("Ctrl".to_string());
    }
    if shortcut.mods.contains(Modifiers::ALT) {
        parts.push("Alt".to_string());
    }
    if shortcut.mods.contains(Modifiers::SHIFT) {
        parts.push("Shift".to_string());
    }
    if shortcut.mods.contains(Modifiers::SUPER) {
        parts.push("Win".to_string());
    }
    parts.push(display_key(shortcut.key));
    parts.join("+")
}

pub fn normalize_shortcut_label(label: &str) -> Option<String> {
    parse_shortcut(label).map(display_shortcut)
}

pub fn shortcut_from_label(label: &str) -> Option<Shortcut> {
    parse_shortcut(label)
}

pub fn opacity_up_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::ArrowUp)
}

pub fn opacity_down_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::ArrowDown)
}

pub fn conflicts_with_fixed_shortcuts(shortcut: &Shortcut) -> bool {
    shortcut == &opacity_up_shortcut() || shortcut == &opacity_down_shortcut()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arbitrary_keyboard_shortcuts_are_normalized_and_parsed() {
        assert_eq!(
            normalize_shortcut_label("ctrl+shift+y").as_deref(),
            Some("Ctrl+Shift+Y")
        );
        assert_eq!(
            normalize_shortcut_label(" win + alt + num1 ").as_deref(),
            Some("Alt+Win+Num1")
        );
        assert_eq!(normalize_shortcut_label("F19").as_deref(), Some("F19"));
        assert_eq!(
            normalize_shortcut_label("ctrl+[").as_deref(),
            Some("Ctrl+[")
        );
        assert!(shortcut_from_label("Ctrl+Shift+Y").is_some());
        assert!(shortcut_from_label("Ctrl+Alt+Delete").is_some());
        assert!(shortcut_from_label("Ctrl+Alt+NotAKey").is_none());
        assert!(shortcut_from_label("Ctrl+Alt").is_none());
    }

    #[test]
    fn configurable_shortcuts_cannot_replace_fixed_opacity_keys() {
        assert!(conflicts_with_fixed_shortcuts(
            &shortcut_from_label(OPACITY_UP_LABEL).unwrap()
        ));
        assert!(!conflicts_with_fixed_shortcuts(
            &shortcut_from_label("Ctrl+Shift+Y").unwrap()
        ));
    }
}
