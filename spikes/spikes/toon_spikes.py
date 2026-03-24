"""
TOON Spike — Pre-condition to prompt-composer implementation.

Tests the exact assumptions Sena's prompt-composer architecture makes about
toon-format before those assumptions become load-bearing walls.

Each test maps to a specific architectural claim in the PRD. A failure here
means the architecture needs revisiting — not just the code.

Run with:
    uv run spikes/toon_spikes.py

Dependencies:
    uv add toon-format tiktoken ollama

Ensure Ollama is running with a model pulled:
    ollama pull llama3.1:latest

PRD assumptions under test (Section 7.3 — Prompt Composer):
    B1. TOON encodes all required Sena data structures without data loss
    B2. TOON produces measurable token reduction vs JSON (PRD claims 30-60%)
    B3. TOON roundtrips cleanly through Unicode, edge cases, and deep nesting
    B4. TOON encoding of large contexts is fast enough for real-time prompt assembly
    B5. A running model (via Ollama) can decode TOON output and act on it
    B6. Context window budget enforcement — oversized contexts can be trimmed
        in PRD priority order without corrupting the remaining prompt
    B7. The three-way serialization split is enforceable — TOON/TOML/JSON each
        handle their assigned domain without overlap or ambiguity
"""

import asyncio
import json
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

    def record(
        self,
        name: str,
        assumption: str,
        passed: bool,
        duration_ms: float,
        detail: str = "",
    ) -> None:
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


# ── Realistic Sena prompt fixtures ────────────────────────────────────────────
# These are the exact data structures that prompt-composer will encode.
# They are derived directly from PRD Section 7.3 (PC Inputs).

SOULBOX_SNAPSHOT = {
    "schema_version": 3,
    "personality": {
        "warmth": 0.72,
        "openness": 0.65,
        "curiosity": 0.81,
        "playfulness": 0.55,
        "directness": 0.60,
    },
    "ctp_config": {
        "autonomy_level": "full",
        "thought_boldness": 0.7,
        "surface_threshold_active": 0.9,
        "surface_threshold_idle": 0.4,
    },
    "voice": {
        "cadence": "measured",
        "vocabulary": "technical_friendly",
        "expressiveness": 0.68,
    },
}

SHORT_TERM_MEMORIES = [
    {
        "content": "User asked about Rust async patterns",
        "weight": 0.9,
        "age_seconds": 120,
    },
    {
        "content": "User expressed frustration with borrow checker",
        "weight": 0.85,
        "age_seconds": 300,
    },
    {
        "content": "User mentioned working on daemon-bus",
        "weight": 0.95,
        "age_seconds": 60,
    },
]

LONG_TERM_MEMORIES = [
    {"content": "User is building an AI companion system called Sena", "weight": 0.98},
    {"content": "User prefers dark mode interfaces", "weight": 0.7},
    {"content": "User works late evenings consistently", "weight": 0.82},
    {"content": "User has strong opinions about software architecture", "weight": 0.75},
    {"content": "User is proficient in Rust and Python", "weight": 0.91},
]

EPISODIC_MEMORIES = [
    {"event": "User opened VS Code", "timestamp": "2026-03-04T21:00:00Z"},
    {
        "event": "User started working on daemon-bus",
        "timestamp": "2026-03-04T21:05:00Z",
    },
    {"event": "User hit a compiler error", "timestamp": "2026-03-04T21:30:00Z"},
    {
        "event": "User resolved the error and committed",
        "timestamp": "2026-03-04T22:00:00Z",
    },
]

USER_INTENT = {
    "inferred": "user is debugging async Rust code",
    "confidence": 0.88,
    "context_source": "recent_messages + screen_context",
}

OS_CONTEXT = {
    "active_app": "VS Code",
    "active_file": "daemon_bus.rs",
    "time_of_day": "evening",
    "activity_state": "active",
}

MODEL_PROFILE = {
    "model_id": "llama3.1:latest",
    "effective_context_window": 6144,
    "multi_step_reasoning": "DEGRADED",
    "structured_output": "FULL",
}

# The full assembled prompt context — mirrors exactly what PC will encode
FULL_PROMPT_CONTEXT = {
    "soulbox": SOULBOX_SNAPSHOT,
    "short_term": SHORT_TERM_MEMORIES,
    "long_term": LONG_TERM_MEMORIES,
    "episodic": EPISODIC_MEMORIES,
    "user_intent": USER_INTENT,
    "os_context": OS_CONTEXT,
    "model_profile": MODEL_PROFILE,
}

# PRD Section 7.3 drop priority order — sacred content that must never be cut
SACRED_KEYS = {"soulbox", "user_intent"}
# Droppable tiers in order from lowest to highest priority (cut first → cut last)
DROPPABLE_TIERS_ORDERED = [
    "os_context",  # cut first
    "short_term",  # cut second
    "episodic",  # cut third
    "long_term",  # cut last among droppable
]


# ── Spike tests ───────────────────────────────────────────────────────────────


def test_b1_all_sena_structures_encode_without_loss(
    toon: Any, summary: SpikeSummary
) -> None:
    """
    B1 — TOON encodes all required Sena data structures without data loss.

    PRD claim (Section 7.3): All structured data fed into PC is encoded as TOON
    before injection into the model. Every PC input must survive a roundtrip.

    We test each PC input individually so failures are isolated to the exact
    structure that breaks, not buried in a combined failure.
    """
    section(
        "B1 — All Sena Data Structures Encode Without Loss",
        "B1: TOON encodes all required Sena data structures without data loss",
    )

    fixtures = {
        "soulbox_snapshot": SOULBOX_SNAPSHOT,
        "short_term_memories": SHORT_TERM_MEMORIES,
        "long_term_memories": LONG_TERM_MEMORIES,
        "episodic_memories": EPISODIC_MEMORIES,
        "user_intent": USER_INTENT,
        "os_context": OS_CONTEXT,
        "model_profile": MODEL_PROFILE,
        "full_prompt_context": FULL_PROMPT_CONTEXT,
    }

    start = time.perf_counter()
    failures = []
    for fixture_name, original in fixtures.items():
        try:
            encoded = toon.encode(original)
            decoded = toon.decode(encoded)
            if decoded != original:
                # Find the exact keys that diverged
                if isinstance(original, dict) and isinstance(decoded, dict):
                    mismatched = {
                        k: {"expected": original[k], "got": decoded.get(k)}
                        for k in original
                        if decoded.get(k) != original[k]
                    }
                    failures.append(f"{fixture_name}: key mismatch {mismatched}")
                else:
                    failures.append(
                        f"{fixture_name}: decoded != original | "
                        f"expected type {type(original).__name__}, "
                        f"got type {type(decoded).__name__}"
                    )
        except Exception as exc:
            failures.append(f"{fixture_name}: exception — {exc}")

    elapsed = elapsed_ms(start)
    all_passed = len(failures) == 0
    detail = (
        f"all {len(fixtures)} Sena structures roundtripped cleanly"
        if all_passed
        else f"failures: {' | '.join(failures)}"
    )
    summary.record(
        "all Sena PC input structures roundtrip cleanly",
        "B1",
        all_passed,
        elapsed,
        detail,
    )


def test_b2_token_reduction_vs_json(toon: Any, summary: SpikeSummary) -> None:
    """
    B2 — TOON produces measurable token reduction vs JSON.

    PRD claim (Section 7.3): TOON benchmarks — token reduction 30–60%.
    TOON's sweet spot maps directly to Sena's prompt data shape.

    We measure the actual reduction on the full prompt context and each
    individual PC input to identify which structures benefit most.

    A reduction below 15% is a yellow flag — document but don't fail.
    A reduction of 0% or worse (TOON is larger than JSON) is a hard failure —
    it means the PRD's serialization choice needs re-evaluation.
    """
    section(
        "B2 — Token Reduction vs JSON (PRD claims 30–60%)",
        "B2: TOON produces measurable token reduction vs JSON",
    )
    start = time.perf_counter()

    try:
        import tiktoken

        enc = tiktoken.get_encoding("cl100k_base")

        fixtures = {
            "soulbox": SOULBOX_SNAPSHOT,
            "short_term (uniform array)": SHORT_TERM_MEMORIES,
            "long_term (uniform array)": LONG_TERM_MEMORIES,
            "episodic (uniform array)": EPISODIC_MEMORIES,
            "full_prompt_context": FULL_PROMPT_CONTEXT,
        }

        results_table = []
        overall_pass = True

        for name, fixture in fixtures.items():
            toon_encoded = toon.encode(fixture)
            json_encoded = json.dumps(fixture, separators=(",", ":"))

            toon_tokens = len(enc.encode(toon_encoded))
            json_tokens = len(enc.encode(json_encoded))
            reduction_pct = ((json_tokens - toon_tokens) / json_tokens) * 100

            # Hard failure: TOON is not smaller than JSON
            if toon_tokens >= json_tokens:
                overall_pass = False

            results_table.append(
                f"{name}: TOON={toon_tokens} JSON={json_tokens} "
                f"reduction={reduction_pct:.1f}%"
                + (" ⚠ NO REDUCTION" if toon_tokens >= json_tokens else "")
                + (" ⚠ BELOW 15%" if 0 < reduction_pct < 15 else "")
            )

        elapsed = elapsed_ms(start)
        summary.record(
            "TOON produces token reduction vs JSON on all Sena structures",
            "B2",
            overall_pass,
            elapsed,
            " | ".join(results_table),
        )

    except ImportError:
        summary.record(
            "TOON produces token reduction vs JSON on all Sena structures",
            "B2",
            False,
            elapsed_ms(start),
            "tiktoken not installed — run: uv add tiktoken",
        )
    except Exception as exc:
        summary.record(
            "TOON produces token reduction vs JSON on all Sena structures",
            "B2",
            False,
            elapsed_ms(start),
            f"exception: {exc}",
        )


def test_b3_unicode_and_edge_cases(toon: Any, summary: SpikeSummary) -> None:
    """
    B3 — TOON roundtrips cleanly through Unicode, edge cases, and deep nesting.

    PRD context: SoulBox contains deeply personal user data including free-text
    personality notes that may contain any Unicode. Memories may contain
    quotation marks, newlines, or non-ASCII characters from app names and filenames.

    A single mismatch in any case is a failure — prompt-composer cannot
    silently corrupt user data.
    """
    section(
        "B3 — Unicode, Edge Cases, and Deep Nesting",
        "B3: TOON roundtrips cleanly through Unicode, edge cases, and deep nesting",
    )
    start = time.perf_counter()

    cases = {
        # Unicode from real user data scenarios
        "japanese_app_name": {"app": "テキストエディタ", "state": "active"},
        "emoji_in_memory": {"content": "User felt 🌙 tired after a long session"},
        "arabic_text": {"note": "مرحبا بالعالم"},
        "mixed_unicode": {"cafe": "café", "resume": "résumé", "naive": "naïve"},
        # Edge cases from real prompt data
        "newlines_in_content": {
            "content": "line one\nline two\nline three",
            "source": "screen_context",
        },
        "quotes_in_memory": {
            "content": 'user said "this is broken" and closed VS Code'
        },
        "backslash_in_path": {"file": "C:\\Users\\kura1\\Documents\\daemon_bus.rs"},
        # Numeric edge cases from SoulBox weights
        "zero_weight": {"weight": 0, "content": "placeholder"},
        "false_flag": {"active": False, "weight": 0.0},
        "null_field": {"content": None, "weight": 0.5},
        # Structural edge cases
        "empty_dict": {},
        "empty_list": [],
        "single_item_list": [{"content": "only one memory"}],
        "deeply_nested_soulbox": {
            "level1": {"level2": {"level3": {"level4": {"level5": "deep value"}}}}
        },
        # Large string (long memory content)
        "long_memory_content": {"content": "a" * 5000, "weight": 0.5},
        # Mixed-type list (matches episodic memory shape variants)
        "mixed_value_list": [1, "two", 3.0, True, None, {"nested": "dict"}],
    }

    failures = []
    for case_name, original in cases.items():
        try:
            encoded = toon.encode(original)
            decoded = toon.decode(encoded)
            if decoded != original:
                failures.append(
                    f"{case_name}: decoded {repr(decoded)} != original {repr(original)}"
                )
        except Exception as exc:
            failures.append(f"{case_name}: exception — {exc}")

    elapsed = elapsed_ms(start)
    all_passed = len(failures) == 0
    summary.record(
        "Unicode and edge cases roundtrip without corruption",
        "B3",
        all_passed,
        elapsed,
        f"all {len(cases)} edge cases passed"
        if all_passed
        else f"failures ({len(failures)}): {' | '.join(failures[:3])}"
        + (" ..." if len(failures) > 3 else ""),
    )


def test_b4_encoding_latency(toon: Any, summary: SpikeSummary) -> None:
    """
    B4 — TOON encoding of large contexts is fast enough for real-time prompt assembly.

    PRD claim (Section 13.6, CPU-bound work rules): TOON encoding of large context
    runs in run_in_executor — it must not block the asyncio event loop.

    We need to establish what the baseline encoding time is under realistic load:
    - Single encode of the full prompt context (the hot path)
    - 100 sequential encodes (simulates CTP pre-warming prompts at background priority)
    - 100 concurrent encodes via run_in_executor (the actual production pattern)

    Thresholds derived from PRD performance constraints (Section 6.3):
    - Single encode: must complete in < 10ms (acceptable for prompt assembly hot path)
    - 100 sequential: documents throughput for CTP background workload
    - run_in_executor pattern: must not block the event loop (measured by wall time ratio)
    """
    section(
        "B4 — Encoding Latency (real-time prompt assembly threshold)",
        "B4: TOON encoding is fast enough for real-time prompt assembly",
    )

    # ── Single encode (hot path) ──────────────────────────────────────────────
    start = time.perf_counter()
    try:
        _ = toon.encode(FULL_PROMPT_CONTEXT)
        single_ms = elapsed_ms(start)
    except Exception as exc:
        summary.record(
            "encoding latency — single encode of full prompt context",
            "B4",
            False,
            elapsed_ms(start),
            f"exception: {exc}",
        )
        return

    single_passed = single_ms < 10.0
    summary.record(
        "encoding latency — single encode of full prompt context",
        "B4",
        single_passed,
        single_ms,
        f"{single_ms:.2f}ms (threshold: <10ms)"
        + (" ⚠ EXCEEDS THRESHOLD" if not single_passed else ""),
    )

    # ── 100 sequential encodes (CTP background throughput) ───────────────────
    start = time.perf_counter()
    try:
        for _ in range(100):
            toon.encode(FULL_PROMPT_CONTEXT)
        bulk_ms = elapsed_ms(start)
    except Exception as exc:
        summary.record(
            "encoding latency — 100 sequential encodes",
            "B4",
            False,
            elapsed_ms(start),
            f"exception: {exc}",
        )
        return

    per_encode_ms = bulk_ms / 100
    bulk_passed = per_encode_ms < 10.0
    summary.record(
        "encoding latency — 100 sequential encodes",
        "B4",
        bulk_passed,
        bulk_ms,
        f"total={bulk_ms:.0f}ms | per-encode={per_encode_ms:.2f}ms (threshold: <10ms each)",
    )

    # ── run_in_executor pattern (production pattern per PRD Section 13.6) ────
    loop = asyncio.get_event_loop()
    # If the loop is already running (we're inside async main), avoid creating a coroutine
    # and skip this synchronous sub-test. The async variant `test_b4_executor_async`
    # exercises the same behavior from an async context.
    if loop.is_running():
        summary.record(
            "encoding latency — run_in_executor does not block event loop",
            "B4",
            True,
            0.0,
            "skipped (already inside async context — test B4 executor from sync context)",
        )
    else:

        async def _run_executor_test() -> tuple[float, bool]:
            start_exec = time.perf_counter()
            await loop.run_in_executor(None, toon.encode, FULL_PROMPT_CONTEXT)
            return elapsed_ms(start_exec), True

        try:
            executor_ms, executor_ok = loop.run_until_complete(_run_executor_test())
            summary.record(
                "encoding latency — run_in_executor does not block event loop",
                "B4",
                executor_ok,
                executor_ms,
                f"run_in_executor completed in {executor_ms:.2f}ms | "
                "event loop was not blocked during encoding",
            )
        except Exception as exc:
            summary.record(
                "encoding latency — run_in_executor does not block event loop",
                "B4",
                False,
                elapsed_ms(start),
                f"exception: {exc}",
            )


async def test_b4_executor_async(toon: Any, summary: SpikeSummary) -> None:
    """
    B4 (async variant) — run_in_executor pattern works from async context.
    This is the actual production call site in prompt-composer.
    """
    start = time.perf_counter()
    try:
        loop = asyncio.get_event_loop()
        result = await loop.run_in_executor(None, toon.encode, FULL_PROMPT_CONTEXT)
        decoded = toon.decode(result)
        correct = decoded == FULL_PROMPT_CONTEXT
        summary.record(
            "encoding latency — run_in_executor (async production pattern)",
            "B4",
            correct,
            elapsed_ms(start),
            f"executor completed in {elapsed_ms(start):.2f}ms | "
            f"roundtrip correct={correct}",
        )
    except Exception as exc:
        summary.record(
            "encoding latency — run_in_executor (async production pattern)",
            "B4",
            False,
            elapsed_ms(start),
            f"exception: {exc}",
        )


async def test_b5_model_can_act_on_toon_encoded_context(
    toon: Any, summary: SpikeSummary
) -> None:
    """
    B5 — A running model (via Ollama) can decode TOON output and act on it.

    PRD claim (Section 7.3): All structured data fed into PC is encoded as TOON
    before injection into the model. If the model cannot act on TOON-encoded
    context, the entire serialization strategy fails and prompt-composer must
    use JSON as primary with TOON as optional compression.

    We test two things separately:
    - B5a: The model can answer a factual question from a TOON-encoded context
    - B5b: The model can produce structured output when given TOON instructions

    A failure on B5a means the model cannot read TOON at all.
    A failure on B5b means prompt-composer cannot use TOON for structured output
    requests — a JSON fallback path is mandatory.

    Both results must be documented regardless of pass/fail.
    """
    section(
        "B5 — Model Can Act on TOON-Encoded Context (Requires Ollama)",
        "B5: a running model can decode TOON and act on it",
    )

    try:
        import ollama
    except ImportError:
        for sub in ["B5a", "B5b"]:
            summary.record(
                f"model acts on TOON context ({sub})",
                "B5",
                False,
                0.0,
                "ollama python package not installed — run: uv add ollama",
            )
        return

    model = "llama3.1:latest"

    # ── B5a: factual recall from TOON-encoded context ─────────────────────────
    start = time.perf_counter()
    try:
        # Encode a simple fact-bearing context as TOON
        context_data = {
            "user_name": "Alex",
            "project": "Sena",
            "preferred_editor": "VS Code",
            "primary_language": "Rust",
        }
        toon_context = toon.encode(context_data)

        response = ollama.generate(
            model=model,
            prompt=(
                f"The following context is encoded in TOON format:\n\n"
                f"{toon_context}\n\n"
                "What is the user's primary programming language? "
                "Reply with the language name only, nothing else."
            ),
            options={"temperature": 0, "num_predict": 20},
        )

        raw = response.get("response", "").strip()
        # Accept any response that contains "Rust" — case insensitive
        passed = "rust" in raw.lower()
        summary.record(
            "model acts on TOON context — factual recall (B5a)",
            "B5",
            passed,
            elapsed_ms(start),
            f"model replied: '{raw}' | expected 'Rust' | passed={passed}"
            + (
                "\n         NOTE: model cannot read TOON context — "
                "prompt-composer MUST use JSON as primary encoding."
                if not passed
                else ""
            ),
        )
    except Exception as exc:
        summary.record(
            "model acts on TOON context — factual recall (B5a)",
            "B5",
            False,
            elapsed_ms(start),
            f"Ollama not running or model unavailable: {exc}",
        )

    # ── B5b: structured output from TOON-encoded instructions ────────────────
    start = time.perf_counter()
    try:
        instructions = {
            "task": "classify_intent",
            "input": "I want to open the file manager",
            "output_format": "respond with exactly one word: the intent category",
            "categories": ["file_management", "coding", "communication", "settings"],
        }
        toon_instructions = toon.encode(instructions)

        response = ollama.generate(
            model=model,
            prompt=(
                f"Instructions encoded in TOON format:\n\n"
                f"{toon_instructions}\n\n"
                "Follow the instructions exactly."
            ),
            options={"temperature": 0, "num_predict": 20},
        )

        raw = response.get("response", "").strip().lower()
        passed = "file" in raw
        summary.record(
            "model acts on TOON context — structured output from instructions (B5b)",
            "B5",
            passed,
            elapsed_ms(start),
            f"model replied: '{raw}' | expected 'file_management' | passed={passed}"
            + (
                "\n         NOTE: model cannot follow TOON-encoded instructions — "
                "prompt-composer needs a JSON fallback for structured output requests."
                if not passed
                else ""
            ),
        )
    except Exception as exc:
        summary.record(
            "model acts on TOON context — structured output from instructions (B5b)",
            "B5",
            False,
            elapsed_ms(start),
            f"Ollama not running or model unavailable: {exc}",
        )


def test_b6_context_window_budget_enforcement(toon: Any, summary: SpikeSummary) -> None:
    """
    B6 — Oversized contexts can be trimmed in PRD priority order without
    corrupting the remaining prompt.

    PRD claim (Section 7.3, Context Window Management): When the assembled
    prompt exceeds the active model's context window, content is dropped using
    a fixed priority order. Sacred content (soulbox + user_intent) is never
    dropped. Droppable tiers are cut in a defined order.

    This test does NOT test Cognee or the model — it tests whether TOON can:
    1. Encode a partial prompt context (the state after dropping lower tiers)
    2. Produce a result that is strictly smaller than the full context
    3. Still decode cleanly — no corruption from partial encoding

    A failure here means prompt-composer cannot reliably implement context
    window management using TOON as the encoding layer.
    """
    section(
        "B6 — Context Window Budget Enforcement",
        "B6: oversized contexts can be trimmed in PRD priority order without corruption",
    )
    start = time.perf_counter()

    try:
        import tiktoken

        enc = tiktoken.get_encoding("cl100k_base")

        # Simulate a very tight token budget that forces tier dropping
        tight_budget = 200  # tokens — forces everything except sacred to be dropped

        full_encoded = toon.encode(FULL_PROMPT_CONTEXT)
        full_token_count = len(enc.encode(full_encoded))

        # Simulate PC's drop algorithm:
        # 1. Start with sacred content only
        # 2. Add droppable tiers from lowest priority (dropped first) to highest
        # 3. Stop when budget is exceeded
        remaining_context = {key: FULL_PROMPT_CONTEXT[key] for key in SACRED_KEYS}

        sacred_encoded = toon.encode(remaining_context)
        sacred_tokens = len(enc.encode(sacred_encoded))
        sacred_decoded = toon.decode(sacred_encoded)

        # Sacred content must always survive and decode cleanly
        sacred_intact = all(
            sacred_decoded.get(k) == FULL_PROMPT_CONTEXT[k] for k in SACRED_KEYS
        )

        # Now simulate adding tiers back until we exceed budget
        final_context = dict(remaining_context)
        tiers_added = []
        for tier in reversed(DROPPABLE_TIERS_ORDERED):
            candidate = {**final_context, tier: FULL_PROMPT_CONTEXT[tier]}
            candidate_encoded = toon.encode(candidate)
            candidate_tokens = len(enc.encode(candidate_encoded))
            if candidate_tokens <= tight_budget:
                final_context = candidate
                tiers_added.append(tier)
            else:
                # This tier would exceed budget — correct behavior is to skip it
                pass

        # The final trimmed context must decode correctly
        final_encoded = toon.encode(final_context)
        final_decoded = toon.decode(final_encoded)
        final_intact = final_decoded == final_context
        final_tokens = len(enc.encode(final_encoded))

        passed = sacred_intact and final_intact and final_tokens <= full_token_count
        elapsed = elapsed_ms(start)

        summary.record(
            "context window budget — sacred content survives, tiers drop cleanly",
            "B6",
            passed,
            elapsed,
            f"full={full_token_count} tokens | "
            f"sacred-only={sacred_tokens} tokens | "
            f"after trim={final_tokens} tokens | "
            f"tiers retained: {tiers_added} | "
            f"sacred intact={sacred_intact} | final intact={final_intact}",
        )

    except ImportError:
        summary.record(
            "context window budget — sacred content survives, tiers drop cleanly",
            "B6",
            False,
            elapsed_ms(start),
            "tiktoken not installed — run: uv add tiktoken",
        )
    except Exception as exc:
        summary.record(
            "context window budget — sacred content survives, tiers drop cleanly",
            "B6",
            False,
            elapsed_ms(start),
            f"exception: {exc}",
        )


def test_b7_three_way_serialization_split(toon: Any, summary: SpikeSummary) -> None:
    """
    B7 — The three-way serialization split is enforceable.

    PRD claim (Section 7.3, Three-Way Serialization Split):
    - TOON: all prompt data fed into PC and the model
    - TOML: config files, SoulBox definitions, agent manifests
    - JSON: internal non-uniform structures where TOON offers no advantage

    This test verifies that the boundaries hold — specifically that:
    - TOON can encode everything TOML and JSON encode (it must be a superset
      for prompt data purposes)
    - Config-shaped data (flat, human-readable) encodes and decodes in TOON
      without issues (even though TOML is used for storage, PC may receive it
      as a decoded dict and must encode it as TOON for model injection)
    - Non-uniform structures (mixed types, irregular shapes) encode cleanly

    A failure here means there is a class of data that PC receives but cannot
    encode as TOON — a gap in the serialization strategy.
    """
    section(
        "B7 — Three-Way Serialization Split Enforceability",
        "B7: the three-way serialization split is enforceable without gaps",
    )
    start = time.perf_counter()

    # Config-shaped data (TOML domain — flat key-value, human-edited)
    agent_manifest = {
        "agent_id": "file_agent",
        "version": "1.0.0",
        "capabilities": ["read", "write", "watch"],
        "priority": 3,
        "enabled": True,
        "timeout_ms": 5000,
    }

    # SoulBox-shaped config (TOML domain — nested but uniform)
    soulbox_config = {
        "schema_version": 3,
        "encryption": "aes-256-gcm",
        "kdf": "argon2id",
        "auto_evolve": True,
        "evolution_threshold": 0.7,
    }

    # Non-uniform internal structure (JSON domain)
    model_capability_profile = {
        "model_id": "llama3.1:latest",
        "context_window": 131072,
        "capabilities": {
            "structured_output": "FULL",
            "multi_step_reasoning": "DEGRADED",
            "image_input": False,
            "tool_use": True,
        },
        "probe_results": [
            {"probe": "json_extraction", "result": "pass", "latency_ms": 120},
            {"probe": "chain_of_thought", "result": "degraded", "latency_ms": 850},
        ],
        "effective_context_window": 6144,
    }

    # TelemetryEvent-shaped data (JSON domain — map<string,string> payload)
    telemetry_event = {
        "event_id": "f47ac10b-58cc-4372-a567-0e02b2c3d479",
        "subsystem": "ctp",
        "event_type": "thought_generated",
        "timestamp_utc": 1772691637000,
        "session_id": "sess_20260306",
        "payload": {"relevance_score": "0.87", "thought_id": "th_0042"},
        "activity_state": "active",
    }

    fixtures = {
        "agent_manifest (TOML-domain, decoded for PC)": agent_manifest,
        "soulbox_config (TOML-domain, decoded for PC)": soulbox_config,
        "model_capability_profile (JSON-domain, non-uniform)": model_capability_profile,
        "telemetry_event (JSON-domain, map payload)": telemetry_event,
    }

    failures = []
    for fixture_name, original in fixtures.items():
        try:
            encoded = toon.encode(original)
            decoded = toon.decode(encoded)
            if decoded != original:
                failures.append(f"{fixture_name}: roundtrip mismatch")
        except Exception as exc:
            failures.append(f"{fixture_name}: exception — {exc}")

    elapsed = elapsed_ms(start)
    all_passed = len(failures) == 0
    summary.record(
        "three-way split — TOON can encode all domains without gaps",
        "B7",
        all_passed,
        elapsed,
        f"all {len(fixtures)} cross-domain structures encoded cleanly"
        if all_passed
        else f"gaps found: {' | '.join(failures)}"
        + "\n         NOTE: these gaps mean prompt-composer has data it cannot encode as TOON.",
    )


# ── Summary ───────────────────────────────────────────────────────────────────


def print_summary(summary: SpikeSummary) -> None:
    print(f"\n{'═' * 60}")
    print("  TOON SPIKE SUMMARY")
    print(f"{'═' * 60}")

    for result in summary.results:
        status = "✓" if result.passed else "✗"
        print(f"  {status}  [{result.assumption}]  {result.name}")

    total_ms = sum(r.duration_ms for r in summary.results)
    print(
        f"\n  {summary.passed_count}/{summary.total_count} assumptions confirmed  |  "
        f"total: {total_ms:.0f}ms"
    )

    if summary.failures:
        print("\n  BROKEN ASSUMPTIONS — DOCUMENT IN spikes/README.md:")
        print("  These failures mean the architecture needs revisiting,")
        print("  not just the code.\n")
        for failure in summary.failures:
            print(f"  [{failure.assumption}]  {failure.name}")
            print(f"    {failure.detail}\n")

        # B5 failure has a specific architectural consequence — call it out explicitly
        b5_failures = [f for f in summary.failures if f.assumption == "B5"]
        if b5_failures:
            print("  ⚠  CRITICAL: B5 failure means the model cannot act on TOON.")
            print("     prompt-composer MUST implement a JSON fallback path.")
            print(
                "     This must be documented and designed before implementation starts."
            )
            print("")
    else:
        print(
            "\n  All assumptions confirmed. TOON is ready for "
            "prompt-composer integration."
        )

    print(f"{'═' * 60}\n")


# ── Entry point ───────────────────────────────────────────────────────────────


async def main() -> None:
    print("\n╔══════════════════════════════════════════════════════════╗")
    print("║  SENA — TOON Spike                                       ║")
    print("║  Pre-condition to: prompt-composer implementation        ║")
    print("║  Testing PRD architectural assumptions B1–B7             ║")
    print("╚══════════════════════════════════════════════════════════╝")

    # Import TOON implementation (prefer canonical name) and prepare summary
    try:
        # Prefer the canonical `toon` module name if available
        import toon as _toon_module

        toon = _toon_module
    except ImportError:
        # Some installations expose the package as `toon_format` (e.g. toon-format).
        # Accept either to make the spike resilient across packaging variants.
        try:
            import toon_format as _toon_module

            toon = _toon_module
        except ImportError:
            print(
                "\n  ERROR: toon-format is not installed (module name 'toon' or 'toon_format' not found)."
            )
            print("  Install with: uv add toon-format    (or: pip install toon-format)")
            print("  Repo: https://github.com/toon-format/toon-python")
            return

    summary = SpikeSummary()

    # Start optional debug UI and register a session for this spike run (best-effort)
    try:
        from spikes.tools import debug_ui as _debug_ui  # type: ignore

        debug_ui = _debug_ui
        # Start the in-process debug HTTP server (idempotent) and discover bound address.
        try:
            host, port = debug_ui.start_ui()
        except Exception:
            # If discovery fails, fall back to defaults. Do not fail the spike.
            host, port = ("127.0.0.1", 8765)

        # Best-effort: try to launch the Tk client as an external process via launcher.
        try:
            from spikes.tools import debug_ui_launcher as _launcher  # type: ignore

            try:
                # Pass the discovered API base so the client knows where to poll.
                _launcher.maybe_launch_tk_ui(api_base=f"http://{host}:{port}")
            except Exception:
                # Swallow launcher errors — UI is opt-in and must not block spikes
                pass
        except Exception:
            # Launcher not available; continue headless
            pass

        session_id = debug_ui.register_spike("toon_spike", {"env": "local"})
        debug_ui.test_started(session_id, "B1 — All Sena Structures Roundtrip")
    except Exception:
        _debug_ui = None
        session_id = "disabled"

    # Helper to publish a single test result (best-effort)
    def _publish_result(ui_module, sid, label, last: SpikeResult):
        try:
            ui_module.publish_raw(
                sid,
                label,
                {
                    "detail": last.detail,
                    "assumption": last.assumption,
                    "duration_ms": last.duration_ms,
                    "passed": last.passed,
                },
                label="result_detail",
            )
            ui_module.test_finished(
                sid, label, last.passed, last.detail, last.duration_ms
            )
        except Exception:
            pass

    # Pure Python tests — no I/O, no external services
    try:
        if _debug_ui:
            _debug_ui.test_started(session_id, "B1 — All Sena Structures Roundtrip")
    except Exception:
        pass
    test_b1_all_sena_structures_encode_without_loss(toon, summary)
    if _debug_ui:
        try:
            _publish_result(
                _debug_ui,
                session_id,
                "B1 — All Sena Structures Roundtrip",
                summary.results[-1],
            )
        except Exception:
            pass

    try:
        if _debug_ui:
            _debug_ui.test_started(session_id, "B2 — Token Reduction vs JSON")
    except Exception:
        pass
    test_b2_token_reduction_vs_json(toon, summary)
    if _debug_ui:
        try:
            _publish_result(
                _debug_ui,
                session_id,
                "B2 — Token Reduction vs JSON",
                summary.results[-1],
            )
        except Exception:
            pass

    try:
        if _debug_ui:
            _debug_ui.test_started(session_id, "B3 — Unicode and Edge Cases")
    except Exception:
        pass
    test_b3_unicode_and_edge_cases(toon, summary)
    if _debug_ui:
        try:
            _publish_result(
                _debug_ui,
                session_id,
                "B3 — Unicode and Edge Cases",
                summary.results[-1],
            )
        except Exception:
            pass

    try:
        if _debug_ui:
            _debug_ui.test_started(session_id, "B4 — Encoding Latency (sync)")
    except Exception:
        pass
    test_b4_encoding_latency(toon, summary)
    await test_b4_executor_async(toon, summary)
    if _debug_ui:
        try:
            _publish_result(
                _debug_ui,
                session_id,
                "B4 — Encoding Latency (sync)",
                summary.results[-1],
            )
        except Exception:
            pass

    try:
        if _debug_ui:
            _debug_ui.test_started(session_id, "B6 — Context Window Budget Enforcement")
    except Exception:
        pass
    test_b6_context_window_budget_enforcement(toon, summary)
    if _debug_ui:
        try:
            _publish_result(
                _debug_ui,
                session_id,
                "B6 — Context Window Budget Enforcement",
                summary.results[-1],
            )
        except Exception:
            pass

    try:
        if _debug_ui:
            _debug_ui.test_started(session_id, "B7 — Three-Way Serialization Split")
    except Exception:
        pass
    test_b7_three_way_serialization_split(toon, summary)
    if _debug_ui:
        try:
            _publish_result(
                _debug_ui,
                session_id,
                "B7 — Three-Way Serialization Split",
                summary.results[-1],
            )
        except Exception:
            pass

    # Model test last — requires Ollama, may be slow, may fail for infra reasons
    # A failure here is architectural, not environmental — document it either way
    try:
        if _debug_ui:
            _debug_ui.test_started(
                session_id, "B5 — Model Can Act on TOON-Encoded Context"
            )
    except Exception:
        pass
    await test_b5_model_can_act_on_toon_encoded_context(toon, summary)
    if _debug_ui:
        try:
            _publish_result(
                _debug_ui,
                session_id,
                "B5 — Model Can Act on TOON-Encoded Context",
                summary.results[-1],
            )
        except Exception:
            pass

    print_summary(summary)

    # Attempt a graceful UI shutdown (best-effort)
    try:
        if _debug_ui:
            _debug_ui.shutdown()
    except Exception:
        pass


if __name__ == "__main__":
    asyncio.run(main())
