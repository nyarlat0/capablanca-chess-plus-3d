# Independent Texel material replication — 2026-08-17

## Status and conclusion

This fixed-size replication **confirms that Texel-tuned material profiles can
outperform the published Terachess II scale**, but it **does not confirm a
stable value for every individual piece**.

The second fitted profile scored **55.99%** against the untouched published
profile over 192 color-swapped pairs (384 games), with a paired 95% confidence
interval of **51.57–60.41%**. Holdout log loss improved by 3.48%, all pieces
exceeded the coverage threshold, and the tool reported `recommended=true`.

The predeclared gate for promoting exact values to the production evaluator did
not pass. Archer reversed from 438 in the first run to 725 in this run, moving
from below to above the published 660, and the two Archer bootstrap intervals
do not overlap. The Troll bootstrap intervals also do not overlap even though
the point estimates are nearly identical; the second interval does not contain
its own full-sample estimate. These are signs of coefficient/interval
instability that must be investigated rather than averaged away silently.

Accordingly, `EvaluationParameters::production()` remains equal to
`EvaluationParameters::published()`.

## Reproduction record

The experiment used the exact executable archived from the first run:

```text
SHA-256 d6575deced3ed09c17ebe5861439c83afe2612b34835241eaab91417417ce0d4
```

It ran on the same AMD Ryzen 9 5900X host (12 cores/24 threads, 32 GiB RAM),
with 24 concurrent pair jobs and 32 MiB TT per engine, approximately 1.5 GiB
of simultaneously allocated transposition tables.

```sh
./target/research/2026-08-17/terastockfish-texel-first-run \
  --preset overnight \
  --jobs 24 \
  --hash 32 \
  --seed 0x5345434f4e445255 \
  --validation-min-pairs 192 \
  --validation-max-pairs 192 \
  --dataset target/terastockfish-texel-seed2.csv \
  --checkpoint target/terastockfish-texel-seed2.chk
```

The checkpoint records seed `6000276086435631701`
(`0x5345434f4e445255`). Collection took 10,492.80 s (2 h 54 min 53 s), and
the cumulative run took 19,778.50 s (5 h 29 min 38 s). The remaining fit,
bootstrap, and validation stages therefore took approximately 2 h 34 min 46 s.

| Artifact | Size | SHA-256 |
|---|---:|---|
| `target/terastockfish-texel-seed2.csv` | 864,154 bytes | `305618ae7dfff27c33c3f58d471b97dcf144faf9553d092b7e885ad6c30e3415` |
| `target/terastockfish-texel-seed2.chk` | 3,858 bytes | `15f2d156752af3e8c1f27d673b832189e7b8b7f90550926b54970b54821876cc` |

The executable hash fixes the code actually run even though the original dirty
source diff and `RUSTFLAGS` were not captured before its build. Preserve the
binary and both data artifacts outside disposable `target/` storage if exact
future reproduction is required.

## Methodology

The collection and fitting procedure was identical to the
[first experiment](2026-08-17-texel-material-values.md), except for its
independent seed and fixed validation size:

- 512 generated openings, each played twice with colors swapped: 1,024 games;
- eight deterministic random opening plies and opposite `exp(±0.12)` material
  perturbations around the published scale;
- 20,000 search nodes per training move, 240-ply limit, and 800-unit sustained
  score adjudication after 12 plies;
- quiet positions sampled from ply 8 every four plies, with a 32-position
  game-level reservoir;
- game-grouped deterministic train/holdout split, Adam Texel fit, regularization
  toward published values, and a 32-replicate game-level bootstrap;
- validation against the unchanged published profile at 50,000 nodes per move,
  with each opening played twice and the pair score used as the statistical
  observation.

Unlike the first run, both `--validation-min-pairs` and
`--validation-max-pairs` were fixed at 192. The result therefore could not stop
at a favorable early interval. This matters here: the interval excluded 50%
after 24 and 48 pairs, crossed 50% again from 72 through 120 pairs, and only
became positive again from 144 through the fixed endpoint.

## Results

### Dataset and predictive fit

The 1,024 games yielded 11,999 quiet positions (11.72 per game): 9,609 training
and 2,390 holdout positions.

| Split | Published loss | Fitted loss | Relative reduction |
|---|---:|---:|---:|
| Training | 0.692604 | 0.639792 | 7.63% |
| Holdout | 0.625640 | 0.603896 | 3.48% |

This independently repeats the first experiment's direction: both training
and unseen-game holdout loss improve, with the expected smaller improvement on
holdout data.

### Second-run material scale

| Piece | Published | Fitted | Δ | Bootstrap 95% | Coverage | Published excluded |
|---|---:|---:|---:|---:|---:|:---:|
| Pawn | 100 | 100 | 0 | 100–100 | anchor | n/a |
| Knight | 400 | 376 | −24 | 290–462 | 2,962 | no |
| Bishop | 680 | 696 | +16 | 638–867 | 5,211 | no |
| Rook | 1,000 | 999 | −1 | 723–1,044 | 4,056 | no |
| Queen | 1,660 | 1,658 | −2 | 1,500–1,756 | 4,690 | no |
| King | 0 | 0 | 0 | 0–0 | anchor | n/a |
| Archbishop | 1,060 | 1,044 | −16 | 863–1,166 | 2,739 | no |
| Chancellor | 1,380 | 1,436 | +56 | 1,394–1,731 | 1,939 | yes |
| Cannon | 1,000 | 1,030 | +30 | 849–1,227 | 2,410 | no |
| Elephant | 400 | 327 | −73 | 119–353 | 3,867 | yes |
| Camel | 360 | 333 | −27 | 242–402 | 3,973 | no |
| Giraffe | 340 | 381 | +41 | 266–423 | 5,072 | no |
| Archer | 660 | 725 | +65 | 644–830 | 3,653 | no |
| Machine | 440 | 390 | −50 | 302–463 | 1,227 | no |
| Amazon | 2,040 | 2,046 | +6 | 1,816–2,187 | 1,453 | no |
| Lion | 1,200 | 1,142 | −58 | 986–1,330 | 1,990 | no |
| Buffalo | 1,080 | 1,133 | +53 | 1,083–1,393 | 4,255 | yes |
| Centaur | 820 | 856 | +36 | 695–989 | 1,309 | no |
| Admiral | 1,200 | 1,175 | −25 | 889–1,218 | 1,651 | no |
| Missionary | 880 | 834 | −46 | 652–916 | 2,548 | no |
| Eagle | 1,680 | 1,596 | −84 | 1,384–1,624 | 5,014 | yes |
| Rhinoceros | 1,220 | 1,240 | +20 | 1,069–1,351 | 3,272 | no |
| Prince | 460 | 437 | −23 | 273–499 | 1,773 | no |
| Sorceress | 1,640 | 1,580 | −60 | 1,352–1,740 | 2,119 | no |
| Duchess | 1,160 | 1,128 | −32 | 931–1,248 | 1,961 | no |
| Troll | 480 | 559 | +79 | 637–785 | 3,255 | yes¹ |

¹ The percentile interval does not contain the full-sample Troll estimate. A
percentile bootstrap is not mathematically required to contain the point
estimate, but a displacement this large is a warning about estimator bias,
bootstrap convergence, or coefficient coupling and must not be treated as a
precise confidence statement.

### Fixed-size playing-strength validation

| Completed pairs | Candidate score | Paired 95% CI |
|---:|---:|---:|
| 24 | 64.6% | 52.1–77.1% |
| 48 | 61.5% | 52.1–70.8% |
| 72 | 56.2% | 48.6–63.9% |
| 96 | 56.5% | 49.9–63.2% |
| 120 | 55.4% | 49.6–61.2% |
| 144 | 55.6% | 50.3–60.9% |
| 168 | 56.7% | 51.9–61.5% |
| 192 | **55.99%** | **51.57–60.41%** |

The candidate recorded 210 wins, 10 draws, and 164 losses over 384 games.
Inference uses the 192 color-swapped pair scores, not 384 games as independent
observations.

## Cross-seed stability

The two complete profiles agree on direction relative to the published value
for 12 of 24 tuned pieces and disagree for the other 12. Most bootstrap
intervals nevertheless overlap, showing that many apparent sign flips are
small relative to current uncertainty. Archer is the material exception; Troll
is an interval-estimation exception.

| Piece | First fit | Second fit | Difference | Same direction | Intervals overlap |
|---|---:|---:|---:|:---:|:---:|
| Knight | 413 | 376 | −37 | no | yes |
| Bishop | 721 | 696 | −25 | yes | yes |
| Rook | 1,014 | 999 | −15 | no | yes |
| Queen | 1,563 | 1,658 | +95 | yes | yes |
| Archbishop | 1,086 | 1,044 | −42 | no | yes |
| Chancellor | 1,317 | 1,436 | +119 | no | yes |
| Cannon | 1,098 | 1,030 | −68 | yes | yes |
| Elephant | 455 | 327 | −128 | no | yes |
| Camel | 376 | 333 | −43 | no | yes |
| Giraffe | 304 | 381 | +77 | no | yes |
| Archer | 438 | 725 | +287 | no | **no** |
| Machine | 399 | 390 | −9 | yes | yes |
| Amazon | 1,971 | 2,046 | +75 | no | yes |
| Lion | 1,178 | 1,142 | −36 | yes | yes |
| Buffalo | 1,168 | 1,133 | −35 | yes | yes |
| Centaur | 846 | 856 | +10 | yes | yes |
| Admiral | 1,160 | 1,175 | +15 | yes | yes |
| Missionary | 890 | 834 | −56 | no | yes |
| Eagle | 1,447 | 1,596 | +149 | yes | yes |
| Rhinoceros | 1,394 | 1,240 | −154 | yes | yes |
| Prince | 473 | 437 | −36 | no | yes |
| Sorceress | 1,722 | 1,580 | −142 | no | yes |
| Duchess | 1,124 | 1,128 | +4 | yes | yes |
| Troll | 567 | 559 | −8 | yes | **no** |

Eagle is the only individual piece for which both runs place the published
value outside the bootstrap interval in the same direction: both indicate that
1,680 is too high. Rhinoceros retains the same upward point direction but loses
individual significance in the replication. Archer directly contradicts the
first run. The joint profiles can still both win because correlated material
features may exchange coefficient weight while producing similar evaluations.

## Decision and next investigation

The result confirms the research program but rejects immediate promotion of
either table. The previously declared stability gate is not weakened after
seeing the data.

Before spending another night on a 200-pair pooled match:

1. merge both datasets and perform a pooled full-convergence fit with 256
   game-level bootstrap replicates, without interpreting a smoke validation as
   strength evidence;
2. report feature correlations and coefficient stability, concentrating on
   Archer and the Troll bootstrap displacement;
3. verify optimizer/bootstrap convergence and compare pooled estimates against
   both single-seed intervals;
4. only if the pooled coefficients are stable, freeze that profile and run the
   independent fixed 200-pair, 100,000-node validation already specified in the
   crate README.

This follow-up is a new diagnostic experiment with criteria stated here before
its result. It must receive its own report regardless of outcome.
