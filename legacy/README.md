# Python prototypes (superseded)

The first working versions of auricle, kept for reference. Use the Rust crate in the repository root instead:
it auto-detects the earbuds, reconnects by itself and has tests.

- `bikegame.py`: reads the sensor and serves `examples/bike-game.html` on http://localhost:8000
- `airmouse.py`: moves the cursor with your head (needs `python-evdev` and root for `/dev/uinput`)

```sh
python3 bikegame.py AA:BB:CC:DD:EE:FF
sudo python3 airmouse.py AA:BB:CC:DD:EE:FF
```
