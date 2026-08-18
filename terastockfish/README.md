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
- capture/promotion quiescence search with complete legal evasions while in
  check, delta pruning, and variant-aware static exchange pruning;
- check extensions, late-move reductions, conservative futility pruning, and
  verified null-move pruning with zugzwang guards;
- TT, MVV-LVA, promotion, killer, and history move ordering;
- indexed per-color board occupancy used by both full and tactical generation;
- a four-entry clustered, fixed-size lock-free transposition table;
- strength-oriented shared-alpha parallel root PVS with a shared TT;
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
setoption name DeterministicNodes value false
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

`DeterministicNodes` defaults to `false`. When enabled for `go nodes N`, each
root worker uses a private TT and deterministic move assignment, so repeated
runs with the same position, history, build, hash size, and thread count return
the same move, score, depth, node count, and PV. The node limit becomes a soft
iterative-deepening boundary and can be exceeded by the final completed
iteration. Time-controlled searches retain the stronger shared-TT mode.

## Rust API

```rust
use capablanca_chess_plus::Variant;
use terastockfish::{SearchLimits, SearchOptions, Searcher};

let position = Variant::TerachessII.starting_position();
let mut searcher = Searcher::new(SearchOptions {
    hash_megabytes: 256,
    threads: 4,
});
searcher.set_deterministic_nodes(false);
let result = searcher.analyze(&position, SearchLimits::depth(7));
println!("best: {:?}, score: {}", result.best_move, result.score);
```

Use `analyze_with` to receive a report after every completed iteration. Clone
the searcher's `SearchControl` before starting analysis when another thread
must stop it. Reuse one `Searcher` across positions to preserve its hash table
and move-ordering history; call `clear_hash` for a completely fresh analysis.

The default evaluator uses `EvaluationParameters::production()`, which now
equals `EvaluationParameters::strategic_v2()`. The preceding validated material
baseline remains available as `EvaluationParameters::empirical_v1()`, and the
historical Terachess II scale as `EvaluationParameters::published()`. Experiments can use
`Searcher::with_evaluation` or `set_evaluation_parameters`; replacing a
profile clears score-dependent hash and history state automatically.

## Evaluation validation

`terastockfish-eval` compares a configurable candidate with the production
baseline in deterministic paired self-play (use `--baseline published` for
historical material-profile checks). Every opening is played twice with colors
swapped. The default validation starts testing significance after 100 games
and stops after at most 400 games; use the smoke mode only to check the tool:

```sh
cargo run --release -p terastockfish --bin terastockfish-eval -- --smoke
cargo run --release -p terastockfish --bin terastockfish-eval -- \
  --set material.queen=1700 --set centrality.knight=3
```

Material, centrality, and advancement accept every `PieceKind` name. Scalar
weights are `doubled_pawn`, `isolated_pawn`, `bishop_pair`, `king_shelter`,
`king_jump`, `tempo`, `connected_pawn`, `passed_pawn`,
`rook_semi_open_file`, and `rook_open_file`. Run with `--help` for match
limits. Output contains
per-game CSV records plus W/D/L, score, and a 95% confidence interval. A
candidate has demonstrated superiority only when the interval's lower bound
exceeds 50%. A practical non-regression promotion with a weaker criterion must
be identified and documented separately.

`--baseline published|empirical-v1|production` selects the opponent explicitly.
`--profile PATH` loads a complete material profile emitted by the Texel tool;
additional `--set` options are applied after loading it.

`--candidate strategic-v2` selects the current positional profile, adding
connected/passed-pawn and open-file terms to `empirical_v1`.

### Fixed strategic-v2 validation

`terastockfish-strategic` is the resumable fixed-sample comparison used for
that promotion decision. It always compares `empirical_v1()` with `strategic-v2`
candidate, plays every opening with colors swapped, never stops early, and
atomically updates a CSV checkpoint after every parallel batch. On the recorded
12-core/24-thread machine, run:

```sh
cargo build --release -p terastockfish --bin terastockfish-strategic
./target/release/terastockfish-strategic \
  --preset overnight --jobs 24 --hash 32
```

The preset runs 192 pairs (384 games) at 50,000 nodes per move. It has no ply
cap, uses ordinary mate/draw rules plus a conservative ±1,500 score sustained
for 20 plies, and consumes roughly `2 × jobs × hash` MiB for search tables
(about 1.5 GiB in the command above). Resume the exact run with:

```sh
./target/release/terastockfish-strategic \
  --preset overnight --jobs 24 --hash 32 --resume
```

`decision=strategic_v2_stronger` means the paired 95% interval lies entirely
above 50%; `strategic_v2_weaker` means it lies below 50%; otherwise the result
is inconclusive. Any ply-capped game makes a study invalid. `quick` and
`smoke` verify the harness only; `deep` runs 256 pairs at 100,000 nodes. Exact
methodology and the precommitted decision rule are in the
[strategic V2 protocol](docs/research/2026-08-18-strategic-v2-validation-protocol.md).
The completed run scored 53.39% with paired 95% CI 48.62–58.15%. It did not
prove superiority, but the user accepted it as a practical non-regression
baseline; the deviation from the strict criterion is recorded in the
[result report](docs/research/2026-08-18-strategic-v2-validation-result.md).

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

Newly written datasets use `TERASTOCKFISH_TEXEL_DATA_V2` and retain the FEN of
every sampled position. This permits later positional-feature fitting without
replaying the original games. The loader remains compatible with V1 material
datasets; their samples have no recoverable FEN and therefore support only the
original material features.

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
therefore promoted as `EvaluationParameters::empirical_v1()` and became the
stable material baseline. Strategic V2 later retained that material table and
added structural terms. Full results and limitations are in the
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

### Starting-position color balance

`terastockfish-balance` measures White's empirical advantage without comparing
different evaluators: `production()` plays both colors. The `eight-hour` preset
first analyzes the exact starting position at several node budgets, then plays
384 reproducible eight-ply randomized openings at 50,000 nodes per move. Search
receives the complete game repetition history. There is no ply cap: games end
by mate, stalemate, threefold repetition, the fifty-move rule, or a conservative
±1,500 evaluation sustained for 20 plies. Recall that every pawn move or capture
resets the fifty-move clock, so it is not a general wall-clock bound on a game
with 128 starting pieces.

If a manual `--max-plies` safety cap is supplied, capped games are recorded as
`unresolved`, excluded from W/D/L scoring, and invalidate conclusions when they
exceed 10% of the sample. The V2 atomic CSV stores opening and final FENs plus
the final White-relative score and doubles as a resume checkpoint. Approximately
4–8 hours remains the operational target, not a hard deadline.

On a 12-core/24-thread machine with 32 GiB RAM, run:

```sh
cargo build --release -p terastockfish --bin terastockfish-balance
./target/release/terastockfish-balance \
  --preset eight-hour \
  --jobs 24 --hash 32 --root-threads 24 \
  --seed 0x42414c414e434531 \
  --output target/terastockfish-balance-v2.csv
```

Resume with the same command plus `--resume --skip-root`. Explicit CLI options
override preset values. The report separates ordinary evidence of a White edge
from practical equivalence within 47–53% and evidence that the edge exceeds
three percentage points. With 384 games the protocol is designed to detect a
repeat of the preliminary 6–7-point edge; failure to do so does not by itself
prove strict ±3-point equivalence. Methodology and the preliminary estimate are recorded in the
[balance prestudy](docs/research/2026-08-17-color-balance-prestudy.md). The
first capped pilot and the corrected V2 protocol are documented in the
[protocol correction](docs/research/2026-08-18-balance-protocol-correction.md).
The completed 384-game V2 study scored White at 53.52%, with a 95% interval of
48.52–58.44%. It showed convergence away from extreme early estimates but
established neither a White advantage nor strict ±3% equivalence; see the
[final balance report](docs/research/2026-08-18-color-balance-v2.md).

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

The dependency-free benchmark uses the full starting array plus fixed seeded
opening, early-middlegame, and middlegame snapshots. Each case reports the
median wall time of three fresh single-thread searches, node count, NPS, and
best move, followed by an aggregate. The current search-foundation benchmark
and its limitations are recorded in the
[research log](docs/research/2026-08-18-search-strength-foundation.md).

Future strength work should validate the strategic V2 evaluator, add
variant-specific piece-square or mobility terms, improve parallel scaling
beyond root splitting, and eventually investigate an NNUE-style evaluator.
All can preserve the public position/depth API and the 18x18 TT move format.
