//! Tiny localhost HTTP server (std only): live dashboard, JSON data API, bundled examples.
use crate::{Config, Sample, Shared};
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const DASHBOARD: &str = include_str!("../web/index.html");
const BIKE_GAME: &str = include_str!("../examples/bike-game.html");

/// One sample as a JSON line, for `auricle stream`.
pub fn json_sample(t_ms: u128, s: &Sample) -> String {
    format!(r#"{{"t":{t_ms},"o1":{},"o2":{},"o3":{},"ax":{},"ay":{},"yaw":{:.3},"pitch":{:.3}}}"#, s.o1, s.o2, s.o3, s.ax, s.ay, s.yaw, s.pitch)
}

/// Pure routing so it can be tested without sockets: path -> (status, content type, body).
pub fn route(path: &str, head: &Shared, cfg: &Config) -> (u16, &'static str, Vec<u8>) {
    const HTML: &str = "text/html; charset=utf-8";
    const JSON: &str = "application/json";
    match path {
        "/" => (200, HTML, DASHBOARD.into()),
        "/examples/bike-game" => (200, HTML, BIKE_GAME.into()),
        "/recal" => { head.lock().unwrap().recenter(); (200, JSON, b"{}".to_vec()) }
        "/data" => {
            let h = head.lock().unwrap();
            let (dy, dp) = h.dev();
            let s = &h.s;
            let body = format!(
                r#"{{"ok":{},"ready":{},"yaw":{:.3},"pitch":{:.3},"dev":{:.3},"pitch_dev":{:.3},"steer":{:.3},"o1":{},"o2":{},"o3":{},"ax":{},"ay":{}}}"#,
                h.ok, h.neutral.is_some(), s.yaw, s.pitch, dy, dp, if h.ok { cfg.steer(dy) } else { 0.0 }, s.o1, s.o2, s.o3, s.ax, s.ay
            );
            (200, JSON, body.into_bytes())
        }
        _ => (404, "text/plain", b"not found".to_vec()),
    }
}

fn handle(mut s: TcpStream, head: &Shared, cfg: &Config) -> io::Result<()> {
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    let (mut buf, mut n) = ([0u8; 4096], 0);
    while n < buf.len() && !buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
        let r = s.read(&mut buf[n..])?;
        if r == 0 { break }
        n += r;
    }
    let req = String::from_utf8_lossy(&buf[..n]);
    let mut parts = req.lines().next().unwrap_or("").split_whitespace();
    let (status, ctype, body) = match (parts.next(), parts.next()) {
        (Some("GET"), Some(p)) => route(p.split('?').next().unwrap_or(p), head, cfg),
        _ => (405, "text/plain", b"method not allowed".to_vec()),
    };
    let reason = match status { 200 => "OK", 404 => "Not Found", _ => "Method Not Allowed" };
    write!(s, "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len())?;
    s.write_all(&body)
}

/// Serves on localhost only; there is no authentication, so it must not be exposed to a network.
pub fn run(port: u16, head: Shared, cfg: Arc<Config>) -> io::Result<()> {
    let l = TcpListener::bind(("127.0.0.1", port)).map_err(|e| io::Error::new(e.kind(), format!("cannot listen on port {port}: {e}")))?;
    eprintln!("auricle: dashboard http://localhost:{port}   example game http://localhost:{port}/examples/bike-game");
    for conn in l.incoming().flatten() {
        let (h, c) = (head.clone(), cfg.clone());
        thread::spawn(move || { let _ = handle(conn, &h, &c); });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Head;
    use std::sync::Mutex;

    fn cfg() -> Config { Config { mac: None, port: 0, full_turn: 10.0, deadzone: 1.0, invert: false } }

    #[test]
    fn routes() {
        let head: Shared = Arc::new(Mutex::new(Head::default()));
        let (st, _, body) = route("/data", &head, &cfg());
        assert_eq!(st, 200);
        assert!(String::from_utf8(body).unwrap().starts_with(r#"{"ok":false,"ready":false"#));
        assert_eq!(route("/nope", &head, &cfg()).0, 404);
        assert_eq!(route("/", &head, &cfg()).0, 200);
        assert_eq!(route("/examples/bike-game", &head, &cfg()).0, 200);
    }

    #[test]
    fn data_reports_a_sample_and_recentre() {
        let head: Shared = Arc::new(Mutex::new(Head::default()));
        { let mut h = head.lock().unwrap(); h.ok = true; h.s = Sample { yaw: 12.0, pitch: 1.0, ..Default::default() }; h.neutral = Some((2.0, 1.0)); }
        let body = String::from_utf8(route("/data", &head, &cfg()).2).unwrap();
        assert!(body.contains(r#""dev":10.000"#) && body.contains(r#""steer":1.000"#), "{body}");
        route("/recal", &head, &cfg());
        assert!(String::from_utf8(route("/data", &head, &cfg()).2).unwrap().contains(r#""dev":0.000"#));
    }

    #[test]
    fn stream_line_is_valid_shape() {
        let s = Sample { o1: 1, o2: -2, o3: 3, ax: 4, ay: 5, yaw: 1.5, pitch: -0.25 };
        assert_eq!(json_sample(7, &s), r#"{"t":7,"o1":1,"o2":-2,"o3":3,"ax":4,"ay":5,"yaw":1.500,"pitch":-0.250}"#);
    }
}
