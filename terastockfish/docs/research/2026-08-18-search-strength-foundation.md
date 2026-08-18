# Search-strength foundation

Date: 2026-08-18

## Question and decision rule

The engineering question was whether TeraStockfish's search could be made
substantially more efficient and prepared for reproducible evaluation research
without changing Terachess II rules or silently promoting unvalidated
positional weights.

The preregistered engineering decision was to retain the implementation if:

1. all engine and search tests, including make/unmake and cross-variant move
   generation checks, passed;
2. Clippy passed with warnings denied;
3. the fixed starting-position benchmark did not regress and the expanded
   fixed corpus exposed no pathological case; and
4. every new evaluation term remained zero in `production()` until a separate
   paired strength experiment validated it.

This is a performance and correctness study, not an Elo experiment. It cannot
establish that the new search wins more games.

## Provenance

- base source revision: `5c50e2ca51afb866f4a0f827af5b2a98aac86a15`;
- worktree: dirty with the engine/search changes described in this report;
- compiler: `rustc 1.93.1 (01f6ddf75 2026-02-11)`, LLVM 21.1.8;
- host: AMD Ryzen 9 5900X, 12 cores / 24 threads, 31 GiB RAM;
- linker configuration: Clang with mold from `.cargo/config.toml`;
- benchmark profile: workspace `release` profile, `opt-level=2`, stripped,
  no LTO, `panic=abort`;
- benchmark command: `cargo bench -p terastockfish --bench search`;
- benchmark search configuration: one thread, 64 MiB hash, three fresh
  searches per position, median elapsed sample;
- snapshot seed: `0x535452454e475448`;
- strength games: none.

Final release artifact SHA-256 values:

- `terastockfish`: `f97fe8396ff59b3064c4e0c596a9f577cee8afad3f44206525b8a4764fdb8c1f`;
- `terastockfish-eval`: `4f78ce5657835ec482069c610178b07fc3bbb7679a640e90fd1fd65c4b573677`;
- `terastockfish-texel`: `7d0efca0663e2d2b47f9f252b35f8ded3a5de3bc80106647bb6c0b7717afd806`;
- `terastockfish-balance`: `c9b6cdcddf7d3d9d7d856b908451e4528827ce7f61624c00d7191bf9e45400d4`.

The baseline was captured before these changes with the former single-position
benchmark. The expanded corpus did not exist at baseline, so only the starting
position has a valid before/after comparison.

## Changes under study

### Move generation and reversible state

The rules board now maintains indexed occupied-square masks for each color.
Full and tactical move generation iterate those masks instead of scanning the
entire potential board. A dedicated legal tactical generator emits captures
and promotions and falls back to all legal evasions in check. Null moves use
the ordinary reversible position-state mechanism and deliberately clear
en-passant state.

### Alpha-beta search

- four-entry clustered lock-free TT with depth, age, and exact-bound-aware
  replacement;
- tactical quiescence search, delta pruning, and a legal variant-aware static
  exchange calculation for selective losing-capture rejection;
- verified null-move pruning guarded by check state, depth, mate bounds,
  halfmove clock, and the presence of non-pawn material;
- shallow reverse/forward futility pruning, revised LMR, and negative history
  updates for failed quiet moves;
- strength mode searches the first root move in the leader, then uses
  shared-alpha parallel PVS and a shared TT for the remaining moves.

This root algorithm is not Lazy SMP: workers divide root moves rather than
running independently varied full searches.

### Reproducible node-limited research

The opt-in UCI/API setting `DeterministicNodes` is off by default. For a
node-limited search it assigns root moves deterministically, gives workers
private TTs and merges their counters/results in stable order. Repeated tests
at 1, 2, 4, and 12 threads produced identical result tuples within each fixed
thread count. It intentionally treats the node target as a soft completed-
iteration boundary, so the reported count may exceed the requested target.

It does not promise identical output between different thread counts, compiler
builds, hash sizes, positions, or histories.

### Evaluation and datasets

Infrastructure was added for connected pawns, passed pawns, and rook-like
pieces on semi-open/open files. `production()`, `empirical_v1()`, and
`published()` assign all four terms a weight of zero. The explicit
`strategic-v2` candidate uses `6`, `5`, `6`, and `12` respectively, but has
only passed a two-game smoke test and is not promoted.

Texel dataset V2 stores each sampled position's FEN so these and later
positional features can be fitted offline. V1 remains readable for material-
only analysis, but its missing positions cannot be reconstructed from the old
rows.

## Correctness results

The verification command was:

```sh
cargo fmt --all -- --check
cargo test -p capablanca-engine -p terastockfish
cargo clippy -p capablanca-engine -p terastockfish --all-targets -- -D warnings
```

All 104 tests passed and Clippy reported no warning. New targeted coverage
includes:

- indexed-board updates and full 18x18 storage capacity;
- null-move exact round trips;
- tactical generator equality with filtered full legal moves across every
  built-in variant over deterministic legal lines;
- clustered-TT collisions;
- defended-capture static exchange behavior;
- incremental strategic-state reconstruction; and
- deterministic fixed-node repetition at 1, 2, 4, and 12 threads.

## Performance results

The old starting-position depth-4 baseline searched 11,253 nodes in 0.349 s,
or 32,243 NPS. The final implementation searched 9,905 nodes in 0.182 s, or
54,363 NPS. Relative to that single baseline observation:

- elapsed time decreased by 47.9%;
- visited nodes decreased by 12.0%;
- reported NPS increased by 68.6%.

Final fixed-corpus output:

| Position | Depth | Nodes | Seconds | NPS |
|---|---:|---:|---:|---:|
| start | 4 | 9,905 | 0.182 | 54,363 |
| opening-8 | 3 | 7,129 | 0.098 | 72,751 |
| middlegame-24 | 3 | 17,597 | 0.180 | 97,977 |
| middlegame-48 | 3 | 6,036 | 0.131 | 46,095 |
| aggregate | — | 40,667 | 0.591 | 68,840 |

Elapsed time and NPS remain sensitive to CPU frequency, thermal state, and
background activity. Node reductions also reflect changed pruning and are not
equivalent to a strength gain. The benchmark is intentionally short and is a
regression sentinel, not a statistically replicated microbenchmark.

## Supported conclusions

- The starting-position search became materially faster on the recorded host
  while visiting fewer nodes.
- The new generators and reversible null-move state satisfy the current rule
  and differential tests.
- Fixed-node experiments can now be reproduced at a chosen thread count
  without making deterministic mode the playing default.
- Dataset V2 is sufficient to fit position-dependent features in later work.

## Unsupported conclusions and limitations

- No Elo, tactical-suite, or paired self-play evidence was collected, so the
  pruning configuration is not yet proven stronger or even strength-neutral.
- The exact static exchange routine is selective but potentially expensive on
  capture-rich positions; the small corpus cannot bound its worst-case cost.
- Root PVS scaling was not benchmarked across thread counts.
- The strategic V2 weights are hand-selected scaffolding, not empirical piece
  knowledge, and must not be described as calibrated.
- The benchmark compares one baseline observation with one final observation;
  it is adequate for a large engineering signal, not precise attribution to
  individual changes.

## Decision and next experiment

Retain the search foundation and keep `DeterministicNodes` opt-in. Keep all new
strategic weights out of production. The next strength experiment should be a
fixed-sample, color-swapped production-search comparison of the old and new
search configurations (or an ablation build), followed separately by fitting
strategic features from V2 FEN data and validating the frozen candidate. Do not
combine search and evaluation changes in one match if the goal is attribution.
