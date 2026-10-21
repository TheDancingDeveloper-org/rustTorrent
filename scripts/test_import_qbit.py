#!/usr/bin/env python3
"""Tests for scripts/import-qbit against a fake WebUI API v2 server.

Run: python3 scripts/test_import_qbit.py
"""
import contextlib
import hashlib
import http.server
import importlib.machinery
import importlib.util
import io
import json
import os
import tempfile
import threading
import unittest
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
_loader = importlib.machinery.SourceFileLoader("import_qbit", os.path.join(HERE, "import-qbit"))
_spec = importlib.util.spec_from_loader("import_qbit", _loader)
iq = importlib.util.module_from_spec(_spec)
_loader.exec_module(iq)


def benc(v):
    if isinstance(v, int):
        return b"i%de" % v
    if isinstance(v, str):
        v = v.encode()
    if isinstance(v, bytes):
        return b"%d:%s" % (len(v), v)
    if isinstance(v, list):
        return b"l" + b"".join(benc(x) for x in v) + b"e"
    if isinstance(v, dict):
        items = sorted((k.encode() if isinstance(k, str) else k, x) for k, x in v.items())
        return b"d" + b"".join(benc(k) + benc(x) for k, x in items) + b"e"
    raise TypeError(v)


def make_torrent(name):
    info = {"name": name, "piece length": 16384, "pieces": b"\0" * 20, "length": 1}
    return benc({"announce": "http://t/a", "info": info}), hashlib.sha1(benc(info)).hexdigest()


class FakeApi:
    def __init__(self, hashes=(), categories=(), password=None):
        self.hashes = set(hashes)
        self.categories = set(categories)
        self.password = password
        self.calls = []
        self.adds = []

    def serve(self):
        api = self

        class H(http.server.BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _send(self, code, body, headers=()):
                data = body if isinstance(body, bytes) else body.encode()
                self.send_response(code)
                for k, v in headers:
                    self.send_header(k, v)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def _authed(self):
                return api.password is None or "SID=good" in (self.headers.get("Cookie") or "")

            def do_GET(self):
                path = urllib.parse.urlparse(self.path).path
                api.calls.append(("GET", path))
                if not self._authed():
                    return self._send(403, "Forbidden")
                if path == "/api/v2/torrents/info":
                    return self._send(200, json.dumps([{"hash": h} for h in api.hashes]))
                if path == "/api/v2/torrents/categories":
                    return self._send(200, json.dumps(
                        {c: {"name": c, "savePath": ""} for c in api.categories}))
                self._send(404, "")

            def do_POST(self):
                path = self.path
                body = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                api.calls.append(("POST", path))
                if path == "/api/v2/auth/login":
                    form = urllib.parse.parse_qs(body.decode())
                    if form.get("password") == [api.password]:
                        return self._send(200, "Ok.", [("Set-Cookie", "SID=good; path=/")])
                    return self._send(200, "Fails.")
                if not self._authed():
                    return self._send(403, "Forbidden")
                if path == "/api/v2/torrents/createCategory":
                    api.categories.add(urllib.parse.parse_qs(body.decode())["category"][0])
                elif path == "/api/v2/torrents/add":
                    api.adds.append(self._multipart(body))
                elif path == "/api/v2/torrents/recheck":
                    api.calls[-1] = ("POST", path, urllib.parse.parse_qs(body.decode())["hashes"][0])
                else:
                    return self._send(404, "")
                self._send(200, "Ok.")

            def _multipart(self, body):
                boundary = self.headers["Content-Type"].split("boundary=")[1].encode()
                fields = {}
                for part in body.split(b"--" + boundary):
                    head, sep, val = part.partition(b"\r\n\r\n")
                    if not sep:
                        continue
                    name = head.split(b'name="')[1].split(b'"')[0].decode()
                    fields[name] = val[:-2]
                return fields

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        return f"http://127.0.0.1:{self.server.server_address[1]}"


class ImportTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.bt = self.tmp.name
        self.hashes = {}
        for name, cat, tags, save in [("alpha", "movies", ["hd"], "/qbtdownloads/movies"),
                                      ("beta", "tv", [], "/qbtdownloads/tv"),
                                      ("gamma", "", [], "/qbtdownloads")]:
            data, h = make_torrent(name)
            self.hashes[name] = h
            with open(os.path.join(self.bt, h + ".torrent"), "wb") as f:
                f.write(data)
            with open(os.path.join(self.bt, h + ".fastresume"), "wb") as f:
                f.write(benc({"qBt-savePath": save, "qBt-category": cat, "qBt-tags": tags,
                              "paused": 0, "qBt-ratioLimit": -2000}))
        with open(os.path.join(self.bt, "0" * 40 + ".fastresume"), "wb") as f:
            f.write(benc({"qBt-savePath": "/x"}))  # orphan: no .torrent

    def tearDown(self):
        self.tmp.cleanup()
        api = getattr(self, "api", None)
        if api:
            api.server.shutdown()
            api.server.server_close()

    def run_tool(self, *args, env=None):
        out, err = io.StringIO(), io.StringIO()
        old = dict(os.environ)
        os.environ.pop("RUSTTORRENT_USERNAME", None)
        os.environ.pop("RUSTTORRENT_PASSWORD", None)
        os.environ.update(env or {})
        try:
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = iq.main(list(args))
        finally:
            os.environ.clear()
            os.environ.update(old)
        return code, (json.loads(out.getvalue()) if out.getvalue() else None)

    def snapshot(self):
        snap = {}
        for f in os.listdir(self.bt):
            with open(os.path.join(self.bt, f), "rb") as fh:
                snap[f] = fh.read()
        return snap

    def test_dry_run_is_default_and_writes_nothing(self):
        self.api = FakeApi(hashes=[self.hashes["beta"]], categories=["tv"])
        url = self.api.serve()
        before = self.snapshot()
        code, rep = self.run_tool(self.bt, "--url", url)
        self.assertEqual(code, 0)
        self.assertEqual(rep["mode"], "dry-run")
        self.assertEqual(rep["status"], {"new": 2, "already-present": 1})
        self.assertEqual(rep["missing_categories"], ["movies"])
        self.assertEqual(rep["errors"], 1)
        self.assertFalse(any(c[0] == "POST" for c in self.api.calls))
        self.assertEqual(self.api.adds, [])
        self.assertEqual(self.snapshot(), before)

    def test_apply_adds_stopped_with_path_category_tags(self):
        self.api = FakeApi(hashes=[self.hashes["beta"]], categories=["tv"], password="pw")
        url = self.api.serve()
        code, rep = self.run_tool(self.bt, "--url", url, "--apply",
                                  env={"RUSTTORRENT_USERNAME": "admin",
                                       "RUSTTORRENT_PASSWORD": "pw"})
        self.assertEqual(code, 0)
        self.assertEqual(rep["result"]["categories_created"], ["movies"])
        self.assertIn("movies", self.api.categories)
        self.assertEqual(len(self.api.adds), 2)
        by_path = {a["savepath"]: a for a in self.api.adds}
        alpha = by_path[b"/qbtdownloads/movies"]
        self.assertEqual(alpha["stopped"], b"true")
        self.assertEqual(alpha["category"], b"movies")
        self.assertEqual(alpha["tags"], b"hd")
        self.assertEqual(hashlib.sha1(iq.info_slice(alpha["torrents"])).hexdigest(),
                         self.hashes["alpha"])
        gamma = by_path[b"/qbtdownloads"]
        self.assertNotIn("category", gamma)

    def test_rerun_is_idempotent(self):
        self.api = FakeApi()
        url = self.api.serve()
        self.run_tool(self.bt, "--url", url, "--apply")
        self.api.hashes.update(self.hashes.values())
        code, rep = self.run_tool(self.bt, "--url", url, "--apply")
        self.assertEqual(code, 0)
        self.assertEqual(rep["result"]["added"], [])
        self.assertEqual(len(self.api.adds), 3)

    def test_recheck_batches(self):
        self.api = FakeApi()
        url = self.api.serve()
        code, rep = self.run_tool(self.bt, "--url", url, "--apply", "--recheck",
                                  "--recheck-batch", "2", "--recheck-delay", "0")
        self.assertEqual(code, 0)
        rechecks = [c for c in self.api.calls if c[1] == "/api/v2/torrents/recheck"]
        self.assertEqual([len(c[2].split("|")) for c in rechecks], [2, 1])
        self.assertEqual(rep["result"]["rechecked"], 3)

    def test_credentials_file(self):
        self.api = FakeApi(password="s3cret")
        url = self.api.serve()
        cred = os.path.join(self.tmp.name, "creds")
        with open(cred, "w") as f:
            f.write("# comment\nusername=admin\npassword=s3cret\n")
        code, rep = self.run_tool(os.path.join(self.bt), "--url", url,
                                  "--credentials-file", cred)
        self.assertEqual(code, 0)
        self.assertEqual(rep["status"]["new"], 3)

    def test_bad_login_fails(self):
        self.api = FakeApi(password="right")
        url = self.api.serve()
        code, rep = self.run_tool(self.bt, "--url", url,
                                  env={"RUSTTORRENT_USERNAME": "a", "RUSTTORRENT_PASSWORD": "wrong"})
        self.assertEqual(code, 2)
        self.assertIsNone(rep)

    def test_offline(self):
        code, rep = self.run_tool(self.bt, "--offline", "--details")
        self.assertEqual(code, 0)
        self.assertEqual(rep["status"], {"new": 3})
        self.assertNotIn("torrent_path", rep["torrents"][0])

    def test_password_not_accepted_on_argv(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            iq.main([self.bt, "--password", "x"])


if __name__ == "__main__":
    unittest.main()
