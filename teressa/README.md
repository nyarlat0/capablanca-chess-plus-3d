# Teressa

Teressa is the native neural research engine for Terachess II. Its purpose is
not to replace exact rules or tactical calculation with a black box. It learns
a long-term, human-readable strategic intention and a move policy, while
TeraStockfish rejects tactically unsound choices.

This crate is a working research pipeline, not a pretrained strong engine. A
checkpoint must first be produced from generated or imported games. The
initial teacher data imitates TeraStockfish. Strategic labels are inferred
from the following two to six moves of each side and stored as a soft
distribution over applicable plans. Human games and later self-play are what
can make its style genuinely less mechanical.

## Design

The inference path deliberately keeps four responsibilities separate:

1. `capablanca-engine` supplies the exact Terachess II position and legal moves.
2. The symbolic planner creates only plans that make sense in that position.
3. The network scores those plans, then predicts a plan-conditioned move
   policy, WDL, scalar value, and per-move tactical risk.
4. TeraStockfish searches the leading neural candidates. Ordinary moves may
   lose no more than 120 centipawns relative to its tactical best; a
   high-confidence expressive plan may use a 300-centipawn window only when
   the predicted WDL loss is at most 0.08. A newly introduced forced mate loss
   is never accepted.

A selected plan normally persists for two to six moves. It is replaced after
completion, loss of its actor/target, a materially stronger new plan, or two
consecutive tactical vetoes. When the actor moves, its symbolic square moves
with it. Every plan is structured data rather than generated prose, so the
same source is rendered deterministically in Russian and English.

Current plan families include development, piece improvement, king attack and
safety, space, pawn breaks, creating or advancing a passer, open files,
diagonals, outposts, restriction, exchanges, and material preservation.

## Network experiments

Both supplied trunks use identical encodings and output heads:

- An 18x18 padded board, so 16x16 Terachess II and possible future 18x18
  variants do not change checkpoint shapes.
- 64 relative-to-side-to-move features per square, including all 26 piece
  types, geometry, attacks, en passant, king jump, halfmove state, and recent
  move markers.
- The last eight plies, including explicit recency, instead of silently
  discarding the history already stored in the dataset.
- A variable-size legal-move list with 13 geometric/special-move features and
  integer origin/destination token indices.
- Heads for legal-move policy, WDL, scalar value, plan class, and tactical risk.

Model V3 fixes three information bottlenecks discovered by the first V2 pilot.
V2 classified plans from linearly projected squares *before* the BDH reasoning
steps and mean-pooled them, so much of the spatial configuration was lost. Its
move head saw only move geometry plus one global position vector: it could not
directly tell which piece occupied the origin or what was on the destination.
The encoded move history was never passed to either trunk. V3 instead:

- performs recurrent/residual reasoning before plan classification;
- uses learned attention pooling rather than an unweighted square average;
- gathers the contextual board embeddings at every legal move's origin and
  destination and combines both with move geometry, global context, and the
  active plan;
- canonicalizes rank coordinates and move direction to the side-to-move
  perspective, so mirrored white/black ideas do not have to be learned twice;
- projects the ordered eight-ply history into the board context;
- normalizes BDH associative retrieval by the number of real board squares,
  not the fixed 18x18 padding size;
- trains the soft plan head at full weight and uses linear warmup followed by
  cosine learning-rate decay.

Plan loss and plan evaluation use the same symbolic candidate mask as play.
Unavailable plan kinds are excluded from the softmax. During native play the
reported plan confidence is likewise renormalized over the currently
applicable kinds instead of being diluted by impossible ideas.

The policy is conditioned on the selected persistent plan, while plan, WDL,
and scalar value are predicted from the position itself. This avoids allowing
the chosen plan to leak into the target that chooses that plan.

`bdh` is the experimental trunk. It adapts the useful part of the published
BDH recurrence to a non-causal board: positive sparse features form an
associative `Q Q^T V` retrieval, multiplicative gating produces an update, and
the same core parameters are reused for every reasoning step. It is an
adaptation, not a claim that the original language architecture has already
been validated for chess.

`residual` is the matched conventional MLP control. BDH is worth retaining only
if it beats this baseline at comparable width, data, optimizer budget, and
tactical-safety settings. BDH-CQ is intentionally deferred until ordinary BDH
shows a reproducible advantage; adding another experimental mechanism before
that would make failures impossible to attribute.

Burn 0.21 runs training and inference through Vulkan/WGPU. This is the portable
path for the project's Radeon RX 6700 XT and does not depend on that card being
officially supported by a particular ROCm release. Teressa is native-only in
this first version; it is not linked into the Bevy WebAssembly bundle.

## Dataset format and reproducibility

`teressa-generate` writes zstd-compressed JSON Lines. Each V2 shard begins with
a format-version header and contains FEN, recent UCI history, the complete
legal move set, a temperature-normalized teacher policy with root scores, WDL,
teacher score, a trajectory-derived soft plan policy, and the concrete
persistent plan used to condition move prediction.

Shards are first written as hidden `.part` files, flushed and synced, then
renamed atomically. Restarting the same command reuses completed shards and
recomputes only incomplete ones. `games-per-shard` is therefore the unit of
lost work after interruption. Seeds, node limits, and per-worker one-threaded
teacher searches make the generated sample reproducible independently of OS
scheduling.

Training, validation, and test partitions are selected by a stable hash of
`game_id`, never by individual position. Consequently adjacent positions from
one game cannot leak into different partitions and inflate the reported
metrics.

Generate a quick sanity dataset:

```sh
cargo run --release -p teressa --bin teressa-generate -- \
  --output target/teressa-data-smoke \
  --games 24 --games-per-shard 4 \
  --jobs 12 --hash 16 --nodes 1000 --max-plies 80
```

For the local 12-core/24-thread, 32-GiB machine, this is a sensible overnight
teacher run. `--hash` is per job, so this configuration uses about 1.5 GiB for
the transposition tables rather than 128 GiB:

```sh
cargo run --release -p teressa --bin teressa-generate -- \
  --output target/teressa-data-v2-fresh \
  --games 1024 --games-per-shard 8 \
  --jobs 12 --hash 128 --nodes 20000 \
  --opening-plies 10 --max-plies 600 --temperature 120 \
  --hours 6 --seed 0x5445524553534131
```

The hour limit stops assigning new shards; it does not corrupt or cut a game
that is already being written. Increasing `jobs` to 24 usually oversubscribes
the 12 physical cores and duplicates search memory, so it should be justified
by a measured throughput gain rather than the logical CPU count alone.

## Relabeling the existing V1 corpus

The original V1 plan labels used the largest hard-coded candidate confidence.
On the pilot corpus this made `develop_piece` 59.4% of all labels and left six
plan kinds completely unrepresented. V1 remains readable solely so the costly
teacher games can be reused, but training now rejects it until relabeled.

Relabel into a separate directory; the source shards are never modified:

```sh
cargo build --release -p teressa --bin teressa-relabel
./target/release/teressa-relabel \
  --input target/teressa-data-v1 \
  --output target/teressa-data-v2 \
  --jobs 12
```

The relabeler works one shard per worker, resumes completed output shards, and
prints selected frequency plus total probability mass for every plan kind. It
follows the actual game trajectory, scores actor/target/method agreement and
normalized progress over the plan horizon, keeps the best concrete candidate
per kind, and applies a softmax across kinds. Candidate count therefore cannot
inflate a class, and no target distribution is artificially forced to be
uniform. Newly generated datasets already use the same trajectory labeler.
The dataset remains V2 and does **not** need to be generated or relabeled
again. The model format is V3. Old `bdh-pilot-368g` and
`bdh-trajectory-v2-bs256` checkpoints remain useful experiment records but
cannot be loaded into the changed network; the loader rejects them explicitly.

## Choosing a safe GPU batch

The benchmark has separate inference and training modes. Inference reuses its
inputs and is useful for deployment throughput:

```sh
cargo run --release -p teressa --bin teressa-benchmark -- \
  --mode inference --architecture bdh \
  --batch-size 64 --moves 328 --iterations 100 \
  --width 192 --steps 4 --heads 4 --sparse-per-head 48
```

Training mode deliberately constructs and uploads a fresh complete batch on
every step, executes the real multi-head loss and AdamW update, then runs a
non-autodiff validation phase. It therefore tests the allocator lifecycle that
a short inference benchmark cannot exercise. For a full-epoch-equivalent soak
on the current 174,717-position corpus:

```sh
cargo run --release -p teressa --bin teressa-benchmark -- \
  --mode training --architecture bdh \
  --batch-size 64 --moves 328 \
  --iterations 2200 --validation-iterations 300 \
  --width 192 --steps 4 --heads 4 --sparse-per-head 48
```

On 2026-08-20 the V2 architecture completed this on the project Radeon RX 6700
XT with a measured
peak of 2165.5 MiB total VRAM use, including 505.4 MiB already in use before
the process. V3 adds endpoint gathers and history tensors, so that number is
now a historical regression measurement rather than a V3 guarantee. Rerun the
benchmark before assuming that a larger batch is safe. Use the largest batch
that completes comfortably without desktop latency; throughput, not allocated
VRAM by itself, is the deciding metric.

The original sudden jump from roughly 1.4 GiB to the complete 12 GiB heap was
at the first epoch boundary, before the first epoch log line. Validation was
running through the autodiff backend, retaining graphs that were never passed
to `backward`, and copied six complete output/target arrays plus loss back to
the host for every batch. Validation now uses `model.valid()`, computes all
the validation metrics on the GPU, and transfers one small aggregate vector
per batch. Training no longer reads loss back on every optimizer step; it reports
`train_probe_loss` from 512 positions after the epoch. Fixed legal-move padding,
periodic synchronization, and explicit WGPU pool cleanup keep long-running
upload/intermediate allocation bounded.

## Training and evaluation

Build the trainer and evaluator once:

```sh
cargo build --release -p teressa --bin teressa-train --bin teressa-eval
```

The first meaningful V3 candidate can reuse the existing V2 dataset. Eight
epochs at batch 256 provide about 4,300 optimizer updates on the current
174,717-position corpus; the old three-epoch run provided too few updates to
judge the repaired architecture. If V3 no longer fits comfortably in VRAM,
reduce only `--batch-size` to 192 or 128 first:

```sh
./target/release/teressa-train \
  --input target/teressa-data-v2 \
  --output target/teressa/bdh-context-v3-masked \
  --architecture bdh \
  --epochs 8 --batch-size 256 --learning-rate 0.0006 \
  --warmup-fraction 0.05 --minimum-lr-ratio 0.10 \
  --plan-loss-weight 1.0 \
  --width 192 --steps 4 --heads 4 --sparse-per-head 48 \
  --seed 0x5445524553534132
```

At every completed epoch the trainer atomically publishes an alternating-slot
full-precision model/AdamW checkpoint. It separately publishes `<output>.mpk`
only when validation improves. The selection loss is
`policy_KL + candidate-masked plan_KL + 0.25 * WDL_CE`: policy and strategy are
primary, WDL remains relevant to the expressive-plan safety gate, and noisy
score/risk auxiliaries cannot replace a better playing model. The manifest
records the selected epoch and loss. If training is interrupted, continue
the same output prefix and exact architecture/split seed; `--epochs` means the
number of **additional** epochs in this invocation:

```sh
./target/release/teressa-train \
  --input target/teressa-data-v2 \
  --output target/teressa/bdh-context-v3-masked \
  --architecture bdh \
  --epochs 4 --batch-size 256 --learning-rate 0.0003 \
  --warmup-fraction 0.03 --minimum-lr-ratio 0.10 \
  --plan-loss-weight 1.0 \
  --width 192 --steps 4 --heads 4 --sparse-per-head 48 \
  --seed 0x5445524553534132 \
  --resume
```

The additional segment gets its own warmup/cosine schedule but retains the
latest model and AdamW moments as well as the best model seen across both
segments. Lowering the peak learning rate for a continuation is intentional.
The selected compact play checkpoint is `<output>.mpk`; resumable files
use `<output>-training-{0,1}.mpk`, `<output>-optimizer-{0,1}.bin`, and a small
state JSON published last.

Check untouched test positions:

```sh
./target/release/teressa-eval \
  --model target/teressa/bdh-context-v3-masked \
  --input target/teressa-data-v2 --batch-size 256 \
  --seed 0x5445524553534132
```

The report now compares every head against a useful trivial baseline. Policy
reports gain over a uniform legal-move distribution, explained fraction,
top-1/3/8 teacher-best inclusion, captured teacher probability mass, and
top-1/3/8 teacher-score regret. Regret is clipped at 2,000 centipawns so mate
scores cannot dominate the mean. WDL reports CE, ordinary accuracy,
majority-class accuracy, balanced accuracy, and per-outcome recall. Plans
report candidate-masked soft-target CE/KL, gain over an empirical prior
renormalized over applicable kinds, explained fraction, top-1/2/3 inclusion,
target/predicted mass, and candidate-masked argmax recall per kind. Score and
tactical-risk MSE are printed separately, making a rise in combined loss
diagnosable. `plan_accuracy` alone is not an acceptance metric: on the old
pilot it hid the collapse into `ImprovePiece`. These remain smoke metrics, not
proof of human-like strategy. A research candidate should
additionally be accepted only after fixed-sample, color-swapped matches report:

- score and confidence interval versus both residual and TeraStockfish;
- tactical veto and blunder rates at the same safety-node budget;
- plan duration, plan turnover, completion, and two-veto cancellation rates;
- WDL calibration and policy cross-entropy on a separately frozen corpus;
- qualitative review of plans before seeing the move or result.

Do not select checkpoints on the final test partition. Use validation while
developing, freeze the candidate, and consume the test set once.

The new evaluator can also reassess the older `bdh-context-v3` checkpoint; its
manifest will simply print unknown selection metadata. For a fair trunk
comparison, however, train a fresh residual output against the fresh masked
BDH command above. Do not compare a newly candidate-masked residual loss with
the older unmasked run. Keep every other option identical:

```sh
./target/release/teressa-train \
  --input target/teressa-data-v2 \
  --output target/teressa/residual-context-v3-masked \
  --architecture residual \
  --epochs 8 --batch-size 256 --learning-rate 0.0006 \
  --warmup-fraction 0.05 --minimum-lr-ratio 0.10 \
  --plan-loss-weight 1.0 \
  --width 192 --steps 4 \
  --seed 0x5445524553534132
```

## Fixed-sample playing-strength arena

Offline metrics do not establish that a model improves actual play. Build the
arena once and run three color-swapped comparisons using the same opening seed
and the same node budget. `stockfish` means pure production TeraStockfish; any
other player value is a Teressa checkpoint prefix:

```sh
cargo build --release -p teressa --bin teressa-arena
```

First run a 24-pair pilot for the old model against the selected masked model:

```sh
./target/release/teressa-arena \
  --preset quick --jobs 2 --search-threads 6 --hash 32 \
  --baseline target/teressa/bdh-context-v3 \
  --candidate target/teressa/bdh-context-v3-masked \
  --output target/teressa-arena-old-vs-masked.jsonl
```

Then compare both neural agents against pure TeraStockfish. Keep the candidate
on the neural side so all scores and confidence intervals have the same
interpretation:

```sh
./target/release/teressa-arena \
  --preset quick --jobs 2 --search-threads 6 --hash 32 \
  --baseline stockfish \
  --candidate target/teressa/bdh-context-v3 \
  --output target/teressa-arena-stockfish-vs-old.jsonl
```

```sh
./target/release/teressa-arena \
  --preset quick --jobs 2 --search-threads 6 --hash 32 \
  --baseline stockfish \
  --candidate target/teressa/bdh-context-v3-masked \
  --output target/teressa-arena-stockfish-vs-masked.jsonl
```

If the pilots are healthy, replace `quick` with `overnight` for 96 pairs per
comparison. A stopped run is resumed by repeating its command with `--resume`;
the JSONL file is atomically replaced only after a complete color-swapped pair.
The number of arena jobs may be changed on resume, but all chess/search
parameters must continue to match the file header. Each job loads its own model
weights while all jobs share one Vulkan runtime. Two jobs with six search
threads each are nevertheless a conservative way to use a 12-core machine
without excessive simultaneous GPU inference and search memory.

The final report includes paired 95% confidence intervals, termination counts,
actual search nodes, neural safety-veto rate, tactical loss and blunder rate,
plan duration, completion, replacement, invalidation, horizon expiry,
two-veto cancellation, and plan-kind usage. A `blunder` is specifically a
selected move whose safety-search score is at least `--blunder-threshold`
centipawns below TeraStockfish's best root move; it is not a claim about human
annotation. Progress lines are provisional. Accept or reject a model only from
the final paired result, and inspect the plan telemetry even if playing score is
statistically inconclusive.

## UCI

The default binary is a native UCI engine:

```sh
cargo run --release -p teressa
```

Relevant options are `ModelPath`, `SafetyNodes`, `Hash`, and `Threads`.
`go nodes N` sets the actual TeraStockfish tactical budget; it is deliberately
not mapped to an invented Elo. Russian and English plans are emitted as
ordinary `info string` messages. Without a valid checkpoint the engine reports
the load error and returns `bestmove 0000` rather than silently pretending that
an untrained model exists.

## What the first checkpoint does and does not establish

The first checkpoint learns compressed regularities from the tactical teacher
and bootstrap planner. That can already produce persistent, inspectable plans
and a less single-line move ordering, but it cannot learn knowledge absent from
its data. The next research stages should be carried out one at a time:

1. Establish residual versus BDH on identical teacher data.
2. Add curated human games or human plan annotations and measure whether plan
   selection improves without increasing the safety-veto rate.
3. Run self-play from diverse legal openings, retaining the fixed external
   test corpus.
4. Only then test BDH-CQ or a larger trunk, because architecture scale cannot
   repair biased targets or a leaky evaluation.
