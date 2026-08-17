# Final material-profile validation — 2026-08-17

## Decision

The frozen `empirical-v1-candidate` passed its preregistered promotion gate.
Against `published()` it scored **55.25%** over 200 color-swapped pairs, with a
paired 95% confidence interval of **50.89–59.61%**. Because the lower bound is
above 50%, the exact candidate values are promoted as
`EvaluationParameters::empirical_v1()` and become the production default.
The historical `published()` profile remains available.

## Question and fixed criterion

The question was whether the pooled, regularization-0.08 material profile from
the preceding convergence study improves playing strength over the published
Terachess II values. Before this run, the candidate, data, regularization,
seeds, sample size, node limit, and decision rule were frozen.

The acceptance criterion was a paired 95% confidence-interval lower bound
strictly above 50% after exactly 200 pairs. Fixing both validation limits at
200 prevents optional stopping after an initially favorable result.

## Provenance and configuration

- Source revision: `5488fa9ce9e74a847fe602a294b7c3bf61609878`
- Working tree: research/tooling changes were uncommitted; the executable hash
  below identifies the tested program.
- Compiler: `rustc 1.93.1 (01f6ddf75 2026-02-11)`, LLVM 21.1.8.
- Hardware reported by the operator: 12 physical cores, 24 hardware threads,
  32 GiB RAM.
- Jobs: 24; hash per search: 32 MiB.
- Validation: 200 paired openings, each played twice with colors swapped;
  100,000 nodes per move.
- Validation seed: `0x56414c4944415445`.
- Fit seeds: split `0x504f4f4c5f53504c`, optimizer
  `0x4f5054494d495a45`, bootstrap `0x424f4f5453545250`.
- Candidate regularization: 0.08; 96 fit epochs; 256 bootstrap replicates at
  48 epochs.
- Total recorded runtime: 24,837.285 seconds (6 h 53 min 57 s).
- `RUSTFLAGS`: not recorded; no non-default value was reported.

The exact command is retained in the crate README and was supplied unchanged
from the preceding report.

## Inputs and artifacts

| Artifact | SHA-256 |
|---|---|
| Merged 24,801-position dataset | `9d2b944664eb7e787e63294ea267c4569ee1b54a071c507456e84813326a2a65` |
| Completed checkpoint | `7964cc90c737230c962895d41232b2d8ff5fa35da80f0a980a2ad1e92ed611f5` |
| Frozen profile | `656c9b5e1b182c9a24b2dd05cd88b1f81989229ad497f954853501c1d4ccfc4e` |
| Validation executable | `e3138a5184e3f6a1168ccdeb3a225e96a97c7d30806deae18fb7b4ec1aace888` |

The dataset contains 2,048 generated games and 24,801 sampled positions. The
game-grouped split contains 20,100 training and 4,701 holdout positions.

## Results

```text
validation pairs: 200
individual games: 400
wins:             209
draws:             24
losses:           167
paired score:   0.5525
paired 95% CI:  0.5089–0.5961

published train loss: 0.6791596543
candidate train loss: 0.6346267438
published holdout loss: 0.6614397866
candidate holdout loss: 0.6218924117
```

The confidence interval is computed over the 200 opening pairs, not over the
400 games as if they were independent. That pairing preserves the color-swap
design and is the correct statistical unit for this experiment.

The promoted material vector, anchored at Pawn=100 and King=0, is:

| Piece | Value | Piece | Value |
|---|---:|---|---:|
| Pawn | 100 | Knight | 392 |
| Bishop | 712 | Rook | 997 |
| Queen | 1,609 | King | 0 |
| Archbishop | 1,066 | Chancellor | 1,405 |
| Cannon | 1,022 | Elephant | 392 |
| Camel | 360 | Giraffe | 347 |
| Archer | 647 | Machine | 439 |
| Amazon | 2,014 | Lion | 1,163 |
| Buffalo | 1,154 | Centaur | 826 |
| Admiral | 1,147 | Missionary | 852 |
| Eagle | 1,462 | Rhinoceros | 1,295 |
| Prince | 466 | Sorceress | 1,609 |
| Duchess | 1,178 | Troll | 510 |

## Interpretation and limitations

Supported: this complete frozen material profile outperformed the published
profile under this engine, opening generator, Terachess II starting position,
100,000-node limit, and paired-match protocol. The predefined promotion rule
was satisfied without changing the candidate after observing validation.

Not supported: the match does not prove every individual fitted coefficient is
an intrinsic or universally correct piece value. Coefficients interact with
the positional evaluation, search, sampled positions, and each other. It also
does not measure performance at other time controls, hardware, future search
versions, or a broad externally curated opening suite.

The observed margin is useful but modest: its lower confidence bound clears
50% by 0.89 percentage points. Future search or positional-evaluation changes
should therefore rerun paired regression matches against both the production
profile and an appropriate current baseline.

## Engine action and next research

The profile is added verbatim as `empirical_v1()` and `production()` points to
it. No values are rounded again or retuned. `published()` remains an explicit
reproducibility baseline.

The next useful work is no longer more material fitting on these same games.
It is to benchmark search improvements and tune non-material positional terms
using fresh data, with separate training and final-validation openings. A
larger external-opening or longer-node confirmation is useful later, but is
not required for this recorded promotion decision.
