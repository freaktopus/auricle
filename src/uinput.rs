//! `auricle mouse`: a virtual pointer device (Linux uinput) driven by head yaw/pitch.
use crate::{Config, Shared};
use std::ffi::{c_char, c_void};
use std::io;
use std::sync::Arc;
use std::time::Duration;

extern "C" {
    fn open(path: *const c_char, flags: i32, ...) -> i32;
    fn write(fd: i32, buf: *const c_void, n: usize) -> isize;
    fn ioctl(fd: i32, req: u64, ...) -> i32;
    fn close(fd: i32) -> i32;
}

const O_WRONLY: i32 = 1;
const O_NONBLOCK: i32 = 0o4000;
const UI_SET_EVBIT: u64 = 0x4004_5564;
const UI_SET_KEYBIT: u64 = 0x4004_5565;
const UI_SET_RELBIT: u64 = 0x4004_5566;
const UI_DEV_CREATE: u64 = 0x5501;
const UI_DEV_DESTROY: u64 = 0x5502;
const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const BTN_LEFT: i32 = 0x110; // a pointer needs at least one button for desktops to treat it as a mouse

const TICK: Duration = Duration::from_millis(16);
const GAIN: f32 = 0.6; // pixels per tick per degree beyond the dead zone
const DEADZONE: f32 = 3.0; // a bigger dead zone than steering: a cursor must sit still when you look ahead

#[repr(C)]
struct UserDev { name: [u8; 80], bustype: u16, vendor: u16, product: u16, version: u16, ff_effects_max: u32, abs: [i32; 256] }
#[repr(C)]
struct InputEvent { sec: i64, usec: i64, ty: u16, code: u16, value: i32 }

struct Device(i32);
impl Device {
    fn create() -> io::Result<Device> {
        let fd = unsafe { open(c"/dev/uinput".as_ptr(), O_WRONLY | O_NONBLOCK) };
        if fd < 0 {
            let e = io::Error::last_os_error();
            return Err(io::Error::new(e.kind(), format!("cannot open /dev/uinput: {e} (run as root, or add a udev rule giving your user write access)")));
        }
        let d = Device(fd);
        for (req, v) in [(UI_SET_EVBIT, EV_REL as i32), (UI_SET_EVBIT, EV_KEY as i32), (UI_SET_RELBIT, REL_X as i32), (UI_SET_RELBIT, REL_Y as i32), (UI_SET_KEYBIT, BTN_LEFT)] {
            if unsafe { ioctl(fd, req, v) } < 0 { return Err(io::Error::last_os_error()) }
        }
        let mut dev = UserDev { name: [0; 80], bustype: 0x06, vendor: 1, product: 1, version: 1, ff_effects_max: 0, abs: [0; 256] };
        dev.name[..14].copy_from_slice(b"auricle-cursor");
        let n = unsafe { write(fd, &dev as *const UserDev as *const c_void, std::mem::size_of::<UserDev>()) };
        if n < 0 || unsafe { ioctl(fd, UI_DEV_CREATE) } < 0 { return Err(io::Error::last_os_error()) }
        Ok(d)
    }
    fn emit(&self, ty: u16, code: u16, value: i32) {
        let e = InputEvent { sec: 0, usec: 0, ty, code, value };
        unsafe { write(self.0, &e as *const InputEvent as *const c_void, std::mem::size_of::<InputEvent>()) };
    }
}
impl Drop for Device { fn drop(&mut self) { unsafe { ioctl(self.0, UI_DEV_DESTROY); close(self.0); } } }

/// Degrees past the dead zone, signed.
fn speed(dev: f32) -> f32 { if dev.abs() < DEADZONE { 0.0 } else { dev - dev.signum() * DEADZONE } }

pub fn run(head: Shared, cfg: Arc<Config>) -> io::Result<()> {
    let dev = Device::create()?;
    let sign = if cfg.invert { -1.0 } else { 1.0 };
    let (mut rx, mut ry) = (0.0f32, 0.0f32); // sub-pixel remainders
    eprintln!("auricle: moving the cursor with your head (Ctrl+C to stop)");
    loop {
        std::thread::sleep(TICK);
        let (ok, (dy, dp)) = { let h = head.lock().unwrap(); (h.ok, h.dev()) };
        if !ok { continue }
        rx += sign * speed(dy) * GAIN;
        ry += -sign * speed(dp) * GAIN; // nose up = cursor up
        let (mx, my) = (rx as i32, ry as i32);
        rx -= mx as f32; ry -= my as f32;
        if mx != 0 { dev.emit(EV_REL, REL_X, mx) }
        if my != 0 { dev.emit(EV_REL, REL_Y, my) }
        if mx != 0 || my != 0 { dev.emit(EV_SYN, 0, 0) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dead_zone() {
        assert_eq!(speed(2.9), 0.0);
        assert_eq!(speed(5.0), 2.0);
        assert_eq!(speed(-5.0), -2.0);
    }
}
