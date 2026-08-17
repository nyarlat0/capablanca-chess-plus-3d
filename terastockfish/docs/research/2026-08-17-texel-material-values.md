# Texel material fit with paired self-play — 2026-08-17

## Status and conclusion

The jointly fitted material profile is a **validated candidate** for the
Terachess II evaluator. Against the untouched published-value baseline it
scored **57.03%** over 96 color-swapped pairs (192 games), with a paired 95%
confidence interval of **50.76–63.30%**. The lower bound exceeded the
predefined 50% threshold, every fitted piece had more than the required 100
informative samples, and holdout log loss improved. Consequently the tool
reported `recommended=true`.

This is positive but narrow evidence: the lower validation bound clears 50%
by only 0.76 percentage points. The experiment supports the complete profile
at this search budget; it does not prove exact context-independent values for
all pieces. The published value lies outside the per-piece bootstrap interval
only for Archer, Eagle, and Rhinoceros. A fresh-seed confirmation run should
precede treating these numbers as a stable final scale.

No production default was changed as part of documenting this experiment.

## Reproduction record

The run was made on an AMD Ryzen 9 5900X (12 cores, 24 threads) with 32 GiB
RAM, using 24 concurrent game-pair jobs and 32 MiB of transposition table per
engine. Two single-threaded searchers exist per active game, so peak TT
allocation was approximately 1.5 GiB. The compiler visible when the result was
archived was Rust 1.93.1 / LLVM 21.1.8 on `x86_64-unknown-linux-gnu`.

```sh
./target/release/terastockfish-texel \
  --preset overnight \
  --jobs 24 \
  --hash 32 \
  --dataset target/terastockfish-texel.csv \
  --checkpoint target/terastockfish-texel.chk
```

The deterministic run seed stored in the checkpoint is
`5716832996796483400` (`0x4f5645524e494748`). Collection took 10,252.68 s
(2 h 50 min 53 s); cumulative collection, fitting, bootstrap, and validation
took 16,023.33 s (4 h 27 min 3 s).

Artifact hashes at archival time:

| Artifact | SHA-256 |
|---|---|
| `target/release/terastockfish-texel` | `d6575deced3ed09c17ebe5861439c83afe2612b34835241eaab91417417ce0d4` |
| `target/terastockfish-texel.csv` | `3de968a8cd7bef80141d97d5e0b27f48f75f8031188475c417b912ea825734ef` |
| `target/terastockfish-texel.chk` | `23fa9faefca81ffa107dee6479e50fe5d0443e95c7a14380ba515a0b36393eab` |

The executable predates this report and does not embed enough provenance to
prove its source revision. At archival time the dirty workspace was based on
commit `bf4e5a35ac33926a5648a39c9348cbc638e1dea3`, but the exact build-time dirty
diff and `RUSTFLAGS` were not recorded. The hashes identify the local artifacts,
but source-level bit-for-bit reproduction is therefore not guaranteed. Future
experiments must capture this metadata before running.

## Methodology

### Dataset generation

- The unit of generation was a pair sharing one opening. Each of 512 openings
  was played twice with colors swapped, producing 1,024 games.
- Each opening began at the standard Terachess II position and applied eight
  deterministic pseudo-random legal moves. Legal moves were sorted by UCI
  notation before selection, making the fixed-seed process reproducible.
- For every pair, all 24 non-pawn/non-king material values were independently
  perturbed around the published profile in log space by `exp(±0.12)` (about
  −11.3%/+12.7%). The two opposite profiles each played White once, reducing
  color and opening bias while ensuring useful material variation.
- Move selection used one search thread, 20,000 nodes per move, and a 32 MiB
  TT per engine. Games ended normally, at 240 plies, or after the same side was
  evaluated at least 800 units ahead for 12 consecutive plies. Ply-limit games
  were labelled draws.
- Candidate samples began at ply 8 and were considered every four plies. A
  position was retained only when the side to move was not in check and the
  selected next move was not a capture. Reservoir sampling kept at most 32
  positions per game without favoring early eligible positions.
- Every retained position received its game's final White result: 1 for a
  White win, 0.5 for a draw, and 0 for a White loss. This yielded 12,802 quiet
  positions, an average of 12.50 per game.

### Texel fit

For each position the model used

`score = fixed positional score + Σ(White count − Black count) × piece value`.

The fixed score retained the published evaluator's non-tuned terms and the
pawn value. A learned logistic scale converted this score to an expected White
result, and binary cross-entropy against the game result was minimized. Pawn
was fixed at 100 to define the unit; King was fixed at 0 because royal value is
represented by terminal mate rather than capturable material.

All positions from one game stayed in the same split. A deterministic hash of
game ID assigned approximately 20% of games to holdout, resulting in 10,074
training and 2,728 holdout positions. The 24 values and logistic scale were
optimized with Adam for at most 48 epochs, batch size 256, early-stopping
patience 8, and regularization 0.01 toward the published scale. Material values
were constrained to 50–4,000.

Uncertainty was estimated with 32 game-level bootstrap replicates. Whole games,
not individual correlated positions, were resampled; each replicate refit for
12 epochs. Reported per-piece bounds are the 2.5th and 97.5th percentiles.
These intervals describe this fitted model and dataset and are not independent
playing-strength tests for individual pieces.

### Independent paired validation

The fitted profile was then matched against the unchanged published profile on
new generated openings. Each of 96 openings was played twice with candidate
colors swapped, at 50,000 nodes per move, an eight-ply randomized opening, a
300-ply limit, and the same 800-for-12-plies adjudication rule.

The statistical observation was the mean candidate score of each two-game
pair, not each game independently. The 95% interval used a normal approximation
to the 96 pair scores. Validation was allowed to stop after at least 24 pairs
when the interval no longer crossed 50%; because work completed in batches of
24, the run continued through 96 pairs before crossing the boundary.

## Results

### Predictive fit

| Split | Published loss | Fitted loss | Relative reduction |
|---|---:|---:|---:|
| Training | 0.681816 | 0.632651 | 7.21% |
| Holdout | 0.629012 | 0.606080 | 3.65% |

Both splits improved. The smaller but still positive holdout improvement is
consistent with genuine predictive signal rather than pure training-set fit,
though both sets originate from the same self-play generator.

### Fitted material scale

`Coverage` counts sampled positions with a non-zero White-minus-Black count for
that piece. `Published excluded` means the published point lies outside this
run's game-bootstrap interval; it is not a standalone strength verdict.

| Piece | Published | Fitted | Δ | Δ % | Bootstrap 95% | Coverage | Published excluded |
|---|---:|---:|---:|---:|---:|---:|:---:|
| Pawn | 100 | 100 | 0 | 0.0% | 100–100 | anchor | n/a |
| Knight | 400 | 413 | +13 | +3.3% | 300–439 | 2,981 | no |
| Bishop | 680 | 721 | +41 | +6.0% | 573–812 | 5,300 | no |
| Rook | 1,000 | 1,014 | +14 | +1.4% | 851–1,113 | 4,189 | no |
| Queen | 1,660 | 1,563 | −97 | −5.8% | 1,460–1,688 | 4,726 | no |
| King | 0 | 0 | 0 | n/a | 0–0 | anchor | n/a |
| Archbishop | 1,060 | 1,086 | +26 | +2.5% | 929–1,276 | 2,748 | no |
| Chancellor | 1,380 | 1,317 | −63 | −4.6% | 1,133–1,462 | 1,917 | no |
| Cannon | 1,000 | 1,098 | +98 | +9.8% | 956–1,172 | 2,451 | no |
| Elephant | 400 | 455 | +55 | +13.8% | 316–522 | 4,411 | no |
| Camel | 360 | 376 | +16 | +4.4% | 327–479 | 4,195 | no |
| Giraffe | 340 | 304 | −36 | −10.6% | 255–390 | 5,635 | no |
| Archer | 660 | 438 | −222 | −33.6% | 295–571 | 3,715 | yes |
| Machine | 440 | 399 | −41 | −9.3% | 353–572 | 1,663 | no |
| Amazon | 2,040 | 1,971 | −69 | −3.4% | 1,777–2,262 | 1,360 | no |
| Lion | 1,200 | 1,178 | −22 | −1.8% | 1,020–1,340 | 2,063 | no |
| Buffalo | 1,080 | 1,168 | +88 | +8.1% | 1,005–1,275 | 4,791 | no |
| Centaur | 820 | 846 | +26 | +3.2% | 615–900 | 1,400 | no |
| Admiral | 1,200 | 1,160 | −40 | −3.3% | 1,011–1,294 | 1,561 | no |
| Missionary | 880 | 890 | +10 | +1.1% | 630–959 | 2,795 | no |
| Eagle | 1,680 | 1,447 | −233 | −13.9% | 1,242–1,560 | 5,336 | yes |
| Rhinoceros | 1,220 | 1,394 | +174 | +14.3% | 1,245–1,581 | 3,594 | yes |
| Prince | 460 | 473 | +13 | +2.8% | 418–635 | 1,576 | no |
| Sorceress | 1,640 | 1,722 | +82 | +5.0% | 1,561–1,886 | 1,980 | no |
| Duchess | 1,160 | 1,124 | −36 | −3.1% | 948–1,319 | 2,326 | no |
| Troll | 480 | 567 | +87 | +18.1% | 443–596 | 3,405 | no |

### Playing-strength validation

| Pairs | Candidate score | Paired 95% CI |
|---:|---:|---:|
| 24 | 54.2% | 41.4–66.9% |
| 48 | 53.6% | 44.9–62.4% |
| 72 | 54.5% | 47.2–61.8% |
| 96 | 57.03% | 50.76–63.30% |

Across the final 192 games the candidate recorded 104 wins, 11 draws, and 77
losses. The color-swapped pair design, rather than the raw W/D/L games, is the
basis of the interval and recommendation.

## Interpretation and limitations

Supported conclusions:

- The complete fitted profile played better than the published profile under
  this engine version, opening generator, and 50,000-node validation budget.
- It predicted held-out outcomes better than the published material scale.
- Archer and Eagle are strongly indicated as overvalued by the published
  scale, while Rhinoceros is indicated as undervalued in this evaluator.

Suggestive, not yet definitive:

- Cannon, Elephant, Buffalo, Sorceress, and Troll trend upward; Queen,
  Chancellor, Giraffe, Machine, and Duchess trend downward. Their intervals
  still contain the published values.
- Wide intervals for rarer or correlated pieces mean their point estimates may
  trade value against one another. Coverage count alone does not remove this
  collinearity.

This experiment cannot establish universal, position-independent piece values.
Its labels come from self-play by the same search/evaluation family; its random
openings remain close to the initial array; quiet-position filtering changes
the sampled distribution; adjudication uses evaluation scores related to the
parameters under study; and only one seed and one search budget were validated.
The 32-replicate bootstrap is adequate for screening but coarse for final
interval estimation. The joint validation proves the profile as a whole and
cannot attribute its gain to any single changed value.

## Next required confirmation

Run the same protocol with a new seed and archived clean source provenance.
The primary confirmation criterion is again a paired 95% lower bound above
50%. Also inspect whether Archer, Eagle, and Rhinoceros move in the same
directions and whether the remaining intervals narrow. Only after replication
should the project decide whether to replace `EvaluationParameters::published()`
or introduce this result as a separately named empirical profile.
