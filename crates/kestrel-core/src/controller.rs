//! Controller state sent to the console. Button bits match libchiaki's
//! ChiakiControllerButton; axes are signed 16-bit, triggers 0..255.

use crate::ffi::KbControllerState;

pub mod buttons {
    pub const CROSS: u32 = 1 << 0;
    pub const CIRCLE: u32 = 1 << 1;
    pub const SQUARE: u32 = 1 << 2;
    pub const TRIANGLE: u32 = 1 << 3;
    pub const DPAD_LEFT: u32 = 1 << 4;
    pub const DPAD_RIGHT: u32 = 1 << 5;
    pub const DPAD_UP: u32 = 1 << 6;
    pub const DPAD_DOWN: u32 = 1 << 7;
    pub const L1: u32 = 1 << 8;
    pub const R1: u32 = 1 << 9;
    pub const L3: u32 = 1 << 10;
    pub const R3: u32 = 1 << 11;
    pub const OPTIONS: u32 = 1 << 12;
    pub const SHARE: u32 = 1 << 13; // "Create" on DualSense
    pub const TOUCHPAD: u32 = 1 << 14;
    pub const PS: u32 = 1 << 15;
}

/// DualSense touchpad coordinate space used by the PS5 protocol.
pub const PS5_TOUCHPAD_MAX_X: u16 = 1919;
pub const PS5_TOUCHPAD_MAX_Y: u16 = 1079;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ControllerState(pub(crate) KbControllerState);

impl Default for ControllerState {
    fn default() -> Self {
        Self::idle()
    }
}

impl ControllerState {
    /// Nothing pressed, sticks centred, no touches, identity orientation.
    pub fn idle() -> Self {
        ControllerState(KbControllerState {
            touch_id: [-1, -1],
            orient_w: 1.0,
            accel_y: 1.0, // gravity, controller lying flat
            ..Default::default()
        })
    }

    pub fn buttons(&self) -> u32 {
        self.0.buttons
    }
    pub fn set_button(&mut self, bit: u32, pressed: bool) {
        if pressed {
            self.0.buttons |= bit;
        } else {
            self.0.buttons &= !bit;
        }
    }
    pub fn set_triggers(&mut self, l2: u8, r2: u8) {
        self.0.l2 = l2;
        self.0.r2 = r2;
    }
    /// Stick values in the protocol's convention: +x right, +y down.
    pub fn set_left_stick(&mut self, x: i16, y: i16) {
        self.0.left_x = x;
        self.0.left_y = y;
    }
    pub fn set_right_stick(&mut self, x: i16, y: i16) {
        self.0.right_x = x;
        self.0.right_y = y;
    }
    /// slot 0/1; `pos` None lifts the finger. Coordinates are clamped to the PS5 touchpad.
    pub fn set_touch(&mut self, slot: usize, id: i8, pos: Option<(u16, u16)>) {
        if slot > 1 {
            return;
        }
        match pos {
            Some((x, y)) => {
                self.0.touch_id[slot] = id & 0x7f;
                self.0.touch_x[slot] = x.min(PS5_TOUCHPAD_MAX_X);
                self.0.touch_y[slot] = y.min(PS5_TOUCHPAD_MAX_Y);
            }
            None => self.0.touch_id[slot] = -1,
        }
    }
    /// Gyro in rad/s, accelerometer in g, orientation as a unit quaternion.
    pub fn set_motion(&mut self, gyro: [f32; 3], accel: [f32; 3], orient: [f32; 4]) {
        [self.0.gyro_x, self.0.gyro_y, self.0.gyro_z] = gyro;
        [self.0.accel_x, self.0.accel_y, self.0.accel_z] = accel;
        [
            self.0.orient_x,
            self.0.orient_y,
            self.0.orient_z,
            self.0.orient_w,
        ] = orient;
    }

    /// Combines two inputs (e.g. gamepad + keyboard): buttons OR'd, the
    /// larger trigger wins, the stick further from centre wins.
    pub fn merge(&self, other: &ControllerState) -> ControllerState {
        fn pick(a: i16, b: i16) -> i16 {
            if (b as i32).abs() > (a as i32).abs() {
                b
            } else {
                a
            }
        }
        let mut out = *self;
        out.0.buttons |= other.0.buttons;
        out.0.l2 = out.0.l2.max(other.0.l2);
        out.0.r2 = out.0.r2.max(other.0.r2);
        out.0.left_x = pick(out.0.left_x, other.0.left_x);
        out.0.left_y = pick(out.0.left_y, other.0.left_y);
        out.0.right_x = pick(out.0.right_x, other.0.right_x);
        out.0.right_y = pick(out.0.right_y, other.0.right_y);
        out
    }

    pub fn raw(&self) -> &KbControllerState {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_bits_and_merge() {
        let mut a = ControllerState::idle();
        a.set_button(buttons::CROSS, true);
        a.set_left_stick(1000, -200);
        let mut b = ControllerState::idle();
        b.set_button(buttons::R1, true);
        b.set_left_stick(-5000, 10);
        b.set_triggers(0, 200);
        let m = a.merge(&b);
        assert_eq!(m.buttons(), buttons::CROSS | buttons::R1);
        assert_eq!(m.raw().left_x, -5000);
        assert_eq!(m.raw().left_y, -200);
        assert_eq!(m.raw().r2, 200);
        a.set_button(buttons::CROSS, false);
        assert_eq!(a.buttons(), 0);
    }

    #[test]
    fn touch_clamped_and_lifted() {
        let mut s = ControllerState::idle();
        s.set_touch(0, 3, Some((5000, 5000)));
        assert_eq!(
            (s.raw().touch_x[0], s.raw().touch_y[0]),
            (PS5_TOUCHPAD_MAX_X, PS5_TOUCHPAD_MAX_Y)
        );
        s.set_touch(0, 3, None);
        assert_eq!(s.raw().touch_id[0], -1);
    }
}
