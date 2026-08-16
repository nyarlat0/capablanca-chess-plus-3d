# TeraStockfish

`terastockfish` is an original large-board alpha-beta analysis engine built on
the legal move generator in `capablanca-engine`. It currently targets
Terachess II (16x16), while all square-indexed search structures and move
encoding reserve capacity for boards up to 18x18.

The crate provides both an embeddable Rust API and an asynchronous UCI process.
It is not derived from Stockfish or Fairy-Stockfish and does not claim equal
playing strength; the name describes the intended role and architecture.

## Search architecture

- iterative deepening with principal-variation search;
- aspiration windows with automatic full-window recovery;
- capture/check quiescence search;
- check extensions, late-move reductions, and conservative shallow razoring;
- TT, MVV-LVA, promotion, killer, and history move ordering;
- a fixed-size lock-free transposition table shared by root workers;
- 21-bit signed TT scores, leaving a safe range for Terachess material and
  distance-to-mate values;
- repetition, fifty-move, mate, and stalemate terminal handling;
- depth, node, fixed move-time, clock, infinite, and external stop limits;
- a deterministic hand-written Terachess evaluation using the published
  relative material scale plus centrality, development, pawn structure,
  bishop pair, king shelter, and tempo terms.

The board hash includes the complete piece array, dimensions, side to move,
castling and initial king-jump rights, en-passant state, rules identity, and
the halfmove clock where relevant. Arbitrary legal Terachess II FEN positions
can therefore be analysed independently of the built-in start position.

## UCI usage

Build or run the native engine:

```sh
cargo build --release -p terastockfish
cargo run --release -p terastockfish
```

Example session:

```text
uci
setoption name Hash value 512
setoption name Threads value 4
isready
position startpos moves a4a6 a13a11
go depth 8
stop
quit
```

`position fen <six fields> [moves ...]` accepts any Terachess II position.
Supported `go` limits are `depth`, `nodes`, `movetime`, `wtime`, `btime`,
`winc`, `binc`, `movestogo`, `infinite`, and the diagnostic `perft` command.
Moves use long coordinate notation, including multi-digit ranks, for example
`a4a6` and `e10e12`.

## Rust API

```rust
use capablanca_chess_plus::Variant;
use terastockfish::{SearchLimits, SearchOptions, Searcher};

let position = Variant::TerachessII.starting_position();
let mut searcher = Searcher::new(SearchOptions {
    hash_megabytes: 256,
    threads: 4,
});
let result = searcher.analyze(&position, SearchLimits::depth(7));
println!("best: {:?}, score: {}", result.best_move, result.score);
```

Use `analyze_with` to receive a report after every completed iteration. Clone
the searcher's `SearchControl` before starting analysis when another thread
must stop it. Reuse one `Searcher` across positions to preserve its hash table
and move-ordering history; call `clear_hash` for a completely fresh analysis.

## Verification and profiling

```sh
cargo test -p capablanca-engine -p terastockfish
cargo bench -p terastockfish --bench search
```

The benchmark prints elapsed time, searched nodes, and nodes per second for the
full Terachess II starting array. It is intentionally dependency-free so the
same binary can be profiled on the eventual deployment machine.

Future strength work can add incremental make/unmake and Zobrist updates,
variant-specific piece-square tables, null-move and stronger pruning verified
for this ruleset, persistent opening data, and an NNUE-style evaluator without
changing the public position/depth API or the 18x18 TT move format.
