"""
pc_encoding_selector.py — Encoding Selection Utility (ESU) prototype

This is a small, self-contained runtime helper that chooses between TOON, JSON,
or TOML for a given payload at prompt-assembly time.

Design goals (matches PRD section added in docs/PRD-v0.5.1.md):
- Deterministic, fast decision: choose_encoding(payload, model_profile, options)
- Use tiktoken when available for accurate token counts; fall back to a
  heuristic token estimator otherwise.
- Default TOON encode options aim to be compact (indent=0).
- Emit a local telemetry event (structured dict) for every choice to allow
  offline tuning. Telemetry emission is a no-op by default; callers should
  integrate with the project's telemetry pipeline (the function returns the
  telemetry dict regardless).
- Safe: never crash the caller — failures in optional libs fall back to JSON.

Note: This is a prototype for the prompt-composer subsystem. It assumes the
presence of a working TOON implementation under the import name `toon_format`
or `toon`. If not installed, ESU will fall back to JSON-only behavior.
"""

from __future__ import annotations

import hashlib
import json
import time
from typing import Any, Dict, Optional, Tuple

# Decision defaults (tunable)
SAVE_THRESHOLD = 0.15  # require >=15% savings to choose TOON
TOON_DEFAULT_OPTIONS = {"indent": 0}
LATENCY_THRESHOLD_MS = 10.0  # target encode latency on hot path (informational)

# Telemetry destination - prototype: write to file if enabled
_DEFAULT_TELEMETRY_ENABLED = False
_TELEMETRY_PATH = "spikes/pc_encoding_telemetry.log"


# -------------------------
# Helpers: safe imports
# -------------------------
def _import_toon_module():
    """
    Try to import the installed TOON implementation. Accepts common names.
    Returns the module object or None.
    """
    candidates = ("toon_format", "toon")
    for name in candidates:
        try:
            module = __import__(name)
            return module
        except Exception:
            continue
    return None


def _import_tiktoken():
    """
    Try to import tiktoken for accurate token counts. Returns module or None.
    """
    try:
        import tiktoken  # type: ignore

        return tiktoken
    except Exception:
        return None


# -------------------------
# Token counting
# -------------------------
def _make_token_counter():
    """
    Return a tuple (count_tokens_fn, desc) where count_tokens_fn(s) -> int.
    If tiktoken is available it will be used; otherwise a heuristic is used.
    """
    tiktoken_mod = _import_tiktoken()
    if tiktoken_mod is not None:
        try:
            enc = tiktoken_mod.get_encoding("cl100k_base")  # common encoding
        except Exception:
            try:
                enc = tiktoken_mod.get_encoding("gpt2")
            except Exception:
                enc = None

        if enc is not None:

            def _count_tokens(s: str) -> int:
                # tiktoken encoding returns a list of token ids
                return len(enc.encode(s))

            return _count_tokens, "tiktoken(cl100k_base)"
    # Fallback heuristic: approximate tokens by splitting word-ish and symbol tokens
    import re

    token_re = re.compile(r"\w+|\S")

    def _heuristic_count(s: str) -> int:
        return len(token_re.findall(s))

    return _heuristic_count, "heuristic"


_count_tokens, TOKEN_COUNTER_DESC = _make_token_counter()


# -------------------------
# Encoding wrappers
# -------------------------
def _json_dumps(payload: Any) -> str:
    # Use compact separators for prompt assembly to minimize noise
    try:
        return json.dumps(
            payload, separators=(",", ":"), ensure_ascii=False, default=str
        )
    except Exception:
        # Last-resort: convert to str
        return json.dumps(str(payload), separators=(",", ":"), ensure_ascii=False)


def _toon_encode(payload: Any, options: Optional[Dict[str, Any]] = None) -> str:
    """
    Try to encode payload via TOON. Raises Exception if no working implementation
    is available or if encoding fails.
    """
    toon = _import_toon_module()
    if toon is None:
        raise RuntimeError(
            "TOON implementation not available (toon_format/toon not installed)"
        )
    # The canonical API is toon.encode(payload, options?) per the spike tests
    if options:
        return toon.encode(payload, options)
    return toon.encode(payload)


# -------------------------
# Heuristics / structural checks
# -------------------------
def _is_uniform_tabular_array(obj: Any) -> bool:
    """
    Detect if obj is a list of dicts with consistent keys (tabular).
    Heuristic only — fast and conservative.
    """
    if not isinstance(obj, list) or not obj:
        return False
    # All elements should be dicts
    if not all(isinstance(x, dict) for x in obj):
        return False
    # Collect key sets
    keys = [tuple(sorted(x.keys())) for x in obj if isinstance(x, dict)]
    return len(keys) >= 1 and all(k == keys[0] for k in keys)


def _is_sacred(payload: Any, options: Optional[Dict[str, Any]] = None) -> bool:
    """
    Determine if payload is 'sacred' by shape or caller hint.
    Caller should pass options={'sacred': True} to force.
    By default, we check for likely SoulBox shape heuristically (has 'schema_version' key).
    """
    if options and options.get("sacred"):
        return True
    if isinstance(payload, dict) and "schema_version" in payload:
        return True
    return False


def _payload_fingerprint(payload: Any) -> str:
    """
    Return a stable fingerprint (hash) for telemetry without containing raw data.
    Uses a compact JSON form if possible.
    """
    try:
        j = _json_dumps(payload)
    except Exception:
        j = str(payload)
    h = hashlib.sha256(j.encode("utf-8")).hexdigest()
    return h[:16]


# -------------------------
# Telemetry
# -------------------------
def _emit_telemetry(
    event: Dict[str, Any], enabled: bool = _DEFAULT_TELEMETRY_ENABLED
) -> None:
    """
    Emit telemetry for offline analysis. Prototype writes a JSON line to a local file.
    This is intentionally minimal and local-only. Integrate with the daemon-bus/telemetry
    system when the prompt-composer subsystem is implemented.
    """
    if not enabled:
        return
    try:
        line = json.dumps(event, ensure_ascii=False)
        with open(_TELEMETRY_PATH, "a", encoding="utf-8") as f:
            f.write(line + "\n")
    except Exception:
        # Telemetry must never break normal operation
        pass


# -------------------------
# Main API
# -------------------------
def choose_encoding(
    payload: Any,
    model_profile: Optional[Dict[str, Any]] = None,
    options: Optional[Dict[str, Any]] = None,
) -> Dict[str, Any]:
    """
        Decide whether to encode `payload` as TOON, JSON, or TOML (TOML used only
        for SoulBox-like config shapes by default).

        Returns a dict:
          {
            "format": "TOON"|"JSON"|"TOML",
            "encoded": str,
            "options": dict,                # chosen encode options for TOON or None
            "tokens": {"json": int, "toon": Optional[int]},
            "savings_pct": Optional[float], # positive => JSON->TOON savings
            "reason": str,
    +       "telemetry": dict              # the telemetry event for optional emission
          }

        Behavior:
        - If payload is sacred (SoulBox or caller-provided hint), prefer TOML/JSON fidelity.
        - If payload is a uniform/tabular array, prefer TOON candidate and compare tokens.
        - Otherwise compare quick token estimates and require >= SAVE_THRESHOLD savings to pick TOON.
        - On any failure to TOON-encode, fall back to JSON and surface the error in the reason.
    """
    model_profile = model_profile or {}
    opts = options or {}

    t0 = time.time()
    result: Dict[str, Any] = {
        "format": None,
        "encoded": None,
        "options": None,
        "tokens": {"json": None, "toon": None},
        "savings_pct": None,
        "reason": "",
        "telemetry": None,
    }

    # 1) Sacred payloads short-circuit to JSON/TOML for fidelity
    if _is_sacred(payload, options=opts):
        encoded_json = _json_dumps(payload)
        result.update(
            {
                "format": "JSON",
                "encoded": encoded_json,
                "options": None,
                "tokens": {"json": _count_tokens(encoded_json), "toon": None},
                "savings_pct": 0.0,
                "reason": "sacred_fidelity",
            }
        )
        # telemetry
        event = {
            "event": "pc.encoding_choice",
            "timestamp_utc": int(t0 * 1000),
            "format_chosen": result["format"],
            "reason_code": result["reason"],
            "json_tokens": result["tokens"]["json"],
            "toon_tokens": None,
            "savings_pct": result["savings_pct"],
            "payload_signature": _payload_fingerprint(payload),
            "model_id": model_profile.get("model_id") if model_profile else None,
        }
        result["telemetry"] = event
        _emit_telemetry(event)
        return result

    # 2) Compute JSON token count
    encoded_json = _json_dumps(payload)
    json_tokens = _count_tokens(encoded_json)
    result["tokens"]["json"] = json_tokens

    # 3) Quick structural heuristic: prefer TOON for uniform tabular arrays
    structural_prefers_toon = False
    if _is_uniform_tabular_array(payload):
        structural_prefers_toon = True

    # 4) Attempt TOON encoding (best-effort). Use default compact options unless caller provided them.
    toon_opts = opts.get("toon_options", TOON_DEFAULT_OPTIONS)
    toon_tokens = None
    toon_encoded = None
    toon_err = None
    try:
        toon_encoded = _toon_encode(payload, toon_opts)
        toon_tokens = _count_tokens(toon_encoded)
    except Exception as exc:
        toon_err = f"{type(exc).__name__}: {exc}"
        toon_tokens = None
        toon_encoded = None

    result["tokens"]["toon"] = toon_tokens
    result["options"] = toon_opts

    # 5) Decision rule
    chosen_format = "JSON"
    reason = ""

    if toon_tokens is not None:
        # If structural heuristic strongly prefers TOON, relax threshold slightly
        threshold = SAVE_THRESHOLD * (0.85 if structural_prefers_toon else 1.0)
        # compute savings (json -> toon, positive means savings)
        savings_pct = (
            (json_tokens - toon_tokens) / json_tokens if json_tokens > 0 else 0.0
        )
        result["savings_pct"] = savings_pct * 100.0
        if savings_pct >= threshold:
            chosen_format = "TOON"
            reason = (
                "savings_above_threshold_structural"
                if structural_prefers_toon
                else "savings_above_threshold"
            )
        else:
            chosen_format = "JSON"
            reason = "no_savings"
    else:
        # TOON encode failed — keep JSON
        chosen_format = "JSON"
        reason = f"toon_encode_failed: {toon_err}"

    # 6) If the chosen format is TOML in some future policy (not currently auto-picked),
    #    callers can request TOML explicitly via options. For now, prefer JSON for fidelity.
    if options and options.get("force_toml"):
        # Caller requested TOML for config-shaped payloads
        # The spike PRD expects TOML for SoulBox; we do not implement full TOML encoding here.
        # We return JSON and mark the reason — actual prompt-composer will convert TOML at storage time.
        chosen_format = "TOML"
        reason = "caller_forced_toml"
        # encode as JSON as placeholder
        chosen_encoded = encoded_json
    else:
        chosen_encoded = toon_encoded if chosen_format == "TOON" else encoded_json

    # Build result
    result.update(
        {
            "format": chosen_format,
            "encoded": chosen_encoded,
            "reason": reason,
        }
    )

    # Telemetry event
    event = {
        "event": "pc.encoding_choice",
        "timestamp_utc": int(time.time() * 1000),
        "format_chosen": result["format"],
        "reason_code": reason,
        "json_tokens": json_tokens,
        "toon_tokens": toon_tokens,
        "savings_pct": result.get("savings_pct"),
        "payload_signature": _payload_fingerprint(payload),
        "model_id": model_profile.get("model_id") if model_profile else None,
        "runtime_ms": int((time.time() - t0) * 1000),
    }
    result["telemetry"] = event

    # Emit to local telemetry store (no-op unless enabled)
    _emit_telemetry(event)

    return result


# -------------------------
# Example usage / quick test harness
# -------------------------
if __name__ == "__main__":
    # Simple smoke test when run directly
    example_payload = {
        "soulbox": {
            "schema_version": 3,
            "personality": {"warmth": 0.72, "openness": 0.65},
        },
        "short_term": [
            {"content": "User asked about Rust async patterns", "weight": 0.9}
        ],
    }
    model_profile = {"model_id": "llama3.1:latest", "effective_context_window": 6144}
    result = choose_encoding(example_payload, model_profile)
    print("Choice result:")
    print(json.dumps({k: v for k, v in result.items() if k != "encoded"}, indent=2))
    print("Encoded preview:")
    if result.get("encoded"):
        print(result["encoded"][:1000])
    else:
        print("<no encoded output>")
