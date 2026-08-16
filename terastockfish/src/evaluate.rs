use capablanca_chess_plus::{Color, PieceKind, Position, Square};

/// Material values are scaled so a Terachess II pawn is 100. Their relative
/// order follows the values published with the variant, then positional terms
/// resolve equal-material positions.
#[must_use]
pub const fn piece_value(kind: PieceKind) -> i32 {
    match kind {
        PieceKind::Pawn => 100,
        PieceKind::Giraffe => 340,
        PieceKind::Camel => 360,
        PieceKind::Knight | PieceKind::Elephant => 400,
        PieceKind::Machine => 440,
        PieceKind::Prince => 460,
        PieceKind::Troll => 480,
        PieceKind::Archer => 660,
        PieceKind::Bishop => 680,
        PieceKind::Centaur => 820,
        PieceKind::Missionary => 880,
        PieceKind::Rook | PieceKind::Cannon => 1_000,
        PieceKind::Archbishop => 1_060,
        PieceKind::Buffalo => 1_080,
        PieceKind::Duchess => 1_160,
        PieceKind::Lion | PieceKind::Admiral => 1_200,
        PieceKind::Rhinoceros => 1_220,
        PieceKind::Chancellor => 1_380,
        PieceKind::Sorceress => 1_640,
        PieceKind::Queen => 1_660,
        PieceKind::Eagle => 1_680,
        PieceKind::Amazon => 2_040,
        PieceKind::King => 0,
    }
}

/// Returns a static score from the side-to-move's point of view.
#[must_use]
pub fn evaluate(position: &Position) -> i32 {
    let size = position.board().size();
    let mut score = [0_i32; 2];
    let mut pawn_files = [[0_u8; 18]; 2];
    let mut bishops = [0_u8; 2];

    for (square, piece) in position.board().pieces() {
        let side = piece.color.index();
        score[side] += piece_value(piece.kind);
        score[side] += centrality_bonus(square, size.files(), size.ranks(), piece.kind);
        score[side] += advancement_bonus(square, size.ranks(), piece.color, piece.kind);
        if piece.kind == PieceKind::Pawn {
            pawn_files[side][usize::from(square.file())] += 1;
        } else if piece.kind == PieceKind::Bishop {
            bishops[side] += 1;
        }
    }

    for color in Color::ALL {
        let side = color.index();
        score[side] += pawn_structure(&pawn_files[side], usize::from(size.files()));
        if bishops[side] >= 2 {
            score[side] += 28;
        }
        score[side] += king_shelter(position, color);
    }

    let white_score = score[Color::White.index()] - score[Color::Black.index()];
    let relative = match position.side_to_move() {
        Color::White => white_score,
        Color::Black => -white_score,
    };
    relative + 12
}

fn centrality_bonus(square: Square, files: u8, ranks: u8, kind: PieceKind) -> i32 {
    if matches!(kind, PieceKind::Pawn | PieceKind::King) {
        return 0;
    }
    let file_distance = (i16::from(square.file()) * 2 - i16::from(files - 1)).unsigned_abs();
    let rank_distance = (i16::from(square.rank()) * 2 - i16::from(ranks - 1)).unsigned_abs();
    let span = u16::from(files - 1) + u16::from(ranks - 1);
    let centrality = span.saturating_sub(file_distance + rank_distance);
    let weight = match kind {
        PieceKind::Knight
        | PieceKind::Camel
        | PieceKind::Giraffe
        | PieceKind::Elephant
        | PieceKind::Machine
        | PieceKind::Buffalo
        | PieceKind::Lion
        | PieceKind::Centaur
        | PieceKind::Duchess
        | PieceKind::Troll => 2,
        _ => 1,
    };
    i32::from(centrality) * weight
}

fn advancement_bonus(square: Square, ranks: u8, color: Color, kind: PieceKind) -> i32 {
    let relative_rank = match color {
        Color::White => square.rank(),
        Color::Black => ranks - 1 - square.rank(),
    };
    let weight = match kind {
        PieceKind::Pawn => 7,
        PieceKind::Prince | PieceKind::Troll => 3,
        _ => 0,
    };
    i32::from(relative_rank) * weight
}

fn pawn_structure(files: &[u8; 18], board_files: usize) -> i32 {
    let mut score = 0;
    for file in 0..board_files {
        let count = files[file];
        if count > 1 {
            score -= i32::from(count - 1) * 18;
        }
        if count > 0 {
            let left = file.checked_sub(1).is_some_and(|left| files[left] > 0);
            let right = file + 1 < board_files && files[file + 1] > 0;
            if !left && !right {
                score -= i32::from(count) * 12;
            }
        }
    }
    score
}

fn king_shelter(position: &Position, color: Color) -> i32 {
    let Some(king) = position.board().king_square(color) else {
        return 0;
    };
    let mut shelter = 0;
    for file_delta in -1..=1 {
        for rank_delta in -1..=1 {
            if file_delta == 0 && rank_delta == 0 {
                continue;
            }
            if king
                .offset(file_delta, rank_delta)
                .filter(|square| position.board().size().contains(*square))
                .and_then(|square| position.board().piece_at(square))
                .is_some_and(|piece| piece.color == color)
            {
                shelter += 5;
            }
        }
    }
    if position.king_jump_available(color) {
        shelter += 20;
    }
    shelter
}
