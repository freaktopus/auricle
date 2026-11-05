# auricle

Read the motion-sensor data of Bluetooth earbuds and use it as a head-tracking input on Linux.

Many earbuds contain an IMU (accelerometer + gyroscope) that is normally used for spatial audio. `auricle`
opens the earbuds' control channel over Bluetooth L2CAP, asks them to stream orientation reports, decodes the
reports, and gives you the data in a few useful forms:

| Command | What you get |
|---|---|
| `auricle stream` | every sensor report as a JSON line on stdout (pipe it anywhere) |
| `auricle serve` | a local JSON API (`/data`), a live dashboard, and bundled example apps |
| `auricle mouse` | a virtual mouse: move the system cursor with your head |
| `auricle scan` | list connected earbuds that expose the sensor |

It has **no dependencies** (Rust standard library + direct Linux syscalls) and no Bluetooth library.

> Unofficial, reverse-engineered, and not affiliated with or endorsed by Apple. It speaks a proprietary
> protocol that may change with any firmware update. "AirPods" is a trademark of Apple Inc.

## Status

- **Platform:** Linux with BlueZ. Developed and tested with BlueZ 5.87.
- **Devices:** developed against AirPods Pro (2nd generation). Other models with the same control channel
  may work; they are untested. `auricle scan` recognises compatible devices by the control-channel service UUID.
- Experimental: reports only arrive while the earbuds are worn and your head moves, and the stream can need
  a retry after reconnecting (see [Troubleshooting](#troubleshooting)).

## Quick start

```sh
cargo build --release          # binary: target/release/auricle
# pair and connect the earbuds with your usual Bluetooth tool, wear them, then:
./target/release/auricle scan
./target/release/auricle stream     # move your head
./target/release/auricle serve      # open http://localhost:8000
```

Options: `--mac AA:BB:CC:DD:EE:FF` (default: auto-detect), `--port 8000`, `--full-turn 10`, `--deadzone 1`,
`--invert`. Run `auricle --help` for details.

### Configuration and private settings

Settings can come from command-line flags or a `config.toml`. Flags win, then `config.toml`, then defaults.
`auricle` looks for `./config.toml`, then `~/.config/auricle/config.toml`. Copy `config.example.toml` to
`config.toml` to keep your earbuds' Bluetooth address and other personal settings out of your shell history
and out of git (`config.toml` is gitignored):

```toml
mac = "AA:BB:CC:DD:EE:FF"   # optional: leave out to auto-detect
port = 8000
full_turn = 10.0
deadzone = 1.0
invert = false
```

### Data format

`stream` prints one JSON object per report:

```json
{"t":4548,"o1":-24881,"o2":4192,"o3":4491,"ax":-4,"ay":1,"yaw":-0.841,"pitch":24.421}
```

`t` is milliseconds since start; `o1..o3`, `ax`, `ay` are the raw signed 16-bit values; `yaw` and `pitch` are
derived angles in degrees (see [How it works](#how-it-works)).

### HTTP API (`serve`, localhost only)

| Route | Returns |
|---|---|
| `GET /` | live dashboard |
| `GET /data` | `{"ok","ready","yaw","pitch","dev","pitch_dev","steer","o1","o2","o3","ax","ay"}`; `dev`/`pitch_dev` are degrees from the centre pose, `steer` is `dev` mapped to -1..1 by `--full-turn`/`--deadzone` |
| `GET /recal` | make the current head pose the centre |
| `GET /examples/bike-game` | a small example app that steers with `/data` |

The server has no authentication and listens on `127.0.0.1` only. Do not expose it to a network.

### Mouse mode

`auricle mouse` creates a virtual pointer through `uinput`: head turn moves the cursor sideways, nodding moves
it up and down; look straight ahead (inside a 3 degree dead zone) and it stays still. It needs write access
to `/dev/uinput`. Either run it as root, or give your user access once:

```sh
# /etc/udev/rules.d/99-uinput.rules
KERNEL=="uinput", GROUP="input", MODE="0660"
# then: add yourself to the "input" group and re-login
```

## How it works

1. Open an L2CAP `SOCK_SEQPACKET` socket to the earbuds on PSM `0x1001` with `BT_SECURITY_MEDIUM`.
   Encryption is required: without it the earbuds answer the handshake but never send motion data.
2. Send the handshake, feature-flags and notification-request messages, ask to own the connection, then send
   the "start tracking" message (re-sent every 2 seconds; two firmware variants are alternated).
3. Motion reports are messages with opcode `0x17` and 70-100 bytes. Three little-endian `int16` orientation
   values sit at byte offsets 43, 45 and 47, and acceleration at 51 and 53.
4. `yaw = (o2 - o3) / 2 / 32000 * 180` and `pitch = (o2 + o3) / 2 / 32000 * 180`, in degrees.
   These are relative angles: the first report (or `/recal`) defines the centre.

## Troubleshooting

- **No data at all:** the sensor only reports while the earbuds are in your ears *and* your head moves. Turn
  your head once or twice.
- **Connects but stays silent:** another program may be holding the control channel (the earbuds allow one
  connection), or an app is recording from the earbuds' microphone, which switches them into call mode.
  Close it and reconnect the earbuds.
- **`link problem, retrying`:** the earbuds are not connected, or are out of range. `auricle` reconnects by itself.
- **`cannot open /dev/uinput`:** see Mouse mode above.
- **Direction is backwards:** use `--invert`.

## Development

```sh
cargo test
```

## Credits

The control-channel message formats come from the community reverse-engineering work of
[LibrePods](https://github.com/kavishdevar/librepods) (GPL-3.0).

## License

MIT, see [LICENSE](LICENSE).
