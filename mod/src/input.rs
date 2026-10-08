//! hooks.input_read / hooks.input_suppress: Elden Ring's in-game pad, through its own key assignment
//! (so the player's remaps apply). Runs in a task right after PadStep, before characters read input.

use eldenring::cs::{CSInGamePad, UserInputKey};
use eldenring::fd4::{FD4PadManager, InputType};
use fromsoftware_shared::FromStatic;

pub fn pad() -> Option<&'static mut CSInGamePad> {
    let man = unsafe { FD4PadManager::instance_mut() }.ok()?;
    man.get_in_game_pad_mut()
}

pub fn down(key: UserInputKey) -> bool {
    pad().is_some_and(|p| p.poll_digital_input(key))
}

pub fn analog(key: UserInputKey) -> f32 {
    pad().map_or(0.0, |p| p.poll_analog_input(key))
}

/// Withhold `key` from the game for this frame: every virtual input it maps to is cleared.
pub fn suppress(key: UserInputKey) {
    let Some(p) = pad() else { return };
    unsafe {
        let groups = p.input_type_group.as_ref();
        let Some(group) = groups.find(&key) else { return };
        let key_assign = p.key_assign.as_ref();
        let dev = p.pad_device.as_mut();
        let multi = dev.virtual_multi_device.as_mut();
        for (mapped, kind) in group.iter() {
            let Some(vi) = key_assign.get_virtual_input_index(mapped) else { continue };
            match kind {
                InputType::IsStickMoving => multi.set_virtual_analog_state(vi as usize, 0.0),
                _ => multi.set_virtual_digital_state(vi as usize, false),
            }
            if let Some(mut mb) = key_assign.mouse_button_states_map.find(&vi).copied().filter(|i| *i != -1) {
                mb -= 1000;
                if mb > 0 && mb <= 80 {
                    dev.unk78.mouse_button_states[mb as usize] = false;
                }
            }
        }
    }
}

/// Give the game a value for an analog input this frame (used to pass one sideways flick for target switching).
pub fn inject_analog(key: UserInputKey, value: f32) {
    let Some(p) = pad() else { return };
    unsafe {
        let groups = p.input_type_group.as_ref();
        let Some(group) = groups.find(&key) else { return };
        let key_assign = p.key_assign.as_ref();
        let multi = p.pad_device.as_mut().virtual_multi_device.as_mut();
        for (mapped, kind) in group.iter() {
            if kind == InputType::IsStickMoving {
                if let Some(vi) = key_assign.get_virtual_input_index(mapped) {
                    multi.set_virtual_analog_state(vi as usize, value);
                }
            }
        }
    }
}

/// Edge detector for digital inputs.
#[derive(Default)]
pub struct Edge {
    prev: bool,
    pub held_s: f32,
    /// How long the input was held before its last release.
    pub last_held_s: f32,
}

impl Edge {
    /// Returns (pressed this frame, released this frame, held now).
    pub fn update(&mut self, now: bool, dt: f32) -> (bool, bool, bool) {
        let pressed = now && !self.prev;
        let released = !now && self.prev;
        if released {
            self.last_held_s = self.held_s;
        }
        self.held_s = if now { self.held_s + dt } else { 0.0 };
        self.prev = now;
        (pressed, released, now)
    }
}
