//! Input bindings of the editor.
//!
//! There is no settings screen yet; the editor reads the defaults. The types
//! are independent of GPUI so the configuration can be serialized later; the
//! `matches` methods are the only bridge to GPUI's input events.

use gpui_kit::{Keystroke, MouseButton};

#[allow(dead_code, reason = "alternatives for the future settings screen")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

impl PointerButton {
    pub fn matches(self, button: MouseButton) -> bool {
        matches!(
            (self, button),
            (PointerButton::Left, MouseButton::Left)
                | (PointerButton::Middle, MouseButton::Middle)
                | (PointerButton::Right, MouseButton::Right)
        )
    }
}

/// A key plus the exact set of modifiers that must be held with it.
///
/// `key` uses GPUI's key names, such as "space", "a" or "1".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyChord {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd on macOS, Super/Win elsewhere.
    pub platform: bool,
}

impl KeyChord {
    pub fn key(key: &str) -> Self {
        Self {
            key: key.into(),
            ctrl: false,
            alt: false,
            shift: false,
            platform: false,
        }
    }

    pub fn ctrl(key: &str) -> Self {
        Self {
            ctrl: true,
            ..Self::key(key)
        }
    }

    pub fn matches(&self, keystroke: &Keystroke) -> bool {
        let modifiers = &keystroke.modifiers;
        keystroke.key.eq_ignore_ascii_case(&self.key)
            && modifiers.control == self.ctrl
            && modifiers.alt == self.alt
            && modifiers.shift == self.shift
            && modifiers.platform == self.platform
    }

    /// Like [`KeyChord::matches`], but ignores the modifiers. Used on key up,
    /// where the modifiers may already have been released.
    pub fn matches_key(&self, keystroke: &Keystroke) -> bool {
        keystroke.key.eq_ignore_ascii_case(&self.key)
    }
}

/// What the mouse wheel does over the canvas.
#[allow(dead_code, reason = "alternatives for the future settings screen")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WheelAction {
    Zoom,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shortcuts {
    /// Holding this button over the canvas pans, whatever the active tool.
    pub pan_button: PointerButton,
    /// Holding this key over the canvas switches to the hand tool.
    pub hand_hold: KeyChord,
    pub wheel: WheelAction,
    /// Zoom factor applied per wheel line.
    pub wheel_zoom_step: f32,
    pub zoom_to_fit: KeyChord,
    pub zoom_to_100: KeyChord,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            pan_button: PointerButton::Middle,
            hand_hold: KeyChord::key("space"),
            wheel: WheelAction::Zoom,
            wheel_zoom_step: 1.1,
            zoom_to_fit: KeyChord::ctrl("1"),
            zoom_to_100: KeyChord::ctrl("0"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keystroke(source: &str) -> Keystroke {
        Keystroke::parse(source).unwrap()
    }

    #[test]
    fn chords_require_the_exact_modifiers() {
        let shortcuts = Shortcuts::default();
        assert!(shortcuts.hand_hold.matches(&keystroke("space")));
        assert!(!shortcuts.hand_hold.matches(&keystroke("ctrl-space")));
        assert!(shortcuts.zoom_to_fit.matches(&keystroke("ctrl-1")));
        assert!(!shortcuts.zoom_to_fit.matches(&keystroke("1")));
        assert!(!shortcuts.zoom_to_fit.matches(&keystroke("ctrl-shift-1")));
        assert!(shortcuts.zoom_to_100.matches(&keystroke("ctrl-0")));
    }

    #[test]
    fn key_up_ignores_modifiers() {
        let hand = Shortcuts::default().hand_hold;
        assert!(hand.matches_key(&keystroke("shift-space")));
        assert!(!hand.matches_key(&keystroke("a")));
    }

    #[test]
    fn pointer_buttons_map_to_gpui() {
        assert!(PointerButton::Middle.matches(MouseButton::Middle));
        assert!(!PointerButton::Middle.matches(MouseButton::Left));
    }
}
