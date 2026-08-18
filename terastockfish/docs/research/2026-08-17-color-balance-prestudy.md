# Retrospective color balance and prospective protocol — 2026-08-17

## Status and conclusion

The two existing Texel training datasets contain preliminary evidence of a
White advantage in the current Terachess II starting arrangement. Across 1,003
complete color-controlled opening pairs, White scored **56.66%**, with paired
95% confidence interval **54.37–58.94%**. The signal replicated in both seeds.

This is not yet a final balance estimate. Those games were generated to tune
material, used perturbed published profiles and score adjudication, and only
parties yielding quiet training samples survive in the CSV. A dedicated
production-vs-production fixed-sample experiment is therefore preregistered
below.

## Retrospective question and data

The retrospective question is whether data already collected for another
purpose are compatible with no White advantage. Each training opening was
played twice: opposite material perturbations were assigned once to White and
once to Black. Averaging White's score over the two games controls the opening
and profile assignment, so one opening pair—not one sampled position—is the
statistical unit.

| Dataset | Complete pairs | White W/D/L | White score | Paired 95% CI |
|---|---:|---:|---:|---:|
| First seed | 503 | 533/110/363 | 58.45% | 55.28–61.62% |
| Second seed | 500 | 498/101/401 | 54.85% | 51.56–58.15% |
| Pooled | 1,003 | 1,031/211/764 | **56.66%** | **54.37–58.94%** |

The difference between seed estimates is 3.60 percentage points, with 95%
interval −0.97 to +8.17 points. Thus the runs differ in magnitude but do not
statistically contradict one another. Both point estimates favor White and
the pooled interval excludes 50%.

The first and second CSV SHA-256 hashes are respectively
`3de968a8cd7bef80141d97d5e0b27f48f75f8031188475c417b912ea825734ef`
and `305618ae7dfff27c33c3f58d471b97dcf144faf9553d092b7e885ad6c30e3415`.

## Retrospective methodology and limitations

The games started from the standard array followed by eight uniformly random
legal half-moves, leaving White to move. They used 20,000 nodes per move, a
240-ply cap, score adjudication at ±800 sustained for 12 plies, and independently
perturbed published material coefficients around `exp(±0.12)`. The CSV stores
White's final outcome on every quiet sampled position; outcomes were deduplicated
by game ID before analysis.

Only 503/512 and 500/512 opening pairs had both games represented. Games with
no eligible quiet sampled position are absent, making the estimate conditional
on the training sampler. Uniformly random opening moves are useful for
diversification but are not a curated set of plausible strong openings. The
engine profile, search budget, adjudication, and maximum length differ from the
current production setup. Consequently the retrospective result rejects
`White score = 50%` for this generator, but cannot establish a theoretical
first-move value or quantify optimal human play.

## Preregistered production balance study

The dedicated `terastockfish-balance` harness removes the material-comparison
confound:

- both colors use the identical `EvaluationParameters::production()` profile;
- exact-start searches at 100,000, 1,000,000, and 10,000,000 nodes report score
  convergence and principal variations;
- 384 independent openings use exactly eight deterministic random legal
  half-moves from the standard array, always leaving White to move;
- every generated game is retained; there is no quiet-position filter;
- each move receives 50,000 nodes, games stop at 300 plies, and score
  adjudication is disabled to avoid circularly converting evaluation bias into
  results;
- the sample size is fixed before results and there is no significance-based
  early stop;
- each CSV row retains White result, termination, plies, nodes, elapsed time,
  and opening FEN; its metadata freezes the complete production material vector,
  and atomic rewrites make the file a resumable checkpoint.

The primary estimate is White's score, with a normal 95% interval over games.
Three claims are reported separately using a preregistered practical margin of
three percentage points:

1. any White edge is supported when the interval lower bound exceeds 50%;
2. practical near-balance is supported when the entire interval lies within
   47–53%;
3. a White edge larger than the practical margin is supported when the lower
   bound exceeds 53%.

Exact-start search is complementary evidence, not an independent game sample.
Repeated deterministic games from the unmodified start would be duplicates,
which is why uncertainty is estimated over randomized opening continuations.

This fixed `eight-hour` profile replaces the earlier 1,024-game/100,000-node
draft before any prospective games were examined. It is sized from the prior
400-game validation runtime of 6 h 54 min. Halving the node budget and using
384 games gives an idealized estimate near 3.3 hours; disabling score
adjudication can lengthen games, so the operational expectation is 4–8 hours.
This is not a hard wall-clock guarantee.

Under the retrospective 56.66% result distribution, 384 games would produce
an expected interval of roughly 51.97–61.34%. The study is therefore powered
to detect a repeat of the observed 6–7-point edge, but it may remain
inconclusive about strict practical equivalence within ±3 points.

## Exact command

For the recorded 12-core/24-thread, 32-GiB machine:

```sh
cargo build --release -p terastockfish --bin terastockfish-balance

./target/release/terastockfish-balance \
  --preset eight-hour \
  --jobs 24 --hash 32 --root-threads 24 \
  --seed 0x42414c414e434531 \
  --output target/terastockfish-balance.csv
```

Resume without repeating exact-start searches by adding both `--resume` and
`--skip-root` to the otherwise identical command. A smoke run only checks the
harness:

```sh
cargo run -p terastockfish --bin terastockfish-balance -- \
  --smoke --output target/terastockfish-balance-smoke.csv
```

Before treating the prospective result as final, append its command output,
artifact and executable hashes, runtime, termination distribution, exact-start
scores, and conclusion to a new report rather than rewriting this preregistration.
Do not rebuild or replace the executable between the initial run and a resume;
the material vector is checked automatically, while the executable hash is the
record of search implementation identity.
