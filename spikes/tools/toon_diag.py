#!/usr/bin/env python3
"""
toon_diag.py — lightweight diagnostic for installed TOON implementation.

Run from the project virtualenv:
    uv run python spikes/toon_diag.py

What this script does:
- Attempts to import `toon` then `toon_format`.
- Prints module path, public attributes, and reported version.
- Checks for `encode` / `decode` and attempts a small encode call.
- Prints source locations where available and any exceptions/tracebacks.
- Detects common "not implemented" stub message and highlights it.

This is a read-only diagnostic — it will not modify your environment.
"""

from __future__ import annotations

import importlib
import inspect
import pkgutil
import sys
import traceback
from types import ModuleType
from typing import Optional


def short_repr(obj, maxlen: int = 400) -> str:
    try:
        r = repr(obj)
    except Exception:
        return f"<unrepresentable {type(obj).__name__}>"
    if len(r) <= maxlen:
        return r
    return r[: maxlen - 1] + "…"


def find_candidate_packages() -> list[str]:
    """List installed top-level modules that contain 'toon' in their name."""
    names = []
    for m in pkgutil.iter_modules():
        try:
            if "toon" in m.name.lower():
                names.append(m.name)
        except Exception:
            continue
    return names


def inspect_attr(obj, name: str):
    attr = getattr(obj, name, None)
    exists = attr is not None
    print(f"  - {name}: {'present' if exists else 'MISSING'}")
    if not exists:
        return
    print(f"    type: {type(attr)!r}")
    # Try to find source file / source text
    try:
        sf = inspect.getsourcefile(attr) or inspect.getfile(attr)
    except Exception:
        sf = None
    if sf:
        print(f"    source file: {sf}")
    else:
        print(f"    source file: (not found or built-in / C extension)")
    # Try to print small snippet of source for Python functions
    try:
        src = inspect.getsource(attr)
        print("    --- source snippet ---")
        print("\n".join(src.splitlines()[:20]))
        if len(src.splitlines()) > 20:
            print("    ... (truncated)")
        print("    --- end snippet ---")
    except Exception:
        # Not printable (likely built-in or C extension)
        pass


def try_encode_call(mod: ModuleType):
    """Attempt a small encode call and report results or exception."""
    if not hasattr(mod, "encode"):
        print("  encode() not available — skipping call attempt.")
        return
    print("  Attempting: mod.encode({'test': 1})")
    try:
        out = mod.encode({"test": 1})
        print("  -> encode returned type:", type(out))
        print("  -> repr (truncated):", short_repr(out, 800))
        # If decode exists, try roundtrip
        if hasattr(mod, "decode"):
            try:
                decoded = mod.decode(out)
                roundtrip_ok = decoded == {"test": 1}
                print(
                    f"  -> decode returned type: {type(decoded)} | roundtrip_ok={roundtrip_ok}"
                )
                if not roundtrip_ok:
                    print("     decode output (truncated):", short_repr(decoded, 800))
            except Exception:
                print("  -> decode() raised an exception:")
                traceback.print_exc(limit=5)
    except Exception as exc:
        print("  -> encode() raised an exception:")
        # Print traceback, but check for common NotImplemented message
        tb_lines = traceback.format_exc().splitlines()
        for l in tb_lines[:10]:
            print("    " + l)
        # Check message for common stub patterns
        msg = str(exc).lower()
        if "not implemented" in msg or "not yet implemented" in msg:
            print(
                "  NOTE: The encode implementation appears to be a stub (NotImplemented)."
            )
        elif "not available" in msg:
            print("  NOTE: encode claims functionality is not available in this build.")
        # end


def inspect_module(name: str) -> Optional[ModuleType]:
    try:
        mod = importlib.import_module(name)
    except Exception as exc:
        print(f"Module '{name}' import FAILED: {type(exc).__name__}: {exc}")
        return None

    print(f"\n=== Inspected module: {name} ===")
    print("module file:", getattr(mod, "__file__", "(package built-in or namespace)"))
    version = getattr(mod, "__version__", None) or getattr(mod, "VERSION", None)
    print("reported version attribute:", version)
    public = [n for n in dir(mod) if not n.startswith("_")]
    print(
        "public names (sample):",
        ", ".join(public[:50]) + (", ..." if len(public) > 50 else ""),
    )
    # Print top-level info for encode/decode
    inspect_attr(mod, "encode")
    inspect_attr(mod, "decode")
    # Attempt encode call
    try_encode_call(mod)

    # If module is a package, attempt to list package files (best-effort)
    pkg_file = getattr(mod, "__file__", None)
    if pkg_file:
        try:
            import os

            pkg_root = os.path.dirname(pkg_file)
            print(f"package dir: {pkg_root}")
            files = sorted(
                [
                    f
                    for f in os.listdir(pkg_root)
                    if f.endswith(".py") or f.endswith(".so") or f.endswith(".pyd")
                ]
            )
            if files:
                print(
                    "  package files:",
                    ", ".join(files[:20]) + (", ..." if len(files) > 20 else ""),
                )
        except Exception:
            pass

    print(f"=== end module: {name} ===\n")
    return mod


def main():
    print("\nTOON diagnostic — scanning environment\n")
    candidates = find_candidate_packages()
    if candidates:
        print("Candidate installed packages matching 'toon':", candidates)
    else:
        print("No top-level packages with 'toon' in the name were found in sys.path.")

    # Try canonical names in order
    for mod_name in ("toon", "toon_format"):
        mod = inspect_module(mod_name)
        if mod:
            print(f"Using module '{mod_name}' for further checks.")
            break
    else:
        print(
            "\nNo usable 'toon' module found. If you have the package installed under a different name,"
        )
        print(
            "re-run this script with PYTHONPATH adjusted or tell me the exact import name."
        )
        print("Example install commands:")
        print("  uv add toon-format")
        print("  pip install toon-format")
        return

    # Extra: print environment snippets helpful for debugging
    print("\nPython executable:", sys.executable)
    print("sys.path (tail):")
    for p in sys.path[-6:]:
        print("  ", p)
    print("\nDiagnostic complete.\n")


if __name__ == "__main__":
    main()
