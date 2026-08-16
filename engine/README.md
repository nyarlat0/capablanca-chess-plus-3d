# Capablanca Chess Plus

A dependency-free Rust rules library for chess on boards up to 16x16.
It supports legal move generation, checks and mates, en passant, promotion,
variant-specific castling, extended FEN, and repetition tracking.

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
| Shako | 10x10 | `C/ERNBQKBNRE/pawns` on three ranks | Orthodox two-square castling from `f2`/`f9` |
| Pemba | 10x10 | `CMVZWWZVMC/ERNBQKBNRE/pawns` on three ranks | Orthodox two-square castling from `f2`/`f9` |
| Terachess II | 16x16 | 64 pieces per side on four ranks | None; one-time initial King jump instead |

`A` is the archbishop/cardinal (bishop + knight). `C` is the
chancellor/marshal (rook + knight). Grand Chess promotion is optional on a
player's eighth and ninth ranks, mandatory on the tenth, and restricted to
captured pieces from that player's initial material.

In Shako, `C` means the Xiangqi-style cannon and `E` means the elephant. A
cannon moves without capture like a rook and captures the first piece beyond
exactly one intervening screen. An elephant leaps one or two squares
diagonally. Pawns promote on rank ten to queen, rook, bishop, knight, elephant,
or cannon.

Pemba adds `M` camel `(3,1)`, `Z` giraffe `(3,2)`, `V` archer (a diagonal
cannon), and `W` machine (one- or two-square orthogonal leaper). Its pawns may
promote to any of its ten non-royal piece types.

Terachess II includes all 26 piece types from its reference rules. Its pawns
and Princes can double-step from any rank; only pawns capture en passant. The
variant also implements the bent paths of the Eagle and Rhinoceros, screened
captures of the Sorceress, compulsory piece-specific promotions, the Troll's
promotion exception, and the King's one-time two-square jump with intermediate
threat checks.

## Library Use

```rust
use capablanca_chess_plus::{Game, Variant};

let mut game = Game::new(Variant::Gothic.starting_position());
game.play_uci("e2e4")?;
println!("{}", game.position().to_fen());

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

Extended FEN uses `A` for archbishop and `E` for elephant. `C` is resolved from
the supplied rules as a Capablanca chancellor or a cannon. Pemba uses `M`, `Z`,
`V`, and `W` for camel, giraffe, archer, and machine; outside Pemba, `M` remains
accepted as a marshal/chancellor alias. Coordinate moves support rank 10, for
example `a9a10q`, the cannon promotion `a9a10c`, or Pemba machine promotion
`a9a10w`. Terachess II follows the reference diagram's letters (`X` cardinal,
`H` marshal, `A` amazon, and the remaining unique letters) and uses `J`/`j` in
the FEN rights field while White's/Black's initial King jump is still available.
Coordinates extend through `p16`.

## Rule References

- [GNU XBoard Gothic Chess rules](https://www.gnu.org/software/xboard/whats_new/rules/Gothic.html)
- [Schoolbook Chess rules from its creator](https://samiam.org/schoolbook/)
- [Grand Chess rules licensed from MindSports](https://www.yucata.de/en/Rules/GrandChess)
- [Shako Chess rules](https://musketeerchess.net/p/games/shako/rules/rules.php)
- [Pemba rules](https://www.chessvariants.com/rules/pemba)
- [Terachess II rules](https://www.chessvariants.com/rules/terachess-ii)
- [Capablanca-family arrays and historical notes](https://mats-winther.github.io/bg/capablanca.htm)

## License

This project is released under The Unlicense.
