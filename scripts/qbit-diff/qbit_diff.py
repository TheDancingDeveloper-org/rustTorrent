#!/usr/bin/env python3
"""Differential WebUI API v2 harness: real qBittorrent vs rustTorrent.

Replays the client fixtures in crates/librtbit/src/http_api/handlers/qbit_replay/
against both servers, in lockstep, and diffs the responses field by field:

- status codes must match;
- every JSON field the reference returns must exist on rustTorrent with the
  same JSON type, unless the parity spec lists it under that endpoint's
  `missing_fields` (the allowlist of intentionally unimplemented fields);
- fields named in a step's `diff.values` must have equal values.

A step's `diff.body` can relax this: `ignore` (status only), `shape` (keys and
types only), `sorted` (compare a JSON array as a sorted list). Each step's
`expect` block is also checked on both servers; a failure on the reference means
the fixture itself is wrong.

Standard library only. Exit status is 1 when any unallowlisted difference or
expectation failure is found (0 with --report-only).

    scripts/qbit-diff/qbit_diff.py \\
        --reference http://localhost:18080 --candidate http://localhost:13030 \\
        --report qbit-diff-report.md
"""

from __future__ import annotations

import argparse
import http.cookiejar
import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
HANDLERS = REPO / "crates/librtbit/src/http_api/handlers"
FIXTURE_DIR = HANDLERS / "qbit_replay"
SPEC = HANDLERS / "qbit_parity_spec.json"


@dataclass
class Target:
    label: str
    base: str
    username: str
    password: str
    save_path: str
    opener: urllib.request.OpenerDirector = field(init=False)

    def __post_init__(self) -> None:
        self.base = self.base.rstrip("/")
        self.opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())
        )

    def subst(self, value: object) -> str:
        s = value if isinstance(value, str) else json.dumps(value)
        return (
            s.replace("{{save_path}}", self.save_path)
            .replace("{{username}}", self.username)
            .replace("{{password}}", self.password)
        )


@dataclass
class Response:
    status: int
    body: bytes

    def json(self):
        try:
            return json.loads(self.body)
        except ValueError:
            return None


def multipart(fields: dict[str, str], torrent: bytes) -> tuple[bytes, str]:
    boundary = "qbitDiffBoundary"
    out = b""
    for k, v in fields.items():
        out += (
            f'--{boundary}\r\ncontent-disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'
        ).encode()
    out += (
        f'--{boundary}\r\ncontent-disposition: form-data; name="torrents"; '
        f'filename="t.torrent"\r\ncontent-type: application/x-bittorrent\r\n\r\n'
    ).encode()
    out += torrent + f"\r\n--{boundary}--\r\n".encode()
    return out, f"multipart/form-data; boundary={boundary}"


def send(target: Target, step: dict, torrent: bytes) -> Response:
    url = target.base + "/api/v2" + step["path"]
    query = {k: target.subst(v) for k, v in step.get("query", {}).items()}
    if query:
        url += "?" + urllib.parse.urlencode(query)
    data = None
    headers = {"Referer": target.base}
    if "multipart" in step:
        fields = {k: target.subst(v) for k, v in step["multipart"].get("fields", {}).items()}
        data, headers["Content-Type"] = multipart(fields, torrent)
    elif step["method"] == "POST":
        form = {k: target.subst(v) for k, v in step.get("form", {}).items()}
        data = urllib.parse.urlencode(form).encode()
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    request = urllib.request.Request(url, data=data, headers=headers, method=step["method"])
    try:
        with target.opener.open(request, timeout=30) as r:
            return Response(r.status, r.read())
    except urllib.error.HTTPError as e:
        return Response(e.code, e.read())


def lookup(root, path: str):
    """Dotted path; returns (found, value)."""
    if path == "":
        return True, root
    v = root
    for seg in path.split("."):
        if isinstance(v, list):
            if not seg.isdigit() or int(seg) >= len(v):
                return False, None
            v = v[int(seg)]
        elif isinstance(v, dict):
            if seg not in v:
                return False, None
            v = v[seg]
        else:
            return False, None
    return True, v


def json_eq(a, b) -> bool:
    if isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool):
        return abs(a - b) < 1e-9
    return a == b


def check_expect(step: dict, resp: Response) -> list[str]:
    exp = step["expect"]
    errors = []
    if "status" in exp and resp.status != exp["status"]:
        errors.append(f"status {resp.status} (want {exp['status']})")
    if "status_in" in exp and resp.status not in exp["status_in"]:
        errors.append(f"status {resp.status} (want one of {exp['status_in']})")
    if "text" in exp and resp.body.decode(errors="replace") != exp["text"]:
        errors.append(f"body {resp.body[:200]!r} (want {exp['text']!r})")
    if exp.get("json"):
        body = resp.json()
        if body is None:
            return errors + [f"body is not JSON: {resp.body[:200]!r}"]
        for c in exp["json"]:
            found, v = lookup(body, c["path"])
            p = c["path"]
            if "exists" in c and found != c["exists"]:
                errors.append(f"`{p}` exists={found} (want {c['exists']})")
                continue
            if not found and any(k in c for k in ("eq", "one_of", "len", "contains")):
                errors.append(f"`{p}` is absent")
                continue
            if "eq" in c and not json_eq(v, c["eq"]):
                errors.append(f"`{p}` = {v!r} (want {c['eq']!r})")
            if "one_of" in c and not any(json_eq(v, o) for o in c["one_of"]):
                errors.append(f"`{p}` = {v!r} (want one of {c['one_of']!r})")
            if "len" in c and (not hasattr(v, "__len__") or len(v) != c["len"]):
                errors.append(f"`{p}` = {v!r} (want length {c['len']})")
            if "contains" in c and not (
                (isinstance(v, list) and any(json_eq(x, c["contains"]) for x in v))
                or (isinstance(v, str) and isinstance(c["contains"], str) and c["contains"] in v)
            ):
                errors.append(f"`{p}` = {v!r} does not contain {c['contains']!r}")
    return errors


def json_type(v) -> str:
    if v is None:
        return "null"
    if isinstance(v, bool):
        return "boolean"
    if isinstance(v, (int, float)):
        return "number"
    return {str: "string", list: "array", dict: "object"}[type(v)]


def generalize(path: str) -> str:
    """`0.files.3.name` -> `*.files.*.name` to match spec `missing_fields`."""
    return ".".join("*" if seg.isdigit() else seg for seg in path.split(".") if seg != "")


def missing_allowlist(spec: dict) -> dict[str, set[str]]:
    out: dict[str, set[str]] = {}
    for ep in spec["endpoints"]:
        fields = (ep.get("response") or {}).get("missing_fields", [])
        out.setdefault(ep["endpoint"], set()).update(fields)
    return out


def allowed(path: str, allow: set[str]) -> bool:
    g = generalize(path)
    # Map keys (torrent hashes, category names) are `*` in the spec.
    candidates = {g}
    parts = g.split(".")
    for i in range(len(parts)):
        candidates.add(".".join(parts[:i] + ["*"] + parts[i + 1 :]))
    return any(c in allow or any(c.startswith(a + ".") for a in allow) for c in candidates)


def shape_diff(ref, cand, path: str, allow: set[str], out: list[str], notes: list[str]) -> None:
    if json_type(ref) != json_type(cand):
        # A JSON null on either side is a stub or an unset value; report it as a
        # note rather than a structural break.
        if ref is None or cand is None:
            notes.append(f"`{path or '<root>'}` null vs {json_type(cand if ref is None else ref)}")
        elif not allowed(path, allow):
            out.append(f"`{path or '<root>'}` type {json_type(cand)} (reference {json_type(ref)})")
        return
    if isinstance(ref, dict):
        for k, v in ref.items():
            sub = f"{path}.{k}" if path else k
            if k not in cand:
                (notes if allowed(sub, allow) else out).append(f"`{sub}` missing")
            else:
                shape_diff(v, cand[k], sub, allow, out, notes)
        extra = sorted(set(cand) - set(ref))
        if extra and len(extra) <= 20:
            notes.append(f"`{path or '<root>'}` extra fields: {', '.join(extra)}")
    elif isinstance(ref, list) and ref and cand:
        # Element shape: compare the first elements.
        shape_diff(ref[0], cand[0], f"{path}.0" if path else "0", allow, out, notes)


def diff_step(step: dict, ref: Response, cand: Response, allow: set[str]) -> tuple[list[str], list[str]]:
    out: list[str] = []
    notes: list[str] = []
    if ref.status != cand.status:
        out.append(f"status {cand.status} (reference {ref.status})")
        return out, notes
    cfg = step.get("diff") or {}
    mode = cfg.get("body", "shape")
    if mode == "ignore":
        return out, notes
    rj, cj = ref.json(), cand.json()
    if rj is None or cj is None:
        if rj is None and cj is None:
            bodies = {ref.body.strip(), cand.body.strip()}
            if bodies == {b"", b"Ok."}:
                # Upstream dropped the legacy `Ok.` success body; clients ignore it.
                notes.append(f"body {cand.body!r} (reference {ref.body!r})")
            elif ref.body.strip() != cand.body.strip():
                out.append(f"body {cand.body[:120]!r} (reference {ref.body[:120]!r})")
        else:
            out.append("one side returned JSON, the other did not")
        return out, notes
    if mode == "sorted":
        if sorted(map(json.dumps, rj)) != sorted(map(json.dumps, cj)):
            out.append(f"sorted body {cj!r} (reference {rj!r})")
        return out, notes
    shape_diff(rj, cj, "", allow, out, notes)
    for path in cfg.get("values", []):
        rf, rv = lookup(rj, path)
        cf, cv = lookup(cj, path)
        if rf != cf or (rf and not json_eq(rv, cv)):
            out.append(f"`{path or '<root>'}` = {cv!r} (reference {rv!r})")
    return out, notes


def run_step(target: Target, step: dict, torrent: bytes, deadline_s: float) -> Response:
    end = time.monotonic() + (deadline_s if step["expect"].get("eventually") else 0)
    while True:
        resp = send(target, step, torrent)
        if not check_expect(step, resp) or time.monotonic() >= end:
            return resp
        time.sleep(0.25)


def wait_ready(target: Target, timeout: float) -> None:
    end = time.monotonic() + timeout
    while True:
        try:
            with urllib.request.urlopen(target.base + "/api/v2/app/webapiVersion", timeout=5) as r:
                if r.status in (200, 403):
                    return
        except urllib.error.HTTPError as e:
            if e.code in (401, 403):
                return
        except OSError:
            pass
        if time.monotonic() >= end:
            sys.exit(f"{target.label} at {target.base} did not come up within {timeout:.0f}s")
        time.sleep(1)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--reference", required=True, help="real qBittorrent WebUI base URL")
    ap.add_argument("--candidate", required=True, help="rustTorrent HTTP API base URL")
    ap.add_argument("--reference-save-path", default="/downloads")
    ap.add_argument("--candidate-save-path", default="/home/rtbit/downloads")
    ap.add_argument("--username", default="admin")
    ap.add_argument("--password", default="adminadmin")
    ap.add_argument("--fixture", action="append", help="fixture name (default: all)")
    ap.add_argument("--report", type=Path, help="write a Markdown report here")
    ap.add_argument("--report-only", action="store_true", help="exit 0 even when differences are found")
    ap.add_argument("--wait", type=float, default=120, help="seconds to wait for both servers")
    ap.add_argument("--eventually", type=float, default=20, help="retry window for `eventually` steps")
    args = ap.parse_args()

    ref = Target("reference", args.reference, args.username, args.password, args.reference_save_path)
    cand = Target("candidate", args.candidate, args.username, args.password, args.candidate_save_path)
    for t in (ref, cand):
        wait_ready(t, args.wait)

    allow = missing_allowlist(json.loads(SPEC.read_text()))
    torrent = (FIXTURE_DIR / "fixture.torrent").read_bytes()
    names = args.fixture or sorted(p.stem for p in FIXTURE_DIR.glob("*.json"))

    lines = ["# WebUI API v2 differential report", ""]
    failures = 0
    for name in names:
        fixture = json.loads((FIXTURE_DIR / f"{name}.json").read_text())
        lines += [f"## {name} — {fixture['client']}", ""]
        for i, step in enumerate(fixture["steps"]):
            r = run_step(ref, step, torrent, args.eventually)
            c = run_step(cand, step, torrent, args.eventually)
            endpoint = step["path"].lstrip("/")
            diffs, notes = diff_step(step, r, c, allow.get(endpoint, set()))
            ref_exp = check_expect(step, r)
            cand_exp = check_expect(step, c)
            gap = step.get("known_gap")
            if gap and c.status == gap["status"]:
                notes.insert(0, f"known gap: {gap['note']}")
                cand_exp = [e for e in cand_exp if not e.startswith("status ")]
                diffs = [d for d in diffs if not d.startswith("status ")]
            problems = (
                [f"reference fails fixture expectation: {e}" for e in ref_exp]
                + [f"candidate fails fixture expectation: {e}" for e in cand_exp]
                + diffs
            )
            mark = "FAIL" if problems else "ok"
            failures += bool(problems)
            lines.append(f"- **{mark}** step {i} `{step['method']} {step['path']}` — {step['name']}")
            lines += [f"  - {p}" for p in problems]
            lines += [f"  - _note:_ {n}" for n in notes]
            print(f"[{mark:4}] {name}/{i} {step['method']} {step['path']} ({step['name']})")
            for p in problems:
                print(f"         {p}")
        lines.append("")
    lines.insert(2, f"{failures} step(s) differ.\n")
    report = "\n".join(lines)
    if args.report:
        args.report.write_text(report)
    print(f"\n{failures} step(s) differ.")
    return 0 if failures == 0 or args.report_only else 1


if __name__ == "__main__":
    sys.exit(main())
