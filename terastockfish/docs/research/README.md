# Terachess II research log

This directory is the permanent record for empirical Terachess II work in
TeraStockfish. A result belongs here even when it is negative, inconclusive,
or superseded. The crate README explains how to use the tools; these reports
explain what was measured, how it was measured, and what may legitimately be
concluded from it.

## Experiments

| Date | Experiment | Result | Engine action |
|---|---|---|---|
| 2026-08-17 | [Texel material fit with paired self-play](2026-08-17-texel-material-values.md) | Candidate scored 57.03%, paired 95% CI 50.76–63.30%; tool recommendation: `true` | Retain as a validated candidate; do not silently replace the published profile |
| 2026-08-17 | [Independent Texel material replication](2026-08-17-texel-material-values-seed2.md) | Candidate scored 55.99%, paired 95% CI 51.57–60.41%; joint strength replicated, individual coefficients unstable | Keep `production=published`; diagnose pooled coefficient stability before final validation |
| 2026-08-17 | [Material feature identifiability diagnostic](2026-08-17-material-feature-identifiability.md) | Both designs are full-rank; pooled standardized condition number 4.46; no severe Archer collinearity | Investigate optimizer/bootstrap convergence and model sensitivity before more self-play |
| 2026-08-17 | [Pooled material convergence and profile freeze](2026-08-17-pooled-material-convergence.md) | Eight restarts, 256×48 bootstrap and eight-split CV stabilize a regularization-0.08 candidate | Freeze candidate; proceed to fixed 200-pair/100k-node validation |
| 2026-08-17 | [Final material-profile validation](2026-08-17-final-material-validation.md) | Frozen candidate scored 55.25%, paired 95% CI 50.89–59.61% over 200 color-swapped pairs | Promote unchanged as `empirical_v1()` and make it the production default |
| 2026-08-17 | [Retrospective color balance and prospective protocol](2026-08-17-color-balance-prestudy.md) | Existing paired training data estimate White at 56.66%, paired 95% CI 54.37–58.94%; important protocol confounds remain | Treat as preliminary evidence; run fixed production-vs-production balance study |
| 2026-08-18 | [Balance pilot invalidation and V2 protocol](2026-08-18-balance-protocol-correction.md) | 33/48 games hit the 300-ply cap and were misclassified as draws; 12 repeated and only 3 mated | Reject the pilot; add full repetition history, uncapped play, unresolved outcomes, conservative adjudication, and Wilson intervals |
| 2026-08-18 | [Production color-balance V2](2026-08-18-color-balance-v2.md) | White scored 53.52% over 384 resolved games, 95% CI 48.52–58.44%; the estimate converged downward but neither advantage nor ±3% equivalence was established | Retain the current array; treat a small White edge as plausible but unproven |
| 2026-08-18 | [Search-strength foundation](2026-08-18-search-strength-foundation.md) | Starting-position depth-4 time fell 47.9% and NPS rose 68.6%; 104 tests pass; no strength match was run | Retain implementation, keep deterministic mode opt-in and strategic V2 unpromoted; run paired search ablations next |

## Required record for future research

Every empirical Terachess II investigation must add a dated report here and
link it in the table above. This includes evaluation tuning, search changes,
performance benchmarks, rule interpretations tested by simulation, opening
experiments, and comparisons with other engines.

Start new reports from [TEMPLATE.md](TEMPLATE.md).

Each report must preserve:

1. the question, hypothesis, and decision criterion chosen before interpreting
   the result;
2. the source revision and dirty-tree status, compiler/build flags, hardware,
   exact command, deterministic seeds, and hashes of retained artifacts;
3. data provenance, sampling and exclusion rules, train/holdout boundaries,
   pairing or randomization, resource limits, and stopping conditions;
4. raw counts as well as derived statistics, uncertainty intervals, and the
   statistical unit used for those intervals;
5. conclusions separated into supported findings, suggestive findings, and
   claims the experiment cannot support;
6. limitations, confounders, reproducibility gaps, and the concrete engine
   decision caused by the result.

Do not rewrite an old report when a result changes. Add a new dated report,
link the predecessor, and state whether the new evidence confirms, supersedes,
or contradicts it. Never select only favorable runs. Smoke tests validate the
harness and must not be reported as strength evidence.

Before a long run, record provenance in the report stub (or alongside the
checkpoint), including `git rev-parse HEAD`, `git status --short`, `rustc -Vv`,
the executable SHA-256, and any `RUSTFLAGS`. Artifacts under `target/` are local
and disposable, so important datasets/checkpoints must be archived separately
if exact reproduction matters.
