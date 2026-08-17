# TeraStockfish

`terastockfish` is an original large-board alpha-beta analysis engine built on
the legal move generator in `capablanca-engine`. It currently targets
Terachess II (16x16), while all square-indexed search structures and move
encoding reserve capacity for boards up to 18x18.

The crate provides both an embeddable Rust API and an asynchronous UCI process.
It is not derived from Stockfish or Fairy-Stockfish and does not claim equal
playing strength; the name describes the intended role and architecture.

## Search architecture

- iterative deepening with principal-variation search;
- aspiration windows with automatic full-window recovery;
- capture/check quiescence search;
- check extensions, late-move reductions, and conservative shallow razoring;
- TT, MVV-LVA, promotion, killer, and history move ordering;
- a fixed-size lock-free transposition table shared by root workers;
- 21-bit signed TT scores, leaving a safe range for Terachess material and
  distance-to-mate values;
- repetition, fifty-move, mate, and stalemate terminal handling;
- reversible make/unmake with incremental position keys and evaluation state;
- cached king locations in the rules board;
- depth, node, fixed move-time, clock, infinite, and external stop limits;
- a deterministic hand-written Terachess evaluation using the published
  relative material scale plus centrality, development, pawn structure,
  bishop pair, king shelter, and tempo terms.

The board hash includes the complete piece array, dimensions, side to move,
castling and initial king-jump rights, en-passant state, rules identity, and
the halfmove clock where relevant. Arbitrary legal Terachess II FEN positions
can therefore be analysed independently of the built-in start position.

## UCI usage

Build or run the native engine:

```sh
cargo build --release -p terastockfish
cargo run --release -p terastockfish
```

Example session:

```text
uci
setoption name Hash value 512
setoption name Threads value 4
isready
position startpos moves a4a6 a13a11
go depth 8
stop
quit
```

`position fen <six fields> [moves ...]` accepts any Terachess II position.
Supported `go` limits are `depth`, `nodes`, `movetime`, `wtime`, `btime`,
`winc`, `binc`, `movestogo`, `infinite`, and the diagnostic `perft` command.
Moves use long coordinate notation, including multi-digit ranks, for example
`a4a6` and `e10e12`.

## Rust API

```rust
use capablanca_chess_plus::Variant;
use terastockfish::{SearchLimits, SearchOptions, Searcher};

let position = Variant::TerachessII.starting_position();
let mut searcher = Searcher::new(SearchOptions {
    hash_megabytes: 256,
    threads: 4,
});
let result = searcher.analyze(&position, SearchLimits::depth(7));
println!("best: {:?}, score: {}", result.best_move, result.score);
```

Use `analyze_with` to receive a report after every completed iteration. Clone
the searcher's `SearchControl` before starting analysis when another thread
must stop it. Reuse one `Searcher` across positions to preserve its hash table
and move-ordering history; call `clear_hash` for a completely fresh analysis.

The default evaluator uses `EvaluationParameters::production()`, which now
equals the independently validated `EvaluationParameters::empirical_v1()`.
The historical Terachess II scale remains available through
`EvaluationParameters::published()`. Experiments can use
`Searcher::with_evaluation` or `set_evaluation_parameters`; replacing a
profile clears score-dependent hash and history state automatically.

## Evaluation validation

`terastockfish-eval` compares a candidate profile with the published baseline
in deterministic paired self-play. Every opening is played twice with colors
swapped. The default validation starts testing significance after 100 games
and stops after at most 400 games; use the smoke mode only to check the tool:

```sh
cargo run --release -p terastockfish --bin terastockfish-eval -- --smoke
cargo run --release -p terastockfish --bin terastockfish-eval -- \
  --set material.queen=1700 --set centrality.knight=3
```

Material, centrality, and advancement accept every `PieceKind` name. Scalar
weights are `doubled_pawn`, `isolated_pawn`, `bishop_pair`, `king_shelter`,
`king_jump`, and `tempo`. Run with `--help` for match limits. Output contains
per-game CSV records plus W/D/L, score, and a 95% confidence interval. A
candidate should replace the published default only when the interval's lower
bound exceeds 50%.

`--baseline published|production` selects the opponent explicitly.
`--profile PATH` loads a complete material profile emitted by the Texel tool;
additional `--set` options are applied after loading it.

### Automatic material tuning

For a practical overnight run, `terastockfish-texel` collects one reusable
self-play dataset, fits all 24 non-pawn/non-king material values offline, and
spends the remaining budget validating the result against the published
profile. On a 12-core/24-thread machine with 32 GiB RAM, the configuration used
by the first recorded full experiment was:

```sh
./target/release/terastockfish-texel \
  --preset overnight --jobs 24 --hash 32 \
  --dataset target/terastockfish-texel.csv \
  --checkpoint target/terastockfish-texel.chk
```

The preset caps dataset collection at eight hours and the cumulative run at
twelve hours. It targets up to 1,024 training games at 20,000 nodes per move,
then up to 192 validation games at 50,000 nodes. Dataset and stage checkpoints
are atomically updated after each parallel batch. Add `--resume` to the same
command after an interruption. The report includes train/holdout loss, sample
coverage and bootstrap 95% intervals for every piece, final W/D/L, ready-to-use
`material.<piece>=<value>` lines, and `recommended=true|false|inconclusive`.
The complete methodology, result tables, limitations, and decision record for
that run are in the [Terachess II research log](docs/research/README.md).

### Independent confirmation and pooled fit

Preserve the binary used for the first dataset before rebuilding it. The first
recorded binary is archived locally at
`target/research/2026-08-17/terastockfish-texel-first-run` with SHA-256
`d6575deced3ed09c17ebe5861439c83afe2612b34835241eaab91417417ce0d4`.
Use that exact executable for the fixed-size second run:

```sh
./target/research/2026-08-17/terastockfish-texel-first-run \
  --preset overnight --jobs 24 --hash 32 \
  --seed 0x5345434f4e445255 \
  --validation-min-pairs 192 --validation-max-pairs 192 \
  --dataset target/terastockfish-texel-seed2.csv \
  --checkpoint target/terastockfish-texel-seed2.chk
```

The independent second run is documented in the
[research log](docs/research/2026-08-17-texel-material-values-seed2.md). It
replicated the playing-strength gain but exposed unstable individual
coefficients, particularly Archer. The subsequent
[pooled convergence study](docs/research/2026-08-17-pooled-material-convergence.md)
resolved that instability with eight restarts, 256×48 game bootstraps and
eight-split regularization cross-validation. It froze regularization 0.08 and
the `empirical-v1-candidate` profile. Its final fixed validation was run with:

```sh
cargo build --release -p terastockfish --bins
./target/release/terastockfish-texel \
  --preset overnight --jobs 24 --hash 32 \
  --input-dataset target/terastockfish-texel.csv \
  --input-dataset target/terastockfish-texel-seed2.csv \
  --dataset target/terastockfish-final-validation.csv \
  --checkpoint target/terastockfish-final-validation.chk \
  --profile-output target/terastockfish-final-validation.profile \
  --profile-name empirical-v1-candidate \
  --fit-epochs 96 --bootstrap-epochs 48 --bootstrap-replicates 256 \
  --split-seed 0x504f4f4c5f53504c \
  --optimizer-seed 0x4f5054494d495a45 \
  --bootstrap-seed 0x424f4f5453545250 \
  --regularization 0.08 \
  --seed 0x454d504952494341 \
  --validation-seed 0x56414c4944415445 \
  --validation-min-pairs 200 --validation-max-pairs 200 \
  --validation-nodes 100000 --total-hours 24
```

Setting minimum and maximum validation pairs to the same value made this a
fixed-sample experiment, avoiding uncorrected early significance stopping.
The frozen candidate scored 55.25% with paired 95% CI 50.89–59.61%; its lower
bound exceeded the preregistered 50% promotion threshold. The exact profile was
therefore promoted as `EvaluationParameters::empirical_v1()` and is now the
production default. Full results and limitations are in the
[final validation report](docs/research/2026-08-17-final-material-validation.md).

The frozen profile can also be rechecked independently without retyping all
material values:

```sh
./target/release/terastockfish-eval \
  --baseline published \
  --profile target/terastockfish-material-candidate-v1.profile \
  --min-pairs 200 --max-pairs 200 --nodes 100000 \
  --jobs 24 --hash 32 --seed 0x46494e414c434845
```

Check the complete pipeline in seconds before an overnight run:

```sh
cargo run --release -p terastockfish --bin terastockfish-texel -- \
  --preset smoke \
  --dataset target/terastockfish-texel-smoke.csv \
  --checkpoint target/terastockfish-texel-smoke.chk
```

The older `terastockfish-tune` performs direct SPSA self-play. It remains
available for exhaustive multi-day confirmation:

`terastockfish-tune` refines all 24 non-royal, non-pawn material values at
once with SPSA self-play. Pawn stays at 100 to anchor the scale and king stays
at 0 because mate is represented separately. Each iteration compares opposite
perturbations in color-swapped games; the final profile is then matched against
the untouched published profile.

Start with a cheap end-to-end check:

```sh
cargo run --release -p terastockfish --bin terastockfish-tune -- \
  --preset quick --jobs 4
```

For a serious long run, choose the number of concurrent pairs for the machine:

```sh
cargo run --release -p terastockfish --bin terastockfish-tune -- \
  --preset deep --jobs 8 --hash 8 \
  --checkpoint target/terastockfish-material-deep.chk
```

The deep preset performs up to 4,096 tuning games, then 100–400 independent
validation games. `--hash` is allocated twice per concurrent job, so the
example uses roughly 128 MiB for search tables. Progress is printed as CSV and
the checkpoint is atomically replaced after every completed iteration. Resume
the same run with:

```sh
cargo run --release -p terastockfish --bin terastockfish-tune -- \
  --preset deep --jobs 8 --hash 8 \
  --checkpoint target/terastockfish-material-deep.chk --resume
```

The final `recommended=true` means the tuned profile beat the published one
with a paired 95% confidence interval entirely above 50%. Until that happens,
the tool prints the experimental values but does not alter the engine default.
Use `--help` to override iterations, pairs, node limits, SPSA coefficients, and
validation size.

## Verification and profiling

```sh
cargo test -p capablanca-engine -p terastockfish
cargo bench -p terastockfish --bench search
cargo run --release -p terastockfish --bin terastockfish-eval -- --smoke
cargo run --release -p terastockfish --bin terastockfish-tune -- --preset smoke
cargo run --release -p terastockfish --bin terastockfish-texel -- --preset smoke
```

The benchmark prints elapsed time, searched nodes, and nodes per second for the
full Terachess II starting array. It is intentionally dependency-free so the
same binary can be profiled on the eventual deployment machine.

Future strength work can add variant-specific piece-square tables, null-move
and stronger pruning verified for this ruleset, persistent opening data, and
an NNUE-style evaluator without changing the public position/depth API or the
18x18 TT move format.
