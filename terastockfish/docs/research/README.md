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
