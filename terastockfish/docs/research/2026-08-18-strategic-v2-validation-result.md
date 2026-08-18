# Strategic V2 fixed-sample validation result

Date completed: 2026-08-18

Predecessor: [frozen validation protocol](2026-08-18-strategic-v2-validation-protocol.md)

## Question and decision criteria

The experiment compared the `empirical_v1` evaluator with Strategic V2, which
keeps every existing weight and adds connected-pawn, passed-pawn, semi-open
rook-file, and open rook-file weights of 6, 5, 6, and 12.

The preregistered superiority rule required the paired 95% confidence interval
to lie entirely above 50%. The run was fixed at 192 pairs with no early stop.
After seeing the final result, the user made a separate product decision to
accept the candidate if it showed a positive point estimate without evidence
of a practically large regression. This second rule is post hoc and must not
be presented as the original statistical criterion.

## Provenance and command

- base revision: `0a26e1f71cc9056fdd7d7a3aae0b70a55e6f8469`;
- worktree: dirty with the audited search/evaluation-tool changes documented
  on 2026-08-18;
- host: AMD Ryzen 9 5900X, 12 cores / 24 threads, 32 GiB RAM;
- compiler: `rustc 1.93.1 (01f6ddf75 2026-02-11)`, LLVM 21.1.8;
- exact study binary archived locally at
  `target/research/2026-08-18-strategic-v2/terastockfish-strategic`;
- binary SHA-256:
  `c7b8fcf055cdfdf4a8658295d1e588c9c2e56d30a5592fd93e2c80b7ef07dfe0`;
- completed CSV archived locally at
  `target/research/2026-08-18-strategic-v2/terastockfish-strategic-v2-overnight.csv`;
- CSV SHA-256:
  `0602786905c85185fa0129c61de5d69a1253bd0d12f15707140507b97949ec1b`.

The run was started and later resumed with the same settings:

```sh
./target/release/terastockfish-strategic \
  --preset overnight \
  --jobs 24 \
  --hash 32 \
  --resume
```

Frozen settings were 192 color-swapped pairs, 50,000 nodes per move, eight
randomized opening plies, seed `0x5354524154454732`, 32 MiB hash per player,
no ply cap, and score adjudication at ±1,500 sustained for 20 plies. The
interruption occurred between atomic batches; resume repeated only the
unfinished batch and did not alter the fixed sample.

## Results

| Measure | Result |
|---|---:|
| Pairs / games | 192 / 384 |
| Strategic V2 wins | 202 |
| Draws | 6 |
| Strategic V2 losses | 176 |
| Candidate score | 53.3854% |
| Paired 95% CI | 48.62–58.15% |
| Candidate as White | 52.0833% (99/2/91) |
| Candidate as Black | 54.6875% (103/4/85) |
| Summed per-game elapsed time | 83.799 h |

Paired-score distribution:

| Candidate points across two color-swapped games | Pair count |
|---:|---:|
| 0.0 | 36 |
| 0.5 | 4 |
| 1.0 | 100 |
| 1.5 | 2 |
| 2.0 | 50 |

Terminations:

| Termination | Games |
|---|---:|
| Checkmate | 49 |
| Stalemate | 0 |
| Fifty-move rule | 1 |
| Threefold repetition | 5 |
| Score adjudication | 329 |
| Ply limit | 0 |

The run is mechanically valid: all 192 complete pairs are present, colors are
swapped within identical opening FENs, no ply cap occurred, and the fixed
sample was completed. Intermediate crossings above 50% were not used for
stopping or the final conclusion.

## Interpretation

### Supported

- The point estimate favors Strategic V2 by 3.39 percentage points.
- The experiment found no statistically significant superiority because the
  paired interval crosses 50%.
- Under this approximate interval, disadvantages larger than 1.38 percentage
  points are outside the 95% interval. This is evidence against a large
  regression, not proof of literal non-inferiority at a zero margin.
- Results were positive with the candidate assigned either color, so the large
  provisional color imbalance seen at 120 pairs did not persist in the final
  sample.

### Not supported

- The experiment does not prove that Strategic V2 is stronger.
- A non-inferiority margin was not preregistered, so the final lower bound
  cannot retroactively satisfy a formal non-inferiority test.
- The result does not isolate which of the four new terms helped or hurt.

The original decision criterion therefore yields `inconclusive`. By explicit
user product decision, Strategic V2 is nevertheless promoted as the practical
production baseline: its estimate is positive, no large regression was found,
and the added terms are inexpensive and positionally motivated. The API keeps
`empirical_v1()` available for exact comparisons, while `production()` now
returns `strategic_v2()`.

## Production action and verification

The promoted profile keeps every `empirical_v1` material and positional value
unchanged and sets only these four additional weights, in the same centipawn
scale where a pawn is 100:

| Weight | Production value |
|---|---:|
| Connected pawn | 6 |
| Passed pawn per relative rank | 5 |
| Rook on semi-open file | 6 |
| Rook on open file | 12 |

`EvaluationParameters::production()`, the default `Searcher`, `evaluate()`,
and `piece_value()` now resolve through Strategic V2. The old profile remains
addressable as `EvaluationParameters::empirical_v1()`. The dedicated strategic
validator explicitly compares `empirical_v1` against `strategic_v2`, so a
future reproduction does not accidentally compare production with itself.
Research checkpoint fingerprints now include the complete evaluation profile,
including these four values.

After promotion, 111 release-mode engine and TeraStockfish tests passed,
Clippy passed for every target with warnings denied, all release binaries
built successfully, and a smoke run confirmed the validator labels the two
profiles correctly. The smoke result is a harness check only and contributes
no strength evidence.

## Limitations and follow-up

Score adjudication ended 329/384 games (85.7%), so the result primarily tests
agreement with a sustained high evaluation rather than conversion to mate.
Both competitors share the same search and differ only in evaluation, and the
opening sample uses deterministic random legal plies rather than a curated
book. The paired interval is a normal approximation.

Future evaluation changes should compare against Strategic V2. If stronger
evidence is later needed, run at least 350–500 independent pairs with a new
seed or perform frozen component ablations; do not reinterpret this completed
sample through repeated optional stopping.
