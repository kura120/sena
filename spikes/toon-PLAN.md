Sena-rebuilt-2-26-2026\spikes\toon-PLAN.md
# PLAN.md — TOON Format Benchmarking Spike

**What:**  
Benchmark and validate the `toon-format` (toon-python SDK) against Sena’s prompt encoding requirements, ensuring it can reliably serialize, deserialize, and optimize context windows for the prompt-composer subsystem.

**Why:**  
Before building prompt-composer around TOON, we need to confirm that the format:
- Handles all required Sena data structures (SoulBox, memories, context, intent, etc.)
- Provides measurable token savings over JSON
- Correctly roundtrips Unicode, edge cases, and deeply nested objects
- Can be reliably decoded by models (via Ollama) for structured output

This mitigates risk of future rework and ensures prompt-composer will be robust and efficient.

**Subsystems affected:**  
- Potential future: `prompt-composer`
- For this spike: isolated, no production subsystems affected

**Assumptions:**  
- `toon-format` SDK is stable and compatible with Sena’s data structures
- Test fixtures accurately reflect real Sena prompt contexts
- Ollama is available for model output validation (optional, but recommended)
- No changes will be made to core Sena subsystems during this spike
- Results will be documented and reviewed before any integration decision

**Out of scope:**  
- Full integration with prompt-composer or other subsystems
- UI, OS hooks, or agent routing
- Production deployment or release
- Any code not directly related to benchmarking `toon-format`
