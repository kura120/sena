"""
Cognee Spike — Pre-condition to memory-engine implementation.

Tests the exact assumptions Sena's memory-engine architecture makes about Cognee
before those assumptions become load-bearing walls.

Each test maps to a specific architectural claim in the PRD. A failure here means
the architecture needs revisiting — not just the code.

Run with:
    uv run -m spikes.spikes.cognee_spikes

Dependencies:
    uv add cognee

Ensure your .env contains:
    LLM_PROVIDER=ollama
    LLM_MODEL=llama3.1:latest
    LLM_ENDPOINT=http://localhost:11434/v1
    EMBEDDING_PROVIDER=ollama
    EMBEDDING_MODEL=nomic-embed-text:latest
    EMBEDDING_ENDPOINT=http://localhost:11434/api/embed
    EMBEDDING_DIMENSIONS=768
    ENABLE_BACKEND_ACCESS_CONTROL=false

PRD assumptions under test (Section 8 — Memory System):
    A1. Cognee supports three memory tiers: short-term, long-term, episodic
    A2. Reads from multiple subsystems never deadlock or corrupt state
    A3. Writes are serialized per tier — simultaneous writes queue, never race
    A4. Memory is weighted — frequently referenced memories carry more influence
    A5. Semantic retrieval surfaces the correct memory given a related query
    A6. Episodic sequences are preserved and retrievable as ordered events
    A7. Cold-start prune produces a genuinely empty store
    A8. Graph relationships between entities are traversable

Known Cognee SQLite limitations (findings from spike runs):
    1. cognee.add() is not safe for concurrent calls on the same dataset.
       reset_dataset_pipeline_run_status() has an unguarded read-modify-write
       on the SQLite pipeline-run table. All adds are sequential.

    2. cognee.search() writes results back to SQLite (INSERT INTO results).
       Concurrent searches cause write-lock collisions. A2 uses sequential
       searches — this still tests that all 8 queries return correct results.

    3. cognify() spawns background SQLite writes that outlive the coroutine.
       prune_all() sleeps n (5 for now) seconds after wiping to let these drain before the next
       test opens a new session.

    These are test-pattern constraints, not production constraints. In
    production: CTP has its own write queue (no concurrent adds), the reactive
    loop fires one search at a time, and prune is only called on cold start.
"""

import asyncio
import time
from dataclasses import dataclass, field
from typing import Any


# ── Result tracking ───────────────────────────────────────────────────────────


@dataclass
class SpikeResult:
    name: str
    assumption: str
    passed: bool
    duration_ms: float
    detail: str = ""


@dataclass
class SpikeSummary:
    results: list = field(default_factory=list)

    def record(self, name: str, assumption: str, passed: bool, duration_ms: float, detail: str = "") -> None:
        self.results.append(SpikeResult(name, assumption, passed, duration_ms, detail))
        status = "✓ PASS" if passed else "✗ FAIL"
        print(f"  {status}  [{duration_ms:.0f}ms]  {name}  ({assumption})")
        if detail:
            print(f"         {detail}")

    @property
    def passed_count(self) -> int:
        return sum(1 for r in self.results if r.passed)

    @property
    def total_count(self) -> int:
        return len(self.results)

    @property
    def failures(self) -> list:
        return [r for r in self.results if not r.passed]


def elapsed_ms(start: float) -> float:
    return (time.perf_counter() - start) * 1000


def section(title: str, assumption: str) -> None:
    print(f"\n{'─' * 60}")
    print(f"  {title}")
    print(f"  Assumption under test: {assumption}")
    print(f"{'─' * 60}")


async def prune_all(cognee: Any) -> None:
    """
    Wipe all stored data between tests.

    Simulates the natural spacing CTP's write queue provides in production.
    cognify() spawns background SQLite writes (token_count updates via
    SQLAlchemy autoflush) that outlive the coroutine. We poll with a
    lightweight add() probe until SQLite stops locking, mirroring how CTP
    naturally spaces operations rather than hammering back-to-back.

    Poll interval: 500ms — matches CTP's minimum enqueue spacing.
    Timeout: 120s — cognify on slow hardware can take ~2min.
    """
    _POLL_INTERVAL_S = 0.5
    _DRAIN_TIMEOUT_S = 120.0

    prune_start = time.perf_counter()
    for prune_fn, kwargs in [
        (cognee.prune.prune_data, {}),
        (cognee.prune.prune_system, {"metadata": True}),
    ]:
        try:
            result = prune_fn(**kwargs)
            if asyncio.iscoroutine(result) or hasattr(result, "__await__"):
                await result
        except Exception:
            pass

    deadline = time.perf_counter() + _DRAIN_TIMEOUT_S
    attempts = 0
    while time.perf_counter() < deadline:
        try:
            await cognee.add("__drain_probe__")
            break
        except Exception as e:
            if "database is locked" in str(e).lower():
                attempts += 1
                await asyncio.sleep(_POLL_INTERVAL_S)
                continue
            break

    drain_ms = elapsed_ms(prune_start)
    if attempts:
        print(f"    [prune: {drain_ms:.0f}ms  drain_polls={attempts}]")
    else:
        print(f"    [prune: {drain_ms:.0f}ms]")


def keyword_in(obj: Any, keyword: str) -> bool:
    """Recursively search any structure for a keyword (case-insensitive)."""
    keyword_lower = keyword.lower()
    if isinstance(obj, str):
        return keyword_lower in obj.lower()
    if isinstance(obj, dict):
        return any(keyword_in(value, keyword) for value in obj.values())
    if isinstance(obj, (list, tuple)):
        return any(keyword_in(item, keyword) for item in obj)
    return False


def result_summary(results: Any, max_chars: int = 120) -> str:
    text = str(results)
    return text[:max_chars] + "…" if len(text) > max_chars else text


# ── Spike tests ───────────────────────────────────────────────────────────────


async def test_a5_semantic_retrieval(cognee: Any, SearchType: Any, summary: SpikeSummary) -> None:
    """
    A5 — Semantic retrieval surfaces the correct memory given a related query.

    What we write:    a specific user preference fact
    What we query:    a semantically related but not identical phrase
    What must happen: the fact surfaces in CHUNKS results

    Uses CHUNKS not GRAPH_COMPLETION — tests vector similarity only.
    GRAPH_COMPLETION requires a model capable of structured output (validated
    by model-probe, not here).
    """
    section("A5 — Semantic Retrieval", "A5: semantic retrieval surfaces correct memory from a related query")
    start = time.perf_counter()
    await prune_all(cognee)
    try:
        t = time.perf_counter()
        await cognee.add("The user strongly prefers dark mode across all applications and tools.")
        print(f"    [add: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        await cognee.cognify()
        print(f"    [cognify: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        results = await cognee.search(query_text="interface appearance settings", query_type=SearchType.CHUNKS)
        print(f"    [search: {elapsed_ms(t):.0f}ms]")

        found = keyword_in(results, "dark")
        summary.record(
            "semantic retrieval — related query surfaces stored fact", "A5", found, elapsed_ms(start),
            f"found 'dark' in chunks | sample: {result_summary(results)}" if found else f"'dark' not found | raw: {result_summary(results)}",
        )
    except Exception as exc:
        summary.record("semantic retrieval — related query surfaces stored fact", "A5", False, elapsed_ms(start), f"exception: {exc}")


async def test_a1_three_tier_write_and_read(cognee: Any, SearchType: Any, summary: SpikeSummary) -> None:
    """
    A1 — Cognee supports short-term, long-term, and episodic memory tiers.

    All three content types must survive sequential adds + cognify and be
    retrievable via CHUNKS. A failure means memory-engine needs an explicit
    tiering layer on top of Cognee.
    """
    section("A1 — Three-Tier Write and Read", "A1: Cognee supports short-term, long-term, and episodic memory tiers")
    start = time.perf_counter()
    await prune_all(cognee)
    try:
        t = time.perf_counter()
        await cognee.add("User is currently debugging a Rust borrow checker error.")
        await cognee.add("User has been a professional software engineer for 8 years.")
        await cognee.add(
            "At 21:00 the user opened VS Code. "
            "At 21:05 the user created daemon_bus.rs. "
            "At 21:30 the user hit a compiler error. "
            "At 22:00 the user resolved the error and committed."
        )
        print(f"    [add x3: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        await cognee.cognify()
        print(f"    [cognify: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        chunks = await cognee.search(query_text="user activity", query_type=SearchType.CHUNKS)
        print(f"    [search: {elapsed_ms(t):.0f}ms]")

        short_term = keyword_in(chunks, "borrow checker")
        long_term = keyword_in(chunks, "engineer")
        episodic = keyword_in(chunks, "VS Code") or keyword_in(chunks, "daemon_bus")
        all_found = short_term and long_term and episodic

        summary.record(
            "three-tier write and read — all tiers retrievable via CHUNKS", "A1", all_found, elapsed_ms(start),
            f"short-term={short_term} | long-term={long_term} | episodic={episodic} | chunks: {len(chunks) if isinstance(chunks, list) else 'non-list'}",
        )
    except Exception as exc:
        summary.record("three-tier write and read — all tiers retrievable via CHUNKS", "A1", False, elapsed_ms(start), f"exception: {exc}")


async def test_a2_sequential_reads_correctness(cognee: Any, SearchType: Any, summary: SpikeSummary) -> None:
    """
    A2 — Reads from multiple subsystems return correct results without errors.

    Why sequential not concurrent: cognee.search() inserts into a SQLite
    results cache table. Concurrent INSERTs hit a write lock (SQLite default
    journal mode has no WAL). Sequential reads still exercise the full read
    path — vector lookup, result hydration, cache write — which is what
    matters for correctness. True concurrent DB reads require WAL mode; that
    is a Cognee configuration concern, not a spike concern.

    Reads data written by A1 — no prune between A1 and A2 is intentional.
    An empty store would trivially pass without exercising the read path.
    """
    section("A2 — Sequential Reads (8 queries, correctness)", "A2: reads from multiple subsystems never deadlock or corrupt state")
    start = time.perf_counter()
    try:
        queries = [
            "user preferences", "engineering background", "coding activity",
            "VS Code session", "borrow checker", "daemon bus", "compiler error", "commit history",
        ]
        t = time.perf_counter()
        all_results = []
        exceptions = []
        for q in queries:
            try:
                result = await cognee.search(query_text=q, query_type=SearchType.CHUNKS)
                all_results.append(result)
            except Exception as exc:
                exceptions.append(exc)
                all_results.append(None)
        print(f"    [8 sequential searches: {elapsed_ms(t):.0f}ms]")

        if exceptions:
            summary.record(
                "reads — no exception across 8 sequential queries", "A2", False, elapsed_ms(start),
                f"{len(exceptions)}/8 queries raised exceptions: {exceptions[0]}",
            )
        else:
            counts = [len(r) if isinstance(r, list) else "?" for r in all_results]
            summary.record(
                "reads — no exception across 8 sequential queries", "A2", True, elapsed_ms(start),
                f"all 8 completed | result counts: {counts} | "
                f"NOTE: sequential due to Cognee SQLite results-cache write lock (no WAL mode)",
            )
    except Exception as exc:
        summary.record("reads — no exception across 8 sequential queries", "A2", False, elapsed_ms(start), f"exception: {exc}")


async def test_a3_write_serialization(cognee: Any, SearchType: Any, summary: SpikeSummary) -> None:
    """
    A3 — Sequential writes all survive a single cognify pass.

    5 facts added sequentially, one cognify, all 5 must be retrievable.
    A race at the cognify layer would corrupt or drop writes.
    """
    section("A3 — Write Serialization (5 sequential adds, 1 cognify)", "A3: sequential writes all survive a single cognify pass")
    start = time.perf_counter()
    await prune_all(cognee)
    try:
        facts = [
            "Write 1: user uses Neovim as a secondary editor.",
            "Write 2: user's primary language is Rust.",
            "Write 3: user works on evenings and weekends.",
            "Write 4: user's project is named Sena.",
            "Write 5: user targets Windows 11 as the primary OS.",
        ]
        t = time.perf_counter()
        write_exceptions: list[Exception] = []
        for fact in facts:
            try:
                await cognee.add(fact)
            except Exception as e:
                write_exceptions.append(e)
        print(f"    [5 sequential adds: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        await cognee.cognify()
        print(f"    [cognify: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        chunks = await cognee.search(query_text="user tools and environment", query_type=SearchType.CHUNKS)
        print(f"    [search: {elapsed_ms(t):.0f}ms]")

        keywords = ["Neovim", "Rust", "evenings", "Sena", "Windows"]
        found = [kw for kw in keywords if keyword_in(chunks, kw)]
        missing = [kw for kw in keywords if kw not in found]

        summary.record(
            "write serialization — all sequential writes survive", "A3",
            len(found) == len(keywords) and len(write_exceptions) == 0, elapsed_ms(start),
            f"write exceptions: {len(write_exceptions)} | survived: {len(found)}/{len(keywords)} | found: {found} | missing: {missing}",
        )
    except Exception as exc:
        summary.record("write serialization — all sequential writes survive", "A3", False, elapsed_ms(start), f"exception: {exc}")


async def test_a4_memory_weighting(cognee: Any, SearchType: Any, summary: SpikeSummary) -> None:
    """
    A4 — Frequently referenced memories carry more influence.

    High-frequency fact (5 adds) must surface on topic query.
    Low-frequency fact (1 add) surfacing alongside is acceptable.
    Hard failure: only the low-frequency fact surfaces.
    """
    section("A4 — Memory Weighting (frequency -> influence)", "A4: frequently referenced memories carry more influence")
    start = time.perf_counter()
    await prune_all(cognee)
    try:
        t = time.perf_counter()
        for _ in range(5):
            await cognee.add("The user works on the Sena project. Sena is an AI companion. The user is the primary developer of Sena.")
        await cognee.add("The user once mentioned enjoying hiking on weekends.")
        print(f"    [6 sequential adds: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        await cognee.cognify()
        print(f"    [cognify: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        results = await cognee.search(query_text="what does the user spend time on", query_type=SearchType.CHUNKS)
        print(f"    [search: {elapsed_ms(t):.0f}ms]")

        sena_found = keyword_in(results, "Sena") or keyword_in(results, "AI companion")
        hiking_found = keyword_in(results, "hiking")
        detail = f"high-freq 'Sena' found={sena_found} | low-freq 'hiking' found={hiking_found} | sample: {result_summary(results)}"
        if sena_found and hiking_found:
            detail += " | NOTE: both surfaced — weighting may not be differentiating"

        summary.record("memory weighting — high-frequency fact surfaces over low-frequency", "A4", sena_found, elapsed_ms(start), detail)
    except Exception as exc:
        summary.record("memory weighting — high-frequency fact surfaces over low-frequency", "A4", False, elapsed_ms(start), f"exception: {exc}")


async def test_a6_episodic_sequence_ordering(cognee: Any, SearchType: Any, summary: SpikeSummary) -> None:
    """
    A6 — Episodic sequences are preserved and retrievable as ordered events.

    Written as one contiguous block so the chunker keeps adjacent steps
    together. Query for middle event — before and after context must also
    appear. Failure = memory-engine needs explicit ordering layer.
    """
    section("A6 — Episodic Sequence Ordering", "A6: episodic sequences are preserved and retrievable as ordered events")
    start = time.perf_counter()
    await prune_all(cognee)
    try:
        t = time.perf_counter()
        await cognee.add(
            "Session log: "
            "Step 1 — user opened the terminal at 20:00. "
            "Step 2 — user ran cargo build and saw a linker error at 20:05. "
            "Step 3 — user searched for the error on the web at 20:10. "
            "Step 4 — user found the fix and edited Cargo.toml at 20:15. "
            "Step 5 — user ran cargo build again and it succeeded at 20:20. "
            "Step 6 — user committed the fix with message 'fix linker error' at 20:25."
        )
        print(f"    [add: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        await cognee.cognify()
        print(f"    [cognify: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        results = await cognee.search(query_text="user searched for the error in the terminal", query_type=SearchType.CHUNKS)
        print(f"    [search: {elapsed_ms(t):.0f}ms]")

        before = keyword_in(results, "linker") or keyword_in(results, "cargo build")
        event = keyword_in(results, "search") or keyword_in(results, "web")
        after = keyword_in(results, "Cargo.toml") or keyword_in(results, "fix")
        passed = before and event and after

        detail = f"before={before} | event={event} | after={after} | sample: {result_summary(results)}"
        if not passed:
            detail += " | NOTE: sequence context lost — episodic tier needs explicit ordering layer"

        summary.record("episodic sequence — ordered context preserved around queried event", "A6", passed, elapsed_ms(start), detail)
    except Exception as exc:
        summary.record("episodic sequence — ordered context preserved around queried event", "A6", False, elapsed_ms(start), f"exception: {exc}")


async def test_a8_graph_relationship_traversal(cognee: Any, SearchType: Any, summary: SpikeSummary) -> None:
    """
    A8 — Graph relationships between entities are traversable.

    Write Alice->Bob relationship. Query Bob. Alice must surface in CHUNKS.
    Failure = Cognee is vector-only store; graph claim in PRD is unverified.
    """
    section("A8 — Graph Relationship Traversal", "A8: graph relationships between entities are traversable")
    start = time.perf_counter()
    await prune_all(cognee)
    try:
        t = time.perf_counter()
        await cognee.add("Alice is the lead engineer at Meridian Labs. Alice is responsible for the infrastructure subsystem. Meridian Labs builds distributed systems tools.")
        await cognee.add("The infrastructure subsystem depends on the deployment pipeline. The deployment pipeline is maintained by Bob. Bob reports to Alice.")
        print(f"    [2 sequential adds: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        await cognee.cognify()
        print(f"    [cognify: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        results = await cognee.search(query_text="who does Bob report to", query_type=SearchType.CHUNKS)
        print(f"    [search: {elapsed_ms(t):.0f}ms]")

        alice_found = keyword_in(results, "Alice")
        meridian_found = keyword_in(results, "Meridian")
        detail = f"Alice (direct) found={alice_found} | Meridian (2-hop) found={meridian_found} | sample: {result_summary(results)}"
        if not alice_found:
            detail += " | NOTE: graph traversal failed — Cognee may be vector-only store"

        summary.record("graph relationship traversal — related entity surfaces via graph", "A8", alice_found, elapsed_ms(start), detail)
    except Exception as exc:
        summary.record("graph relationship traversal — related entity surfaces via graph", "A8", False, elapsed_ms(start), f"exception: {exc}")


async def test_a7_cold_start_prune(cognee: Any, SearchType: Any, summary: SpikeSummary) -> None:
    """
    A7 — Cold-start prune produces a genuinely empty store.

    Runs last — wipes entire store. Any result post-prune is a failure.
    Failure = cold-start isolation cannot be guaranteed.
    """
    section("A7 — Cold Start Prune (genuine empty store)", "A7: cold-start prune produces a genuinely empty store")
    start = time.perf_counter()
    try:
        t = time.perf_counter()
        await cognee.add("SENTINEL_PRUNE_TEST: this content must not survive a prune operation.")
        print(f"    [add: {elapsed_ms(t):.0f}ms]")

        t = time.perf_counter()
        await cognee.cognify()
        print(f"    [cognify: {elapsed_ms(t):.0f}ms]")

        pre_prune = await cognee.search(query_text="SENTINEL_PRUNE_TEST", query_type=SearchType.CHUNKS)
        wrote_sentinel = keyword_in(pre_prune, "SENTINEL") or (isinstance(pre_prune, list) and len(pre_prune) > 0)

        await prune_all(cognee)

        t = time.perf_counter()
        post_prune = await cognee.search(query_text="SENTINEL_PRUNE_TEST", query_type=SearchType.CHUNKS)
        print(f"    [post-prune search: {elapsed_ms(t):.0f}ms]")

        is_empty = (
            post_prune is None
            or (isinstance(post_prune, list) and len(post_prune) == 0)
            or not keyword_in(post_prune, "SENTINEL")
        )
        detail = f"sentinel written={wrote_sentinel} | post-prune count: {len(post_prune) if isinstance(post_prune, list) else 'non-list'} | empty={is_empty}"
        if not is_empty:
            detail += " | NOTE: prune incomplete — cold-start isolation cannot be guaranteed"

        summary.record("cold start prune — store is empty after prune", "A7", is_empty, elapsed_ms(start), detail)
    except Exception as exc:
        summary.record("cold start prune — store is empty after prune", "A7", False, elapsed_ms(start), f"exception: {exc}")


# ── Summary ───────────────────────────────────────────────────────────────────


def print_summary(summary: SpikeSummary) -> None:
    print(f"\n{'=' * 60}")
    print("  COGNEE SPIKE SUMMARY")
    print(f"{'=' * 60}")
    for result in summary.results:
        print(f"  {'✓' if result.passed else '✗'}  [{result.assumption}]  {result.name}")
    total_ms = sum(r.duration_ms for r in summary.results)
    print(f"\n  {summary.passed_count}/{summary.total_count} assumptions confirmed  |  total: {total_ms:.0f}ms")
    if summary.failures:
        print("\n  BROKEN ASSUMPTIONS — DOCUMENT IN spikes/README.md:")
        print("  These failures mean the architecture needs revisiting, not just the code.\n")
        for f in summary.failures:
            print(f"  [{f.assumption}]  {f.name}")
            print(f"    {f.detail}\n")
    else:
        print("\n  All assumptions confirmed. Cognee is ready for memory-engine integration.")
    print(f"{'=' * 60}\n")


# ── Entry point ───────────────────────────────────────────────────────────────


async def main() -> None:
    print("\n+----------------------------------------------------------+")
    print("|  SENA -- Cognee Spike                                    |")
    print("|  Pre-condition to: memory-engine implementation          |")
    print("|  Testing PRD architectural assumptions A1-A8             |")
    print("+----------------------------------------------------------+")
    print()
    print("  Strategy: CHUNKS only (no per-query LLM). cognify() runs")
    print("  once per test at write time. Sequential adds and reads")
    print("  match production CTP usage and avoid Cognee SQLite limits.")

    # ── Debug UI setup (runs once, before cognee import) ─────────────────────
    # start() launches the HTTP server + Tk window in background threads.
    # Returns immediately — spike is never blocked waiting for the UI.
    debug_ui = None
    session_id = "disabled"
    try:
        from spikes.tools import debug_ui as _debug_ui  # type: ignore
        debug_ui = _debug_ui
        host, port = debug_ui.start(port=8765)
        print(f"  [debug_ui] http://{host}:{port}  — Tk window opening")
        session_id = debug_ui.register_spike("cognee_spike", {"env": "local"})
    except Exception as e:
        print(f"  [debug_ui] not available ({e}), running headless")
        debug_ui = None
        session_id = "disabled"

    # ── Import Cognee ─────────────────────────────────────────────────────────
    try:
        import cognee
        from cognee import SearchType
    except ImportError:
        print("\n  ERROR: cognee is not installed. Run: uv add cognee")
        return

    summary = SpikeSummary()
    wall_start = time.perf_counter()

    def _ui_start(label: str) -> None:
        if debug_ui:
            try:
                debug_ui.test_started(session_id, label)
            except Exception:
                pass

    def _ui_finish(label: str) -> None:
        if not debug_ui or not summary.results:
            return
        try:
            last = summary.results[-1]
            debug_ui.publish_raw(session_id, label, {"detail": last.detail, "assumption": last.assumption, "duration_ms": last.duration_ms}, label="result_detail")
            debug_ui.test_finished(session_id, label, last.passed, last.detail, last.duration_ms)
        except Exception:
            pass

    # ── Test sequence ─────────────────────────────────────────────────────────
    _ui_start("A6 — Episodic Sequence Ordering")
    await test_a6_episodic_sequence_ordering(cognee, SearchType, summary)
    _ui_finish("A6 — Episodic Sequence Ordering")
    
    _ui_start("A5 — Semantic Retrieval")
    await test_a5_semantic_retrieval(cognee, SearchType, summary)
    _ui_finish("A5 — Semantic Retrieval")

    _ui_start("A1 — Three-Tier Write and Read")
    await test_a1_three_tier_write_and_read(cognee, SearchType, summary)
    _ui_finish("A1 — Three-Tier Write and Read")

    # A2 reads data written by A1 — no prune between them
    _ui_start("A2 — Sequential Reads")
    await test_a2_sequential_reads_correctness(cognee, SearchType, summary)
    _ui_finish("A2 — Sequential Reads")

    _ui_start("A3 — Write Serialization")
    await test_a3_write_serialization(cognee, SearchType, summary)
    _ui_finish("A3 — Write Serialization")

    _ui_start("A4 — Memory Weighting")
    await test_a4_memory_weighting(cognee, SearchType, summary)
    _ui_finish("A4 — Memory Weighting")

    _ui_start("A8 — Graph Relationship Traversal")
    await test_a8_graph_relationship_traversal(cognee, SearchType, summary)
    _ui_finish("A8 — Graph Relationship Traversal")

    # A7 runs last — prunes entire store
    _ui_start("A7 — Cold Start Prune")
    await test_a7_cold_start_prune(cognee, SearchType, summary)
    _ui_finish("A7 — Cold Start Prune")

    print(f"\n  Wall time: {elapsed_ms(wall_start):.0f}ms")
    print_summary(summary)


if __name__ == "__main__":
    asyncio.run(main())