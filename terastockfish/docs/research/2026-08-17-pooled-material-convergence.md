# Pooled material convergence and profile freeze — 2026-08-17

## Status and decision

The two seed datasets were pooled into 24,801 positions and the instability of
the earlier individual fits was investigated without generating new games.
Optimizer restarts, game-level bootstrap, regularization sensitivity, and
multi-split cross-validation now support freezing a single material candidate.

The frozen profile was `empirical-v1-candidate`, fitted with regularization
**0.08**. It subsequently passed the preregistered independent 200-pair,
100,000-node self-play match and was promoted unchanged to `empirical_v1()`.
See the [final validation report](2026-08-17-final-material-validation.md).

## Tooling and inputs

The Texel tool was extended before examining pooled results with:

- `--fit-only`, avoiding meaningless smoke self-play during diagnostics;
- independent split, optimizer, bootstrap, and validation seeds;
- repeated optimizer shuffles on one fixed game-level split;
- configurable fit/bootstrap epochs and bootstrap replicates;
- regularization sweeps and game-level cross-validation splits;
- machine-readable diagnostic CSV and portable material-profile output.

Both source datasets were merged with game IDs remapped so positions from
different games cannot collide during split or bootstrap grouping:

| Input | Positions | SHA-256 |
|---|---:|---|
| First seed | 12,802 | `3de968a8cd7bef80141d97d5e0b27f48f75f8031188475c417b912ea825734ef` |
| Second seed | 11,999 | `305618ae7dfff27c33c3f58d471b97dcf144faf9553d092b7e885ad6c30e3415` |

The final diagnostic executable SHA-256 was
`e3138a5184e3f6a1168ccdeb3a225e96a97c7d30806deae18fb7b4ec1aace888`.

## Diagnostic sequence

### Initial pooled convergence run

The first pooled fit used 96 fit epochs, eight optimizer seeds on one fixed
split, regularization values 0.0025–0.04, and 256 game-level bootstrap
replicates refitted for 48 epochs. It completed in 29.3 seconds. Unlike the
earlier 32×12 bootstraps, all principal point estimates were covered by their
intervals.

At regularization 0.01, optimizer restart spans were small: Archer 11 units,
Troll 16, and at most 63 for Eagle. The holdout loss improved from 0.661440 to
0.622337. Archer pooled to 633 with interval 479–689, resolving the contradictory
single-seed points as sampling instability rather than optimizer instability.

The best tested holdout loss occurred at the upper sweep boundary, so this
profile was not frozen. The range was expanded before selecting a value.

### Expanded regularization bracket

| Regularization | Fixed-split holdout loss |
|---:|---:|
| 0.04 | 0.621881857 |
| 0.08 | 0.621892412 |
| 0.16 | 0.622301035 |
| 0.32 | 0.623071219 |
| 0.64 | 0.623689158 |
| 1.28 | 0.624306090 |

The optimum was bracketed around 0.04–0.08. Because selecting between nearly
equal values on a single holdout split would overfit that split, a separate
eight-split comparison was performed.

### Eight-split regularization cross-validation

Every split kept complete games together. Each regularization used the same
eight deterministic split seeds.

| Regularization | Mean holdout loss | Minimum | Maximum |
|---:|---:|---:|---:|
| 0.01 | 0.639305662 | 0.623913297 | 0.649524406 |
| 0.02 | 0.639025448 | 0.624534826 | 0.649520542 |
| 0.04 | **0.638749878** | 0.625098946 | 0.649058968 |
| 0.08 | 0.638751741 | 0.626038103 | 0.648865800 |
| 0.16 | 0.639021306 | 0.627330307 | 0.649164916 |
| 0.32 | 0.639584594 | 0.628620083 | 0.650358612 |

The mean difference `loss(0.08) − loss(0.04)` was only +0.000001863. Its
paired standard error across splits was 0.000176839, about 95 times larger than
the observed difference. The two settings are empirically tied. The stronger
regularization 0.08 was selected by the predefined conservative tie-break:
when predictive quality is indistinguishable, prefer the profile closer to the
published prior.

## Frozen candidate fit

The chosen profile was refit using all pooled data, the fixed grouped split,
96 fit epochs, eight optimizer restarts, and a 256-replicate/48-epoch
game-level bootstrap. It completed in 27.77 seconds.

```text
training positions: 20,100
holdout positions:   4,701
published train loss: 0.6791596543
candidate train loss: 0.6346267438  (6.56% reduction)
published holdout loss: 0.6614397866
candidate holdout loss: 0.6218924117 (5.98% reduction)
```

| Piece | Published | Candidate | Bootstrap 95% | Restart range |
|---|---:|---:|---:|---:|
| Pawn | 100 | 100 | 100–100 | anchor |
| Knight | 400 | 392 | 379–411 | 390–396 |
| Bishop | 680 | 712 | 660–746 | 710–717 |
| Rook | 1,000 | 997 | 935–1,062 | 992–1,001 |
| Queen | 1,660 | 1,609 | 1,519–1,692 | 1,603–1,621 |
| King | 0 | 0 | 0–0 | anchor |
| Archbishop | 1,060 | 1,066 | 1,023–1,139 | 1,062–1,072 |
| Chancellor | 1,380 | 1,405 | 1,337–1,486 | 1,405–1,413 |
| Cannon | 1,000 | 1,022 | 982–1,087 | 1,021–1,025 |
| Elephant | 400 | 392 | 363–405 | 387–394 |
| Camel | 360 | 360 | 344–378 | 360–366 |
| Giraffe | 340 | 347 | 322–362 | 347–351 |
| Archer | 660 | 647 | 599–676 | 646–652 |
| Machine | 440 | 439 | 418–451 | 438–442 |
| Amazon | 2,040 | 2,014 | 1,907–2,149 | 2,005–2,020 |
| Lion | 1,200 | 1,163 | 1,096–1,227 | 1,163–1,170 |
| Buffalo | 1,080 | 1,154 | 1,081–1,210 | 1,149–1,162 |
| Centaur | 820 | 826 | 786–850 | 812–826 |
| Admiral | 1,200 | 1,147 | 1,113–1,222 | 1,143–1,150 |
| Missionary | 880 | 852 | 796–902 | 849–857 |
| Eagle | 1,680 | 1,462 | 1,365–1,551 | 1,448–1,486 |
| Rhinoceros | 1,220 | 1,295 | 1,236–1,366 | 1,291–1,300 |
| Prince | 460 | 466 | 443–477 | 455–466 |
| Sorceress | 1,640 | 1,609 | 1,540–1,735 | 1,604–1,612 |
| Duchess | 1,160 | 1,178 | 1,086–1,228 | 1,175–1,179 |
| Troll | 480 | 510 | 499–543 | 510–516 |

Archer and Troll now satisfy the convergence requirement. Across eight
optimizer shuffles Archer spans six units and Troll six units; both point
estimates lie inside their 256-replicate intervals. Eagle remains clearly below
published, while Rhinoceros and Troll remain above it. Buffalo's lower interval
exceeds published by one unit, which is too marginal for an individual claim.

## Frozen artifacts

| Artifact | SHA-256 |
|---|---|
| Pooled dataset | `9d2b944664eb7e787e63294ea267c4569ee1b54a071c507456e84813326a2a65` |
| Fit checkpoint | `335eae116ad2f82d462a6872f70c5d23bc18eaf6859274dd6520b66ee9d66700` |
| Candidate profile | `656c9b5e1b182c9a24b2dd05cd88b1f81989229ad497f954853501c1d4ccfc4e` |
| Restart diagnostics | `bbff13671aee9dd48ed234fa2033e0189e34b3abc8f4acb7a803c4ccb08ad6bb` |
| Cross-validation diagnostics | `6e101d59ab1aede5bf419b0e7c6dc04ce5764fd98853b4f980584fff3c924921` |

## Subsequent gate

The candidate played one fixed, independent match against `published`: 200
color-swapped pairs at 100,000 nodes per move, using the preregistered seed.
It scored 55.25% with paired 95% CI 50.89–59.61%, passing the predefined lower
bound above 50%. The exact frozen values were therefore promoted without any
post-result retuning. `published()` remains permanently available.
