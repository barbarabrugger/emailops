"""A loopback-only, read-only HTTP adapter; all domain logic stays in Rust."""
import argparse
from dataclasses import dataclass
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hmac
import json
import os
from pathlib import Path
import subprocess

@dataclass(frozen=True)
class GatewayConfig:
    cli: str
    data_dir: str
    account: str
    manifest: str
    token: str

    def __post_init__(self):
        if len(self.token) < 32 or not self.token.isascii():
            raise ValueError("EMAILOPS_BRIDGE_TOKEN must contain at least 32 ASCII characters")

    def command(self):
        return [self.cli, "--json", "--quiet", "--data-dir", self.data_dir,
                "--account", self.account, "project", "--manifest", self.manifest]

def run_cli(args):
    # No shell, caller-controlled arguments, stdin, or inherited bearer token.
    env = dict(os.environ)
    env.pop("EMAILOPS_BRIDGE_TOKEN", None)
    result = subprocess.run(args, stdin=subprocess.DEVNULL, capture_output=True,
                            text=True, encoding="utf-8", timeout=30, env=env, check=False)
    if result.returncode != 0:
        raise RuntimeError("CLI failed")
    envelope = json.loads(result.stdout)
    if not isinstance(envelope, dict) or envelope.get("ok") is not True:
        raise RuntimeError("CLI did not return a success envelope")
    return envelope

class GatewayServer(ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, address, config, runner=run_cli):
        if address[0] != "127.0.0.1":
            raise ValueError("the prototype binds only to loopback")
        self.config = config
        self.run_cli = runner
        super().__init__(address, Handler)

class Handler(BaseHTTPRequestHandler):
    server_version = "EmailOpsPrototype"

    def setup(self):
        super().setup()
        self.connection.settimeout(35)

    def log_message(self, *args):
        # Do not put source content, bearer tokens or request paths in logs.
        pass

    def send_json(self, status, body):
        data = json.dumps(body, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(data)

    def fail(self, status, message):
        self.send_json(status, {"ok": False, "data": None, "error": {"message": message}})

    def do_GET(self):
        provided = self.headers.get("Authorization", "").encode("utf-8")
        expected = ("Bearer " + self.server.config.token).encode("ascii")
        if not hmac.compare_digest(provided, expected):
            return self.fail(401, "Unauthorized")
        if self.path != "/v1/project":
            return self.fail(404, "Unknown endpoint")
        try:
            envelope = self.server.run_cli(self.server.config.command())
            if not isinstance(envelope, dict) or envelope.get("ok") is not True:
                raise RuntimeError("CLI failed")
        except (RuntimeError, OSError, ValueError, subprocess.SubprocessError):
            return self.fail(502, "Project unavailable; check the CLI locally")
        self.send_json(200, envelope)

    def reject_write(self):
        self.fail(405, "Read-only endpoint")

    do_POST = reject_write
    do_PUT = reject_write
    do_PATCH = reject_write
    do_DELETE = reject_write

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--data-dir", required=True, type=Path)
    parser.add_argument("--account", required=True)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--port", type=int, default=8787)
    args = parser.parse_args()
    for path in [args.cli, args.data_dir / "emailops.db", args.manifest]:
        if not path.is_file():
            parser.error("CLI, database and manifest must exist")
    try:
        config = GatewayConfig(str(args.cli.resolve()), str(args.data_dir.resolve()), args.account,
                               str(args.manifest.resolve()), os.environ.get("EMAILOPS_BRIDGE_TOKEN", ""))
    except ValueError as error:
        parser.error(str(error))
    with GatewayServer(("127.0.0.1", args.port), config) as server:
        print("EmailOps project endpoint ready on loopback", flush=True)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass

if __name__ == "__main__":
    main()
