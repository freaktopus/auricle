# auricle

![License: MIT](https://img.shields.io/badge/license-MIT-blue)
![Platform: Linux](https://img.shields.io/badge/platform-Linux-lightgrey)
![Language: Rust](https://img.shields.io/badge/rust-2021-orange)
![Dependencies: none](https://img.shields.io/badge/dependencies-none-brightgreen)
![Earbuds: Apple only](https://img.shields.io/badge/earbuds-Apple%20only-red)
![Status: experimental](https://img.shields.io/badge/status-experimental-yellow)

**Read the head-motion sensor of AirPods over Bluetooth L2CAP on Linux, and turn it into a stream of head angles you can use anywhere.**

Earbuds with spatial audio carry a motion sensor (an IMU) and report it to the phone that owns them. `auricle` is a
small Linux program that talks to the earbuds directly, asks them for that motion data, and decodes it. It has no
dependencies: just Rust's standard library and the Linux Bluetooth socket API.

> [!IMPORTANT]
> **Apple earbuds only.** `auricle` is a client for Apple's proprietary control channel (L2CAP PSM `0x1001`), not a
> generic earbud-sensor reader. It has been tested on **one pair of AirPods Pro (2nd generation)**. Other Apple
> models with head tracking (AirPods 3, AirPods Pro 1, AirPods Max, some Beats) probably use the same channel but
> are **untested**. Sony, Samsung, Bose, Jabra and other brands use different proprietary protocols and are
> **not supported**. See [Which devices work?](#which-devices-work).

> [!WARNING]
> **Experimental software with real limits.**
> - **Unofficial and fragile.** The protocol is undocumented and reverse-engineered by the community; any firmware update can break it.
> - **Linux only.** It needs BlueZ's L2CAP sockets (and `uinput` for mouse mode).
> - **The sensor reports only while the earbuds are worn and your head is moving.** A still head produces no data.
> - **Reliability is not fully understood.** During development the stream worked and stopped working for periods without a confirmed cause.
> - **Angle accuracy is unmeasured.** The yaw/pitch formulas come from community notes and have not been checked against an independent reference. Only yaw and pitch are derived; roll is not decoded.
>
> Details: [Limits and open questions](#limits-and-open-questions).

> [!NOTE]
> Not affiliated with or endorsed by Apple. "AirPods" is a trademark of Apple Inc.

## Features

- **Raw sensor data.** Three orientation values and two acceleration values, each a signed 16-bit integer, about 23 times per second while your head moves.
- **Head angles.** Yaw and pitch in degrees, relative to a centre pose you can reset at any time.
- **Auto-detection.** `auricle scan` finds connected, compatible earbuds; no Bluetooth address needed.
- **Four front ends.** A JSON-lines stream, a local HTTP API with a live dashboard, a head-driven mouse, and a bike game as an example app.
- **Zero dependencies.** One small binary built from the Rust standard library and direct Linux syscalls.

| Command | What you get |
|---|---|
| `auricle scan` | List connected earbuds that expose the sensor. |
| `auricle stream` | Every sensor report as a JSON line on stdout. |
| `auricle serve` | Local HTTP API, live dashboard, and the example app. |
| `auricle mouse` | A virtual mouse driven by head yaw and pitch (Linux `uinput`). |

**Contents:** [Quick start](#quick-start) · [Which devices work?](#which-devices-work) · [How is the sensor data captured?](#how-is-the-sensor-data-captured) · [What can you build with it?](#what-can-you-build-with-it) · [Context for agents](#context-for-agents-and-contributors) · [Troubleshooting](#troubleshooting) · [Limits and open questions](#limits-and-open-questions) · [Development](#development)

---

## Quick start

**Requirements:** Linux with BlueZ (developed and tested with BlueZ 5.87), a Rust toolchain, and Apple earbuds paired
and connected with your usual Bluetooth tool.

```sh
cargo build --release                     # one binary: target/release/auricle
./target/release/auricle scan             # list connected earbuds that expose the sensor
./target/release/auricle stream           # put the earbuds in and turn your head
./target/release/auricle serve            # dashboard at http://localhost:8000
```

`stream` prints one JSON object per report:

```json
{"t":4548,"o1":-24881,"o2":4192,"o3":4491,"ax":-4,"ay":1,"yaw":-0.841,"pitch":24.421}
```

`t` is milliseconds since start, `o1..o3`, `ax` and `ay` are the raw values, and `yaw` / `pitch` are derived angles in
degrees. Run `auricle --help` for all options.

## Which devices work?

| Device | Status |
|---|---|
| AirPods Pro (2nd generation) | **Tested** on one pair (it reports model `A2698`). |
| Other Apple earbuds with head tracking (AirPods 3, AirPods Pro 1, AirPods Max, some Beats) | Probably the same control channel; **untested**. |
| Sony, Samsung, Bose, Jabra and other brands | **Not supported.** Each uses its own proprietary protocol; a new backend would be needed. |

`auricle scan` checks a connected device for the vendor service UUID of Apple's control channel
(`74ec2172-0bad-4d01-8f77-997b2be0722a`), so it tells you whether a device is *potentially* usable. It does not
prove the motion sensor works. Supporting another vendor would mean writing a second producer of `Sample` values;
see [Context for agents](#context-for-agents-and-contributors) for where that plugs in.

## How is the sensor data captured?

The earbuds do not expose their motion sensor as a standard Bluetooth device (there is no GATT characteristic or HID
profile for it on the Linux side). Instead the sensor is one feature of the **control channel** that the earbuds
open to the device that owns them. `auricle` connects to that channel itself and speaks enough of the protocol to
request motion reports. This section walks through each step, from the socket to the angle.

```
 earbuds                                   Linux host
 ┌────────────┐  L2CAP, PSM 0x1001        ┌───────────────────────────────────────────────┐
 │ IMU        │  encrypted, SEQPACKET     │ aap.rs      connect, handshake, start, recv   │
 │ AAP server │ <───────────────────────> │   │  decode() -> Sample{o1,o2,o3,ax,ay,yaw,pitch}
 └────────────┘                           │   ▼                                           │
                                          │ Head (Arc<Mutex>)  ──►  stream | serve | mouse │
                                          └───────────────────────────────────────────────┘
```

**Figure 1:** The capture path. One reader thread owns the Bluetooth socket and publishes the latest sample;
every front end (stream, HTTP server, mouse) only reads it.

### Step 1. Open the control channel

The channel is an L2CAP connection on **PSM `0x1001`**. L2CAP is the packet layer under most Bluetooth profiles,
and Linux exposes it as a normal socket, so no Bluetooth library is needed:

- **Socket:** `socket(AF_BLUETOOTH, SOCK_SEQPACKET, BTPROTO_L2CAP)`. `SEQPACKET` preserves message boundaries, so one
  `recv` returns one protocol message.
- **Address:** a `sockaddr_l2` with the PSM (little-endian) and the device address. BlueZ stores the six address
  bytes **in reverse order**, so `AA:BB:CC:DD:EE:FF` becomes `[FF, EE, DD, CC, BB, AA]`.
- **Security:** `setsockopt(SOL_BLUETOOTH, BT_SECURITY, BT_SECURITY_MEDIUM)` *before* `connect`, so the link is
  encrypted. In our testing the earbuds answered the handshake either way, but we only saw motion data on encrypted
  connections. We did not isolate this setting from the other steps, so treat it as required in practice.
- **Timeouts:** `SO_SNDTIMEO` (10 s) bounds `connect`; `SO_RCVTIMEO` (250 ms) lets the loop wake up to re-send the
  start message.

The earbuds accept **one** control connection at a time and advertise a vendor service UUID
(`74ec2172-0bad-4d01-8f77-997b2be0722a`) for it. `auricle scan` recognises compatible devices by that UUID using
`bluetoothctl`; no address needs to be configured.

### Step 2. Handshake, then ask for motion

Messages are byte strings with a four-byte prefix `04 00 04 00` followed by a two-byte opcode (little-endian) and
a payload (the handshake is the exception: it starts `00 00 04 00`). `auricle` sends them in this order, 300 ms
apart, then waits 1.2 s:

| # | Message | Bytes (hex) | Purpose |
|---|---|---|---|
| 1 | Handshake | `00 00 04 00 01 00 02 00 00 00 00 00 00 00 00 00` | Opens the session; the earbuds answer with their state. |
| 2 | Feature flags | `04 00 04 00 4d 00 d7 00 00 00 00 00 00 00` | Tells the earbuds which optional features the host supports. |
| 3 | Notifications | `04 00 04 00 0f 00 ff ff ff ff` | Subscribes to status updates (battery, ear detection, ...). |
| 4 | Take over | `04 00 04 00 09 00 06 01 00 00 00` | Control command `0x06 = 1`: "I own this connection." |
| 5 | **Start tracking** | `04 00 04 00 17 00 00 00 10 00 ...` | Opcode `0x17`: begin streaming motion reports. |

Message 5 comes in two variants that differ in a few payload bytes (`0f 00 08 73 ...` and `10 00 08 a1 02 ...`).
Different firmware appears to answer different variants, so `auricle` **alternates both** and re-sends one every
2 seconds. The reports stop if the request is not repeated.

After the start request the earbuds answer with a few small acknowledgements, then large `0x17` messages
(about 468 and 356 bytes) that look like sensor and report descriptors (they contain strings such as
`ReportDescriptor` and `devmotion`), then the actual reports. `auricle` ignores the descriptors; decoding them
properly is an open question (see [Limits](#limits-and-open-questions)).

### Step 3. Recognise a motion report

A motion report is a message with opcode `0x17` and a length of roughly 80 bytes (we observed 80 and 81; the decoder
accepts 70-100). Fields are little-endian `int16`:

| Offset | Field | What we know |
|---|---|---|
| 0-3 | prefix | `04 00 04 00` |
| 4-5 | opcode | `17 00` |
| 43-44 | `o1` | Orientation component 1. Large magnitude (around -25000 in our captures); meaning not decoded. |
| 45-46 | `o2` | Orientation component 2. |
| 47-48 | `o3` | Orientation component 3. |
| 51-52 | `ax` | Acceleration, horizontal (per the community protocol notes). |
| 53-54 | `ay` | Acceleration, vertical. |

**Figure 2:** Byte layout of a motion report, as used by `decode()` in `src/aap.rs`.

The earbuds also send reports of the same size right after start-up in which `o1`, `o2` and `o3` are all zero. They
carry no orientation yet, so `decode()` drops them (otherwise the first one would become the centre pose and every
later angle would be offset).

### Step 4. From integers to degrees

The community notes derive head angles from `o2` and `o3`:

```
yaw   = (o2 - o3) / 2 / 32000 * 180      degrees
pitch = (o2 + o3) / 2 / 32000 * 180      degrees
```

So the full `int16` scale around zero corresponds to roughly +/-180 degrees (`32000` is an empirical constant, not
`32768`). These are **relative** angles: the first report becomes the centre pose, and `GET /recal` (or `R` in the
example game) makes the current pose the new centre. We measured about 23 reports per second.
`o1` and the acceleration values are exposed raw; `auricle` does not use them.

### Step 5. Stay connected

The reader thread (`aap::run`) loops forever: open, handshake, stream. On any error (the earbuds go into the case,
leave range, or drop the link) it marks the data as stale, waits 2 s and starts over. Consumers see this as
`"ok": false` in `/data`.

### What is measured and what is inferred

| Claim | Status |
|---|---|
| PSM `0x1001`, the handshake, start messages and the report offsets | From community reverse engineering ([LibrePods](https://github.com/kavishdevar/librepods)); confirmed to work on one pair of earbuds. |
| Reports arrive about 23 times per second while the head moves | Measured on one pair. |
| Reports stop when the head is still | Observed repeatedly; not explained. |
| Encryption is required for the stream | Observed, not isolated from the other steps. |
| `32000` maps to 180 degrees; yaw/pitch formulas | From community notes; **no ground truth** here (no independent angle measurement). |
| What `o1` means; how the descriptors define the report | Unknown. |

## What can you build with it?

### HTTP API (`serve`, localhost only)

| Route | Returns |
|---|---|
| `GET /` | Live dashboard (head-angle dial, raw values). |
| `GET /data` | `{"ok","ready","yaw","pitch","dev","pitch_dev","steer","o1","o2","o3","ax","ay"}`. `dev` and `pitch_dev` are degrees from the centre pose; `steer` is `dev` mapped to -1..1 by `full_turn` and `deadzone`. |
| `GET /recal` | Make the current head pose the centre. |
| `GET /examples/bike-game` | A browser game steered by head angle (hold the mouse for gas). |

The server has no authentication and binds to `127.0.0.1` only. Do not expose it to a network.

### Mouse mode

`auricle mouse` creates a virtual pointer: head turn moves the cursor sideways, nodding moves it up and down, and
looking straight ahead (inside a 3 degree dead zone) keeps it still. It needs write access to `/dev/uinput`: run it
as root, or add a udev rule once:

```sh
# /etc/udev/rules.d/99-uinput.rules
KERNEL=="uinput", GROUP="input", MODE="0660"
# then add yourself to the "input" group and log in again
```

### Configuration

Settings come from flags or a `config.toml`; flags win, then `config.toml`, then defaults. `auricle` looks for
`./config.toml`, then `~/.config/auricle/config.toml`. Copy `config.example.toml` to `config.toml` to keep your
earbuds' address out of your shell history and out of git (`config.toml` is gitignored):

```toml
mac = "AA:BB:CC:DD:EE:FF"   # optional: leave out to auto-detect
port = 8000
full_turn = 10.0            # degrees of head turn that map to a full axis value of 1.0
deadzone = 1.0              # ignore angles below this
invert = false
```

## Troubleshooting

| Symptom | Likely cause | What to try |
|---|---|---|
| Nothing prints from `stream` | The sensor reports only while the earbuds are worn and your head moves. | Wear both earbuds and turn your head. |
| Connects but stays silent | Another program holds the single control connection, or the earbuds are in a stuck state. | Close other apps that talk to the earbuds; disconnect and reconnect them (or put them in the case for 15 s). |
| `link problem, retrying` | Earbuds not connected or out of range. | Connect them in your Bluetooth settings; `auricle` retries every 2 s. |
| `cannot open /dev/uinput` | No permission. | See [Mouse mode](#mouse-mode). |
| Left and right are swapped | Axis direction. | Use `--invert` or `invert = true`. |

One more observation, not a proven cause: during testing the stream was most reliable when nothing else was
recording from the earbuds' microphone, which puts them into call mode.

## Limits and open questions

- **Apple earbuds only.** See [Which devices work?](#which-devices-work). One model tested.
- **Accuracy is unmeasured.** The angle formulas come from community notes and have not been validated against an independent reference. Treat degrees as approximate.
- **Roll is not decoded.** Only yaw and pitch are derived. Whether `o1` carries roll, and how the three values relate to a quaternion or Euler angles, is open.
- **Reports are event-driven.** A still head often produces no data, so consumers must treat "no data" as "unchanged", not "disconnected".
- **Reliability is not fully understood.** The stream worked and stopped working for periods during development without a confirmed cause.
- **Descriptors are ignored.** The earbuds describe their report format in the large `0x17` messages; reading that description instead of hard-coding offsets would make `auricle` robust to firmware changes.
- **Linux only.** It relies on BlueZ's L2CAP sockets and `uinput`.

## Development

```sh
cargo test        # 10 tests: decoding, config parsing, routing, mouse dead zone
cargo build --release
```

## Credits

The control-channel message formats come from the community reverse-engineering work of
[LibrePods](https://github.com/kavishdevar/librepods) (GPL-3.0). This project is an independent implementation.

## License

MIT, see [LICENSE](LICENSE).
