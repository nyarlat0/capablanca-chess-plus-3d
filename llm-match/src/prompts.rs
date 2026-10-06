use capablanca_chess_plus::{CastleSide, Color, Position, PromotionRule, Variant};
use std::fmt::Write;

const COMPOUNDS: &str = "A=archbishop (bishop+knight), C=chancellor (rook+knight).";
const SHAKO: &str = "C=cannon: noncapturing rook slides; captures along a rook line over exactly one intervening piece of either color. E=elephant: diagonal step or leap of exactly two squares, jumping intervening pieces.";

/// Each variant gets its own rules prompt; castling/promotion details come from
/// the same rules object used to validate moves, not a separate move generator.
pub fn chess_rules(variant: Variant) -> String {
    let rules = variant.rules();
    let size = rules.board_size();
    let mut text = format!(
        "You are playing {} on a {}x{} board, files a-{}, ranks 1-{}. White advances toward increasing ranks, Black toward decreasing ranks. Uppercase symbols are White, lowercase Black.\n",
        rules.name(),
        size.files(),
        size.ranks(),
        (b'a' + size.files() - 1) as char,
        size.ranks()
    );
    text.push_str(match variant {
        Variant::Capablanca => "Capablanca starting arrangement. ",
        Variant::Gothic => "Gothic starting arrangement. ",
        Variant::Embassy => "Embassy starting arrangement. ",
        Variant::Schoolbook => "Schoolbook starting arrangement. ",
        Variant::Bird => "Bird starting arrangement. ",
        Variant::Carrera => "Carrera starting arrangement. ",
        Variant::Grand => "Grand: pawns start on ranks 3/8. Promotion is optional on the last three ranks and compulsory on the final rank, and can only restore a missing originally available non-pawn, non-king piece. If no restoration is available a pawn cannot enter the last rank, but still attacks it. ",
        Variant::Shako => "Shako: pawns start on ranks 3/8. ",
        Variant::Pemba => "Pemba: pawns start on ranks 3/8. M=camel (1,3 leaper), Z=giraffe (1,4 leaper), V=archer (diagonal cannon: quiet bishop slides, captures across exactly one screen), W=machine (orthogonal step or two-square leap). ",
        Variant::TerachessII => "Terachess II: A=amazon (queen+knight), X=archbishop (bishop+knight), H=chancellor (rook+knight), M=camel (1,3 leaper), Z=giraffe (1,4 leaper), V=archer (diagonal cannon: quiet bishop slides, captures across exactly one screen), W=machine (orthogonal step or two-square leap), L=lion (nonroyal king+knight+two-square orthogonal/diagonal leaps), F=buffalo (knight+camel+giraffe), J=centaur (nonroyal king+knight), S=admiral (rook+one diagonal step), Y=missionary (bishop+one orthogonal step), G=eagle (one diagonal step then optionally an outward orthogonal slide), U=rhinoceros (one orthogonal step then optionally an outward diagonal slide); eagle/rhino need the first square empty to continue. I=prince (nonroyal king plus a quiet forward double-step through an empty square), O=sorceress (cannon along orthogonal or diagonal lines), D=duchess (leaps 1, 2 or 3 squares in any orthogonal/diagonal direction), T=troll (three-square orthogonal/diagonal leap plus ordinary pawn single advance and diagonal capture). All leaps ignore intervening pieces. Pawns and princes can double-step from any rank if both squares are empty; both can be captured en passant by a pawn after a double-step (even if that double-step promoted the victim). No castling. An unmoved king may instead jump to an empty square at Chebyshev distance two, not out of/into check; straight/diagonal jumps must pass a safe intermediate square, knight-shaped jumps require at least one of the two intermediate squares safe. FEN J/j records this right. On reaching the final rank: pawn/troll -> queen (q), prince -> amazon (a), knight/camel/giraffe -> buffalo (f), elephant/machine/centaur -> lion (l). Troll promotion requires its pawn-like move, not a leap. These promotion suffixes are mandatory. ",
    });
    match variant {
        Variant::Shako | Variant::Pemba | Variant::TerachessII => text.push_str(SHAKO),
        _ => text.push_str(COMPOUNDS),
    }
    text.push('\n');
    if !rules.castling().any() {
        text.push_str("No castling.\n");
    }
    for color in [Color::White, Color::Black] {
        for side in CastleSide::ALL {
            if let Some(route) = rules.castling().route(color, side) {
                writeln!(text, "{color:?} castling: king {}->{}, rook {}->{}; send the king coordinates. Ordinary unmoved/empty-path/check restrictions apply.", route.king_from, route.king_to, route.rook_from, route.rook_to).unwrap();
            }
        }
    }
    if let PromotionRule::LastRank { choices } = rules.promotion() {
        let suffixes: String = choices.iter().map(|kind| kind.fen_char()).collect();
        writeln!(
            text,
            "Last-rank pawn promotion requires one suffix from: {suffixes}."
        )
        .unwrap();
    }
    if variant == Variant::Grand {
        text.push_str("Restoration UCI suffixes: q,r,b,n,a,c; availability is limited by missing original material, not arbitrary choice.\n");
    }
    text
}

pub fn board_state(position: &Position) -> String {
    let fen = position.to_fen();
    let size = position.board().size();
    let mut text = format!(
        "AUTHORITATIVE CURRENT POSITION. This state overrides any inferred position from history.\nExtended FEN: {fen}\nSide to move: {:?}\nASCII board (same symbols as FEN; . = empty):\n",
        position.side_to_move()
    );
    for (index, row) in fen
        .split_whitespace()
        .next()
        .unwrap()
        .split('/')
        .enumerate()
    {
        write!(text, "{:>2} ", usize::from(size.ranks()) - index).unwrap();
        let mut empty = 0usize;
        for c in row.chars().chain(std::iter::once('!')) {
            if let Some(digit) = c.to_digit(10) {
                empty = empty * 10 + digit as usize;
                continue;
            }
            for _ in 0..empty {
                text.push_str(". ");
            }
            empty = 0;
            if c != '!' {
                write!(text, "{c} ").unwrap();
            }
        }
        text.push('\n');
    }
    text.push_str("   ");
    for file in 0..size.files() {
        write!(text, "{} ", (b'a' + file) as char).unwrap();
    }
    text
}
