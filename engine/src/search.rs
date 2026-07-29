use crate::{Color, Move, MoveKind, PieceKind, Position};

const CHECKMATE_SCORE: i32 = 1_000_000;
const INFINITY: i32 = CHECKMATE_SCORE + 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchLimits {
    pub depth: u8,
    pub node_limit: Option<u64>,
}

impl SearchLimits {
    #[must_use]
    pub const fn depth(depth: u8) -> Self {
        Self {
            depth,
            node_limit: None,
        }
    }

    #[must_use]
    pub const fn with_node_limit(mut self, node_limit: u64) -> Self {
        self.node_limit = Some(node_limit);
        self
    }
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self::depth(4)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub best_move: Move,
    /// Centipawns from the root side's perspective. Mate scores are near
    /// +/-1,000,000.
    pub score: i32,
    pub depth: u8,
    pub nodes: u64,
    pub principal_variation: Vec<Move>,
}

/// A deterministic alpha-beta engine suitable for analysis and as a reference
/// implementation for applications built on the rules library.
#[derive(Clone, Debug, Default)]
pub struct Engine;

impl Engine {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[must_use]
    pub fn search(&self, position: &Position, limits: SearchLimits) -> Option<SearchResult> {
        let depth = limits.depth.max(1);
        let mut moves = position.legal_moves();
        if moves.is_empty() {
            return None;
        }
        order_moves(position, &mut moves);

        let mut context = SearchContext {
            nodes: 0,
            node_limit: limits.node_limit,
        };
        let mut best_move = moves[0];
        let mut best_score = -INFINITY;
        let mut best_line = Vec::new();
        let mut alpha = -INFINITY;

        for chess_move in moves {
            let child = position.after_move_unchecked(chess_move);
            let mut child_line = Vec::new();
            let score = -negamax(
                &child,
                depth - 1,
                1,
                -INFINITY,
                -alpha,
                &mut context,
                &mut child_line,
            );
            if score > best_score {
                best_score = score;
                best_move = chess_move;
                best_line.clear();
                best_line.push(chess_move);
                best_line.extend(child_line);
            }
            alpha = alpha.max(score);
            if context.limit_reached() {
                break;
            }
        }

        Some(SearchResult {
            best_move,
            score: best_score,
            depth,
            nodes: context.nodes,
            principal_variation: best_line,
        })
    }
}

struct SearchContext {
    nodes: u64,
    node_limit: Option<u64>,
}

impl SearchContext {
    fn visit(&mut self) {
        self.nodes = self.nodes.saturating_add(1);
    }

    fn limit_reached(&self) -> bool {
        self.node_limit.is_some_and(|limit| self.nodes >= limit)
    }
}

fn negamax(
    position: &Position,
    depth: u8,
    ply: i32,
    mut alpha: i32,
    beta: i32,
    context: &mut SearchContext,
    principal_variation: &mut Vec<Move>,
) -> i32 {
    context.visit();
    if context.limit_reached() || position.halfmove_clock() >= 100 {
        return evaluate(position);
    }

    let mut moves = position.legal_moves();
    if moves.is_empty() {
        return if position.is_in_check(position.side_to_move()) {
            -CHECKMATE_SCORE + ply
        } else {
            0
        };
    }
    if depth == 0 {
        return evaluate(position);
    }

    order_moves(position, &mut moves);
    let mut best_line = Vec::new();
    let mut best_score = -INFINITY;
    for chess_move in moves {
        let child = position.after_move_unchecked(chess_move);
        let mut child_line = Vec::new();
        let score = -negamax(
            &child,
            depth - 1,
            ply + 1,
            -beta,
            -alpha,
            context,
            &mut child_line,
        );
        if score > best_score {
            best_score = score;
            best_line.clear();
            best_line.push(chess_move);
            best_line.extend(child_line);
        }
        alpha = alpha.max(score);
        if alpha >= beta || context.limit_reached() {
            break;
        }
    }
    *principal_variation = best_line;
    best_score
}

fn evaluate(position: &Position) -> i32 {
    let mut white = 0;
    let mut black = 0;
    let size = position.board().size();
    let center_file_twice = i32::from(size.files() - 1);
    let center_rank_twice = i32::from(size.ranks() - 1);

    for (square, piece) in position.board().pieces() {
        let centrality = 8
            - (i32::from(square.file()) * 2 - center_file_twice).abs()
            - (i32::from(square.rank()) * 2 - center_rank_twice).abs();
        let activity = if matches!(
            piece.kind,
            PieceKind::Knight | PieceKind::Bishop | PieceKind::Archbishop
        ) {
            centrality.max(0) * 2
        } else {
            centrality.max(0)
        };
        let value = piece.kind.material_value() + activity;
        match piece.color {
            Color::White => white += value,
            Color::Black => black += value,
        }
    }

    let score = white - black;
    match position.side_to_move() {
        Color::White => score,
        Color::Black => -score,
    }
}

fn order_moves(position: &Position, moves: &mut [Move]) {
    moves.sort_unstable_by_key(|chess_move| -move_priority(position, *chess_move));
}

fn move_priority(position: &Position, chess_move: Move) -> i32 {
    let moving_value = position
        .board()
        .piece_at(chess_move.from)
        .map_or(0, |piece| piece.kind.material_value());
    let captured_value = match chess_move.kind {
        MoveKind::EnPassant => PieceKind::Pawn.material_value(),
        _ => position
            .board()
            .piece_at(chess_move.to)
            .map_or(0, |piece| piece.kind.material_value()),
    };
    let promotion_value = chess_move.promotion.map_or(0, PieceKind::material_value);
    captured_value * 10 - moving_value + promotion_value
}
