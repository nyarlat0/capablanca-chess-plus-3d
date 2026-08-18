# Engine audit and Strategic V2 tool verification

Date: 2026-08-18

## Scope

This audit rechecked the complete `capablanca-engine` and `terastockfish`
crates after the search-foundation work, with special attention to reversible
state, variant move generation, search terminal handling, Strategic V2 feature
semantics, and the new long-running validation workflow. It did not inspect or
build the Bevy frontend or backend.

## Provenance

- base revision: `0a26e1f71cc9056fdd7d7a3aae0b70a55e6f8469`;
- worktree: dirty with the fixes and tool described here;
- compiler: `rustc 1.93.1 (01f6ddf75 2026-02-11)`, LLVM 21.1.8;
- release binary SHA-256:
  - `terastockfish`: `91a85a64e6cd8e4328a1afedcc400af3335097cc762309a3b059a6ea74967d09`;
  - `terastockfish-strategic`:
    `c7b8fcf055cdfdf4a8658295d1e588c9c2e56d30a5592fd93e2c80b7ef07dfe0`;
- smoke CSV SHA-256:
  `f021a568db2f5b784f7d11d5dcbaa20e7e0f8599d3d0ce5619d146faf6253d08`.

## Findings and corrections

### Strategic connected-pawn semantics

The initial candidate treated a pawn as connected whenever any friendly pawn
occupied an adjacent file, regardless of rank. The audit rejected that broad
definition. Strategic V2 now rewards a pawn only when a friendly pawn is one
file away and at most one rank away, representing a chain or phalanx. Exact
feature tests cover connected, disconnected, passed, blocked, open,
semi-open, and closed-file cases.

### Zero ply cap

The generic validation harness previously interpreted `maximum_plies=0` as a
one-ply cap through `.max(1)`. Zero now consistently means no artificial cap,
matching the balance tool and the documented primary protocol. Positive caps
retain their old behavior.

### Stalemate at the quiescence boundary

At a quiet qsearch node, an empty tactical list previously meant both “legal
quiet moves only” and “no legal move”. The latter could evaluate a horizon
stalemate as a normal position. The tactical generator now returns both its
move list and whether any legal move exists, checking quiet moves only until
one legal example is found. Qsearch returns zero for actual stalemate.

### Long-run integrity

The new `terastockfish-strategic` tool:

- hard-codes production versus the frozen Strategic V2 candidate;
- uses fixed color-swapped pairs and no early significance stopping;
- writes an atomic CSV after every batch;
- resumes only when all frozen metadata and profile fingerprints match;
- verifies contiguous complete pairs with matching opening FENs;
- reports W/D/L, paired 95% CI, terminations, validity, and decision;
- marks any ply-capped study invalid;
- includes a semantic implementation identifier in the checkpoint header.

The older general `terastockfish-eval` now defaults to the production baseline;
published material remains explicitly selectable.

## Verification results

Commands:

```sh
cargo fmt --all -- --check
cargo test --release -p capablanca-engine -p terastockfish
cargo clippy -p capablanca-engine -p terastockfish --all-targets -- -D warnings
cargo bench -p terastockfish --bench search
cargo build --release -p terastockfish --bins
printf 'uci\nposition startpos\ngo perft 4\nquit\n' | target/release/terastockfish
```

Results:

- 111 release tests passed, zero failed;
- Clippy passed with warnings denied;
- deterministic random 128-ply lines for every built-in variant preserved FEN
  parsing and exact make/unmake state;
- existing independent perft fixtures remained unchanged;
- Terachess II start-position perft(4): `10,562,564` nodes;
- UCI initialization and diagnostic perft completed normally;
- Strategic smoke run wrote a complete pair, and `--resume` recognized it
  without replaying games;
- the smoke run correctly reported `valid=false` because its deliberate
  four-ply cap terminated both diagnostic games.

Final fixed search corpus:

| Position | Depth | Nodes | Seconds | NPS |
|---|---:|---:|---:|---:|
| start | 4 | 9,905 | 0.192 | 51,458 |
| opening-8 | 3 | 7,129 | 0.108 | 66,151 |
| middlegame-24 | 3 | 17,597 | 0.199 | 88,294 |
| middlegame-48 | 3 | 6,036 | 0.142 | 42,643 |
| aggregate | — | 40,667 | 0.641 | 63,433 |

The start remains approximately 45% faster than the original recorded 0.349 s
baseline. Timing is a short local regression sentinel, not strength evidence.

## Conclusion and limitations

No failing rule, state, search, serialization, release-mode, or lint check
remains in the audited scope. The audit found and corrected three real edge
cases rather than merely rerunning tests. This increases confidence but cannot
prove the absence of undiscovered bugs.

No Strategic V2 strength games were interpreted. The smoke pair is explicitly
invalid as evidence. Production remains unchanged until the separately frozen
[Strategic V2 protocol](2026-08-18-strategic-v2-validation-protocol.md) is run
and documented.
