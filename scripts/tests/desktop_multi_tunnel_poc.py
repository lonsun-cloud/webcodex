#!/usr/bin/env python3
"""Real CLI/process/network PoC with fake control-plane credentials, never two real accounts.

python3 scripts/tests/desktop_multi_tunnel_poc.py --webcodex <built CLI>
The only replaced component is tunnel-client/the external control plane. Production
`webcodex server tunnel` owns the children, authorization files, and health observers.
"""
from __future__ import annotations
import argparse
import http.server
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

FIXTURE_TOKEN = "desktop-poc-local-fixture-only"


def client_fixture() -> None:
    args = sys.argv[2:]
    if args == ["--version"]:
        print("0.0.12 fixture")
        return
    assert os.environ.get("CONTROL_PLANE_API_KEY", "").startswith("fixture-only-")
    assert os.environ.get("CONTROL_PLANE_TUNNEL_ID", "").startswith("tunnel_")
    if args[0] in ("admin", "doctor"):
        return
    assert args[0] == "run"
    def option(name: str) -> str:
        return args[args.index(name) + 1]
    assert option("--health.listen-addr") == "127.0.0.1:0"
    target = option("--mcp.server-url").split(",channel=", 1)[0].removeprefix("url=")
    auth_file = Path(option("--mcp.extra-headers").split("file:", 1)[1])
    authorization = auth_file.read_text().strip()
    assert authorization == f"Bearer {FIXTURE_TOKEN}"
    request = urllib.request.Request(target, headers={"Authorization": authorization})
    assert json.load(urllib.request.urlopen(request, timeout=2))["endpoint"] == "/mcp"
    class Health(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            self.send_response(200 if self.path in ("/healthz", "/readyz") else 404)
            self.end_headers()
            self.wfile.write(b"{}")
        def log_message(self, *_):
            pass
    health = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Health)
    Path(option("--health.url-file")).write_text(f"http://127.0.0.1:{health.server_port}")
    Path(option("--log.file")).write_text(json.dumps({"fixture": True, "pid": os.getpid(), "target": target}))
    health.serve_forever(poll_interval=0.05)


class Tunnel:
    def __init__(self, binary: Path, env_file: Path, fixture: Path, index: int):
        env = os.environ.copy()
        for name in ("WEBCODEX_TOKEN", "OPENAI_API_KEY", "OPENAI_ADMIN_KEY", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
            env.pop(name, None)
        env.update(CONTROL_PLANE_TUNNEL_ID=f"tunnel_{index:032x}", CONTROL_PLANE_API_KEY=f"fixture-only-{index}", WEBCODEX_TUNNEL_CLIENT_BIN=str(fixture), WEBCODEX_TUNNEL_PROFILE_ID=f"fixture-{index}")
        self.process = subprocess.Popen([str(binary), "server", "tunnel", "--provider", "openai", "--env-file", str(env_file), "--json", "--stop-on-stdin-eof"], env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, start_new_session=True)
        self.events: queue.Queue = queue.Queue()
        self.lines: list[str] = []
        self.readers = [threading.Thread(target=self.drain, args=(pipe, events), daemon=True) for pipe, events in ((self.process.stdout, True), (self.process.stderr, False))]
        for reader in self.readers:
            reader.start()
        self.runtime = None

    def drain(self, pipe, events: bool) -> None:
        for line in pipe:
            if len(self.lines) < 200:
                self.lines.append(line)
            if events:
                try:
                    self.events.put(json.loads(line))
                except json.JSONDecodeError:
                    pass
        pipe.close()

    def wait(self, predicate, description: str, timeout: float = 15):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                event = self.events.get(timeout=min(0.2, max(0.001, deadline - time.monotonic())))
                if predicate(event):
                    return event
                assert event.get("event") not in ("error", "failed"), f"CLI failed while waiting for {description}; raw errors intentionally withheld"
            except queue.Empty:
                assert self.process.poll() is None, f"CLI exited before {description}"
        raise AssertionError(f"Deadline expired waiting for {description}")

    def ready(self):
        self.runtime = self.wait(lambda e: e.get("event") == "ready", "ready event")["runtime"]
        self.healthy(True)
        return self.runtime

    def healthy(self, local: bool):
        return self.wait(lambda e: e.get("event") == "health" and e.get("local_mcp_ready") is local and e.get("tunnel_ready") is True, f"local MCP health = {local}")

    def close(self):
        if self.process.stdin and not self.process.stdin.closed:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGKILL)
            self.process.wait(timeout=3)
            raise AssertionError("Tunnel failed graceful EOF cleanup")
        for reader in self.readers:
            reader.join(timeout=1)
        if self.runtime:
            assert not Path(self.runtime["directory"]).exists(), "Authorization/runtime directory survived EOF"
        assert FIXTURE_TOKEN not in "".join(self.lines)
        assert "fixture-only-" not in "".join(self.lines)


def run_poc(binary: Path) -> dict:
    available = threading.Event()
    available.set()
    requests = []
    class Mcp(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            authenticated = self.headers.get("Authorization") == f"Bearer {FIXTURE_TOKEN}"
            status = 200 if self.path == "/mcp" and authenticated and available.is_set() else 503
            requests.append((self.path, authenticated))
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(b'{"name":"webcodex","protocol":"mcp","endpoint":"/mcp"}')
        def log_message(self, *_):
            pass
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Mcp)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    tunnels: list[Tunnel] = []
    try:
        with tempfile.TemporaryDirectory(prefix="webcodex-multi-tunnel-poc-") as directory:
            root = Path(directory).resolve()
            os.chmod(root, 0o700)
            fixture = root / "tunnel-client"
            # Use the executing Python explicitly, not an interactive PATH dependency.
            fixture.write_text(f"#!{sys.executable}\nimport runpy,sys\nsys.argv=[{str(Path(__file__).resolve())!r},'--fixture-client']+sys.argv[1:]\nrunpy.run_path(sys.argv[0],run_name='__main__')\n")
            fixture.chmod(0o700)
            env_file = root / "webcodex.env"
            env_file.write_text(f"WEBCODEX_ADDR=127.0.0.1:{server.server_port}\nWEBCODEX_TOKEN={FIXTURE_TOKEN}\n")
            env_file.chmod(0o600)
            a = Tunnel(binary, env_file, fixture, 1); tunnels.append(a)
            b = Tunnel(binary, env_file, fixture, 2); tunnels.append(b)
            ar, br = a.ready(), b.ready()
            for field in ("directory", "health_url", "log_file", "tunnel_client_pid"):
                assert ar[field] != br[field], f"Collision in {field}"
            assert ar["local_mcp_url"] == br["local_mcp_url"] == f"http://127.0.0.1:{server.server_port}/mcp"
            for runtime in (ar, br):
                assert Path(runtime["log_file"]).is_file()
                assert Path(runtime["directory"]).stat().st_mode & 0o777 == 0o700
                authorization_files = [p for p in Path(runtime["directory"]).iterdir() if p.is_file() and FIXTURE_TOKEN in p.read_text()]
                assert len(authorization_files) == 1
                assert authorization_files[0].stat().st_mode & 0o777 == 0o600
            b_pid = b.process.pid
            a.close()
            assert b.process.poll() is None and b.process.pid == b_pid
            b.healthy(True)
            a2 = Tunnel(binary, env_file, fixture, 1); tunnels.append(a2)
            a2r = a2.ready()
            assert a2r["directory"] not in (ar["directory"], br["directory"])
            assert b.process.pid == b_pid and b.process.poll() is None
            # A shared upstream outage is observed independently, not a PID restart.
            available.clear()
            a2.healthy(False); b.healthy(False)
            available.set()
            a2.healthy(True); b.healthy(True)
            assert a2.process.poll() is None and b.process.poll() is None
            os.kill(br["tunnel_client_pid"], signal.SIGTERM)
            b.process.wait(timeout=8)
            assert b.process.returncode != 0
            a2.healthy(True)
            a2.close(); b.close()
            assert not list(root.glob("regular-tunnel-runtime/openai-*"))
            assert all(path == "/mcp" and authenticated for path, authenticated in requests)
            assert not list(root.rglob("*.db")), "A tunnel must not start or open a Server database"
            return {"fixture_only": True, "production_cli": str(binary), "simultaneous_profiles": 2, "local_mcp_endpoints": 1, "unique_health_ports": True, "unique_runtime_and_log_paths": True, "independent_stop_restart_failure": True, "upstream_outage_and_recovery_without_restart": True, "authorization_cleanup": True, "database_files_created": 0, "authenticated_mcp_probes": len(requests)}
    finally:
        for tunnel in tunnels:
            if tunnel.process.poll() is None:
                tunnel.close()
        server.shutdown(); server.server_close(); thread.join(timeout=2)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--fixture-client":
        client_fixture()
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--webcodex", required=True, type=Path)
        args = parser.parse_args()
        print(json.dumps(run_poc(args.webcodex.resolve()), indent=2))
