# llm-match

Chess with a local LLM (KoboldCpp). Rust checks move legality. **No engine move
suggestions, search, scores, or best-move ranking.** Grounder provides geometric
observations; Proposer suggests moves; Selector chooses among model suggestions.

## Classic chess pipeline

1. **Grounder:** The initial position is computed by the engine when
   `--no-auto-initial-grounder` is not set (default). Otherwise, the full
   initial map comes from the LLM.
2. **Incremental Grounder:** Every subsequent accepted move invalidates
   affected piece controls and x-rays. LLM `OneGrounder` recalculates these
   entries. Rust checks each returned entry against the rules and retries
   invalid results. Untouched entries are retained.
3. **Proposer:** A deliberately short prompt asks for 2–3 SAN candidates.
   It can return `{"move":"Nxd5"}`, `{"moves":["Nxd5","e6"]}`, a bare SAN/UCI
   string, or candidates with optional opponent replies.
4. **Candidate inspection:** Each legal model-proposed move is applied to a
   *copy* of the game. Incremental LLM Grounder updates its geometry and a
   before/after delta is generated. Optional legal opponent replies are
   inspected independently; illegal proposals/replies never veto another
   legal candidate.
5. **Selector:** If there are multiple legal candidates, an LLM selects one
   using the verified Grounder deltas. If there's only one, it is accepted
   directly. A selector failure falls back to the first *legal move proposed
   by Proposer*, without an engine evaluation.
6. Only the selected move is committed. Its already validated Grounder map
   is retained so the next turn needs to recalculate only the new changes.

`free_by_count` means an occupied piece has more geometric attackers than
defenders. This is the intentionally simplified heuristic, not a guaranteed
profitable capture.

### Fast tests vs normal match

`--auto-grounder` explicitly opts into engine-calculated Grounder geometry
at **all** positions, useful for testing Proposer/Selector quickly. Without
it, incremental updates are computed by the LLM, including during normal
interactive matches and candidate inspection.

Initial engine grounding is enabled **by default**: only the initial map
is engine-calculated. To ask the LLM to calculate that map as well, use
`--no-auto-initial-grounder`.

`--test-pipeline` evaluates one model move on a clone of the provided FEN;
it does not change the underlying game. Testing from a supplied FEN with
`--auto-grounder` avoids LLM Grounder calls; omitting it exercises the
incremental LLM Grounder. A `--moves` replay in a regular match preserves
the initial snapshot, allowing the normal incremental updates to run.

## Run

```sh
cargo test -p llm-match
cargo build --release -p llm-match

# Regular match: initial engine Grounder, subsequent updates via LLM.
RUST_LOG=info ./target/release/llm-match

# Verify which pieces the LLM re-grounds:
RUST_LOG=info ./target/release/llm-match --test-pipeline \
  --fen 'r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5'

# Quick Proposer/Selector test: automatic Grounder at all positions:
./target/release/llm-match --test-pipeline --auto-grounder \
  --fen 'r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5'

# Isolated Proposer with engine-provided reference facts:
./target/release/llm-match --stage proposer \
  --fen 'r1bqkb1r/pp2pppp/2n2n2/2PN4/5B2/8/PPP1PPPP/R2QKBNR b KQkq - 0 5'
```

When the LLM updates geometric controls, logs contain
`incremental one-grounder` and `stage=OneGrounder`. Without
`--auto-grounder`, absence of those log entries during an ordinary match
with accepted moves is a sign of a pipeline integration problem.

Images are off by default. `--image` attaches a board PNG;
`--no-image` and `--disable-image` disable it. FEN and the Grounder report
remain text-only. Thinking/sampling for every stage comes **only** from the
corresponding preset, e.g. `presets/proposer.json`.

## Code

- `src/pipeline/classic_pipeline.rs`: model candidates, inspections, selector,
  preserving the selected branch's map.
- `src/pipeline/incremental.rs`: per-move invalidation and LLM OneGrounder.
- `src/grounder.rs`: map types, geometric cross-checks, and board observations.
- `src/pipeline/delta.rs`: changed occupancy, controlled squares, x-rays,
  and attackers/defenders (without engine scoring).
- `templates/pipeline/proposer.json`: intentionally minimal Proposer prompt.
- `presets/proposer.json`: model thinking and sampling settings.

All hypothetical branches use independent match clones; engine legality
checks do not introduce moves that the model did not suggest.
