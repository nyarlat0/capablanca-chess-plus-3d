# Balance pilot invalidation and V2 protocol — 2026-08-18

## Status and decision

The first prospective balance pilot is invalid for estimating color balance.
After 48 games, 33 had reached the artificial 300-ply cap and were incorrectly
recorded as draws, 12 ended by threefold repetition, and only three ended by
checkmate. The run must not be combined with the corrected experiment.

The harness is replaced by V2. Search now receives real game repetition
history, the `eight-hour` preset has no ply cap, a manually requested cap yields
an unresolved observation rather than a draw, and uncertainty uses a Wilson
interval that does not collapse to zero after an all-draw batch.

## Pilot evidence

The pilot used the production profile for both sides, 48 of the planned 384
randomized openings, 50,000 nodes per move, eight opening plies, and no score
adjudication. Its exact-start analyses were:

| Nodes | Score | Depth | Best line |
|---:|---:|---:|---|
| 100,000 | +12 | 4 | `d2g5 d15g12 m2j5 m15j12` |
| 1,000,000 | +12 | 6 | `d2g5 d15g12 m2j5 m15j12 g5g8 g12g9` |
| 10,000,000 | +12 | 6 | same six-ply line |

The +12 score equals the evaluator's tempo term and is descriptive, not proof
of theoretical equality.

| Termination | Games | Mean plies | Mean wall time per game |
|---|---:|---:|---:|
| Artificial ply limit | 33 | 300.0 | 23.08 min |
| Threefold repetition | 12 | 182.8 | 16.48 min |
| Checkmate | 3 | 178.3 | 16.36 min |

Raw White outcomes were one win, 45 draws, and two losses. Because 33 nominal
draws were actually censored games, neither the 48.96% White score nor its
reported interval is a balance result. The initial all-draw batch also produced
the nonsensical interval `[0.5000, 0.5000]`, exposing failure of the normal
sample-variance interval at a boundary distribution.

The retained pilot CSV SHA-256 is
`e4ad045e6d1c9f68ddcec8707795de60b2b4adf540309b3f70ea57587673f617`.
It uses `TERASTOCKFISH_BALANCE_V1` and is intentionally incompatible with V2.

## Causes

The engine can recognize and deliver mate: the pilot contains mates at 72,
171, and 292 plies, and search represents mate separately from static scores.
However, 50,000 nodes are shallow on a 16×16 board; even the 10-million-node
root search completed only depth six. More importantly, search previously saw
only repetition inside its current tree, not positions repeated before the
root, so it could select a move completing a real third occurrence without
valuing it as a draw.

The fifty-move rule is not a substitute for a runtime limit. Its 100-halfmove
counter resets after every pawn move or capture. Terachess II begins with 128
pieces and many pawns, so a legal game may remain below that counter for a very
long time.

## Corrected V2 protocol

- `SearchHistory` records the repetition key after every real move and seeds
  each subsequent search; a stale history is rejected rather than mixed into a
  different root.
- Repetition keys include an en-passant marker only when a legal en-passant
  capture exists, matching the game-rule definition of identical positions.
- `maximum_plies=0` disables the cap. Any nonzero manual cap produces
  `unresolved`, never `draw`.
- The `eight-hour` preset uses a conservative score adjudication only after the
  same side remains at least 1,500 centipawns ahead for 20 consecutive plies.
  Adjudicated results remain separately identifiable.
- W/D/L and White score exclude unresolved games. If unresolved games exceed
  10%, the report marks every balance claim invalid.
- A Wilson score interval with draws worth half a point replaces the degenerate
  normal interval. This retains uncertainty even when every observed game is a
  draw.
- V2 records opening FEN, final FEN, final White-relative evaluation,
  termination, plies, nodes, and elapsed time for every game.

Conservative score adjudication is still an engine-model outcome rather than a
literal checkmate. Using the identical evaluator for both colors and a high,
sustained threshold limits color asymmetry, while the separate termination
count permits a sensitivity assessment.

## Corrected command

The old process should be stopped and the V1 CSV retained only as failed-pilot
evidence. Build once, record the new binary hash, and start a new V2 file:

```sh
cargo build --release -p terastockfish --bin terastockfish-balance

./target/release/terastockfish-balance \
  --preset eight-hour \
  --jobs 24 --hash 32 --root-threads 24 \
  --seed 0x42414c414e434531 \
  --output target/terastockfish-balance-v2.csv
```

Resume with the identical binary and command plus `--resume --skip-root`.
Because the hard cap is gone, 4–8 hours is a target rather than a guarantee.
If runtime is unacceptable, stop and resume later; do not change parameters or
classify incomplete V2 output as the final fixed-sample result.
