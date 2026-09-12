use iced_core::keyboard::{Modifiers, key};
use lapiz_runtime::service::Service;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

#[derive(Debug, Default, Clone)]
pub struct KeyboardState {
    pressed: SmallVec<[key::Code; 8]>,
    modifiers: Modifiers,
}

impl KeyboardState {
    pub fn press(&mut self, key: key::Code) {
        if !self.is_pressed(key) {
            self.pressed.push(key);
        }
    }

    pub fn release(&mut self, key: key::Code) {
        self.pressed.retain(|&mut c| c != key);
    }

    pub fn clear(&mut self) {
        self.pressed.clear();
        self.modifiers = Modifiers::empty();
    }

    pub fn set_modifiers(&mut self, modifiers: Modifiers) {
        self.modifiers = modifiers;
    }

    pub fn is_pressed(&self, key: key::Code) -> bool {
        self.pressed.contains(&key)
    }

    pub fn has_pressed(&self) -> bool {
        !self.pressed.is_empty()
    }

    pub fn all_pressed(&self) -> impl Iterator<Item = key::Code> + '_ {
        self.pressed.iter().copied()
    }

    pub fn last_key(&self) -> Option<key::Code> {
        self.pressed.last().copied()
    }

    pub fn modifiers(&self) -> Modifiers {
        self.modifiers
    }

    pub fn get_sequence(&self) -> KeySequence {
        KeySequence {
            key: self.last_key(),
            modifiers: self.modifiers,
        }
    }
}

impl Service for KeyboardState {}

#[derive(Debug, thiserror::Error)]
pub enum KeyParseError {
    #[error("Multiple non-modifier keys found: {0:?}")]
    MultipleNonModifierKeys(Vec<key::Code>),
    #[error("Invalid keystroke: {0:?}")]
    InvalidKeystroke(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeySequence {
    pub key: Option<key::Code>,
    pub modifiers: Modifiers,
}

impl Serialize for KeySequence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let string = self.unparse();
        string.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for KeySequence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let string = String::deserialize(deserializer)?;
        Self::parse(&string).map_err(serde::de::Error::custom)
    }
}

impl KeySequence {
    pub fn into_codes(self) -> Vec<key::Code> {
        let mut codes = Vec::new();
        if self.modifiers.contains(Modifiers::CTRL) {
            codes.push(key::Code::ControlLeft);
        }
        if self.modifiers.contains(Modifiers::ALT) {
            codes.push(key::Code::AltLeft);
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            codes.push(key::Code::ShiftLeft);
        }
        if self.modifiers.contains(Modifiers::LOGO) {
            codes.push(key::Code::SuperLeft);
        }
        if let Some(key) = self.key {
            codes.push(key);
        }
        codes
    }

    pub fn from_codes(codes: impl Iterator<Item = key::Code>) -> Result<Self, KeyParseError> {
        let mut modifiers = Modifiers::empty();
        let mut keys = SmallVec::<[_; 4]>::new();

        for code in codes {
            match code {
                key::Code::ControlLeft | key::Code::ControlRight => {
                    modifiers |= Modifiers::CTRL;
                }
                key::Code::AltLeft | key::Code::AltRight => {
                    modifiers |= Modifiers::ALT;
                }
                key::Code::ShiftLeft | key::Code::ShiftRight => {
                    modifiers |= Modifiers::SHIFT;
                }
                key::Code::SuperLeft | key::Code::SuperRight => {
                    modifiers |= Modifiers::LOGO;
                }
                _ => {
                    keys.push(code);
                }
            }
        }

        if keys.len() > 1 {
            Err(KeyParseError::MultipleNonModifierKeys(keys.to_vec()))
        } else {
            Ok(KeySequence {
                key: keys.first().copied(),
                modifiers,
            })
        }
    }

    pub fn parse(s: &str) -> Result<Self, KeyParseError> {
        let mut modifiers = Modifiers::empty();
        let mut key = None;

        let components = s.split('-').peekable();
        for component in components {
            if component.eq_ignore_ascii_case("ctrl") {
                modifiers |= Modifiers::CTRL;
            } else if component.eq_ignore_ascii_case("alt") {
                modifiers |= Modifiers::ALT;
            } else if component.eq_ignore_ascii_case("shift") {
                modifiers |= Modifiers::SHIFT;
            } else if component.eq_ignore_ascii_case("cmd")
                || component.eq_ignore_ascii_case("super")
                || component.eq_ignore_ascii_case("win")
            {
                modifiers |= Modifiers::LOGO;
            } else {
                if key.is_some() {
                    return Err(KeyParseError::InvalidKeystroke(s.to_string()));
                }
                key = Some(
                    parse_key_name(&component.to_ascii_lowercase())
                        .ok_or_else(|| KeyParseError::InvalidKeystroke(s.to_string()))?,
                );
            }
        }

        Ok(KeySequence { key, modifiers })
    }

    pub fn unparse(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(Modifiers::CTRL) {
            parts.push("ctrl");
        }
        if self.modifiers.contains(Modifiers::ALT) {
            parts.push("alt");
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            parts.push("shift");
        }
        if self.modifiers.contains(Modifiers::LOGO) {
            parts.push("super");
        }
        if let Some(key) = self.key {
            parts.push(key_name(key));
        }
        parts.join("-")
    }
}

impl std::fmt::Display for KeySequence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();
        if self.modifiers.contains(Modifiers::CTRL) {
            parts.push("Ctrl");
        }
        if self.modifiers.contains(Modifiers::ALT) {
            parts.push("Alt");
        }
        if self.modifiers.contains(Modifiers::SHIFT) {
            parts.push("Shift");
        }
        if self.modifiers.contains(Modifiers::LOGO) {
            parts.push("Super");
        }
        let mut display = parts.join("+");
        if let Some(key) = self.key {
            let name = key_name(key);
            let mut chars = name.chars();
            if let Some(first) = chars.next() {
                if !parts.is_empty() {
                    display.push('+');
                }
                display.extend(first.to_uppercase());
                display.push_str(chars.as_str());
            }
        }
        write!(f, "{}", display)
    }
}

fn parse_key_name(name: &str) -> Option<key::Code> {
    Some(match name {
        "a" => key::Code::KeyA,
        "b" => key::Code::KeyB,
        "c" => key::Code::KeyC,
        "d" => key::Code::KeyD,
        "e" => key::Code::KeyE,
        "f" => key::Code::KeyF,
        "g" => key::Code::KeyG,
        "h" => key::Code::KeyH,
        "i" => key::Code::KeyI,
        "j" => key::Code::KeyJ,
        "k" => key::Code::KeyK,
        "l" => key::Code::KeyL,
        "m" => key::Code::KeyM,
        "n" => key::Code::KeyN,
        "o" => key::Code::KeyO,
        "p" => key::Code::KeyP,
        "q" => key::Code::KeyQ,
        "r" => key::Code::KeyR,
        "s" => key::Code::KeyS,
        "t" => key::Code::KeyT,
        "u" => key::Code::KeyU,
        "v" => key::Code::KeyV,
        "w" => key::Code::KeyW,
        "x" => key::Code::KeyX,
        "y" => key::Code::KeyY,
        "z" => key::Code::KeyZ,
        "0" => key::Code::Digit0,
        "1" => key::Code::Digit1,
        "2" => key::Code::Digit2,
        "3" => key::Code::Digit3,
        "4" => key::Code::Digit4,
        "5" => key::Code::Digit5,
        "6" => key::Code::Digit6,
        "7" => key::Code::Digit7,
        "8" => key::Code::Digit8,
        "9" => key::Code::Digit9,
        "," => key::Code::Comma,
        "." => key::Code::Period,
        "/" => key::Code::Slash,
        ";" => key::Code::Semicolon,
        "'" => key::Code::Quote,
        "[" => key::Code::BracketLeft,
        "]" => key::Code::BracketRight,
        "\\" => key::Code::Backslash,
        "-" => key::Code::Minus,
        "=" => key::Code::Equal,
        "`" => key::Code::Backquote,
        "space" => key::Code::Space,
        "enter" => key::Code::Enter,
        "tab" => key::Code::Tab,
        "backspace" => key::Code::Backspace,
        "delete" => key::Code::Delete,
        "escape" => key::Code::Escape,
        "home" => key::Code::Home,
        "end" => key::Code::End,
        "pageup" => key::Code::PageUp,
        "pagedown" => key::Code::PageDown,
        "insert" => key::Code::Insert,
        "capslock" => key::Code::CapsLock,
        "printscreen" => key::Code::PrintScreen,
        "scrolllock" => key::Code::ScrollLock,
        "pause" => key::Code::Pause,
        "numlock" => key::Code::NumLock,
        "up" => key::Code::ArrowUp,
        "down" => key::Code::ArrowDown,
        "left" => key::Code::ArrowLeft,
        "right" => key::Code::ArrowRight,
        "shift" => key::Code::ShiftLeft,
        "control" => key::Code::ControlLeft,
        "alt" => key::Code::AltLeft,
        "platform" => key::Code::SuperLeft,
        "menu" => key::Code::ContextMenu,
        "f1" => key::Code::F1,
        "f2" => key::Code::F2,
        "f3" => key::Code::F3,
        "f4" => key::Code::F4,
        "f5" => key::Code::F5,
        "f6" => key::Code::F6,
        "f7" => key::Code::F7,
        "f8" => key::Code::F8,
        "f9" => key::Code::F9,
        "f10" => key::Code::F10,
        "f11" => key::Code::F11,
        "f12" => key::Code::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{KeySequence, KeyboardState};
    use iced_core::keyboard::{Modifiers, key};

    #[test]
    fn clearing_removes_pressed_keys_and_modifiers() {
        let mut state = KeyboardState::default();
        state.press(key::Code::KeyO);
        state.set_modifiers(Modifiers::ALT);

        state.clear();

        assert_eq!(
            state.get_sequence(),
            KeySequence {
                key: None,
                modifiers: Modifiers::empty(),
            }
        );
    }
}

fn key_name(code: key::Code) -> &'static str {
    match code {
        key::Code::KeyA => "a",
        key::Code::KeyB => "b",
        key::Code::KeyC => "c",
        key::Code::KeyD => "d",
        key::Code::KeyE => "e",
        key::Code::KeyF => "f",
        key::Code::KeyG => "g",
        key::Code::KeyH => "h",
        key::Code::KeyI => "i",
        key::Code::KeyJ => "j",
        key::Code::KeyK => "k",
        key::Code::KeyL => "l",
        key::Code::KeyM => "m",
        key::Code::KeyN => "n",
        key::Code::KeyO => "o",
        key::Code::KeyP => "p",
        key::Code::KeyQ => "q",
        key::Code::KeyR => "r",
        key::Code::KeyS => "s",
        key::Code::KeyT => "t",
        key::Code::KeyU => "u",
        key::Code::KeyV => "v",
        key::Code::KeyW => "w",
        key::Code::KeyX => "x",
        key::Code::KeyY => "y",
        key::Code::KeyZ => "z",
        key::Code::Digit0 => "0",
        key::Code::Digit1 => "1",
        key::Code::Digit2 => "2",
        key::Code::Digit3 => "3",
        key::Code::Digit4 => "4",
        key::Code::Digit5 => "5",
        key::Code::Digit6 => "6",
        key::Code::Digit7 => "7",
        key::Code::Digit8 => "8",
        key::Code::Digit9 => "9",
        key::Code::Comma => ",",
        key::Code::Period => ".",
        key::Code::Slash => "/",
        key::Code::Semicolon => ";",
        key::Code::Quote => "'",
        key::Code::BracketLeft => "[",
        key::Code::BracketRight => "]",
        key::Code::Backslash => "\\",
        key::Code::Minus => "-",
        key::Code::Equal => "=",
        key::Code::Backquote => "`",
        key::Code::Space => "space",
        key::Code::Enter => "enter",
        key::Code::Tab => "tab",
        key::Code::Backspace => "backspace",
        key::Code::Delete => "delete",
        key::Code::Escape => "escape",
        key::Code::Home => "home",
        key::Code::End => "end",
        key::Code::PageUp => "pageup",
        key::Code::PageDown => "pagedown",
        key::Code::Insert => "insert",
        key::Code::CapsLock => "capslock",
        key::Code::PrintScreen => "printscreen",
        key::Code::ScrollLock => "scrolllock",
        key::Code::Pause => "pause",
        key::Code::NumLock => "numlock",
        key::Code::ArrowUp => "up",
        key::Code::ArrowDown => "down",
        key::Code::ArrowLeft => "left",
        key::Code::ArrowRight => "right",
        key::Code::ShiftLeft => "shift",
        key::Code::ControlLeft => "control",
        key::Code::AltLeft => "alt",
        key::Code::SuperLeft => "platform",
        key::Code::ContextMenu => "menu",
        key::Code::F1 => "f1",
        key::Code::F2 => "f2",
        key::Code::F3 => "f3",
        key::Code::F4 => "f4",
        key::Code::F5 => "f5",
        key::Code::F6 => "f6",
        key::Code::F7 => "f7",
        key::Code::F8 => "f8",
        key::Code::F9 => "f9",
        key::Code::F10 => "f10",
        key::Code::F11 => "f11",
        key::Code::F12 => "f12",
        _ => "",
    }
}
