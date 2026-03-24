#!/usr/bin/env python3
"""
_sample_session.py

Small helper script to start the debug UI server, register a synthetic session
and a couple of harmless synthetic tests, keep the server running for 120s,
then shut it down.

Intended for local dev/debug only. This script publishes only synthetic,
non-sensitive payloads to let you inspect the UI behavior.

Usage:
    python spikes/tools/_sample_session.py
"""

from __future__ import annotations

import time
import traceback
import uuid

# Default run interval (seconds) that the script keeps the server alive.
RUN_SECONDS = 120
POLL_INTERVAL = 5.0


def main() -> None:
    try:
        # Import the debug UI server helper from the tools package. If the
        # package isn't available, bail gracefully.
        from spikes.tools import debug_ui  # type: ignore
    except Exception as exc:
        print("debug sample: unable to import spikes.tools.debug_ui:", exc)
        traceback.print_exc()
        return

    server_started = False
    session_id = "disabled"
    try:
        # Start the debug UI server (non-blocking). Do not open a browser here.
        try:
            debug_ui.start_ui(open_browser=False)
            server_started = True
            print("debug sample: debug UI server started (HTTP server running).")
        except Exception as exc:
            print("debug sample: debug_ui.start_ui() failed:", exc)
            traceback.print_exc()

        # Register a synthetic session (harmless sample)
        try:
            session_id = debug_ui.register_spike(
                "sample_spike", {"synthetic": True, "note": "UI demo"}
            )
            print("debug sample: registered session_id:", session_id)
        except Exception as exc:
            print("debug sample: register_spike failed:", exc)
            traceback.print_exc()
            session_id = "disabled"

        # Publish two small synthetic tests so the UI has something to show.
        # These payloads are intentionally non-sensitive and are safe to inspect.
        if session_id != "disabled":
            try:
                debug_ui.test_started(session_id, "T1 — quick check")
                debug_ui.publish_raw(
                    session_id,
                    "T1 — quick check",
                    {"msg": "synthetic test payload", "seq": 1},
                    label="sample",
                )
                debug_ui.test_finished(
                    session_id, "T1 — quick check", True, "ok", elapsed_ms=123.4
                )
            except Exception:
                # non-fatal
                traceback.print_exc()

            try:
                debug_ui.test_started(session_id, "T2 — second check")
                debug_ui.publish_raw(
                    session_id,
                    "T2 — second check",
                    {"msg": "another synthetic payload", "items": [1, 2, 3]},
                    label="sample",
                )
                debug_ui.test_finished(
                    session_id, "T2 — second check", True, "ok", elapsed_ms=45.6
                )
            except Exception:
                traceback.print_exc()

        # Keep the server running for RUN_SECONDS so you can open the UI and
        # inspect the synthetic session. We print a small heartbeat so the
        # user knows the script is alive.
        start = time.time()
        elapsed = 0.0
        while elapsed < RUN_SECONDS:
            remaining = int(RUN_SECONDS - elapsed)
            print(f"debug sample: server running — {remaining}s remaining")
            time.sleep(POLL_INTERVAL)
            elapsed = time.time() - start

    except KeyboardInterrupt:
        print("debug sample: interrupted by user (KeyboardInterrupt)")

    finally:
        # Attempt a graceful shutdown of the debug server (best-effort).
        try:
            if server_started:
                debug_ui.shutdown()
                print("debug sample: debug UI server shut down.")
        except Exception as exc:
            print("debug sample: shutdown failed:", exc)
            traceback.print_exc()


if __name__ == "__main__":
    main()
