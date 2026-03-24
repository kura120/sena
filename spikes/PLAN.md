# PLAN.md — Spike Debug UI (mini UI) for Spikes

What
----
Deliver a small, dependency-light "Spike Debug UI" that exposes spike execution progress, raw model responses, and per-test diagnostics in a usable interface. The UI will be usable from all spike files in `spikes/` (initial targets: `cognee_spikes.py` and `toon_spikes.py`). It is intended as a development/debug tool — not production UI.

Why
---
During spike runs we frequently need to inspect:
- raw LLM completions (to diagnose structured output failures),
- intermediate results from Cognee operations (raw CHUNKS, GRAPH_COMPLETION payloads),
- timing and retry behavior (Tenacity sleep cycles),
- prune / DB state and concurrent operations.

A small UI lowers the feedback loop for the spike author: instead of pouring through logs, you get interactive quick-inspection and can replay or re-run failing steps. This reduces time-to-diagnosis and prevents ad-hoc edits (which are brittle).

Subsystems affected
-------------------
- spikes/ (new debug helper will be imported by spike scripts)
- prompt-composer (only for UX expectations; not modified)
- Observability (logs + optional file output)

Primary owners:
- `spikes/` (owner: spike author)
- `platform/windows` (consumer only; no changes required)

Assumptions
-----------
- Spikes run locally during development (not in CI).
- Developer machine has Python 3.11+ (repo uses 3.13) and common stdlib modules available.
- Minimal third-party dependencies are acceptable, but prefer none. If needed, recommend `fastapi` + `uvicorn` (optional) or standard library `http.server` for zero-dep.
- Spikes are allowed to import a local `spikes/debug_ui.py` helper module that exposes a simple API.
- UI should be optional — spikes must work without it (headless fallback).

Out of scope
------------
- A polished production UI or integration into Sena's Freya UI.
- Persistent remote telemetry or sending user data off-device.
- Complex authentication or RBAC — the UI is a local dev tool.
- Cross-platform packaged installer. The UI runs as a dev helper only.

Design Goals (constraints)
--------------------------
- Zero-or-minimal dependencies. Prefer stdlib when UX is acceptable.
- Low friction to integrate: single-line import + 2-3 API calls per spike.
- Non-blocking by default: spike executables should not await UI startup unless requested.
- Simple HTML web UI (hosted locally) with endpoints the spike can POST to; and a small JS frontend that shows:
  - running tests list with status (pending/running/ok/fail)
  - raw response viewer (pretty-printed JSON/text)
  - timing histogram per test
  - ability to re-run a single test (calls into spike's re-run hook if implemented)
- Spike modules may opt-in to additional debug channels (e.g., log file capture, extended traces).

Implementation approach (high level)
-----------------------------------
1. Create `spikes/debug_ui.py` which contains:
   - `start_ui(port: int = 8765, open_browser: bool = False) -> None`
     - Starts a tiny HTTP server in a background thread or an asyncio task (depending on runtime).
     - Minimal implementation options:
       - Zero-dep: stdlib `http.server` serving static HTML/JS and a minimal POST handler using `socketserver.ThreadingMixIn`.
       - Optional better UX: `fastapi` + `uvicorn` (conditionally used if present).
   - `register_spike(name: str, metadata: dict) -> None`
     - Register a spike run/session. Returns a spike id or uses `name` if unique.
   - `test_started(spike: str, test_name: str) -> None`
   - `test_progress(spike: str, test_name: str, message: str, level: str = 'info') -> None`
   - `test_finished(spike: str, test_name: str, passed: bool, detail: str = "", elapsed_ms: float = 0.0) -> None`
   - `publish_raw(spike: str, test_name: str, payload: Any, label: str = "raw") -> None`
     - Accepts strings or serializable objects; UI will pretty-print JSON or show text.
   - `shutdown()` — stops server threads cleanly.
   - The module must be safe to import when the UI isn't desired; provide an environment variable toggle `SPIKE_UI_ENABLED` (default `1`) and a no-op implementation if disabled.

2. Add `spikes/ui_static/` minimal frontend:
   - `index.html` — lists sessions and tests. Simple HTML+vanilla JS that polls `/api/sessions` and `/api/session/{id}/tests`.
   - `main.js` — minimal code to fetch session state, show raw payloads in a modal, and request a single test re-run via `/api/session/{id}/run_test`.
   - Styling is minimal (inline CSS).

3. Spike integration pattern (small changes to spikes)
   - At top of each spike script:
     ```py
     from spikes import debug_ui
     debug_ui.start_ui(port=8765)  # non-blocking
     spike_session = debug_ui.register_spike("cognee_spike", {"branch": "local"})
     ```
   - Around each test:
     ```py
     debug_ui.test_started(spike_session, "A5 semantic retrieval")
     # inside try/except/finally:
     debug_ui.publish_raw(spike_session, "A5 semantic retrieval", results, label="post-search-raw")
     debug_ui.test_finished(spike_session, "A5 semantic retrieval", passed, detail, elapsed_ms)
     ```
   - If the spike wants a re-run hook, it can optionally register a callback:
     ```py
     def rerun_a5():
         asyncio.run(test_a5_semantic_retrieval(...))  # careful: keep event loop compatibility

     debug_ui.register_rerun_handler(spike_session,"A5 semantic retrieval", rerun_a5)
     ```
   - Alternatively, spikes may provide a small `async def rerun(test_name):` function referenced by a mapping the UI can call via a POST.

4. Server behavior
   - Keep an in-memory session store (dict).
   - Expose REST API endpoints:
     - `GET /api/sessions` → list sessions
     - `GET /api/session/{id}` → session metadata and test summary
     - `GET /api/session/{id}/test/{test_name}` → test details and raw payloads
     - `POST /api/session/{id}/run_test` → optional re-run (spike must implement callback)
     - `POST /api/log` → generic log ingestion (optional)
   - Frontend polls `/api/sessions` every 1s for updates.

5. Safety and privacy
   - UI only binds to `localhost` by default.
   - Spike authors must *not* call `publish_raw` with secrets. Add small sanitizer utility that masks things like `LLM_API_KEY` keys if present in posted payloads.
   - Document in README usage and privacy considerations.

6. Minimal dependency fallback strategy
   - If `fastapi` is installed, prefer `fastapi` server (better async, simpler handler).
   - Else use a pure-stdlib HTTP server.
   - The debug module should detect available stack and pick best option.

7. Testing & acceptance
   - Unit tests for `debug_ui`:
     - starting server and registering sessions works.
     - `test_started` / `publish_raw` / `test_finished` update internal state and API returns correct JSON.
   - Manual acceptance:
     - Run `uv run spikes/cognee_spikes.py` with UI enabled, open `http://localhost:8765` and see session populate as tests run, inspect raw responses from GRAPH_COMPLETION that previously caused Pydantic failures.
   - Performance: UI must not add >10ms overhead per test call in typical runs.

Concrete API (spikes call site)
-------------------------------
- API surface in `spikes/debug_ui.py` (synchronous-friendly):
  - `start_ui(port: int = 8765, open_browser: bool = False) -> None`
  - `register_spike(name: str, metadata: Optional[dict] = None) -> str`  # returns session_id
  - `test_started(session_id: str, test_name: str) -> None`
  - `publish_raw(session_id: str, test_name: str, payload: Any, label: str = "raw") -> None`
  - `test_finished(session_id: str, test_name: str, passed: bool, detail: str = "", elapsed_ms: float = 0.0) -> None`
  - `register_rerun_handler(session_id: str, test_name: str, callback: Callable[[], Any]) -> None`  # optional
  - `shutdown() -> None`
- All functions are no-ops if `SPIKE_UI_ENABLED=0` or if `start_ui` was not called — spikes can call them unconditionally.

Implementation plan and tasks
-----------------------------
1. Add `spikes/debug_ui.py` with the API and two backends (fastapi optional / stdlib fallback).
2. Add `spikes/ui_static/index.html` + `main.js` + minimal CSS.
3. Update `spikes/README.md` with instructions to enable UI:
   - `export SPIKE_UI_ENABLED=1`
   - `uv run spikes/cognee_spikes.py`
   - open `http://localhost:8765`
4. Light edits to `spikes/cognee_spikes.py` and `spikes/toon_spikes.py` to call the new API as described. Keep edits minimal and wrapped so the script still runs headless if UI disabled.
5. Add unit tests for `spikes/debug_ui.py` in `tests/spikes/` (optional for CI).
6. Manual test and document results in `spikes/README.md` after verification.

Estimated effort
----------------
- Prototype `debug_ui.py` + HTML/JS static UI (zero-dep stdlib): 2–4 hours.
- Small edits to two spike files to integrate API: 30–90 minutes.
- Optional fastapi/uvicorn integration and extra polish: +2–4 hours.
- Tests and documentation: +1–2 hours.

Acceptance criteria
-------------------
- Running a spike with `SPIKE_UI_ENABLED=1` and `debug_ui.start_ui()` shows a live session in `http://localhost:8765`.
- Tests show status updates in the UI in near-real-time (polling interval ≤ 1s).
- Clicking a test shows the raw post-search payload for inspection.
- Spike runs with `SPIKE_UI_ENABLED=0` or without calling `start_ui()` are unchanged (headless behavior preserved).
- Sensitive fields are masked if accidentally published via `publish_raw`.

Rollout plan
------------
- Implement the debug helper and UI in `spikes/debug_ui.py` and `spikes/ui_static` (branch: `spikes/debug-ui`).
- Integrate into `cognee_spikes.py` and `toon_spikes.py` with small changes; run locally and iterate UI.
- Document usage in `spikes/README.md` and this `spikes/PLAN.md`.

Open questions / decisions
-------------------------
- Do we want the UI to support authenticated access? (Recommendation: no — local-dev only.)
- Should the UI persist sessions to disk for later review? (Recommendation: optional — start without persistence.)
- Rerun hooks: some tests are async and embedded in the spike runner — adding re-run will require carefully designed reentrancy; implement as optional, developer-opt-in.

Appendix — example spike integration snippet
-------------------------------------------
(Example code to add to spikes; intentionally minimal / safe to call when UI disabled.)

```py
from spikes import debug_ui
debug_ui.start_ui(port=8765)
session = debug_ui.register_spike("cognee_spike", {"env": "local"})
debug_ui.test_started(session, "A5 semantic retrieval")
# run test...
debug_ui.publish_raw(session, "A5 semantic retrieval", raw_results, label="post-search-raw")
debug_ui.test_finished(session, "A5 semantic retrieval", passed, detail, elapsed_ms)
```

If you approve this PLAN I will:
- Create `spikes/debug_ui.py` and the small static UI artifacts,
- Apply the minimal integrations to `cognee_spikes.py` and `toon_spikes.py`,
- Add usage docs to `spikes/README.md`.
