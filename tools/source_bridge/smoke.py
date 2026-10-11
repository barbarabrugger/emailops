"""Run the real Linux CLI -> gateway -> n8n chain using disposable synthetic state."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import socket
import subprocess
import tempfile
import threading
from urllib.request import Request, urlopen
from gateway import GatewayConfig, GatewayServer

N8N_IMAGE = "docker.n8n.io/n8nio/n8n:2.42.6"

def command(args, **kwargs):
    result = subprocess.run(args, capture_output=True, text=True, timeout=300, **kwargs)
    if result.returncode:
        raise RuntimeError(f"Command failed ({args[0]}): {result.stderr[-2000:]}")
    return result.stdout

def native(binary, directory, *args):
    output = command([str(binary), "--json", "--quiet", "--data-dir", str(directory), *args])
    envelope = json.loads(output)
    assert envelope["ok"], envelope
    return envelope["data"]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    args = parser.parse_args()
    binary = args.cli.resolve()
    here = Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="emailops-prototype-") as temporary:
        root = Path(temporary)
        data = root / "data"
        native(binary, data, "prototype-seed")
        manifest = data / "project-manifest.json"
        project = native(binary, data, "--account", "synthetic", "project", "--manifest", str(manifest))
        assert project["complete"] is True
        assert len(project["tasks"]) == 1 and len(project["events"]) == 1
        assert project["email_sources"][0]["completeness"]["original_source_complete"] is True
        before = hashlib.sha256((data / "emailops.db").read_bytes()).hexdigest()
        token = secrets.token_hex(32)
        config = GatewayConfig(str(binary), str(data), "synthetic", str(manifest), token)
        with GatewayServer(("127.0.0.1", 0), config) as gateway:
            worker = threading.Thread(target=gateway.serve_forever, daemon=True)
            worker.start()
            try:
                endpoint = f"http://127.0.0.1:{gateway.server_port}/v1/project"
                request = Request(endpoint, headers={"Authorization": "Bearer " + token})
                with urlopen(request, timeout=40) as response:
                    assert json.load(response)["data"] == project
                handoff = root / "input"
                state = root / "n8n"
                handoff.mkdir()
                state.mkdir()
                workflow = json.loads((here / "n8n-project.json").read_text())
                workflow["id"] = "emailopsPrototype"
                workflow["nodes"][1]["parameters"]["url"] = endpoint
                (handoff / "workflow.json").write_text(json.dumps(workflow))
                credentials = [{"id": "emailopsPrototypeHeader", "name": "EmailOps prototype bearer",
                    "type": "httpHeaderAuth", "data": {"name": "Authorization", "value": "Bearer " + token}}]
                (handoff / "credentials.json").write_text(json.dumps(credentials))
                env = dict(os.environ, N8N_ENCRYPTION_KEY=secrets.token_hex(32))
                with socket.socket() as probe:
                    probe.bind(("127.0.0.1", 0))
                    broker_port = probe.getsockname()[1]
                docker = ["docker", "run", "--rm", "--network", "host",
                    "--user", f"{os.getuid()}:{os.getgid()}",
                    "-v", f"{state}:/state", "-v", f"{handoff}:/input:ro",
                    "-e", "N8N_USER_FOLDER=/state", "-e", "HOME=/state",
                    "-e", "N8N_ENCRYPTION_KEY", "-e", "N8N_DIAGNOSTICS_ENABLED=false",
                    "-e", "N8N_VERSION_NOTIFICATIONS_ENABLED=false",
                    "-e", "N8N_TEMPLATES_ENABLED=false", "-e", "N8N_LOG_LEVEL=info",
                    "-e", f"N8N_RUNNERS_BROKER_PORT={broker_port}",
                    N8N_IMAGE]
                command(docker + ["import:credentials", "--input=/input/credentials.json"], env=env)
                command(docker + ["import:workflow", "--input=/input/workflow.json"], env=env)
                output = command(docker + ["execute", "--id=emailopsPrototype", "--rawOutput"], env=env)
                # Some n8n builds print startup notices before --rawOutput JSON.
                start = output.find('{')
                execution = json.loads(output[start:])
                result = execution["data"]["resultData"]
                assert not result.get("error"), result.get("error")
                received = result["runData"]["Read project dossier"][0]["data"]["main"][0][0]["json"]
                assert received["ok"] is True
                assert received["data"] == project, "n8n must receive the exact native dossier"
                assert hashlib.sha256((data / "emailops.db").read_bytes()).hexdigest() == before
                print("PASS: native CLI -> authenticated HTTP -> n8n; sources/tasks/events identical; DB unchanged")
            finally:
                gateway.shutdown()
                worker.join()

if __name__ == "__main__":
    main()
