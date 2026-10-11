"""Synthetic contract tests for the local, read-only CLI gateway."""
import json
import threading
import unittest
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from gateway import GatewayServer, GatewayConfig

TOKEN = "synthetic-test-token-" * 3

class GatewayTests(unittest.TestCase):
    def setUp(self):
        self.calls = []
        def run(args):
            self.calls.append(args)
            return {"ok": True, "data": {"title": "Room Renovation"}, "error": None}
        self.server = GatewayServer(("127.0.0.1", 0),
            GatewayConfig("emailops-cli", "/synthetic", "synthetic", "/synthetic/project.json", TOKEN), run)
        self.worker = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.worker.start()
        self.base = "http://127.0.0.1:" + str(self.server.server_port)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.worker.join()

    def request(self, path, token=TOKEN, method="GET"):
        req = Request(self.base + path, headers={"Authorization": "Bearer " + token}, method=method)
        try:
            response = urlopen(req, timeout=3)
        except HTTPError as error:
            response = error
        with response:
            return response.status, json.loads(response.read())

    def test_unauthenticated_request_never_invokes_cli(self):
        self.assertEqual(self.request("/v1/project", "wrong")[0], 401)
        self.assertEqual(self.calls, [])

    def test_project_is_a_fixed_argument_array_and_no_client_path_is_accepted(self):
        status, body = self.request("/v1/project")
        self.assertEqual(status, 200)
        self.assertTrue(body["ok"])
        self.assertEqual(self.calls, [["emailops-cli", "--json", "--quiet", "--data-dir",
            "/synthetic", "--account", "synthetic", "project", "--manifest", "/synthetic/project.json"]])

    def test_query_cannot_override_dataset(self):
        self.assertEqual(self.request("/v1/project?manifest=/elsewhere")[0], 404)
        self.assertEqual(self.calls, [])

    def test_write_methods_are_rejected_without_invoking_cli(self):
        self.assertEqual(self.request("/v1/project", method="POST")[0], 405)
        self.assertEqual(self.calls, [])

    def test_failed_cli_is_a_failure_not_partial_success(self):
        self.server.run_cli = lambda args: {"ok": False, "data": None, "error": {"message": "missing source"}}
        self.assertEqual(self.request("/v1/project")[0], 502)

    def test_exception_does_not_leak_process_details(self):
        def fail(args):
            raise RuntimeError("private command output")
        self.server.run_cli = fail
        status, body = self.request("/v1/project")
        self.assertEqual(status, 502)
        self.assertNotIn("private", json.dumps(body))

    def test_weak_token_is_rejected(self):
        with self.assertRaises(ValueError):
            GatewayConfig("cli", "data", "account", "project", "short")

if __name__ == "__main__":
    unittest.main()
