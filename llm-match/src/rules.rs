use crate::{OutputFormat, Representation, StateFormat, representation::coordinate};
use anyhow::Result;
use capablanca_chess_plus::{CastleSide, Color, Piece, PieceKind, PromotionRule, Variant};
use prompt_core::{PromptBuilder, PromptData};
use serde_json::json;
use std::fmt::Write;

pub fn variant_key(variant: Variant) -> &'static str {
    match variant {
        Variant::Capablanca => "capablanca",
        Variant::Gothic => "gothic",
        Variant::Embassy => "embassy",
        Variant::Schoolbook => "schoolbook",
        Variant::Bird => "bird",
        Variant::Carrera => "carrera",
        Variant::Grand => "grand",
        Variant::Shako => "shako",
        Variant::Pemba => "pemba",
        Variant::TerachessII => "terachess-ii",
    }
}
pub(crate) fn builtin_template(variant: Variant) -> &'static str {
    match variant {
        Variant::Capablanca => include_str!("../rules/capablanca.txt"),
        Variant::Gothic => include_str!("../rules/gothic.txt"),
        Variant::Embassy => include_str!("../rules/embassy.txt"),
        Variant::Schoolbook => include_str!("../rules/schoolbook.txt"),
        Variant::Bird => include_str!("../rules/bird.txt"),
        Variant::Carrera => include_str!("../rules/carrera.txt"),
        Variant::Grand => include_str!("../rules/grand.txt"),
        Variant::Shako => include_str!("../rules/shako.txt"),
        Variant::Pemba => include_str!("../rules/pemba.txt"),
        Variant::TerachessII => include_str!("../rules/terachess-ii.txt"),
    }
}

pub fn render_rules(variant: Variant, profile: &Representation, template: &str) -> Result<String> {
    let rules = variant.rules();
    let size = rules.board_size();
    let numeric = profile.state_format != StateFormat::PiecesAscii;
    let header = format!(
        "You are playing {} on a {}x{} board. White forward is +y, Black forward is -y. Coordinates are absolute; no move has displacement (0,0). dx=x_to-x_from; dy=y_to-y_from. Destination must be on the board and not occupied by a friendly piece. All moves must preserve royal king safety.",
        rules.name(),
        size.files(),
        size.ranks()
    );
    let mut legend = String::from(if profile.state_format == StateFormat::Numeric {
        "PIECE LEGEND\nPiece names and movement rules are authoritative. Never infer piece identities from memory or starting positions.\n"
    } else {
        "PIECE LEGEND\nNames and symbol meanings are authoritative. Never infer piece identities from standard chess, memory, starting squares, or the letter itself.\n"
    });
    let mut kinds: Vec<_> = PieceKind::ALL
        .into_iter()
        .filter(|k| rules.uses_piece(*k))
        .collect();
    kinds.sort_by_key(|k| k.display_name());
    for kind in kinds {
        let label = if profile.state_format == StateFormat::Numeric {
            kind.display_name().into()
        } else {
            format!(
                "{} = {}",
                rules.piece_symbol(Piece::new(Color::White, kind)),
                kind.display_name()
            )
        };
        writeln!(
            legend,
            "{label}: {}",
            if numeric {
                kind.geometric_description()
            } else {
                kind.movement_description()
            }
        )
        .unwrap();
    }
    let mut castles = String::new();
    if !rules.castling().any() {
        castles.push_str("Castling: none\n");
    }
    for color in [Color::White, Color::Black] {
        for side in CastleSide::ALL {
            if let Some(r) = rules.castling().route(color, side) {
                writeln!(castles,"{color:?} {side:?} castling: King {} -> {}, Rook {} -> {}. Retained rights, empty routes and king-safe start/transit/destination required. Output the King's source and destination.",coordinate(r.king_from,numeric),coordinate(r.king_to,numeric),coordinate(r.rook_from,numeric),coordinate(r.rook_to,numeric)).unwrap();
            }
        }
    }
    let pawns = if rules.rapid_pawns() {
        "Pawns and Princes can advance quietly with dx=0,dy=2*forward from any rank if both squares are empty. Only a Pawn may capture en passant after a Pawn/Prince double-step, even if that double-step promoted the victim. Prince/Troll cannot capture en passant.".into()
    } else {
        format!(
            "Pawn quiet double-step: dx=0,dy=2*forward, both squares empty; only from White y={} or Black y={}. Ordinary en passant applies.",
            rules.pawn_start_rank(Color::White) + 1,
            rules.pawn_start_rank(Color::Black) + 1
        )
    };
    let mut promotions = String::new();
    let choices = match rules.promotion() {
        PromotionRule::LastRank { choices } => choices.clone(),
        PromotionRule::Grand => PieceKind::PROMOTION_PIECES.to_vec(),
        PromotionRule::TerachessII => vec![
            PieceKind::Queen,
            PieceKind::Amazon,
            PieceKind::Buffalo,
            PieceKind::Lion,
        ],
    };
    if matches!(rules.promotion(), PromotionRule::LastRank { .. }) {
        promotions.push_str("Mandatory pawn promotion on the final rank. ");
    }
    promotions.push_str("Promotion piece identities: ");
    promotions.push_str(
        &choices
            .iter()
            .map(|k| {
                if profile.output_format == OutputFormat::Uci {
                    format!("{}={}", k.fen_char(), k.display_name())
                } else {
                    k.display_name().to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
    );
    let initial = if numeric {
        "Initial back rank x=1..10: Rook, Knight, Bishop, Queen, Chancellor, King, Archbishop, Bishop, Knight, Rook (not the current position)."
    } else {
        "Initial back rank (not the current position), a-j: R N B Q C K A B N R"
    };
    let data = PromptData {
        custom: json!({"board-header":header,"piece-legend":legend,"castling-rules":castles,"pawn-rules":pawns,"promotion-rules":promotions,"initial-back-rank":initial,"numeric":numeric}),
        ..Default::default()
    };
    PromptBuilder::new().render_text(template, &data)
}
