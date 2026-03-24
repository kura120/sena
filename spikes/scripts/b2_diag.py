#!/usr/bin/env python3
"""
B2 diagnostic — compare token counts for JSON vs TOON encode options.

Place this file at: spikes/b2_diag.py
Run from the project virtualenv:
    uv run python spikes/b2_diag.py

What it does:
- Imports the two fixtures used by the TOON spike (soulbox and full prompt).
- Tries to import the installed TOON implementation (`toon_format` or `toon`).
- Tries to import `tiktoken` for accurate token counts. If unavailable,
  falls back to a conservative character-based proxy.
- Encodes the fixtures as JSON and as TOON with several option variants.
- Prints token counts, deltas, sample TOON output (truncated), and recommends
  the best TOON options for each fixture.

Notes:
- This is a diagnostic helper — it does not modify your environment.
- If you want me to run this for you, tell me and I'll run it and paste the results.
"""

from __future__ import annotations

import json
import sys
import textwrap
import time
from typing import Any, Dict, List, Optional, Tuple

# Try to import fixtures from the spike file where they're defined
try:
    from spikes.toon_spikes import FULL_PROMPT_CONTEXT, SOULBOX_SNAPSHOT
except Exception:
    # Fallback: small example fixtures if the module isn't importable
    SOULBOX_SNAPSHOT = {
        "schema_version": 3,
        "personality": {"warmth": 0.72, "openness": 0.65},
    }
    FULL_PROMPT_CONTEXT = {
        "soulbox": SOULBOX_SNAPSHOT,
        "short_term": [
            {"content": "User asked about Rust async patterns", "weight": 0.9}
        ],
    }


def safe_import_toon() -> Optional[object]:
    """Try common import names for the TOON implementation."""
    candidates = ("toon_format", "toon")
    for name in candidates:
        try:
            mod = __import__(name)
            return mod
        except Exception:
            continue
    return None


def safe_import_tiktoken():
    """Try to import tiktoken; return None if unavailable."""
    try:
        import tiktoken

        return tiktoken
    except Exception:
        return None


def make_count_tokens(tiktoken_module) -> Tuple[callable, Optional[str]]:
    """
    Returns (count_tokens_fn, desc).
    count_tokens_fn(s) -> int.
    If tiktoken_module is None, return a fallback that approximates tokens.
    """
    if tiktoken_module is not None:
        try:
            enc = tiktoken_module.get_encoding("cl100k_base")
        except Exception:
            # Fallback to a basic encoder name if this fails
            try:
                enc = tiktoken_module.get_encoding("gpt2")
            except Exception:
                enc = None

        if enc is not None:

            def _count_tokens(s: str) -> int:
                return len(enc.encode(s))

            return _count_tokens, "tiktoken(cl100k_base)"
    # Fallback: simple heuristic by splitting on whitespace and punctuation-ish chars.
    import re

    token_re = re.compile(r"\w+|\S")

    def _heuristic_count(s: str) -> int:
        # approximate tokens by word-like and symbol chunks
        return len(token_re.findall(s))

    return _heuristic_count, None


def pretty_opts(opts: Optional[Dict[str, Any]]) -> str:
    if not opts:
        return "<default>"
    return ", ".join(f"{k}={v!r}" for k, v in opts.items())


def encode_with_options(
    toon_mod, obj: Any, opts: Optional[Dict[str, Any]]
) -> Tuple[Optional[str], Optional[str]]:
    """
    Try to encode the object with toon_mod.encode(obj, opts).
    Returns (encoded_str or None, error_message or None).
    """
    try:
        if opts:
            s = toon_mod.encode(obj, opts)
        else:
            s = toon_mod.encode(obj)
        if not isinstance(s, str):
            s = str(s)
        return s, None
    except Exception as exc:
        return None, f"{type(exc).__name__}: {exc}"


def run_diag_for_fixture(
    name: str,
    obj: Any,
    toon_mod,
    count_tokens_fn,
    options_list: List[Optional[Dict[str, Any]]],
):
    print("\n" + ("=" * 72))
    print(f"Fixture: {name}")
    print("=" * 72)
    json_str = json.dumps(obj, separators=(",", ":"))
    json_tokens = count_tokens_fn(json_str)
    print(f"JSON length (chars): {len(json_str)} | JSON tokens: {json_tokens}")
    results = []
    for opts in options_list:
        toon_str, err = encode_with_options(toon_mod, obj, opts)
        if err:
            print(f"  TOON opts={pretty_opts(opts)} -> ENCODE ERROR: {err}")
            results.append((opts, None, err, None))
            continue
        toon_tokens = count_tokens_fn(toon_str)
        # keep a truncated sample of the TOON string for inspection
        sample = (
            toon_str if len(toon_str) <= 800 else toon_str[:800] + "\n... (truncated)"
        )
        print(
            f"  TOON opts={pretty_opts(opts)} -> tokens={toon_tokens} | delta={toon_tokens - json_tokens}"
        )
        results.append((opts, toon_tokens, None, sample))

    # find best valid option by token count (ignore failures)
    valid = [r for r in results if r[1] is not None]
    if not valid:
        print("No successful TOON encodes for this fixture.")
        return

    best = min(valid, key=lambda r: r[1])
    best_opts, best_tokens, _, best_sample = best
    reduction_pct = (
        (json_tokens - best_tokens) / json_tokens * 100 if json_tokens > 0 else 0.0
    )

    print("\nBest option:")
    print(f"  opts: {pretty_opts(best_opts)}")
    print(
        f"  TOON tokens: {best_tokens} | JSON tokens: {json_tokens} | reduction: {reduction_pct:.1f}%"
    )
    print("\nSample TOON (truncated):\n")
    print(textwrap.indent(best_sample or "<no sample>", "  "))
    print("\nRecommendation:")
    if reduction_pct >= 15.0:
        print(
            "  Use TOON with the flagged options for this fixture (sensible savings)."
        )
    elif reduction_pct > 0:
        print(
            "  Modest savings observed. Consider TOON for cost-sensitive flows but validate on target models."
        )
    else:
        print(
            "  No token savings observed for this fixture. Prefer JSON/TOML for fidelity; use TOON only for droppable/tabular tiers."
        )


def main():
    print("\nB2 Diagnostic: JSON vs TOON token comparison\n")
    toon_mod = safe_import_toon()
    if not toon_mod:
        print(
            "ERROR: No TOON implementation found. Install the working package (toon_format) and re-run."
        )
        print(
            "Hints: uv add toon-format  OR  pip install git+https://github.com/toon-format/toon-python.git@main"
        )
        sys.exit(2)

    print(
        f"Using TOON module: {toon_mod.__name__} (file: {getattr(toon_mod, '__file__', None)})"
    )

    tiktoken_mod = safe_import_tiktoken()
    count_tokens_fn, tdesc = make_count_tokens(tiktoken_mod)
    if tdesc:
        print(f"Token counting: using {tdesc}")
    else:
        print(
            "Token counting: using heuristic fallback (approximate) - install tiktoken for accurate counts"
        )

    # Try a few encode option variants that commonly affect size
    options_list: List[Optional[Dict[str, Any]]] = [
        None,
        {"indent": 0},
        {"indent": 0, "lengthMarker": "#"},
        {"indent": 0, "delimiter": "\t"},
        {"indent": 1},
    ]

    # Run diagnostics for the two primary fixtures
    run_diag_for_fixture(
        "soulbox", SOULBOX_SNAPSHOT, toon_mod, count_tokens_fn, options_list
    )
    run_diag_for_fixture(
        "full_prompt_context",
        FULL_PROMPT_CONTEXT,
        toon_mod,
        count_tokens_fn,
        options_list,
    )

    print("\nDiagnostic complete.\n")


if __name__ == "__main__":
    main()
