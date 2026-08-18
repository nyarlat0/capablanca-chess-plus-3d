# Terachess II production color balance V2 — 2026-08-18

## Status and decision

The corrected fixed-sample V2 study completed all 384 games with no unresolved
observations and no ply-limit terminations. White scored 53.52%, with a 95%
Wilson interval of 48.52–58.44%. The cumulative estimate visibly moved toward
the mid-50s as the sample grew, but the final interval crosses both equality
and the preregistered practical margin.

The result does not establish a White advantage, does not establish practical
equivalence within 47–53%, and does not establish an advantage greater than
three percentage points. It is consistent with no color advantage as well as
with a moderate White advantage. No starting-array balance change is justified
by this run.

This report completes the protocol defined in the
[V2 correction](2026-08-18-balance-protocol-correction.md). The invalid V1
pilot remains excluded from every number below.

## Provenance and method

- Source base: Git commit `cda5331cd6e81324cb701d51349d39a13d8321c5`,
  with a dirty worktree containing the documented V2 harness and repetition
  fixes. The executable hash, not the base commit alone, identifies the tested
  build.
- Executable SHA-256:
  `534d44b671d0ebb79eba7c138d9d16eb7948ead24e2b3477d3aa6bc044dd1104`.
- CSV SHA-256:
  `86c8d317839b91a2b7e6361fa73fd8867ce837b61abebed0d72c22af30314053`.
- CSV: `target/terastockfish-balance-v2.csv`, 151,054 bytes.
- Compiler: Rust 1.93.1, LLVM 21.1.8, `x86_64-unknown-linux-gnu`.
- Link configuration: Clang with mold; no additional `RUSTFLAGS` were present
  in the recorded environment.
- Hardware: AMD Ryzen 9 5900X, 12 cores and 24 hardware threads, 32 GiB RAM.
- Statistical unit: one independently generated game. A draw contributes 0.5
  to White's score; unresolved games contribute nothing. Uncertainty is the
  harness's 95% Wilson interval over game scores.
- Both colors used the unchanged production evaluation profile. Each game
  began after eight deterministic randomized legal plies from the published
  starting array, with White to move. Search received the complete repetition
  history, including those opening plies.
- Each move received 50,000 nodes and a private 32 MiB hash table. Twenty-four
  games ran concurrently. There was no ply cap. A result could arise from game
  rules or from the same side retaining an evaluation of at least 1,500 cp for
  20 consecutive plies.
- Sampling was fixed at 384 games; interim reports did not stop or select the
  run.

Exact command:

```sh
./target/release/terastockfish-balance \
  --preset eight-hour \
  --jobs 24 \
  --hash 32 \
  --root-threads 24 \
  --seed 0x42414c414e434531 \
  --output target/terastockfish-balance-v2.csv
```

## Exact-start analysis

| Nodes | Score | Depth | Best line |
|---:|---:|---:|---|
| 100,000 | +12 | 4 | `d2g5 d15g12 m2j5 m15j12` |
| 1,000,000 | +12 | 6 | `d2g5 d15g12 m2j5 m15j12 g5g8 g12g9` |
| 10,000,000 | +12 | 6 | same six-ply line |

The stable +12 equals the evaluator's tempo bonus. It is not an independent
estimate of White's theoretical advantage, and the depth-six ceiling confirms
that 50,000-node game searches are shallow for this branching factor.

## Results

| Result for White | Games |
|---|---:|
| Win | 188 |
| Draw | 35 |
| Loss | 161 |
| Unresolved | 0 |

White's score was `(188 + 0.5 × 35) / 384 = 0.5352`. The point estimate is a
3.52-percentage-point White edge. Its 95% interval, 48.52–58.44%, includes
50%, so `white_edge=false`; it is not contained within 47–53%, so
`balanced_within_margin=false`; and its lower bound is below 53%, so
`white_edge_over_margin=false`.

The cumulative White score was 64.58% after 24 games, 56.25% after 192, and
53.52% after all 384. Descriptively, the first 192 games scored 56.25% for
White, while the second 192 scored 50.78%. This is visible convergence away
from the extreme early estimate, but it is not proof that another fixed sample
would converge to exactly 50% or 53.5%.

| Termination | Games | White wins | Draws | White losses | White score |
|---|---:|---:|---:|---:|---:|
| Sustained-score adjudication | 330 | 177 | 0 | 153 | 53.64% |
| Checkmate | 19 | 11 | 0 | 8 | 57.89% |
| Threefold repetition | 35 | 0 | 35 | 0 | 50.00% |
| Stalemate | 0 | 0 | 0 | 0 | — |
| Fifty-move rule | 0 | 0 | 0 | 0 | — |
| Ply limit | 0 | 0 | 0 | 0 | — |

Natural terminations considered together scored 52.78% for White over only 54
games. Their direction does not contradict adjudicated games, but that
post-hoc subset is far too small for a separate balance conclusion. Because
85.94% of games were adjudicated, the primary result measures practical
same-engine balance under this evaluation and adjudication policy rather than
the frequency of played-out checkmates.

The games contained 3,618,955,704 searched nodes and averaged 187.9 plies. The
sum of per-game elapsed times was 80.59 game-hours. The longest game lasted 555
plies; the slowest lasted 25.27 minutes. The largest final fifty-move counter
was only 52 halfmoves. This confirms empirically that the fifty-move rule is
not a usable runtime cap in Terachess II: pawn moves and captures reset it long
before 100 while the overall game can continue for hundreds of plies.

## Interpretation and limitations

### Supported

- The corrected harness finishes uncapped games without manufacturing draws:
  all 384 observations resolved and none hit a ply limit.
- The earlier apparent mass of draws was a V1 censoring artifact. Under V2,
  only 9.11% of games were draws, all by genuine threefold repetition.
- There is no statistically demonstrated White advantage in this sample, and
  no evidence that a White advantage exceeds three percentage points under the
  preregistered criterion.

### Suggestive

- The 53.52% point estimate and 27-game excess of White wins suggest that a
  small first-move advantage remains plausible.
- The cumulative estimate's downward movement and the near-equal second half
  suggest the 56–65% early readings were sampling noise rather than a stable
  large imbalance.

### Not supported

- The run cannot prove that the position is exactly balanced.
- It cannot establish practical equivalence within ±3 percentage points; that
  requires the whole confidence interval to lie inside 47–53%.
- It cannot be interpreted as human-game balance or perfect-play balance.
  Search is shallow, openings are synthetic, and most outcomes are evaluation
  adjudications.

## Engine action and future threshold

Retain the current Terachess II starting array and production profile. Archive
the executable and CSV if exact reproduction matters; `target/` alone is not
permanent storage.

A tighter follow-up is optional rather than required for the current engine.
To target roughly ±3 percentage-point precision near equality, preregister at
least 1,024 games with the same seed scheme and frozen executable/profile.
Report adjudicated and natural endings separately again. A deeper or
adjudication-disabled sensitivity run would answer a different question and
must be recorded as a new experiment rather than merged into this dataset.
