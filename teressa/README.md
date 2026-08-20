# Teressa

Teressa is the native neural research engine for Terachess II. Its purpose is
not to replace exact rules or tactical calculation with a black box. It learns
a long-term, human-readable strategic intention and a move policy, while
TeraStockfish rejects tactically unsound choices.

This crate is a working research pipeline, not a pretrained strong engine. A
checkpoint must first be produced from generated or imported games. The
initial teacher data imitates TeraStockfish and uses deterministic strategic
heuristics as bootstrap plan labels; human games and later self-play are what
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
- A variable-size legal-move list with 12 geometric/special-move features.
- Heads for legal-move policy, WDL, scalar value, plan class, and tactical risk.

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

`teressa-generate` writes zstd-compressed JSON Lines. Each shard begins with a
format-version header and contains FEN, recent UCI history, the complete legal
move set, a temperature-normalized teacher policy with root scores, WDL,
teacher score, and the structured plan label.

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
  --output target/teressa-data-v1 \
  --games 1024 --games-per-shard 8 \
  --jobs 12 --hash 128 --nodes 20000 \
  --opening-plies 10 --max-plies 600 --temperature 120 \
  --hours 6 --seed 0x5445524553534131
```

The hour limit stops assigning new shards; it does not corrupt or cut a game
that is already being written. Increasing `jobs` to 24 usually oversubscribes
the 12 physical cores and duplicates search memory, so it should be justified
by a measured throughput gain rather than the logical CPU count alone.

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

On 2026-08-20 this completed on the project Radeon RX 6700 XT with a measured
peak of 2165.5 MiB total VRAM use, including 505.4 MiB already in use before
the process. This is a regression measurement for that machine and driver, not
a universal memory guarantee. Use the largest batch that completes comfortably
without desktop latency; throughput, not allocated VRAM by itself, is the
deciding metric.

The original sudden jump from roughly 1.4 GiB to the complete 12 GiB heap was
at the first epoch boundary, before the first epoch log line. Validation was
running through the autodiff backend, retaining graphs that were never passed
to `backward`, and copied six complete output/target arrays plus loss back to
the host for every batch. Validation now uses `model.valid()`, computes all
three accuracies on the GPU, and transfers one four-float metric vector per
batch. Training no longer reads loss back on every optimizer step; it reports
`train_probe_loss` from 512 positions after the epoch. Fixed legal-move padding,
periodic synchronization, and explicit WGPU pool cleanup keep long-running
upload/intermediate allocation bounded.

## Training and evaluation

Train the BDH candidate:

```sh
cargo run --release -p teressa --bin teressa-train -- \
  --input target/teressa-data-v1 \
  --output target/teressa/bdh-v1 \
  --architecture bdh \
  --epochs 20 --batch-size 16 --learning-rate 0.0003 \
  --width 192 --steps 4 --heads 4 --sparse-per-head 48 \
  --seed 0x5445524553534132
```

Train the control with the same input, split seed, width, epochs, and batch:

```sh
cargo run --release -p teressa --bin teressa-train -- \
  --input target/teressa-data-v1 \
  --output target/teressa/residual-v1 \
  --architecture residual \
  --epochs 20 --batch-size 16 --learning-rate 0.0003 \
  --width 192 --steps 4 \
  --seed 0x5445524553534132
```

Check untouched test positions:

```sh
cargo run --release -p teressa --bin teressa-eval -- \
  --model target/teressa/bdh-v1 \
  --input target/teressa-data-v1 --batch-size 32 \
  --seed 0x5445524553534132
```

The report currently contains the combined loss, teacher-policy top-1 match,
WDL accuracy, and plan accuracy. These are necessary smoke metrics, but they do
not prove human-like strategy. A research candidate should additionally be
accepted only after fixed-sample, color-swapped matches report:

- score and confidence interval versus both residual and TeraStockfish;
- tactical veto and blunder rates at the same safety-node budget;
- plan duration, plan turnover, completion, and two-veto cancellation rates;
- WDL calibration and policy cross-entropy on a separately frozen corpus;
- qualitative review of plans before seeing the move or result.

Do not select checkpoints on the final test partition. Use validation while
developing, freeze the candidate, and consume the test set once.

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
