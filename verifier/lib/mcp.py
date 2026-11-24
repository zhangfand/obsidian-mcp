"""Minimal MCP Streamable HTTP client (JSON-RPC over POST), stdlib only.

Records the size of every tool reply so rule 10 can measure all of them.
"""

import json
import urllib.error
import urllib.request

PROTOCOL = "2025-06-18"
LIMIT = 8192


class Reply:
    def __init__(self, tool, args, is_error, text, data, raw_error=None):
        self.tool, self.args = tool, args
        self.is_error = is_error
        self.text = text
        self.data = data            # parsed JSON object, or None
        self.raw_error = raw_error  # JSON-RPC level error, if any
        self.size = len(text.encode("utf-8")) if text is not None else 0

    @property
    def kind(self):
        return self.data.get("error") if self.is_error and isinstance(self.data, dict) else None

    def short(self):
        t = self.text if self.text is not None else json.dumps(self.raw_error)
        return (t or "")[:300]


class Client:
    def __init__(self, url, timeout=60):
        self.url = url
        self.timeout = timeout
        self.session = None
        self.next_id = 1
        self.replies = []   # every tool Reply, for rule 10
        self.oversize = []  # (tool, args, size)

    def _post(self, payload, expect_body=True):
        body = json.dumps(payload).encode("utf-8")
        req = urllib.request.Request(self.url, data=body, method="POST")
        req.add_header("Content-Type", "application/json")
        req.add_header("Accept", "application/json, text/event-stream")
        req.add_header("MCP-Protocol-Version", PROTOCOL)
        if self.session:
            req.add_header("Mcp-Session-Id", self.session)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                sid = resp.headers.get("Mcp-Session-Id")
                if sid:
                    self.session = sid
                ctype = resp.headers.get("Content-Type", "")
                raw = resp.read().decode("utf-8")
        except urllib.error.HTTPError as e:
            raw = e.read().decode("utf-8", "replace")
            if not expect_body:
                return None
            try:
                return json.loads(raw)
            except ValueError:
                return {"error": {"code": e.code, "message": raw[:500]}}
        if not expect_body or not raw.strip():
            return None
        if "text/event-stream" in ctype:
            msgs = []
            for line in raw.splitlines():
                if line.startswith("data:"):
                    try:
                        msgs.append(json.loads(line[5:].strip()))
                    except ValueError:
                        pass
            for m in msgs:
                if m.get("id") == payload.get("id"):
                    return m
            return msgs[-1] if msgs else None
        return json.loads(raw)

    def _id(self):
        self.next_id += 1
        return self.next_id

    def initialize(self):
        r = self._post({"jsonrpc": "2.0", "id": self._id(), "method": "initialize",
                        "params": {"protocolVersion": PROTOCOL, "capabilities": {},
                                   "clientInfo": {"name": "obsidian-mcp-verifier", "version": "1"}}})
        self._post({"jsonrpc": "2.0", "method": "notifications/initialized"}, expect_body=False)
        return r

    def call(self, tool, args):
        r = self._post({"jsonrpc": "2.0", "id": self._id(), "method": "tools/call",
                        "params": {"name": tool, "arguments": args}})
        if r is None or "result" not in r:
            rep = Reply(tool, args, True, None, None, raw_error=(r or {}).get("error", r))
        else:
            res = r["result"]
            content = res.get("content") or []
            text = "".join(c.get("text", "") for c in content if c.get("type") == "text")
            try:
                data = json.loads(text)
            except ValueError:
                data = None
            rep = Reply(tool, args, bool(res.get("isError")), text, data)
        self.replies.append(rep)
        if rep.size > LIMIT:
            self.oversize.append((tool, args, rep.size))
        return rep

    def pages(self, tool, args, max_pages=200):
        """Call and follow next_page. Returns (list of replies, error string or None)."""
        out = []
        page = None
        seen = set()
        for _ in range(max_pages):
            a = dict(args)
            if page is not None:
                a["page"] = page
            rep = self.call(tool, a)
            out.append(rep)
            if rep.is_error or not isinstance(rep.data, dict):
                return out, f"{tool} {json.dumps(args, ensure_ascii=False)} page {len(out)}: error {rep.short()}"
            page = rep.data.get("next_page")
            if page is None:
                return out, None
            if page in seen:
                return out, f"{tool}: next_page token repeats"
            seen.add(page)
        return out, f"{tool}: more than {max_pages} pages"
