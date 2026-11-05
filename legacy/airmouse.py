#!/usr/bin/env python3
"""AirPods head-tracking -> mouse cursor. Run: sudo python3 airmouse.py  (root needed for /dev/uinput)

Look straight ahead for the first second (calibration), then tilt/turn your head: the further
past the dead zone, the faster the cursor moves. Ctrl+C to quit.
"""
import socket, struct, sys, time
from evdev import UInput, ecodes as e

# Earbuds' Bluetooth address, first argument (find it with `bluetoothctl devices` or `auricle scan`).
MAC = sys.argv[1] if len(sys.argv) > 1 else sys.exit("usage: sudo python3 airmouse.py AA:BB:CC:DD:EE:FF")
GAIN = 1.5        # pixels per packet per degree past the dead zone
DEADZONE = 3.0    # degrees
SMOOTH = 0.4      # 0..1, higher = less smoothing
INVERT_X, INVERT_Y = 1, 1   # set to -1 if the cursor moves the wrong way
CALIB = 20        # packets averaged for the neutral pose

H = bytes.fromhex
INIT = ["00000400010002000000000000000000", "040004004d00d700000000000000", "040004000f00ffffffff"]
START = [H("0400040017000000100" "00F00087342" "0B08101002" "1A0501409C0000".replace(" ", "")),
         H("04000400170000001000100008A102420B080E10021A0501409C0000")]

s = socket.socket(socket.AF_BLUETOOTH, socket.SOCK_SEQPACKET, socket.BTPROTO_L2CAP)
s.setsockopt(274, 4, bytes([2, 0]))  # BT_SECURITY_MEDIUM: without encryption the buds never stream
s.settimeout(10)
s.connect((MAC, 0x1001))
s.settimeout(0.25)
for p in INIT:
    s.send(H(p)); time.sleep(0.3)

ui = UInput({e.EV_REL: [e.REL_X, e.REL_Y]}, name="head-mouse")
cal, neutral, fx, fy, nxt, i = [], None, 0.0, 0.0, 0.0, 0
rx = ry = 0.0  # sub-pixel remainder
print("Calibrating - look straight ahead...")
try:
    while True:
        if time.time() >= nxt:  # the buds stop streaming unless re-asked
            s.send(START[i % 2]); i += 1; nxt = time.time() + 2
        try:
            d = s.recv(4096)
        except socket.timeout:
            continue
        if d[4:6] != b"\x17\x00" or not 70 <= len(d) <= 100:
            continue
        _, o2, o3 = struct.unpack_from("<hhh", d, 43)
        pitch, yaw = (o2 + o3) / 2 / 32000 * 180, (o2 - o3) / 2 / 32000 * 180  # formula from LibrePods
        if neutral is None:
            cal.append((pitch, yaw))
            if len(cal) >= CALIB:
                neutral = tuple(sum(v) / len(cal) for v in zip(*cal))
                print("Ready - move your head.")
            continue
        dp, dy = pitch - neutral[0], yaw - neutral[1]
        def speed(v): return 0.0 if abs(v) < DEADZONE else (abs(v) - DEADZONE) * (1 if v > 0 else -1)
        fx += SMOOTH * (speed(dy) - fx); fy += SMOOTH * (speed(dp) - fy)
        rx += INVERT_X * fx * GAIN; ry += INVERT_Y * fy * GAIN
        mx, my = int(rx), int(ry); rx -= mx; ry -= my
        if mx: ui.write(e.EV_REL, e.REL_X, mx)
        if my: ui.write(e.EV_REL, e.REL_Y, my)
        ui.syn()
except KeyboardInterrupt:
    pass
finally:
    ui.close(); s.close()
