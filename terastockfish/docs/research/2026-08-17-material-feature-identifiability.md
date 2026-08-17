# Material feature identifiability diagnostic — 2026-08-17

## Status and conclusion

This read-only diagnostic tested whether the cross-seed instability reported in
the two Texel fits is explained by a rank-deficient or severely collinear
material feature matrix. It is not: both datasets and their union have full
column rank, and the standardized pooled matrix has condition number **4.46**.

Archer's largest pairwise correlation is only +0.211 in either individual
dataset and +0.190 after pooling. Troll has a repeatable moderate negative
correlation with Eagle (about −0.35), but no near-linear dependency. Therefore
the Archer reversal cannot be dismissed as a simple duplicate-feature problem.
The next investigation must focus on fit/bootstrap convergence, regularization
sensitivity, and possible position-dependent value missing from the scalar
material model.

## Inputs and method

The exact CSV artifacts from the two completed experiments were used:

| Dataset | Samples | SHA-256 |
|---|---:|---|
| First seed | 12,802 | `3de968a8cd7bef80141d97d5e0b27f48f75f8031188475c417b912ea825734ef` |
| Second seed | 11,999 | `305618ae7dfff27c33c3f58d471b97dcf144faf9553d092b7e885ad6c30e3415` |

Python with NumPy 2.4.4 read the CSVs without modifying them. Pawn and King
were excluded because they are fixed anchors; `game_id`, outcome, and fixed
score are not material-count predictors. For the remaining 24 White-minus-
Black piece-count columns the diagnostic computed:

- numerical matrix rank for each dataset and the pooled matrix;
- Pearson pairwise correlations for Archer and Troll;
- singular values and condition number after standardizing every pooled column
  to zero mean and unit variance.

This is a geometry diagnostic, not an outcome or strength test. Individual
positions need not be statistically independent for the stated rank and
conditioning calculations.

## Results

| Matrix | Rows | Rank |
|---|---:|---:|
| First seed | 12,802 | 24/24 |
| Second seed | 11,999 | 24/24 |
| Pooled | 24,801 | 24/24 |

The pooled standardized condition number is 4.4601; its smallest singular
value is 45.8905. This is not evidence of a numerically singular design.

### Largest Archer correlations

| Dataset | Correlated piece | Pearson r |
|---|---|---:|
| First | Archbishop | +0.169 |
| First | Admiral | −0.143 |
| First | Duchess | −0.138 |
| Second | Archbishop | +0.211 |
| Second | Admiral | −0.181 |
| Second | Rhinoceros | +0.148 |
| Pooled | Archbishop | +0.190 |
| Pooled | Admiral | −0.162 |
| Pooled | Rhinoceros | +0.137 |

### Largest Troll correlations

| Dataset | Correlated piece | Pearson r |
|---|---|---:|
| First | Eagle | −0.352 |
| First | Bishop | −0.136 |
| Second | Eagle | −0.356 |
| Second | Bishop | −0.164 |
| Pooled | Eagle | −0.354 |
| Pooled | Bishop | −0.150 |
| Pooled | Rook | +0.079 |

## Interpretation and next criterion

Full rank does not guarantee that scalar material coefficients are stable or
correct. The two datasets may cover different tactical contexts; the current
model may omit mobility/screen-density interactions important to Archer; game
outcomes are a noisy target; and the 12-epoch, 32-replicate bootstrap may not
match the convergence behavior of the primary 48-epoch fit.

Before final strength validation, the pooled diagnostic fit must:

1. use all 24,801 positions while keeping whole games in one split;
2. run multiple deterministic optimizer seeds and show negligible spread in
   final loss and material values;
3. use at least 256 game-level bootstrap replicates with enough epochs to check
   convergence rather than assuming 12 is sufficient;
4. report sensitivity to nearby regularization strengths;
5. explain or bound Archer variation and eliminate the displaced Troll
   interval pathology.

Only a frozen pooled profile that passes those checks proceeds to the fixed
200-pair, 100,000-node match. This diagnostic does not change production values.

The prescribed follow-up is complete in the
[pooled convergence report](2026-08-17-pooled-material-convergence.md). It
resolved the Archer/Troll instability and froze a candidate for final
playing-strength validation.
