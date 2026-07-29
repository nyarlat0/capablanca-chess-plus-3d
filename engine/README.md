# Capablanca Chess Plus

A dependency-free Rust rules and search library for chess on 10-file boards.
It supports legal move generation, checks and mates, en passant, promotion,
variant-specific castling, extended FEN, repetition tracking, and a reference
alpha-beta engine.

## Variants

| Preset | Board | Back rank | Castling |
| --- | --- | --- | --- |
| Capablanca | 10x8 | `RNABQKBCNR` | King `f` to `c` or `i` |
| Gothic | 10x8 | `RNBQCKABNR` | King `f` to `c` or `i` |
| Embassy | 10x8 | `RNBQKCABNR` | King `e` to `b` or `h` |
| Schoolbook | 10x8 | `RQNBAKBNCR` | King `f` to `c` or `i` |
| Bird | 10x8 | `RNBCQKABNR` | King `f` to `c` or `i` |
| Carrera | 10x8 | `RCNBKQBNAR` | None (historical rules) |
| Grand | 10x10 | Grand Chess array | None |

`A` is the archbishop/cardinal (bishop + knight). `C` is the
chancellor/marshal (rook + knight). Grand Chess promotion is optional on a
player's eighth and ninth ranks, mandatory on the tenth, and restricted to
captured pieces from that player's initial material.

## Library Use

```rust
use capablanca_chess_plus::{Engine, Game, SearchLimits, Variant};

let mut game = Game::new(Variant::Gothic.starting_position());
game.play_uci("e2e4")?;

let result = Engine::new()
    .search(game.position(), SearchLimits::depth(4))
    .expect("the game is not over");
println!("best move: {}", result.best_move);

# Ok::<(), Box<dyn std::error::Error>>(())
```

Load a position by pairing FEN with its rules:

```rust
use capablanca_chess_plus::{Position, Variant};

let position = Position::from_fen(
    Variant::Embassy.rules(),
    "4k5/10/10/10/10/10/10/R3K4R w KQ - 0 1",
)?;
assert!(position.legal_moves().iter().any(|m| m.to_uci() == "e1b1"));

# Ok::<(), Box<dyn std::error::Error>>(())
```

Custom mirrored 10x8 arrays use the same rules core:

```rust
use capablanca_chess_plus::{PieceKind::*, VariantRules};

let rules = VariantRules::capablanca_family(
    "My Array",
    [Rook, Knight, Archbishop, Queen, King,
     Chancellor, Bishop, Knight, Bishop, Rook],
    true,
)?;
let position = rules.into_starting_position();

# Ok::<(), Box<dyn std::error::Error>>(())
```

Extended FEN uses `A` for archbishop and `C` for chancellor. The parser also
accepts `M` as a marshal alias. Coordinate moves support rank 10, for example
`a9a10q`.

## Rule References

- [GNU XBoard Gothic Chess rules](https://www.gnu.org/software/xboard/whats_new/rules/Gothic.html)
- [Schoolbook Chess rules from its creator](https://samiam.org/schoolbook/)
- [Grand Chess rules licensed from MindSports](https://www.yucata.de/en/Rules/GrandChess)
- [Capablanca-family arrays and historical notes](https://mats-winther.github.io/bg/capablanca.htm)

## License

This project is released under The Unlicense.
