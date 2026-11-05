#!/usr/bin/env python3
"""Head-steered bike game. Run: python3 bikegame.py  then open http://localhost:8000
Turn your head left/right to steer continuously. No sudo needed. Press R in the game to re-centre."""
import json, socket, struct, sys, threading, time, os
from http.server import BaseHTTPRequestHandler, HTTPServer

# Earbuds' Bluetooth address, first argument (find it with `bluetoothctl devices` or `auricle scan`).
MAC = sys.argv[1] if len(sys.argv) > 1 else sys.exit("usage: python3 bikegame.py AA:BB:CC:DD:EE:FF")
FULL_TURN = 25.0   # degrees of head turn for full steering
DEADZONE = 2.0
INVERT = 1         # set to -1 if steering is reversed
H = bytes.fromhex
INIT = ["00000400010002000000000000000000", "040004004d00d700000000000000", "040004000f00ffffffff", "0400040009000601000000"]  # last = take ownership of the buds
START = [H("04000400170000001000" "0F00087342" "0B08101002" "1A0501409C0000"),
         H("04000400170000001000100008A102420B080E10021A0501409C0000")]
state = {"steer": 0.0, "yaw": 0.0, "neutral": None, "cal": [], "ok": False, "dev": 0.0}


def reader():
    while True:
        try:
            s = socket.socket(socket.AF_BLUETOOTH, socket.SOCK_SEQPACKET, socket.BTPROTO_L2CAP)
            s.setsockopt(274, 4, bytes([2, 0]))  # BT_SECURITY_MEDIUM: required or the buds never stream
            s.settimeout(10); s.connect((MAC, 0x1001)); s.settimeout(0.25)
            for p in INIT:
                s.send(H(p)); time.sleep(0.3)
            time.sleep(1.2)
            nxt, i = 0, 0
            while True:
                if time.time() >= nxt:
                    s.send(START[i % 2]); i += 1; nxt = time.time() + 2
                try:
                    d = s.recv(4096)
                except socket.timeout:
                    continue
                if d[4:6] != b"\x17\x00" or not 70 <= len(d) <= 100:
                    continue
                _, o2, o3 = struct.unpack_from("<hhh", d, 43)
                yaw = (o2 - o3) / 2 / 32000 * 180
                state["yaw"], state["ok"] = yaw, True
                if state["neutral"] is None:  # first packet = centre; press R in the game to re-centre
                    state["neutral"] = yaw
                dev = yaw - state["neutral"]
                state["dev"] = dev  # raw degrees, shown in the page
                dev = 0.0 if abs(dev) < DEADZONE else dev - DEADZONE * (1 if dev > 0 else -1)
                state["steer"] = max(-1.0, min(1.0, INVERT * dev / (FULL_TURN - DEADZONE)))
        except OSError as e:
            state["ok"] = False; state["steer"] = 0.0
            print("bud link problem, retrying:", e); time.sleep(2)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a): pass

    def do_GET(self):
        if self.path in ("/steer", "/data"):
            body, ctype = json.dumps({"steer": state["steer"], "dev": state["dev"], "ok": state["ok"],
                                      "ready": state["neutral"] is not None}).encode(), "application/json"
        elif self.path == "/recal":
            state["neutral"] = state["yaw"] if state["ok"] else None
            body, ctype = b"{}", "application/json"
        else:
            body, ctype = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "examples", "bike-game.html"), "rb").read(), "text/html"
        self.send_response(200); self.send_header("Content-Type", ctype)
        self.send_header("Cache-Control", "no-store"); self.end_headers(); self.wfile.write(body)


threading.Thread(target=reader, daemon=True).start()
print("open http://localhost:8000")
HTTPServer(("127.0.0.1", 8000), Handler).serve_forever()
