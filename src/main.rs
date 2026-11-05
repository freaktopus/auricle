//! auricle: read the motion-sensor data of Bluetooth earbuds and use it as a head-tracking input.
mod aap;
mod server;
mod uinput;

use std::sync::{Arc, Mutex};

pub type Shared = Arc<Mutex<Head>>;

/// One decoded sensor report: raw 16-bit values plus the angles derived from them (degrees).
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub o1: i16,
    pub o2: i16,
    pub o3: i16,
    pub ax: i16,
    pub ay: i16,
    pub yaw: f32,
    pub pitch: f32,
}

/// Latest sample plus the "centre" pose it is measured against.
#[derive(Default)]
pub struct Head {
    pub ok: bool,
    pub s: Sample,
    pub neutral: Option<(f32, f32)>,
}

impl Head {
    /// (yaw, pitch) relative to the centre pose; the first pose is the centre until set otherwise.
    pub fn dev(&self) -> (f32, f32) {
        let n = self.neutral.unwrap_or((self.s.yaw, self.s.pitch));
        (self.s.yaw - n.0, self.s.pitch - n.1)
    }
    /// Make the current pose the centre (or the next packet's pose if none has arrived yet).
    pub fn recenter(&mut self) {
        self.neutral = if self.ok { Some((self.s.yaw, self.s.pitch)) } else { None };
    }
}

pub struct Config {
    pub mac: Option<String>,
    pub port: u16,
    pub full_turn: f32,
    pub deadzone: f32,
    pub invert: bool,
}

impl Config {
    /// Head angle (degrees past centre) -> normalised axis in [-1, 1].
    pub fn steer(&self, dev: f32) -> f32 {
        let a = if dev.abs() < self.deadzone { 0.0 } else { dev - dev.signum() * self.deadzone };
        let s = (a / (self.full_turn - self.deadzone)).clamp(-1.0, 1.0);
        if self.invert { -s } else { s }
    }
}

impl Config {
    /// Defaults, overridden by the `key = value` pairs of a config.toml.
    fn from_toml(text: &str) -> Result<Config, String> {
        let mut c = Config { mac: None, port: 8000, full_turn: 10.0, deadzone: 1.0, invert: false };
        for (k, v) in parse_toml(text) {
            let num = |what: &str| v.parse::<f32>().map_err(|_| format!("{what} must be a number"));
            match k.as_str() {
                "mac" => c.mac = Some(v.clone()).filter(|m| !m.is_empty()),
                "port" => c.port = v.parse().map_err(|_| "port must be a whole number".to_string())?,
                "full_turn" => c.full_turn = num("full_turn")?,
                "deadzone" => c.deadzone = num("deadzone")?,
                "invert" => c.invert = match v.as_str() { "true" => true, "false" => false, _ => return Err("invert must be true or false".into()) },
                other => return Err(format!("unknown setting '{other}'")),
            }
        }
        Ok(c)
    }
}

/// Minimal TOML subset: flat `key = value` lines, `#` comments, quoted strings, numbers, booleans.
fn parse_toml(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|l| {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with('[') { return None }
        let (k, v) = l.split_once('=')?;
        let v = v.trim();
        let v = match v.strip_prefix('"') { Some(q) => q.split('"').next().unwrap_or(""), None => v.split('#').next().unwrap_or("").trim() };
        Some((k.trim().to_string(), v.to_string()))
    }).collect()
}

/// ./config.toml, else ~/.config/auricle/config.toml; defaults when neither exists.
fn load_config() -> Config {
    let home = std::env::var("HOME").map(|h| format!("{h}/.config/auricle/config.toml"));
    for path in ["config.toml".to_string()].into_iter().chain(home) {
        if let Ok(text) = std::fs::read_to_string(&path) {
            return Config::from_toml(&text).unwrap_or_else(|e| die(&format!("{path}: {e}")));
        }
    }
    Config::from_toml("").unwrap()
}

const USAGE: &str = "auricle - motion-sensor data from Bluetooth earbuds (Linux, BlueZ)

USAGE:
    auricle <COMMAND> [OPTIONS]

COMMANDS:
    scan     list connected earbuds that expose a motion sensor
    stream   print every sensor report to stdout as JSON lines
    serve    local JSON API + live dashboard at http://localhost:PORT
    mouse    move the system cursor with your head (needs write access to /dev/uinput)

OPTIONS:
    --mac AA:BB:CC:DD:EE:FF   device address (default: auto-detect a connected pair)
                              every option can also be set in ./config.toml (see config.example.toml)
    --port N                  port for `serve` (default 8000)
    --full-turn DEG           head angle that maps to a full axis value of 1.0 (default 10)
    --deadzone DEG            ignore head angles below this (default 1)
    --invert                  flip the axes
    -h, --help                show this help
";

fn die(msg: &str) -> ! {
    eprintln!("auricle: {msg}");
    std::process::exit(2);
}

fn parse_args(args: &[String]) -> (String, Config) {
    let mut cmd = None;
    let mut cfg = load_config();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut val = |name: &str| it.next().cloned().unwrap_or_else(|| die(&format!("{name} needs a value")));
        match a.as_str() {
            "-h" | "--help" => { print!("{USAGE}"); std::process::exit(0) }
            "--mac" => cfg.mac = Some(val("--mac")),
            "--port" => cfg.port = val("--port").parse().unwrap_or_else(|_| die("--port must be a number")),
            "--full-turn" => cfg.full_turn = val("--full-turn").parse().unwrap_or_else(|_| die("--full-turn must be a number")),
            "--deadzone" => cfg.deadzone = val("--deadzone").parse().unwrap_or_else(|_| die("--deadzone must be a number")),
            "--invert" => cfg.invert = true,
            s if !s.starts_with('-') && cmd.is_none() => cmd = Some(s.to_string()),
            s => die(&format!("unknown argument '{s}' (try --help)")),
        }
    }
    if !(cfg.deadzone >= 0.0 && cfg.full_turn > cfg.deadzone) {
        die("--full-turn must be larger than --deadzone, and --deadzone must be >= 0");
    }
    match cmd {
        Some(c) => (c, cfg),
        None => { print!("{USAGE}"); std::process::exit(2) }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (cmd, cfg) = parse_args(&args);
    match cmd.as_str() {
        "scan" => {
            let found = aap::find_devices();
            if found.is_empty() { die("no connected earbuds with a motion sensor found (pair and connect them first)") }
            for (mac, name) in found { println!("{mac}  {name}") }
        }
        "stream" | "serve" | "mouse" => {
            let mac = cfg.mac.clone().unwrap_or_else(|| {
                aap::find_devices().into_iter().next().map(|d| d.0)
                    .unwrap_or_else(|| die("no connected earbuds found; connect them or pass --mac"))
            });
            let mac_bytes = aap::parse_mac(&mac).unwrap_or_else(|| die("--mac must look like AA:BB:CC:DD:EE:FF"));
            let head: Shared = Arc::new(Mutex::new(Head::default()));
            let tap: Option<Box<dyn FnMut(&Sample) + Send>> = if cmd == "stream" {
                let t0 = std::time::Instant::now();
                Some(Box::new(move |s| {
                    use std::io::Write;
                    // a closed pipe (e.g. `| head`) is a normal way to stop, not an error
                    if writeln!(std::io::stdout(), "{}", server::json_sample(t0.elapsed().as_millis(), s)).is_err() { std::process::exit(0) }
                }))
            } else { None };
            let h = head.clone();
            let reader = std::thread::spawn(move || aap::run(mac_bytes, h, tap));
            let cfg = Arc::new(cfg);
            let result = match cmd.as_str() {
                "serve" => server::run(cfg.port, head, cfg),
                "mouse" => uinput::run(head, cfg),
                _ => { let _ = reader.join(); Ok(()) } // stream: runs until Ctrl+C
            };
            if let Err(e) = result { die(&e.to_string()) }
        }
        other => die(&format!("unknown command '{other}' (try --help)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cfg() -> Config { Config { mac: None, port: 8000, full_turn: 10.0, deadzone: 1.0, invert: false } }

    #[test]
    fn axis_curve() {
        let c = cfg();
        assert_eq!(c.steer(0.5), 0.0); // inside dead zone
        assert!((c.steer(5.5) - 0.5).abs() < 1e-6); // (5.5-1)/9
        assert_eq!(c.steer(40.0), 1.0); // clamped
        assert_eq!(c.steer(-40.0), -1.0);
        assert_eq!(Config { invert: true, ..cfg() }.steer(40.0), -1.0);
    }

    #[test]
    fn toml_parsing() {
        let v = parse_toml("# c\nmac = \"AA:BB:CC:DD:EE:FF\"  # trailing\n[table]\nport = 9000 # x\n\nnot a pair\n");
        assert_eq!(v, vec![("mac".into(), "AA:BB:CC:DD:EE:FF".into()), ("port".into(), "9000".into())]);
    }

    #[test]
    fn config_from_toml() {
        let c = Config::from_toml("mac = \"AA:BB:CC:DD:EE:FF\"\ninvert = true\nfull_turn = 15").unwrap();
        assert_eq!((c.mac.as_deref(), c.port, c.full_turn, c.invert), (Some("AA:BB:CC:DD:EE:FF"), 8000, 15.0, true));
        assert!(Config::from_toml("port = abc").is_err());
        assert!(Config::from_toml("invert = maybe").is_err());
        assert!(Config::from_toml("colour = red").is_err()); // typos are reported, not ignored
        assert_eq!(Config::from_toml("").unwrap().mac, None);
    }

    #[test]
    fn recenter_uses_current_pose() {
        let mut h = Head { ok: true, s: Sample { yaw: 12.0, pitch: -3.0, ..Default::default() }, neutral: None };
        assert_eq!(h.dev(), (0.0, 0.0)); // no centre yet: the first pose is the centre
        h.recenter();
        h.s.yaw = 20.0;
        assert_eq!(h.dev(), (8.0, 0.0));
    }
}
