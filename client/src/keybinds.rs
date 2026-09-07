//! Rebindable input. Every gameplay system reads keys through [`KeyBindings`]
//! instead of hard-coding `KeyCode`s, so the settings menu can remap them.

use bevy::input::ButtonInput;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// A single bound input — a keyboard key or a mouse button.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Binding {
    Key(KeyCode),
    Mouse(MouseButton),
}

impl Binding {
    pub fn pressed(&self, k: &ButtonInput<KeyCode>, m: &ButtonInput<MouseButton>) -> bool {
        match self {
            Binding::Key(c) => k.pressed(*c),
            Binding::Mouse(b) => m.pressed(*b),
        }
    }

    pub fn just_pressed(&self, k: &ButtonInput<KeyCode>, m: &ButtonInput<MouseButton>) -> bool {
        match self {
            Binding::Key(c) => k.just_pressed(*c),
            Binding::Mouse(b) => m.just_pressed(*b),
        }
    }

    /// Short human label for the settings UI.
    pub fn label(&self) -> String {
        match self {
            Binding::Key(c) => key_label(*c),
            Binding::Mouse(MouseButton::Left) => "Mouse Left".into(),
            Binding::Mouse(MouseButton::Right) => "Mouse Right".into(),
            Binding::Mouse(MouseButton::Middle) => "Mouse Middle".into(),
            Binding::Mouse(MouseButton::Back) => "Mouse Back".into(),
            Binding::Mouse(MouseButton::Forward) => "Mouse Fwd".into(),
            Binding::Mouse(MouseButton::Other(n)) => format!("Mouse {n}"),
        }
    }
}

fn key_label(c: KeyCode) -> String {
    use KeyCode::*;
    let s = match c {
        KeyA => "A", KeyB => "B", KeyC => "C", KeyD => "D", KeyE => "E", KeyF => "F", KeyG => "G",
        KeyH => "H", KeyI => "I", KeyJ => "J", KeyK => "K", KeyL => "L", KeyM => "M", KeyN => "N",
        KeyO => "O", KeyP => "P", KeyQ => "Q", KeyR => "R", KeyS => "S", KeyT => "T", KeyU => "U",
        KeyV => "V", KeyW => "W", KeyX => "X", KeyY => "Y", KeyZ => "Z",
        Digit0 => "0", Digit1 => "1", Digit2 => "2", Digit3 => "3", Digit4 => "4", Digit5 => "5",
        Digit6 => "6", Digit7 => "7", Digit8 => "8", Digit9 => "9",
        Space => "Space", Enter => "Enter", Tab => "Tab", Backspace => "Backspace",
        ShiftLeft => "L Shift", ShiftRight => "R Shift", ControlLeft => "L Ctrl",
        ControlRight => "R Ctrl", AltLeft => "L Alt", AltRight => "R Alt", SuperLeft => "L Super",
        SuperRight => "R Super",
        ArrowUp => "Up", ArrowDown => "Down", ArrowLeft => "Left", ArrowRight => "Right",
        F1 => "F1", F2 => "F2", F3 => "F3", F4 => "F4", F5 => "F5", F6 => "F6", F7 => "F7",
        F8 => "F8", F9 => "F9", F10 => "F10", F11 => "F11", F12 => "F12",
        other => return format!("{other:?}"),
    };
    s.to_string()
}

/// The full set of rebindable actions. Field order is the order shown in the
/// keybinds menu.
#[derive(Resource, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyBindings {
    pub forward: Binding,
    pub back: Binding,
    pub left: Binding,
    pub right: Binding,
    pub jump: Binding,
    pub sprint: Binding,
    pub fire: Binding,
    pub aim: Binding,
    pub reload: Binding,
    pub teleport_home: Binding,
    pub cycle_anim: Binding,
}

impl Default for KeyBindings {
    fn default() -> Self {
        use Binding::{Key, Mouse};
        Self {
            forward: Key(KeyCode::KeyW),
            back: Key(KeyCode::KeyS),
            left: Key(KeyCode::KeyA),
            right: Key(KeyCode::KeyD),
            jump: Key(KeyCode::KeyB),
            sprint: Key(KeyCode::ShiftLeft),
            fire: Mouse(MouseButton::Left),
            aim: Mouse(MouseButton::Right),
            reload: Key(KeyCode::KeyR),
            teleport_home: Key(KeyCode::KeyT),
            cycle_anim: Key(KeyCode::KeyL),
        }
    }
}

/// `(label, accessor)` for each action — drives the keybinds list and the
/// rebind flow. The accessor yields a mutable reference so one entry can be
/// both displayed and reassigned.
pub const SLOTS: &[(&str, fn(&mut KeyBindings) -> &mut Binding)] = &[
    ("Move Forward", |b| &mut b.forward),
    ("Move Back", |b| &mut b.back),
    ("Move Left", |b| &mut b.left),
    ("Move Right", |b| &mut b.right),
    ("Jump", |b| &mut b.jump),
    ("Sprint (toggle)", |b| &mut b.sprint),
    ("Fire", |b| &mut b.fire),
    ("Aim Down Sight", |b| &mut b.aim),
    ("Reload", |b| &mut b.reload),
    ("Teleport to Spawn", |b| &mut b.teleport_home),
    ("Cycle Animation (dev)", |b| &mut b.cycle_anim),
];

impl KeyBindings {
    pub fn slot(&self, i: usize) -> Binding {
        // The accessors take `&mut`; clone through a throwaway copy for reads.
        let mut tmp = self.clone();
        *SLOTS[i].1(&mut tmp)
    }

    pub fn set_slot(&mut self, i: usize, binding: Binding) {
        *SLOTS[i].1(self) = binding;
    }
}
