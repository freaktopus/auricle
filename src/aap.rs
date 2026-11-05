//! Bluetooth side: open the earbuds' L2CAP control channel (PSM 0x1001), ask for motion reports, decode them.
//!
//! The channel speaks Apple's AAP protocol. Message formats here follow the community reverse-engineering
//! work of the LibrePods project (https://github.com/kavishdevar/librepods).
use crate::{Sample, Shared};
use std::io;
use std::process::Command;
use std::time::{Duration, Instant};
use std::{mem, thread};

extern "C" {
    fn socket(domain: i32, ty: i32, proto: i32) -> i32;
    fn connect(fd: i32, addr: *const SockaddrL2, len: u32) -> i32;
    fn setsockopt(fd: i32, level: i32, name: i32, val: *const u8, len: u32) -> i32;
    fn send(fd: i32, buf: *const u8, len: usize, flags: i32) -> isize;
    fn recv(fd: i32, buf: *mut u8, len: usize, flags: i32) -> isize;
    fn close(fd: i32) -> i32;
}

const AF_BLUETOOTH: i32 = 31;
const SOCK_SEQPACKET: i32 = 5;
const BTPROTO_L2CAP: i32 = 0;
const SOL_BLUETOOTH: i32 = 274;
const BT_SECURITY: i32 = 4;
const SOL_SOCKET: i32 = 1;
const SO_RCVTIMEO: i32 = 20;
const SO_SNDTIMEO: i32 = 21;
const PSM_AAP: u16 = 0x1001;
/// Service UUID the earbuds advertise for this control channel; used to recognise compatible devices.
const AAP_UUID: &str = "74ec2172-0bad-4d01-8f77-997b2be0722a";

#[repr(C)]
struct SockaddrL2 { family: u16, psm: u16, bdaddr: [u8; 6], cid: u16, bdaddr_type: u8 }
#[repr(C)]
struct Timeval { sec: i64, usec: i64 }

const HANDSHAKE: [u8; 16] = [0, 0, 4, 0, 1, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const FEATURES: [u8; 14] = [4, 0, 4, 0, 0x4d, 0, 0xd7, 0, 0, 0, 0, 0, 0, 0];
const NOTIFY: [u8; 10] = [4, 0, 4, 0, 0x0f, 0, 0xff, 0xff, 0xff, 0xff];
const TAKE_OVER: [u8; 11] = [4, 0, 4, 0, 9, 0, 6, 1, 0, 0, 0]; // ask to own the connection
/// Two start-tracking variants; different firmware answers different ones, so both are re-sent alternately.
const START: [&[u8]; 2] = [
    &[4, 0, 4, 0, 0x17, 0, 0, 0, 0x10, 0, 0x0f, 0, 0x08, 0x73, 0x42, 0x0b, 0x08, 0x10, 0x10, 0x02, 0x1a, 0x05, 0x01, 0x40, 0x9c, 0, 0],
    &[4, 0, 4, 0, 0x17, 0, 0, 0, 0x10, 0, 0x10, 0, 0x08, 0xa1, 0x02, 0x42, 0x0b, 0x08, 0x0e, 0x10, 0x02, 0x1a, 0x05, 0x01, 0x40, 0x9c, 0, 0],
];

pub struct Link(i32);
impl Link {
    pub fn open(mac: [u8; 6]) -> io::Result<Link> {
        let fd = unsafe { socket(AF_BLUETOOTH, SOCK_SEQPACKET, BTPROTO_L2CAP) };
        if fd < 0 { return Err(io::Error::last_os_error()) }
        let l = Link(fd); // closes on drop, including on the error paths below
        // Encryption is required: without it the earbuds answer the handshake but never stream motion.
        l.opt(SOL_BLUETOOTH, BT_SECURITY, &[2u8, 0])?; // BT_SECURITY_MEDIUM
        l.timeout(SO_SNDTIMEO, 10_000)?; // bounds connect()
        l.timeout(SO_RCVTIMEO, 250)?;
        let addr = SockaddrL2 { family: AF_BLUETOOTH as u16, psm: PSM_AAP.to_le(), bdaddr: mac, cid: 0, bdaddr_type: 0 };
        if unsafe { connect(fd, &addr, mem::size_of::<SockaddrL2>() as u32) } < 0 { return Err(io::Error::last_os_error()) }
        Ok(l)
    }
    fn opt(&self, level: i32, name: i32, v: &[u8]) -> io::Result<()> {
        if unsafe { setsockopt(self.0, level, name, v.as_ptr(), v.len() as u32) } < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
    fn timeout(&self, name: i32, ms: i64) -> io::Result<()> {
        let tv = Timeval { sec: ms / 1000, usec: (ms % 1000) * 1000 };
        let p = unsafe { std::slice::from_raw_parts(&tv as *const Timeval as *const u8, mem::size_of::<Timeval>()) };
        self.opt(SOL_SOCKET, name, p)
    }
    pub fn send(&self, data: &[u8]) -> io::Result<()> {
        if unsafe { send(self.0, data.as_ptr(), data.len(), 0) } < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }
    /// Ok(None) = nothing arrived within the receive timeout.
    pub fn recv(&self, buf: &mut [u8]) -> io::Result<Option<usize>> {
        let n = unsafe { recv(self.0, buf.as_mut_ptr(), buf.len(), 0) };
        if n > 0 { return Ok(Some(n as usize)) }
        if n == 0 { return Err(io::ErrorKind::ConnectionAborted.into()) }
        let e = io::Error::last_os_error();
        match e.kind() { io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted => Ok(None), _ => Err(e) }
    }
}
impl Drop for Link { fn drop(&mut self) { unsafe { close(self.0); } } }

/// "AA:BB:CC:DD:EE:FF" -> the byte order the kernel expects (reversed).
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let parts: Vec<u8> = s.split(':').map(|p| u8::from_str_radix(p, 16)).collect::<Result<_, _>>().ok()?;
    let mut m: [u8; 6] = parts.try_into().ok()?;
    m.reverse();
    Some(m)
}

/// Decode a motion report; None for every other AAP message.
pub fn decode(p: &[u8]) -> Option<Sample> {
    if !(70..=100).contains(&p.len()) || p[4..6] != [0x17, 0] { return None }
    let i = |o: usize| i16::from_le_bytes([p[o], p[o + 1]]);
    let (o1, o2, o3) = (i(43), i(45), i(47));
    if o1 == 0 && o2 == 0 && o3 == 0 { return None } // start-up reports carry no orientation yet
    Some(Sample {
        o1, o2, o3, ax: i(51), ay: i(53),
        yaw: (o2 as f32 - o3 as f32) / 2.0 / 32000.0 * 180.0,
        pitch: (o2 as f32 + o3 as f32) / 2.0 / 32000.0 * 180.0,
    })
}

/// Connected devices that expose the motion channel, as (address, name), found through bluetoothctl.
pub fn find_devices() -> Vec<(String, String)> {
    let run = |args: &[&str]| Command::new("bluetoothctl").args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let mut out = vec![];
    for line in run(&["devices"]).lines() {
        let mut it = line.splitn(3, ' ');
        if let (Some("Device"), Some(mac), name) = (it.next(), it.next(), it.next().unwrap_or("")) {
            let info = run(&["info", mac]);
            if info.contains("Connected: yes") && info.to_lowercase().contains(AAP_UUID) { out.push((mac.to_string(), name.to_string())) }
        }
    }
    out
}

/// Keep a session alive forever: reconnect on any error. `tap` sees every decoded sample.
pub fn run(mac: [u8; 6], head: Shared, mut tap: Option<Box<dyn FnMut(&Sample) + Send>>) {
    loop {
        if let Err(e) = session(mac, &head, &mut tap) { eprintln!("auricle: link problem, retrying: {e}") }
        head.lock().unwrap().ok = false;
        thread::sleep(Duration::from_secs(2));
    }
}

fn session(mac: [u8; 6], head: &Shared, tap: &mut Option<Box<dyn FnMut(&Sample) + Send>>) -> io::Result<()> {
    let link = Link::open(mac)?;
    for m in [&HANDSHAKE[..], &FEATURES, &NOTIFY, &TAKE_OVER] { link.send(m)?; thread::sleep(Duration::from_millis(300)) }
    thread::sleep(Duration::from_millis(1200));
    let (mut next, mut i, mut buf) = (Instant::now(), 0, [0u8; 4096]);
    loop {
        if Instant::now() >= next { link.send(START[i % 2])?; i += 1; next = Instant::now() + Duration::from_secs(2) } // reports stop unless re-asked
        let Some(n) = link.recv(&mut buf)? else { continue };
        if let Some(s) = decode(&buf[..n]) {
            { let mut h = head.lock().unwrap(); h.ok = true; h.s = s; if h.neutral.is_none() { h.neutral = Some((s.yaw, s.pitch)) } }
            if let Some(t) = tap.as_mut() { t(&s) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_is_reversed() {
        assert_eq!(parse_mac("01:02:03:04:05:0A"), Some([0x0a, 5, 4, 3, 2, 1]));
        assert_eq!(parse_mac("nope"), None);
        assert_eq!(parse_mac("01:02:03"), None);
    }

    #[test]
    fn decodes_a_motion_report() {
        let mut p = vec![0u8; 81];
        p[4] = 0x17;
        p[45..47].copy_from_slice(&(-3346i16).to_le_bytes()); // o2 and o3 from a real capture
        p[47..49].copy_from_slice(&6179i16.to_le_bytes());
        let s = decode(&p).unwrap();
        assert!((s.yaw - -26.79).abs() < 0.01, "{}", s.yaw);
        assert!((s.pitch - 7.97).abs() < 0.01, "{}", s.pitch);
        assert!(decode(&p[..60]).is_none()); // too short (acknowledgements, descriptors...)
        p[4] = 0x09;
        assert!(decode(&p).is_none()); // other opcode
        assert!(decode(&[vec![0u8; 4], vec![0x17, 0], vec![0u8; 75]].concat()).is_none()); // all-zero orientation
    }
}
