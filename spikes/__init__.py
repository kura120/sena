"""
spikes package initializer.

Purpose:
- Make `spikes/` an importable package.
- Expose a convenient `tools` accessor for developer helpers (e.g. debug UI,
  encoding selector) while remaining import-safe when those modules or
  dependencies are missing.

Design notes (why):
- Many spike helper modules are optional developer tools and may import
  heavyweight or environment-specific dependencies. Import-time failures
  must not break normal spike execution or CI runs that don't use the tools.
- To keep import-time behavior predictable we attempt a best-effort import
  of `spikes.tools` and otherwise expose a small safe accessor function
  `get_tool(module_name)` that lazily imports a submodule on demand.
"""

from __future__ import annotations

import importlib
import types
from types import ModuleType
from typing import Optional

__all__ = ["tools", "get_tool"]


# Try to import the tools package (best-effort). If the package is not
# available (e.g. during partial checkouts or restricted CI), fall back
# to a safe proxy object so callers can still import `spikes` without error.
try:
    # Relative import to prefer package-local `spikes.tools`
    tools: Optional[ModuleType] = importlib.import_module(".tools", package=__name__)
except Exception:
    tools = None  # type: ignore


def get_tool(module_name: str) -> Optional[ModuleType]:
    """
    Lazily import and return a module from the `spikes.tools` package.

    Args:
        module_name: The short module name inside `spikes.tools`, e.g. "debug_ui"
                     or "pc_encoding_selector".

    Returns:
        The imported module on success, or None if the import failed.
    """
    full_name = f"{__name__}.tools.{module_name}"
    try:
        return importlib.import_module(full_name)
    except Exception:
        return None


# If the `tools` package wasn't importable at package-import-time, provide a
# small convenience namespace so callers can still do `from spikes import tools`
# and call `tools.get(...)`. This avoids conditional call-sites across the repo.
if tools is None:
    _tools_ns = types.SimpleNamespace()

    def _tools_get(name: str) -> Optional[ModuleType]:
        return get_tool(name)

    # Expose the lazy getter on the proxy namespace
    setattr(_tools_ns, "get", _tools_get)

    # Provide a small __repr__ so debugging prints are clearer
    def _proxy_repr() -> str:  # pragma: no cover - trivial helper
        return "<spikes.tools proxy (lazy loader)>"

    setattr(_tools_ns, "__repr__", _proxy_repr)  # type: ignore
    tools = _tools_ns  # type: ignore
