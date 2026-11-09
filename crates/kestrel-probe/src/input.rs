//! Game controllers via SDL3: DualSense (USB/Bluetooth), Xbox/XInput and
//! anything else SDL maps as a gamepad. Produces the PS5 controller state and
//! applies feedback (rumble, light bar, adaptive triggers) coming back.

use std::collections::HashMap;

use anyhow::{anyhow, Result};
use kestrel_core::controller::{buttons, ControllerState, PS5_TOUCHPAD_MAX_X, PS5_TOUCHPAD_MAX_Y};
use sdl3::event::Event;
use sdl3::gamepad::{Axis, Button, Gamepad, GamepadType};
use sdl3::joystick::JoystickId;

pub struct Pad {
    pub gamepad: Gamepad,
    pub name: String,
    pub kind: GamepadType,
    touches: [Option<(u16, u16)>; 2],
}

impl Pad {
    pub fn is_dualsense(&self) -> bool {
        matches!(self.kind, GamepadType::PS5)
    }
}

pub struct Input {
    _sdl: sdl3::Sdl,
    subsystem: sdl3::GamepadSubsystem,
    pump: sdl3::EventPump,
    pub pads: HashMap<JoystickId, Pad>,
    touch_seq: i8,
    touch_ids: [i8; 2],
}

pub enum InputChange {
    Connected(String),
    Disconnected(String),
}

impl Input {
    pub fn new() -> Result<Self> {
        // Let SDL drive DualSense over HID so rumble, LED and trigger effects work
        // over both USB and Bluetooth.
        sdl3::hint::set("SDL_JOYSTICK_HIDAPI_PS5", "1");
        sdl3::hint::set("SDL_JOYSTICK_HIDAPI_PS5_RUMBLE", "1");
        let sdl = sdl3::init().map_err(|e| anyhow!("SDL init: {e}"))?;
        let subsystem = sdl
            .gamepad()
            .map_err(|e| anyhow!("SDL gamepad subsystem: {e}"))?;
        let pump = sdl
            .event_pump()
            .map_err(|e| anyhow!("SDL event pump: {e}"))?;
        let mut input = Input {
            _sdl: sdl,
            subsystem,
            pump,
            pads: HashMap::new(),
            touch_seq: 0,
            touch_ids: [-1, -1],
        };
        if let Ok(ids) = input.subsystem.gamepads() {
            for id in ids {
                let _ = input.open(id);
            }
        }
        Ok(input)
    }

    fn open(&mut self, id: JoystickId) -> Option<String> {
        if self.pads.contains_key(&id) {
            return None;
        }
        let gamepad = self.subsystem.open(id).ok()?;
        let name = gamepad.name().unwrap_or_else(|| "Gamepad".into());
        let kind = gamepad.r#type();
        self.pads.insert(
            id,
            Pad {
                gamepad,
                name: name.clone(),
                kind,
                touches: [None, None],
            },
        );
        Some(name)
    }

    /// Drains SDL events; returns connection changes for display.
    pub fn poll(&mut self) -> Vec<InputChange> {
        let mut changes = Vec::new();
        let events: Vec<Event> = self.pump.poll_iter().collect();
        for event in events {
            match event {
                Event::GamepadAdded { which, .. } => {
                    if let Some(name) = self.open(which) {
                        changes.push(InputChange::Connected(name));
                    }
                }
                Event::GamepadRemoved { which, .. } => {
                    if let Some(p) = self.pads.remove(&which) {
                        changes.push(InputChange::Disconnected(p.name));
                    }
                }
                Event::GamepadTouchpadDown {
                    which,
                    finger,
                    x,
                    y,
                    ..
                }
                | Event::GamepadTouchpadMotion {
                    which,
                    finger,
                    x,
                    y,
                    ..
                } => {
                    if let Some(p) = self.pads.get_mut(&which) {
                        if (0..2).contains(&finger) {
                            let pos = (
                                (x.clamp(0.0, 1.0) * PS5_TOUCHPAD_MAX_X as f32) as u16,
                                (y.clamp(0.0, 1.0) * PS5_TOUCHPAD_MAX_Y as f32) as u16,
                            );
                            p.touches[finger as usize] = Some(pos);
                        }
                    }
                }
                Event::GamepadTouchpadUp { which, finger, .. } => {
                    if let Some(p) = self.pads.get_mut(&which) {
                        if (0..2).contains(&finger) {
                            p.touches[finger as usize] = None;
                        }
                    }
                }
                _ => {}
            }
        }
        changes
    }

    /// Current state merged across all connected pads.
    pub fn state(&mut self) -> ControllerState {
        let mut merged = ControllerState::idle();
        let mut touch: [Option<(u16, u16)>; 2] = [None, None];
        for pad in self.pads.values() {
            merged = merged.merge(&read_pad(&pad.gamepad));
            for (slot, t) in pad.touches.iter().enumerate() {
                if t.is_some() && touch[slot].is_none() {
                    touch[slot] = *t;
                }
            }
        }
        // Each new finger contact gets a fresh 7-bit id, as the protocol expects.
        for slot in 0..2 {
            match touch[slot] {
                Some(pos) => {
                    if self.touch_ids[slot] < 0 {
                        self.touch_ids[slot] = self.touch_seq;
                        self.touch_seq = (self.touch_seq + 1) & 0x7f;
                    }
                    merged.set_touch(slot, self.touch_ids[slot], Some(pos));
                }
                None => {
                    self.touch_ids[slot] = -1;
                    merged.set_touch(slot, -1, None);
                }
            }
        }
        merged
    }

    pub fn rumble(&mut self, left: u8, right: u8) {
        for pad in self.pads.values_mut() {
            // Values are re-sent by the console while active; a 5 s cap stops a
            // stuck motor if the stream dies mid-rumble.
            let _ = pad
                .gamepad
                .set_rumble((left as u16) << 8, (right as u16) << 8, 5000);
        }
    }

    pub fn set_led(&mut self, rgb: [u8; 3]) {
        for pad in self.pads.values_mut().filter(|p| p.is_dualsense()) {
            let _ = pad.gamepad.set_led(rgb[0], rgb[1], rgb[2]);
        }
    }

    /// Applies DualSense adaptive-trigger effects using the same HID effect
    /// report layout as chiaki-ng (SDL's DS5EffectsState_t).
    pub fn trigger_effects(
        &mut self,
        type_left: u8,
        left: &[u8; 10],
        type_right: u8,
        right: &[u8; 10],
    ) {
        let report = ds5_trigger_report(type_left, left, type_right, right);
        for pad in self.pads.values_mut().filter(|p| p.is_dualsense()) {
            let _ = pad.gamepad.send_effect(&report);
        }
    }

    pub fn reset_feedback(&mut self) {
        self.rumble(0, 0);
        let off = [0u8; 10];
        self.trigger_effects(0x05, &off, 0x05, &off);
    }
}

pub fn ds5_trigger_report(
    type_left: u8,
    left: &[u8; 10],
    type_right: u8,
    right: &[u8; 10],
) -> [u8; 47] {
    let mut r = [0u8; 47];
    r[0] |= 0x04 | 0x08; // ucEnableBits1: left + right trigger effects
    r[1] |= 0x40; // ucEnableBits2: apply effect intensity
    r[10] = type_right; // rgucRightTriggerEffect[11] at offset 10
    r[11..21].copy_from_slice(right);
    r[21] = type_left; // rgucLeftTriggerEffect[11] at offset 21
    r[22..32].copy_from_slice(left);
    r[36] = 0x00; // rgucUnknown1[4]: effect intensity, 0 = full
    r
}

fn axis_u8(v: i16) -> u8 {
    // SDL triggers are 0..=32767.
    ((v.max(0) as i32 * 255) / 32767) as u8
}

fn read_pad(g: &Gamepad) -> ControllerState {
    let mut s = ControllerState::idle();
    // SDL uses positional names; South = Cross, East = Circle on every layout.
    let map: [(Button, u32); 15] = [
        (Button::South, buttons::CROSS),
        (Button::East, buttons::CIRCLE),
        (Button::West, buttons::SQUARE),
        (Button::North, buttons::TRIANGLE),
        (Button::DPadLeft, buttons::DPAD_LEFT),
        (Button::DPadRight, buttons::DPAD_RIGHT),
        (Button::DPadUp, buttons::DPAD_UP),
        (Button::DPadDown, buttons::DPAD_DOWN),
        (Button::LeftShoulder, buttons::L1),
        (Button::RightShoulder, buttons::R1),
        (Button::LeftStick, buttons::L3),
        (Button::RightStick, buttons::R3),
        (Button::Start, buttons::OPTIONS),
        (Button::Back, buttons::SHARE),
        (Button::Guide, buttons::PS),
    ];
    for (b, bit) in map {
        s.set_button(bit, g.button(b));
    }
    s.set_button(buttons::TOUCHPAD, g.button(Button::Touchpad));
    let l2 = axis_u8(g.axis(Axis::TriggerLeft));
    let r2 = axis_u8(g.axis(Axis::TriggerRight));
    s.set_triggers(l2, r2);
    s.set_left_stick(g.axis(Axis::LeftX), g.axis(Axis::LeftY));
    s.set_right_stick(g.axis(Axis::RightX), g.axis(Axis::RightY));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_report_layout() {
        let l = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let r = [11, 12, 13, 14, 15, 16, 17, 18, 19, 20];
        let rep = ds5_trigger_report(0x21, &l, 0x26, &r);
        assert_eq!(rep.len(), 47);
        assert_eq!(rep[0], 0x0c);
        assert_eq!(rep[1], 0x40);
        assert_eq!(rep[10], 0x26);
        assert_eq!(&rep[11..21], &r);
        assert_eq!(rep[21], 0x21);
        assert_eq!(&rep[22..32], &l);
    }

    #[test]
    fn trigger_scaling() {
        assert_eq!(axis_u8(0), 0);
        assert_eq!(axis_u8(32767), 255);
        assert_eq!(axis_u8(-5), 0);
    }
}
