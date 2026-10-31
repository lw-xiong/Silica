import json, os, socket, sys
SOCK = os.environ.get("SILICA_SOCK", "/run/user/1000/silica-demo.sock")
class Ctl:
    def __init__(self, sock=SOCK):
        self.s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); self.s.connect(sock)
        self.f = self.s.makefile("rw"); self.n = 0
    def call(self, method, **params):
        self.n += 1
        self.f.write(json.dumps({"id": self.n, "method": method, "params": params}) + "\n"); self.f.flush()
        resp = json.loads(self.f.readline())
        if resp.get("error"): raise RuntimeError(f"{method}: {resp['error']}")
        return resp.get("result")
    def idle(self, frames=2): return self.call("wait_idle", frames=frames, timeout_ms=3000)
if __name__ == "__main__":
    c = Ctl(); args = sys.argv[1:]
    method = args[0]; params = json.loads(args[1]) if len(args) > 1 else {}
    out = c.call(method, **params)
    if len(args) > 2:
        for k in args[2].split("."): out = out[k]
    print(json.dumps(out, indent=1, ensure_ascii=False))
