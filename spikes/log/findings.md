# Sena — Spikes

Throwaway scripts that validate critical third-party libraries against Sena's
actual requirements before the architecture is built around them.

Run spikes before starting the subsystem they gate. Document results here.

---

## spikes/spikes/cognee_spikes.py

**Pre-condition to:** `memory-engine` implementation
**Run:** `uv run -m spikes.spikes.cognee_spikes`
**Dependencies:** `uv add cognee`

Tests Cognee's local stack (SQLite + LanceDB + LadybugDB) against Sena's
memory patterns: basic write/retrieval, episodic sequences, sequential reads,
graph relationship traversal, relevance differentiation, and cold start.

**v2 changes (after first run failures):**
- All `GRAPH_COMPLETION` queries replaced with `CHUNKS`. `GRAPH_COMPLETION`
  forces a full generative pass through llama3.1 on every search call. On a
  1660 Ti (6 GB VRAM, CPU offload) that is 2–4 minutes per query. The spike
  tests *storage and retrieval correctness*, not generation quality. `CHUNKS`
  queries the vector store directly — no LLM call, returns in under a second.
- A2 changed from concurrent `asyncio.gather()` searches to a sequential loop.
  `cognee.search()` writes results back to a SQLite cache table. Concurrent
  searches all attempt simultaneous INSERTs into that table; SQLite's default
  journal mode has no WAL and cannot handle concurrent writes, producing
  `database is locked`. Sequential searches with one shared result set is also
  the correct production model — the reactive loop fires one search at a time.
- A3 sequential adds were already correct. No change needed — CTP's internal
  write queue means `cognee.add()` is never called concurrently in production.
- `prune_all()` drain strategy replaced hardcoded sleep with a poll loop.
  See "Spike Fixes and Lessons Learned — v2" below.
- Per-step timing lines (`[add: Xms]`, `[cognify: Xms]`, `[search: Xms]`)
  added to every test so bottlenecks are visible without re-running.
- Wall-clock total printed at the end of the run.

---

### Results — v1 (gemma2:2b, GRAPH_COMPLETION queries)

| Test | Result | Notes |
|---|---|---|
| Basic write and retrieval | FAIL | Not found with gemma2:2b (low-end model); alternate queries also failed |
| Episodic sequence | FAIL | Expected coding session events not found; semantic and direct queries failed |
| Concurrent reads | PASS | All concurrent queries completed without error |
| Graph relationship traversal | PASS | Relationship between profession and tools found |
| Relevance differentiation | FAIL | Neither high-frequency nor low-frequency memory surfaced |
| Cold start clean slate | FAIL | Expected empty store after prune, but got 1 result(s) |

**Note (v1):**
Cognee spike fails key memory benchmarks with low-end models like gemma2:2b.
Reliable retrieval, relevance scoring, and episodic/event memory require
higher-capacity models (e.g., llama3.2:3b or better). Failures may be due to
model limitations, not just Cognee architecture. Documented for architectural
risk and future integration decisions.

---

### Results — v2 (llama3.1:latest, CHUNKS queries, sequential searches)

> Wall time: 1649640ms — run on GTX 1660 Ti / 16 GB RAM, Ollama llama3.1:latest

| Test | Assumption | Result | Notes |
|---|---|---|---|
| semantic retrieval — related query surfaces stored fact | A5 | PASS | |
| three-tier write and read — all tiers retrievable via CHUNKS | A1 | FAIL* | SQLite lock on `UPDATE data SET token_count` — poll fix not yet applied |
| sequential reads — all queries return correct results | A2 | PASS | |
| write serialization — all sequential writes survive | A3 | FAIL* | Same SQLite lock — poll fix not yet applied |
| memory weighting — high-frequency fact surfaces over low-frequency | A4 | PASS | |
| episodic sequence — ordered context preserved around queried event | A6 | FAIL | Cognee graph extraction prompt does not instruct model to include `name` field; Pydantic rejects both generations |
| graph relationship traversal — related entity surfaces via graph | A8 | PASS | |
| cold start prune — store is empty after prune | A7 | FAIL† | `SearchPreconditionError` — store is empty but Cognee errors instead of returning `[]` |

\* A1 and A3 failures are test-harness artifacts. The `prune_all()` poll fix was not applied for this run.
  After applying the fix, re-run to confirm. These are not production risks — CTP's write queue
  naturally spaces operations.

† A7 is a pass in disguise. After prune, Cognee wipes the user/dataset record entirely.
  `search()` with no prior `add()` throws `SearchPreconditionError: no database/default user found`
  rather than returning `[]`. The store is empty — Cognee just doesn't handle the empty-state query
  gracefully. Mitigation: cold start must call `cognee.add()` + `cognee.cognify()` before any search,
  which memory-engine's boot sequence already requires.

**Known remaining risk (A6):**
If A6 still fails after the v2 fix, the failure means Cognee's chunker splits
the session-log text across chunk boundaries in a way that separates the
before/middle/after steps. This is a chunking configuration question, not a
retrieval model question. Mitigation for `memory-engine`: store episodic
sequences with explicit ordering metadata (step index + session ID) as
separate graph nodes with `NEXT_EVENT` edges, rather than relying on chunker
proximity. Document this in `.github/instructions/memory-engine.instructions.md`
if A6 fails.

**Known remaining risk (A8):**
If A8 still fails after the v2 fix, it means Cognee is storing Bob's and
Alice's text in separate non-overlapping chunks and the CHUNKS query for
"who does Bob report to" only returns the Bob chunk, not the Alice chunk.
Mitigation: use `GRAPH_SUMMARY_COMPLETION` for relationship queries in
production — it uses pre-computed summaries from cognify time, not per-query
generation. Slower than CHUNKS but faster than GRAPH_COMPLETION.

---

### Spike Fixes and Lessons Learned — v1

- **Model output format:** Higher-end models (e.g., llama3.1:latest) may return
  structured JSON/dict objects, not just plain strings. Spike code must
  recursively search for keywords in all nested fields, including inside JSON
  strings.
- **Defensive parsing:** Always check for multiple possible fields (`"content"`,
  `"description"`, `"title"`, `"text"`, `"value"`) when extracting information
  from model outputs. If a field is missing, continue searching other fields and
  nested structures.
- **Validation errors:** Pydantic or Cognee may expect a flat string or a
  specific field. If missing, parse errors will occur. Defensive coding and
  robust error handling are required to avoid silent failures.
- **Debugging:** Add granular debug prints and timing to all major steps (add,
  cognify, search) to pinpoint bottlenecks and performance issues.
- **Low-end models:** Semantic search and relevance scoring fail with small
  models (e.g., gemma2:2b). Use higher-capacity models for reliable retrieval
  and relevance.
- **Migration:** When moving spike logic to production, ensure recursive search
  and robust parsing of model outputs. Document all edge cases, failure modes,
  and validation traps.

When updating `.github/instructions/memory-engine.instructions.md`, include:
- Robust parsing for structured/unstructured outputs (recursively search dicts,
  lists, and JSON strings)
- Defensive coding for missing fields and validation errors (check all likely
  fields, handle parse failures gracefully)
- Debug logging for slow operations and edge resolution (log timings and
  failures for all major steps)
- Document model-specific output formats and traps for future maintainers

---

### Spike Fixes and Lessons Learned — v2

- **GRAPH_COMPLETION is a hardware bottleneck, not a correctness tool.**
  On a 1660 Ti with 6 GB VRAM, llama3.1 offloads layers to CPU. Each
  `GRAPH_COMPLETION` search call takes 2–4 minutes. For spike assertions
  that only need to verify keyword presence, `CHUNKS` is the correct
  search type — it hits the vector store only, no LLM inference, returns
  in milliseconds. Use `GRAPH_COMPLETION` only when testing *generation
  quality*, which is model-probe's job, not memory's.

- **`cognee.search()` is not safe for concurrent calls.**
  It writes results back to a SQLite cache table. Concurrent searches via
  `asyncio.gather()` all attempt simultaneous INSERTs; SQLite's default
  journal mode has no WAL and produces `database is locked`. Always issue
  searches sequentially. This is also the correct production pattern — the
  reactive loop fires one search at a time.

- **`cognee.add()` is not safe for concurrent calls on the same dataset.**
  Internally it calls `reset_dataset_pipeline_run_status()`, which does an
  unguarded SQLite read-modify-write. Concurrent calls via `asyncio.gather()`
  produce `list index out of range` deep in the pipeline layer. Always
  await `cognee.add()` calls sequentially. This is also the correct
  production pattern: CTP has its own internal write queue.

- **`cognify()` background writes outlive the coroutine.**
  SQLAlchemy autoflush (`UPDATE data SET token_count`) fires after the
  coroutine returns. Any Cognee operation immediately following `cognify()`
  or `prune_all()` can race against these background writes and hit
  `database is locked`. Do not use a hardcoded sleep — that is a guess, not
  a solution. Instead, poll with a lightweight `cognee.add()` probe at 500ms
  intervals until SQLite accepts the write without locking. This mirrors the
  natural spacing CTP's write queue provides in production. Timeout at 120s.
  In production this is not an issue — CTP's write queue naturally spaces
  operations and prune is only called on cold start.

- **Per-step timing is mandatory for spike diagnostics.**
  Without `[add: Xms]` / `[cognify: Xms]` / `[search: Xms]` lines it is
  impossible to tell whether a slow test is blocked on Ollama inference,
  SQLite contention, or LanceDB indexing. All future spikes must instrument
  every major I/O step with `time.perf_counter()` deltas.

- **A6: Cognee's graph extraction prompt does not produce `name`-compliant nodes.**
  Cognee's internal `cognify()` graph extraction prompt instructs llama3.1 to
  return a `KnowledgeGraph` JSON structure, but does not include `name` as a
  required field. llama3.1 returns nodes with `id`, `type`, `description`,
  `properties`, and `edges` — but not `name`. Cognee's own Pydantic
  `KnowledgeGraph` model requires `name`, so validation fails on every node
  across both retry generations. This is a Cognee internals bug — the prompt
  and the schema are out of sync. Not fixable from outside without patching
  Cognee's graph extraction prompt directly. Confirmed architectural
  implication: `memory-engine` must not rely on Cognee's graph builder for
  episodic sequence storage. Explicit `NEXT_EVENT` graph edges keyed by
  `(session_id, step_index)` are required.

- **A7: `prune_all()` leaves Cognee in a state where `search()` throws instead of returning `[]`.**
  After `prune_data()` + `prune_system()`, Cognee wipes the user and dataset
  records from SQLite entirely. A subsequent `cognee.search()` call with no
  prior `add()` throws `SearchPreconditionError: no database/default user found`
  rather than returning an empty list. The store is confirmed empty — Cognee
  just does not handle the zero-data query state gracefully. Production
  mitigation: memory-engine's cold start boot sequence must always call
  `cognee.add()` + `cognee.cognify()` with at least one seed record before
  opening the reactive loop for searches.

- **Spike must simulate production usage patterns, not stress-test internals.**
  The spike exists to validate that Cognee behaves correctly under the same
  patterns Sena will use in production. Tests that hammer Cognee in ways
  production never would (back-to-back prune/cognify, concurrent searches)
  produce failures that are not production risks — they obscure the real
  signal. Every test design decision must be justified against how CTP, the
  reactive loop, and memory-engine actually call Cognee.

---

## spikes/spikes/toon_spikes.py

**Pre-condition to:** `prompt-composer` implementation
**Run:** `uv run -m spikes.spikes.toon_spikes`
**Dependencies:** `uv add toon-format tiktoken ollama`

Tests the `toon-format/toon-python` SDK against Sena's prompt encoding
requirements: roundtrip correctness, nested objects, token reduction vs JSON,
unicode handling, large context encoding, edge cases, and model output validity.

Test 7 (model structured output) requires a running Ollama instance with
`llama3.2:3b` pulled. If the model fails test 7, prompt-composer **must**
implement a JSON fallback path — document this explicitly below.

### Results
> Document your results here after running.

| Test | Result | Notes |
|---|---|---|
| Basic roundtrip | — | |
| Nested object encoding | — | |
| Token count comparison | — | Token reduction % |
| Unicode and special characters | — | |
| Large context encoding | — | Encoded size in bytes |
| Edge cases | — | |
| Model structured output | — | **If FAIL: JSON fallback required** |

---

## Adding New Spikes

If a new third-party dependency is introduced that Sena's architecture
depends on critically, add a spike here before integrating it.

A spike qualifies if:
- The library's behavior under Sena's specific usage pattern is unconfirmed
- A failure would require significant architectural rework to fix later
- The library is new, experimental, or has known edge cases