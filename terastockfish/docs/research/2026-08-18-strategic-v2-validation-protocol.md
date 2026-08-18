# Strategic V2 validation protocol

Date prepared: 2026-08-18

Status: protocol frozen before the strength run; no strength result yet.

## Question and frozen candidate

Does adding four inexpensive structural terms to the current production
evaluator improve TeraStockfish's Terachess II playing strength?

The candidate differs from `EvaluationParameters::production()` only by:

| Term | Production | Strategic V2 |
|---|---:|---:|
| connected pawn | 0 | 6 per pawn |
| passed pawn | 0 | 5 per relative rank |
| rook-like piece on semi-open file | 0 | 6 |
| rook-like piece on open file | 0 | 12 |

A connected pawn has a friendly pawn on an adjacent file and no more than one
rank away, covering pawn chains and phalanxes without connecting pieces on
unrelated ranks. A passed pawn has no enemy pawn ahead on its own or either
adjacent file. Rook-like file terms apply to Rook, Chancellor, and Admiral.
All material and pre-existing positional weights are identical between arms.

## Hypothesis and decision rule

The null hypothesis is candidate paired score 50%. The primary run is fixed at
192 color-swapped pairs; there is no significance-based early stopping.

- valid paired 95% CI entirely above 50%: evidence to promote Strategic V2;
- valid paired 95% CI entirely below 50%: reject Strategic V2;
- interval crossing 50%: inconclusive, keep production unchanged;
- any game reaching a configured ply cap: invalidate the run rather than
  silently treating truncations as trustworthy draws.

The interval's statistical unit is the mean score of the two games sharing an
opening. This preserves the color-swapped pairing. The tool reports ordinary
game W/D/L as descriptive raw counts.

## Primary command

On the AMD Ryzen 9 5900X host with 24 logical CPUs and 32 GiB RAM:

```sh
cargo build --release -p terastockfish --bin terastockfish-strategic
./target/release/terastockfish-strategic \
  --preset overnight \
  --jobs 24 \
  --hash 16
```

Frozen primary settings:

- 192 pairs / 384 games;
- 50,000 nodes per move;
- eight deterministic randomized opening plies;
- seed `0x5354524154454732` (`6004514643731892018`);
- one search thread per game and 24 concurrent pairs;
- 16 MiB TT per player, approximately 768 MiB total TT allocation at peak;
- no ply cap;
- score adjudication only after the same side remains at least 1,500
  centipawns ahead for 20 consecutive half-moves;
- baseline `production`, candidate `strategic-v2`;
- atomic output `target/terastockfish-strategic-v2-overnight.csv`.

Resume after interruption using the identical command plus `--resume`. The CSV
header freezes the sample size, seed, node budget, hash, termination settings,
full parameter fingerprints, and a semantic study implementation identifier.
Changing any of these rejects resume. Do not rebuild or replace the executable
during one run; archive its SHA-256 with the completed report.

## Pairing, sampling, and stopping

Pair index and seed deterministically generate one legal opening. Strategic V2
plays that position once as White and once as Black. Pair indices are
independent, so complete pairs can run concurrently and resume in index order.
Each individual search is single-threaded and node-limited. Pair count was
chosen before observing results and all 192 pairs are played even if an interim
interval excludes 50%.

Games terminate by checkmate, stalemate, fifty-move rule, threefold repetition,
or the sustained-score rule. The primary preset sets `max_plies=0`, which now
correctly disables the old safety cap. Termination counts must be reported with
the result because excessive score adjudication is an important limitation.

## Verification before the run

The implementation must pass release tests, Clippy with warnings denied, the
strategic tool's smoke/resume round trip, existing perft fixtures, randomized
FEN and make/unmake stress across every built-in variant, and the fixed search
benchmark. Smoke and quick presets are harness diagnostics, never strength
evidence.

## Known limitations

- Both arms use the same engine and differ only in static evaluation. Results
  measure practical compatibility with this search, not an engine-independent
  truth about the features.
- Random legal opening plies are paired but are not a curated opening book;
  external validity across plausible human openings is limited.
- Score adjudication depends on the competing evaluators. Its high sustained
  threshold limits but does not eliminate adjudication bias.
- One frozen joint candidate cannot identify which individual term caused a
  win or loss. Component ablations should follow only if attribution is needed.
- The normal paired interval is an approximation; the fixed sample and raw
  pair records are retained so a bootstrap sensitivity analysis can be added
  without replaying games.

## Required completion record

After the run, append a new dated result report rather than rewriting this
protocol. Preserve the source revision and dirty status, `rustc -Vv`, exact
command, executable SHA-256, CSV SHA-256, W/D/L, pair count, termination counts,
paired score and interval, elapsed time, supported and unsupported conclusions,
and the resulting production-profile decision.
