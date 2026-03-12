"""
spikes/tools/debug_ui.py — Retro debugger-style Tk UI for spike runs.

Architecture:
- HTTP server runs in a daemon thread (serves session/test data as JSON)
- Tk client runs in a SEPARATE daemon thread so it never blocks the spike
- Spike runs in the main thread, calls publish/test_started/test_finished
- Tk polls the HTTP server every 500ms for live updates

Usage:
    from spikes.tools import debug_ui
    debug_ui.start(port=8765)                          # starts HTTP + Tk
    sid = debug_ui.register_spike("cognee_spike")
    debug_ui.test_started(sid, "A5")
    debug_ui.publish_raw(sid, "A5", {"result": ...})
    debug_ui.test_finished(sid, "A5", passed=True, detail="ok", elapsed_ms=123)
"""

from __future__ import annotations

import http.server
import json
import os
import socketserver
import threading
import time
import urllib.parse
import urllib.request
import uuid
from typing import Any, Callable, Dict, Optional

_ENABLED = os.environ.get("SPIKE_UI_ENABLED", "1").lower() not in ("0", "false", "no")
_DEFAULT_HOST = "127.0.0.1"
_DEFAULT_PORT = 8765

_server: Optional[Any] = None
_server_lock = threading.RLock()
_sessions: Dict[str, Dict] = {}
_sessions_lock = threading.RLock()
_rerun_handlers: Dict[str, Dict[str, Callable]] = {}
_rerun_lock = threading.RLock()
_tk_thread: Optional[threading.Thread] = None


# ── Colours (Z80 debugger palette) ────────────────────────────────────────────
C = {
    "bg":       "#0a0e14",
    "bg2":      "#0f1520",
    "border":   "#1e3a5f",
    "header":   "#1a2a3a",
    "text":     "#c8d8e8",
    "dim":      "#4a6a8a",
    "green":    "#00ff88",
    "red":      "#ff3333",
    "yellow":   "#ffcc00",
    "cyan":     "#00ccff",
    "orange":   "#ff8800",
    "running":  "#00ccff",
    "pass":     "#00ff88",
    "fail":     "#ff3333",
    "pending":  "#4a6a8a",
    "mono":     "Courier New",
}


# ── HTTP server ───────────────────────────────────────────────────────────────

def _now_ms() -> int:
    return int(time.time() * 1000)


def _sanitize(obj: Any, depth: int = 4) -> Any:
    if depth <= 0:
        return "<…>"
    if isinstance(obj, dict):
        return {k: _sanitize(v, depth - 1) for k, v in obj.items()}
    if isinstance(obj, list):
        return [_sanitize(i, depth - 1) for i in obj[:50]]
    if isinstance(obj, (str, int, float, bool)) or obj is None:
        return obj
    try:
        return repr(obj)
    except Exception:
        return "<unserializable>"


def _json_resp(handler: http.server.BaseHTTPRequestHandler, obj: Any, status: int = 200) -> None:
    body = json.dumps(obj, default=str, ensure_ascii=False).encode()
    handler.send_response(status)
    handler.send_header("Content-Type", "application/json; charset=utf-8")
    handler.send_header("Access-Control-Allow-Origin", "*")
    handler.send_header("Content-Length", str(len(body)))
    handler.end_headers()
    handler.wfile.write(body)


class _Handler(http.server.BaseHTTPRequestHandler):
    def do_OPTIONS(self):
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET,POST,OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type")
        self.end_headers()

    def do_GET(self):
        p = urllib.parse.urlparse(self.path).path
        if p == "/api/sessions":
            with _sessions_lock:
                data = [{"session_id": sid, "name": s["name"], "tests": list(s["tests"].keys()), "created_ms": s["created_ms"]} for sid, s in _sessions.items()]
            _json_resp(self, {"sessions": data})
        elif p.startswith("/api/session/"):
            parts = p.strip("/").split("/")
            sid = parts[2] if len(parts) > 2 else None
            with _sessions_lock:
                s = _sessions.get(sid)
            if not s:
                _json_resp(self, {"error": "not found"}, 404)
            elif len(parts) >= 5 and parts[3] == "test":
                tname = urllib.parse.unquote(parts[4])
                t = s["tests"].get(tname)
                _json_resp(self, t or {"error": "not found"})
            else:
                _json_resp(self, {"session_id": sid, "name": s["name"], "tests": s["tests"], "created_ms": s["created_ms"]})
        else:
            _json_resp(self, {"error": "unknown"}, 404)

    def do_POST(self):
        p = urllib.parse.urlparse(self.path).path
        length = int(self.headers.get("Content-Length", "0") or "0")
        body = json.loads(self.rfile.read(length).decode()) if length else {}
        if p.startswith("/api/session/") and p.endswith("/run_test"):
            sid = p.strip("/").split("/")[2]
            tname = body.get("test_name")
            with _rerun_lock:
                cb = _rerun_handlers.get(sid, {}).get(tname)
            if cb:
                threading.Thread(target=cb, daemon=True).start()
                _json_resp(self, {"status": "started"})
            else:
                _json_resp(self, {"error": "no handler"}, 404)
        else:
            _json_resp(self, {"error": "unknown"}, 404)

    def log_message(self, *_):
        pass


class _Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True


# ── Public API ────────────────────────────────────────────────────────────────

def start(port: int = _DEFAULT_PORT, host: str = _DEFAULT_HOST) -> tuple[str, int]:
    """Start HTTP server + Tk UI. Non-blocking. Returns (host, port)."""
    global _server, _tk_thread
    if not _ENABLED:
        return host, port

    with _server_lock:
        if _server is not None:
            addr = getattr(_server, "server_address", (host, port))
            _ensure_tk(f"http://{addr[0]}:{addr[1]}")
            return addr[0], addr[1]

        port_try = port
        for _ in range(11):
            try:
                srv = _Server((host, port_try), _Handler)
                _server = srv
                break
            except OSError:
                port_try += 1
        else:
            raise RuntimeError("Could not bind debug UI server")

        threading.Thread(target=_server.serve_forever, daemon=True, name="spike_http").start()

    api_base = f"http://{host}:{port_try}"
    _ensure_tk(api_base)
    return host, port_try


def _ensure_tk(api_base: str) -> None:
    global _tk_thread
    if _tk_thread is not None and _tk_thread.is_alive():
        return
    t = threading.Thread(target=_run_tk, args=(api_base,), daemon=True, name="spike_tk")
    _tk_thread = t
    t.start()
    time.sleep(0.3)  # give Tk time to initialize before spike starts printing


def register_spike(name: str, metadata: Optional[Dict] = None) -> str:
    if not _ENABLED:
        return f"disabled-{name}"
    sid = str(uuid.uuid4())
    with _sessions_lock:
        _sessions[sid] = {"name": name, "metadata": metadata or {}, "created_ms": _now_ms(), "tests": {}}
    return sid


def test_started(session_id: str, test_name: str) -> None:
    if not _ENABLED:
        return
    with _sessions_lock:
        s = _sessions.get(session_id)
        if not s:
            return
        s["tests"].setdefault(test_name, {
            "status": "running", "started_ms": _now_ms(),
            "passed": None, "detail": "", "elapsed_ms": None,
            "messages": [], "raw": [],
        })["status"] = "running"


def test_finished(session_id: str, test_name: str, passed: bool, detail: str = "", elapsed_ms: float = 0.0) -> None:
    if not _ENABLED:
        return
    with _sessions_lock:
        s = _sessions.get(session_id)
        if not s:
            return
        t = s["tests"].setdefault(test_name, {"status": "running", "started_ms": _now_ms(), "passed": None, "detail": "", "elapsed_ms": None, "messages": [], "raw": []})
        t["status"] = "passed" if passed else "failed"
        t["passed"] = passed
        t["detail"] = detail
        t["elapsed_ms"] = elapsed_ms


def publish_raw(session_id: str, test_name: str, payload: Any, label: str = "raw") -> None:
    if not _ENABLED:
        return
    entry = {"label": label, "ts": _now_ms(), "payload": _sanitize(payload)}
    with _sessions_lock:
        s = _sessions.get(session_id)
        if not s:
            return
        t = s["tests"].setdefault(test_name, {"status": "running", "started_ms": _now_ms(), "passed": None, "detail": "", "elapsed_ms": None, "messages": [], "raw": []})
        t.setdefault("raw", []).append(entry)


def register_rerun_handler(session_id: str, test_name: str, callback: Callable) -> None:
    if not _ENABLED:
        return
    with _rerun_lock:
        _rerun_handlers.setdefault(session_id, {})[test_name] = callback


# ── Tk UI ─────────────────────────────────────────────────────────────────────

def _run_tk(api_base: str) -> None:
    try:
        import tkinter as tk
        from tkinter import ttk
    except Exception as e:
        print(f"  [debug_ui] Tkinter not available: {e}")
        return

    import json as _json
    import urllib.error

    POLL_MS = 500

    def _get(path: str):
        try:
            url = api_base.rstrip("/") + "/" + path.lstrip("/")
            with urllib.request.urlopen(url, timeout=2) as r:
                return _json.loads(r.read().decode())
        except Exception:
            return None

    root = tk.Tk()
    root.title("SENA SPIKE DEBUGGER")
    root.configure(bg=C["bg"])
    root.geometry("1200x750")
    root.resizable(True, True)

    # ── Title bar ─────────────────────────────────────────────────────────────
    title_frame = tk.Frame(root, bg=C["header"], pady=4)
    title_frame.pack(fill=tk.X)

    tk.Label(title_frame, text="▶  SENA SPIKE DEBUGGER", font=(C["mono"], 11, "bold"),
             bg=C["header"], fg=C["cyan"], padx=12).pack(side=tk.LEFT)
    status_var = tk.StringVar(value="CONNECTING...")
    tk.Label(title_frame, textvariable=status_var, font=(C["mono"], 9),
             bg=C["header"], fg=C["yellow"], padx=12).pack(side=tk.RIGHT)

    # ── Register panel (top) — session + pass/fail counts ─────────────────────
    reg_frame = tk.Frame(root, bg=C["bg2"], pady=6, padx=8)
    reg_frame.pack(fill=tk.X)

    reg_vars: dict[str, tk.StringVar] = {}
    reg_labels = ["SESSION", "SPIKE", "PASSED", "FAILED", "RUNNING", "TOTAL", "ELAPSED"]
    for i, label in enumerate(reg_labels):
        col = tk.Frame(reg_frame, bg=C["bg2"], padx=6)
        col.pack(side=tk.LEFT)
        tk.Label(col, text=label, font=(C["mono"], 7), bg=C["bg2"], fg=C["dim"]).pack()
        v = tk.StringVar(value="----")
        reg_vars[label] = v
        color = C["green"] if label == "PASSED" else C["red"] if label == "FAILED" else C["cyan"] if label == "RUNNING" else C["text"]
        tk.Label(col, textvariable=v, font=(C["mono"], 10, "bold"),
                 bg=C["bg2"], fg=color, width=10).pack()

    # ── Separator ─────────────────────────────────────────────────────────────
    tk.Frame(root, bg=C["border"], height=2).pack(fill=tk.X)

    # ── Main panes ────────────────────────────────────────────────────────────
    main = tk.Frame(root, bg=C["bg"])
    main.pack(fill=tk.BOTH, expand=True)

    # Left — test list
    left = tk.Frame(main, bg=C["bg"], width=340)
    left.pack(side=tk.LEFT, fill=tk.Y, padx=(8, 0), pady=8)
    left.pack_propagate(False)

    tk.Label(left, text="  TESTS", font=(C["mono"], 8, "bold"),
             bg=C["header"], fg=C["dim"], anchor="w").pack(fill=tk.X)

    test_frame = tk.Frame(left, bg=C["bg"])
    test_frame.pack(fill=tk.BOTH, expand=True, pady=(2, 0))

    test_scroll = tk.Scrollbar(test_frame, bg=C["bg2"], troughcolor=C["bg"])
    test_scroll.pack(side=tk.RIGHT, fill=tk.Y)

    test_list = tk.Listbox(
        test_frame,
        bg=C["bg"], fg=C["text"],
        selectbackground=C["border"], selectforeground=C["cyan"],
        font=(C["mono"], 9),
        borderwidth=0, highlightthickness=1, highlightcolor=C["border"],
        highlightbackground=C["border"],
        activestyle="none",
        yscrollcommand=test_scroll.set,
    )
    test_list.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
    test_scroll.config(command=test_list.yview)

    # Right — inspector
    right = tk.Frame(main, bg=C["bg"])
    right.pack(side=tk.LEFT, fill=tk.BOTH, expand=True, padx=8, pady=8)

    tk.Label(right, text="  INSPECTOR", font=(C["mono"], 8, "bold"),
             bg=C["header"], fg=C["dim"], anchor="w").pack(fill=tk.X)

    # Detail row (assumption + timing)
    detail_frame = tk.Frame(right, bg=C["bg2"], pady=4, padx=8)
    detail_frame.pack(fill=tk.X, pady=(2, 0))

    detail_assumption = tk.StringVar(value="")
    detail_status = tk.StringVar(value="")
    detail_timing = tk.StringVar(value="")
    detail_text = tk.StringVar(value="")

    tk.Label(detail_frame, textvariable=detail_assumption, font=(C["mono"], 8, "bold"),
             bg=C["bg2"], fg=C["cyan"], anchor="w").pack(fill=tk.X)
    row2 = tk.Frame(detail_frame, bg=C["bg2"])
    row2.pack(fill=tk.X)
    tk.Label(row2, textvariable=detail_status, font=(C["mono"], 9, "bold"),
             bg=C["bg2"], fg=C["text"], width=10, anchor="w").pack(side=tk.LEFT)
    tk.Label(row2, textvariable=detail_timing, font=(C["mono"], 9),
             bg=C["bg2"], fg=C["dim"], anchor="w").pack(side=tk.LEFT, padx=(8, 0))
    tk.Label(detail_frame, textvariable=detail_text, font=(C["mono"], 8),
             bg=C["bg2"], fg=C["text"], anchor="w", wraplength=700, justify="left").pack(fill=tk.X, pady=(4, 0))

    # Raw payload viewer
    tk.Frame(right, bg=C["border"], height=1).pack(fill=tk.X, pady=(6, 0))
    tk.Label(right, text="  RAW PAYLOAD", font=(C["mono"], 8, "bold"),
             bg=C["header"], fg=C["dim"], anchor="w").pack(fill=tk.X)

    raw_frame = tk.Frame(right, bg=C["bg"])
    raw_frame.pack(fill=tk.BOTH, expand=True, pady=(2, 0))

    raw_scroll_y = tk.Scrollbar(raw_frame, bg=C["bg2"], troughcolor=C["bg"])
    raw_scroll_y.pack(side=tk.RIGHT, fill=tk.Y)
    raw_scroll_x = tk.Scrollbar(raw_frame, orient=tk.HORIZONTAL, bg=C["bg2"], troughcolor=C["bg"])
    raw_scroll_x.pack(side=tk.BOTTOM, fill=tk.X)

    raw_text = tk.Text(
        raw_frame,
        bg=C["bg"], fg=C["text"],
        font=(C["mono"], 9),
        insertbackground=C["cyan"],
        borderwidth=0, highlightthickness=1,
        highlightcolor=C["border"], highlightbackground=C["border"],
        wrap=tk.NONE,
        yscrollcommand=raw_scroll_y.set,
        xscrollcommand=raw_scroll_x.set,
    )
    raw_text.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
    raw_scroll_y.config(command=raw_text.yview)
    raw_scroll_x.config(command=raw_text.xview)

    # Text tags for syntax colouring
    raw_text.tag_configure("key",    foreground=C["cyan"])
    raw_text.tag_configure("str",    foreground=C["green"])
    raw_text.tag_configure("num",    foreground=C["orange"])
    raw_text.tag_configure("bool",   foreground=C["yellow"])
    raw_text.tag_configure("null",   foreground=C["dim"])

    # ── Flags bar (bottom) ────────────────────────────────────────────────────
    tk.Frame(root, bg=C["border"], height=2).pack(fill=tk.X)
    flags_frame = tk.Frame(root, bg=C["header"], pady=4, padx=8)
    flags_frame.pack(fill=tk.X)
    flags_var = tk.StringVar(value="flags: ----")
    tk.Label(flags_frame, textvariable=flags_var, font=(C["mono"], 8),
             bg=C["header"], fg=C["dim"], anchor="w").pack(side=tk.LEFT)
    clock_var = tk.StringVar(value="")
    tk.Label(flags_frame, textvariable=clock_var, font=(C["mono"], 8),
             bg=C["header"], fg=C["dim"], anchor="e").pack(side=tk.RIGHT)

    # ── State ─────────────────────────────────────────────────────────────────
    _state = {
        "session_id": None,
        "selected_test": None,
        "test_data": {},
        "wall_start": time.time(),
        "last_sessions": [],
    }

    STATUS_SYMBOL = {"running": "►", "passed": "✓", "failed": "✗", "pending": "·"}
    STATUS_COLOR  = {"running": C["running"], "passed": C["pass"], "failed": C["fail"], "pending": C["pending"]}

    def _colorize_json(widget: tk.Text, text: str) -> None:
        widget.config(state=tk.NORMAL)
        widget.delete("1.0", tk.END)
        import re
        pos = 0
        patterns = [
            ("key",  r'"([^"]+)"\s*:'),
            ("str",  r':\s*"([^"]*)"'),
            ("num",  r':\s*(-?\d+\.?\d*)'),
            ("bool", r':\s*(true|false)'),
            ("null", r':\s*(null)'),
        ]
        lines = text.split("\n")
        for line in lines:
            remaining = line
            offset = 0
            segments = []
            for tag, pat in patterns:
                for m in re.finditer(pat, line):
                    segments.append((m.start(), m.end(), tag, m.group(0)))
            segments.sort(key=lambda x: x[0])
            if not segments:
                widget.insert(tk.END, line + "\n", "")
            else:
                prev = 0
                for start, end, tag, chunk in segments:
                    if start >= prev:
                        widget.insert(tk.END, line[prev:start], "")
                        widget.insert(tk.END, chunk, tag)
                        prev = end
                widget.insert(tk.END, line[prev:] + "\n", "")
        widget.config(state=tk.DISABLED)

    def _show_test(test_name: str) -> None:
        t = _state["test_data"].get(test_name)
        if not t:
            detail_assumption.set("")
            detail_status.set("")
            detail_timing.set("")
            detail_text.set("")
            raw_text.config(state=tk.NORMAL)
            raw_text.delete("1.0", tk.END)
            raw_text.config(state=tk.DISABLED)
            return

        status = t.get("status", "pending")
        passed = t.get("passed")
        elapsed = t.get("elapsed_ms")

        detail_assumption.set(f"  {test_name}")
        color = STATUS_COLOR.get(status, C["text"])

        if status == "running":
            detail_status.set("► RUNNING")
            detail_status_label = detail_frame.winfo_children()[1].winfo_children()[0]
            try:
                detail_status_label.config(fg=C["running"])
            except Exception:
                pass
        elif passed is True:
            detail_status.set("✓ PASS")
        elif passed is False:
            detail_status.set("✗ FAIL")
        else:
            detail_status.set("· PENDING")

        detail_timing.set(f"{int(elapsed)}ms" if elapsed else "")
        detail_text.set(t.get("detail", ""))

        raw_entries = t.get("raw", [])
        if raw_entries:
            try:
                pretty = _json.dumps(raw_entries, indent=2, ensure_ascii=False)
            except Exception:
                pretty = str(raw_entries)
            _colorize_json(raw_text, pretty)
        else:
            raw_text.config(state=tk.NORMAL)
            raw_text.delete("1.0", tk.END)
            raw_text.insert(tk.END, "(no raw payload yet)")
            raw_text.config(state=tk.DISABLED)

    def _on_test_select(_event=None) -> None:
        sel = test_list.curselection()
        if not sel:
            return
        label = test_list.get(sel[0])
        # strip the leading status symbol + spaces
        name = label[2:].strip() if len(label) > 2 else label.strip()
        # strip timing suffix like [1234ms]
        if "  [" in name:
            name = name[:name.index("  [")]
        _state["selected_test"] = name
        _show_test(name)

    test_list.bind("<<ListboxSelect>>", _on_test_select)

    def _render_test_list(tests: dict) -> None:
        sel = _state.get("selected_test")
        test_list.delete(0, tk.END)
        for i, (name, t) in enumerate(tests.items()):
            status = t.get("status", "pending")
            sym = STATUS_SYMBOL.get(status, "·")
            elapsed = t.get("elapsed_ms")
            timing = f"  [{int(elapsed)}ms]" if elapsed else ""
            label = f"{sym} {name}{timing}"
            test_list.insert(tk.END, label)
            test_list.itemconfig(i, fg=STATUS_COLOR.get(status, C["text"]))
            if name == sel:
                test_list.selection_set(i)
                test_list.see(i)

    def _poll() -> None:
        try:
            data = _get("/api/sessions")
            if not data:
                status_var.set("NO DATA")
                root.after(POLL_MS, _poll)
                return

            sessions = data.get("sessions", [])
            _state["last_sessions"] = sessions

            # pick most recent session
            sid = _state["session_id"]
            if not sid and sessions:
                sid = sessions[-1]["session_id"]
                _state["session_id"] = sid

            if not sid:
                status_var.set("WAITING FOR SPIKE...")
                root.after(POLL_MS, _poll)
                return

            detail = _get(f"/api/session/{sid}")
            if not detail:
                root.after(POLL_MS, _poll)
                return

            tests = detail.get("tests", {})
            _state["test_data"] = tests

            # update registers
            spike_name = detail.get("name", "----")
            passed = sum(1 for t in tests.values() if t.get("passed") is True)
            failed = sum(1 for t in tests.values() if t.get("passed") is False)
            running = sum(1 for t in tests.values() if t.get("status") == "running")
            total = len(tests)
            elapsed_s = f"{time.time() - _state['wall_start']:.1f}s"

            reg_vars["SESSION"].set(sid[:8] + "…")
            reg_vars["SPIKE"].set(spike_name[:10])
            reg_vars["PASSED"].set(str(passed))
            reg_vars["FAILED"].set(str(failed))
            reg_vars["RUNNING"].set(str(running))
            reg_vars["TOTAL"].set(str(total))
            reg_vars["ELAPSED"].set(elapsed_s)

            # flags bar
            all_done = total > 0 and running == 0 and (passed + failed) == total
            if all_done:
                flag = "HALT" if failed > 0 else "DONE"
                flags_var.set(f"flags: Z={int(failed==0)} N={int(failed>0)} C=0  |  {flag}")
                status_var.set("HALT — SPIKE COMPLETE" if failed > 0 else "DONE — ALL PASS")
            elif running > 0:
                flags_var.set(f"flags: Z=0 N=0 C=0  |  RUN")
                status_var.set(f"RUNNING — {running} active")
            else:
                flags_var.set("flags: ----")
                status_var.set("IDLE")

            clock_var.set(f"wall: {elapsed_s}")

            _render_test_list(tests)

            # auto-refresh selected test inspector
            sel = _state.get("selected_test")
            if sel and sel in tests:
                _show_test(sel)
            elif not sel and tests:
                # auto-select running test
                for name, t in tests.items():
                    if t.get("status") == "running":
                        _state["selected_test"] = name
                        _show_test(name)
                        break

        except Exception as e:
            status_var.set(f"ERR: {e}")

        root.after(POLL_MS, _poll)

    root.after(100, _poll)
    root.mainloop()


# ── CLI ───────────────────────────────────────────────────────────────────────

def _cli_main(argv=None):
    import argparse
    p = argparse.ArgumentParser()
    p.add_argument("--tk", action="store_true")
    p.add_argument("--api-base", default=None)
    args = p.parse_args(argv)
    if args.tk:
        _run_tk(args.api_base or f"http://{_DEFAULT_HOST}:{_DEFAULT_PORT}")
        return 0
    p.print_help()
    return 2


if __name__ == "__main__":
    raise SystemExit(_cli_main())