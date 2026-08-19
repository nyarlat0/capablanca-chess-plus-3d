use capablanca_chess_plus::{Color, Move, PieceKind, Position, Square};
use serde::{Deserialize, Serialize};
use terastockfish::piece_value;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanColor {
    White,
    Black,
}

impl From<Color> for PlanColor {
    fn from(value: Color) -> Self {
        match value {
            Color::White => Self::White,
            Color::Black => Self::Black,
        }
    }
}

impl PlanColor {
    #[must_use]
    pub const fn engine(self) -> Color {
        match self {
            Self::White => Color::White,
            Self::Black => Color::Black,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct PlanSquare {
    pub file: u8,
    pub rank: u8,
}

impl From<Square> for PlanSquare {
    fn from(value: Square) -> Self {
        Self {
            file: value.file(),
            rank: value.rank(),
        }
    }
}

impl PlanSquare {
    #[must_use]
    pub const fn engine(self) -> Square {
        Square::new(self.file, self.rank)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardRegion {
    QueenSide,
    Center,
    KingSide,
    OwnCamp,
    EnemyCamp,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    DevelopPiece,
    ImprovePiece,
    KingAttack,
    KingSafety,
    GainSpace,
    PawnBreak,
    CreatePasser,
    AdvancePasser,
    UseOpenFile,
    UseDiagonal,
    OccupyOutpost,
    RestrictPiece,
    Trade,
    PreserveMaterial,
}

impl PlanKind {
    pub const COUNT: usize = 14;
    pub const ALL: [Self; Self::COUNT] = [
        Self::DevelopPiece,
        Self::ImprovePiece,
        Self::KingAttack,
        Self::KingSafety,
        Self::GainSpace,
        Self::PawnBreak,
        Self::CreatePasser,
        Self::AdvancePasser,
        Self::UseOpenFile,
        Self::UseDiagonal,
        Self::OccupyOutpost,
        Self::RestrictPiece,
        Self::Trade,
        Self::PreserveMaterial,
    ];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMethod {
    Activate,
    Centralize,
    BuildPressure,
    Reinforce,
    AdvancePawn,
    OpenLine,
    OccupySquare,
    ExchangeDefender,
    AvoidExchange,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanReason {
    UndevelopedPiece,
    PoorlyPlacedPiece,
    ExposedKing,
    WeakKingZone,
    SpaceDeficit,
    BlockedCenter,
    PassedPawnPotential,
    ExistingPassedPawn,
    OpenLine,
    WeakSquare,
    ActiveEnemyPiece,
    MaterialAdvantage,
    MaterialDisadvantage,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelCondition {
    Completed,
    ActorLost,
    TargetUnavailable,
    ForcedThreat,
    QueenExchange,
    PositionChanged,
    SafetyVetoTwice,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlanActor {
    pub piece: String,
    pub square: PlanSquare,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum PlanTarget {
    Square(PlanSquare),
    Region(BoardRegion),
    File(u8),
    Diagonal(PlanSquare),
    Piece(PlanActor),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrategicPlan {
    pub side: PlanColor,
    pub kind: PlanKind,
    pub actor: Option<PlanActor>,
    pub target: PlanTarget,
    pub method: PlanMethod,
    pub reason: PlanReason,
    /// Expected number of the player's own moves, clamped to 2..=6.
    pub horizon: u8,
    pub confidence: f32,
    pub progress: f32,
    pub cancel_conditions: Vec<CancelCondition>,
}

impl StrategicPlan {
    #[must_use]
    pub fn applicable(&self, position: &Position) -> bool {
        if let Some(actor) = &self.actor {
            let Some(piece) = position.board().piece_at(actor.square.engine()) else {
                return false;
            };
            if piece.color != self.side.engine() || piece_name(piece.kind) != actor.piece {
                return false;
            }
        }
        match &self.target {
            PlanTarget::Square(square) | PlanTarget::Diagonal(square) => {
                position.board().size().contains(square.engine())
            }
            PlanTarget::File(file) => *file < position.board().size().files(),
            PlanTarget::Piece(target) => position
                .board()
                .piece_at(target.square.engine())
                .is_some_and(|piece| piece_name(piece.kind) == target.piece),
            PlanTarget::Region(_) => true,
        }
    }

    #[must_use]
    pub fn measured_progress(&self, position: &Position) -> f32 {
        if !self.applicable(position) {
            return 0.0;
        }
        let side = self.side.engine();
        match self.kind {
            PlanKind::DevelopPiece | PlanKind::ImprovePiece => {
                self.actor.as_ref().map_or(0.0, |actor| {
                    centrality(
                        actor.square.engine(),
                        position.board().size().files(),
                        position.board().size().ranks(),
                    )
                })
            }
            PlanKind::KingAttack => king_zone_pressure(position, side),
            PlanKind::KingSafety => 1.0 - king_zone_pressure(position, side.opposite()),
            PlanKind::GainSpace => space_score(position, side),
            PlanKind::PawnBreak => pawn_advance_score(position, side),
            PlanKind::CreatePasser | PlanKind::AdvancePasser => passed_pawn_score(position, side),
            PlanKind::UseOpenFile => self.actor.as_ref().map_or(0.0, |actor| {
                centrality(
                    actor.square.engine(),
                    position.board().size().files(),
                    position.board().size().ranks(),
                )
            }),
            PlanKind::UseDiagonal | PlanKind::OccupyOutpost => {
                self.actor.as_ref().map_or(0.0, |actor| {
                    centrality(
                        actor.square.engine(),
                        position.board().size().files(),
                        position.board().size().ranks(),
                    )
                })
            }
            PlanKind::RestrictPiece => 1.0 - enemy_activity(position, side),
            PlanKind::Trade => material_simplicity(position),
            PlanKind::PreserveMaterial => material_ratio(position, side),
        }
        .clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn text_ru(&self) -> String {
        render(self, Language::Russian)
    }

    #[must_use]
    pub fn text_en(&self) -> String {
        render(self, Language::English)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanState {
    pub current: Option<StrategicPlan>,
    pub own_moves_elapsed: u8,
    pub consecutive_safety_vetoes: u8,
    pub last_cancel_reason: Option<CancelCondition>,
}

impl PlanState {
    pub fn choose(&mut self, position: &Position, mut proposed: StrategicPlan) -> &StrategicPlan {
        proposed.horizon = proposed.horizon.clamp(2, 6);
        proposed.confidence = proposed.confidence.clamp(0.0, 1.0);
        proposed.progress = proposed.measured_progress(position);
        let keep_current = self.current.as_ref().is_some_and(|current| {
            current.applicable(position)
                && self.own_moves_elapsed < current.horizon
                && current.measured_progress(position) < 0.85
                && proposed.confidence < current.confidence + 0.15
        });
        if !keep_current {
            self.current = Some(proposed);
            self.own_moves_elapsed = 0;
            self.consecutive_safety_vetoes = 0;
            self.last_cancel_reason = None;
        }
        self.current
            .as_ref()
            .expect("a proposed plan was installed")
    }

    pub fn record_own_move(&mut self) {
        self.own_moves_elapsed = self.own_moves_elapsed.saturating_add(1);
    }

    /// Advance the symbolic identity of the plan actor together with the move.
    ///
    /// Engine pieces intentionally have no persistent entity id. Remembering
    /// the actor's new square here prevents a multi-move plan from being
    /// discarded merely because its own first move succeeded.
    pub fn record_chosen_move(&mut self, chess_move: Move) {
        if let Some(plan) = &mut self.current
            && plan
                .actor
                .as_ref()
                .is_some_and(|actor| actor.square.engine() == chess_move.from)
        {
            plan.actor.as_mut().expect("actor was checked").square = chess_move.to.into();
        }
        self.record_own_move();
    }

    pub fn record_safety_veto(&mut self) {
        self.consecutive_safety_vetoes = self.consecutive_safety_vetoes.saturating_add(1);
        if self.consecutive_safety_vetoes >= 2 {
            self.cancel(CancelCondition::SafetyVetoTwice);
        }
    }

    pub fn record_safe_choice(&mut self) {
        self.consecutive_safety_vetoes = 0;
    }

    pub fn cancel(&mut self, reason: CancelCondition) {
        self.current = None;
        self.own_moves_elapsed = 0;
        self.consecutive_safety_vetoes = 0;
        self.last_cancel_reason = Some(reason);
    }
}

#[must_use]
pub fn generate_plan_candidates(position: &Position) -> Vec<StrategicPlan> {
    let side = position.side_to_move();
    let opponent = side.opposite();
    let size = position.board().size();
    let center = PlanTarget::Region(BoardRegion::Center);
    let own_king = position.board().king_square(side);
    let enemy_king = position.board().king_square(opponent);
    let mut candidates = Vec::new();

    if let Some(king) = enemy_king {
        candidates.push(plan(
            side,
            PlanKind::KingAttack,
            None,
            PlanTarget::Square(king.into()),
            PlanMethod::BuildPressure,
            PlanReason::WeakKingZone,
            4,
            0.58 + 0.25 * king_zone_pressure(position, side),
        ));
    }
    if let Some(king) = own_king {
        candidates.push(plan(
            side,
            PlanKind::KingSafety,
            Some(actor(PieceKind::King, king)),
            PlanTarget::Square(king.into()),
            PlanMethod::Reinforce,
            PlanReason::ExposedKing,
            3,
            0.55 + 0.3 * king_zone_pressure(position, opponent),
        ));
    }
    candidates.push(plan(
        side,
        PlanKind::GainSpace,
        None,
        center.clone(),
        PlanMethod::Centralize,
        PlanReason::SpaceDeficit,
        4,
        0.56,
    ));

    let own_pieces = position
        .board()
        .pieces()
        .filter(|(_, piece)| piece.color == side)
        .collect::<Vec<_>>();
    if let Some((square, piece)) = own_pieces.iter().copied().find(|(square, piece)| {
        !matches!(piece.kind, PieceKind::Pawn | PieceKind::King)
            && relative_rank(*square, side, size.ranks()) <= 3
    }) {
        candidates.push(plan(
            side,
            PlanKind::DevelopPiece,
            Some(actor(piece.kind, square)),
            center.clone(),
            PlanMethod::Activate,
            PlanReason::UndevelopedPiece,
            3,
            0.68,
        ));
    }
    if let Some((square, piece)) = own_pieces
        .iter()
        .copied()
        .filter(|(_, piece)| !matches!(piece.kind, PieceKind::Pawn | PieceKind::King))
        .min_by(|(left, _), (right, _)| {
            centrality(*left, size.files(), size.ranks()).total_cmp(&centrality(
                *right,
                size.files(),
                size.ranks(),
            ))
        })
    {
        candidates.push(plan(
            side,
            PlanKind::ImprovePiece,
            Some(actor(piece.kind, square)),
            center.clone(),
            PlanMethod::Centralize,
            PlanReason::PoorlyPlacedPiece,
            3,
            0.61,
        ));
    }
    if let Some((square, _)) = own_pieces
        .iter()
        .copied()
        .find(|(_, piece)| piece.kind == PieceKind::Pawn)
    {
        let target_rank = (i16::from(square.rank()) + i16::from(side.pawn_direction()))
            .clamp(0, i16::from(size.ranks() - 1)) as u8;
        candidates.push(plan(
            side,
            PlanKind::PawnBreak,
            Some(actor(PieceKind::Pawn, square)),
            PlanTarget::Square(Square::new(square.file(), target_rank).into()),
            PlanMethod::AdvancePawn,
            PlanReason::BlockedCenter,
            3,
            0.57,
        ));
        let passed = is_passed_pawn(position, square, side);
        candidates.push(plan(
            side,
            if passed {
                PlanKind::AdvancePasser
            } else {
                PlanKind::CreatePasser
            },
            Some(actor(PieceKind::Pawn, square)),
            PlanTarget::File(square.file()),
            PlanMethod::AdvancePawn,
            if passed {
                PlanReason::ExistingPassedPawn
            } else {
                PlanReason::PassedPawnPotential
            },
            5,
            if passed { 0.72 } else { 0.54 },
        ));
    }
    if let Some((square, piece)) = own_pieces.iter().copied().find(|(square, piece)| {
        matches!(
            piece.kind,
            PieceKind::Rook | PieceKind::Chancellor | PieceKind::Admiral
        ) && file_is_open(position, square.file())
    }) {
        candidates.push(plan(
            side,
            PlanKind::UseOpenFile,
            Some(actor(piece.kind, square)),
            PlanTarget::File(square.file()),
            PlanMethod::OpenLine,
            PlanReason::OpenLine,
            4,
            0.7,
        ));
    }
    if let Some((square, piece)) = own_pieces.iter().copied().find(|(_, piece)| {
        matches!(
            piece.kind,
            PieceKind::Bishop | PieceKind::Archbishop | PieceKind::Missionary
        )
    }) {
        candidates.push(plan(
            side,
            PlanKind::UseDiagonal,
            Some(actor(piece.kind, square)),
            PlanTarget::Diagonal(square.into()),
            PlanMethod::BuildPressure,
            PlanReason::OpenLine,
            4,
            0.59,
        ));
    }
    if let Some(chess_move) = position.legal_moves().into_iter().find(|chess_move| {
        position
            .board()
            .piece_at(chess_move.from)
            .is_some_and(|piece| piece.kind != PieceKind::Pawn && piece.kind != PieceKind::King)
            && position.board().piece_at(chess_move.to).is_none()
            && centrality(chess_move.to, size.files(), size.ranks()) >= 0.62
            && relative_rank(chess_move.to, side, size.ranks()) >= size.ranks() / 2
    }) {
        let piece = position
            .board()
            .piece_at(chess_move.from)
            .expect("outpost candidate starts on an occupied square");
        candidates.push(plan(
            side,
            PlanKind::OccupyOutpost,
            Some(actor(piece.kind, chess_move.from)),
            PlanTarget::Square(chess_move.to.into()),
            PlanMethod::OccupySquare,
            PlanReason::WeakSquare,
            3,
            0.6,
        ));
    }
    if let Some((square, piece)) = position
        .board()
        .pieces()
        .filter(|(_, piece)| piece.color == opponent && piece.kind != PieceKind::King)
        .max_by_key(|(_, piece)| piece_value(piece.kind))
    {
        candidates.push(plan(
            side,
            PlanKind::RestrictPiece,
            None,
            PlanTarget::Piece(actor(piece.kind, square)),
            PlanMethod::BuildPressure,
            PlanReason::ActiveEnemyPiece,
            3,
            0.58,
        ));
    }
    let balance = material_balance(position, side);
    candidates.push(plan(
        side,
        if balance >= 0 {
            PlanKind::Trade
        } else {
            PlanKind::PreserveMaterial
        },
        None,
        PlanTarget::Region(BoardRegion::Center),
        if balance >= 0 {
            PlanMethod::ExchangeDefender
        } else {
            PlanMethod::AvoidExchange
        },
        if balance >= 0 {
            PlanReason::MaterialAdvantage
        } else {
            PlanReason::MaterialDisadvantage
        },
        4,
        0.52 + (balance.unsigned_abs().min(800) as f32 / 8_000.0),
    ));
    for candidate in &mut candidates {
        candidate.progress = candidate.measured_progress(position);
        candidate.confidence = candidate.confidence.clamp(0.0, 1.0);
    }
    candidates
}

#[allow(clippy::too_many_arguments)]
fn plan(
    side: Color,
    kind: PlanKind,
    actor: Option<PlanActor>,
    target: PlanTarget,
    method: PlanMethod,
    reason: PlanReason,
    horizon: u8,
    confidence: f32,
) -> StrategicPlan {
    StrategicPlan {
        side: side.into(),
        kind,
        actor,
        target,
        method,
        reason,
        horizon,
        confidence,
        progress: 0.0,
        cancel_conditions: vec![
            CancelCondition::Completed,
            CancelCondition::ActorLost,
            CancelCondition::ForcedThreat,
            CancelCondition::SafetyVetoTwice,
        ],
    }
}

fn actor(kind: PieceKind, square: Square) -> PlanActor {
    PlanActor {
        piece: piece_name(kind).to_owned(),
        square: square.into(),
    }
}

fn relative_rank(square: Square, color: Color, ranks: u8) -> u8 {
    match color {
        Color::White => square.rank(),
        Color::Black => ranks - 1 - square.rank(),
    }
}

fn centrality(square: Square, files: u8, ranks: u8) -> f32 {
    let file_center = (files - 1) as f32 * 0.5;
    let rank_center = (ranks - 1) as f32 * 0.5;
    let distance =
        (square.file() as f32 - file_center).abs() + (square.rank() as f32 - rank_center).abs();
    1.0 - distance / (file_center + rank_center).max(1.0)
}

fn king_zone_pressure(position: &Position, attacking: Color) -> f32 {
    let Some(king) = position.board().king_square(attacking.opposite()) else {
        return 0.0;
    };
    let mut attacked = 0_u8;
    let mut total = 0_u8;
    for file_delta in -2..=2 {
        for rank_delta in -2..=2 {
            if let Some(square) = king.offset(file_delta, rank_delta)
                && position.board().size().contains(square)
            {
                total += 1;
                attacked += u8::from(position.is_square_attacked(square, attacking));
            }
        }
    }
    f32::from(attacked) / f32::from(total.max(1))
}

fn space_score(position: &Position, side: Color) -> f32 {
    let size = position.board().size();
    let occupied_advanced = position
        .board()
        .pieces()
        .filter(|(square, piece)| {
            piece.color == side && relative_rank(*square, side, size.ranks()) >= size.ranks() / 2
        })
        .count();
    occupied_advanced as f32 / 24.0
}

fn pawn_advance_score(position: &Position, side: Color) -> f32 {
    let ranks = position.board().size().ranks();
    let (total, count) = position
        .board()
        .pieces()
        .filter(|(_, piece)| piece.color == side && piece.kind == PieceKind::Pawn)
        .fold((0_u32, 0_u32), |(total, count), (square, _)| {
            (
                total + u32::from(relative_rank(square, side, ranks)),
                count + 1,
            )
        });
    total as f32 / (count.max(1) as f32 * (ranks - 1) as f32)
}

fn passed_pawn_score(position: &Position, side: Color) -> f32 {
    let ranks = position.board().size().ranks();
    position
        .board()
        .pieces()
        .filter(|(square, piece)| {
            piece.color == side
                && piece.kind == PieceKind::Pawn
                && is_passed_pawn(position, *square, side)
        })
        .map(|(square, _)| relative_rank(square, side, ranks) as f32 / (ranks - 1) as f32)
        .fold(0.0, f32::max)
}

fn is_passed_pawn(position: &Position, square: Square, side: Color) -> bool {
    position.board().pieces().all(|(enemy_square, piece)| {
        if piece.color == side || piece.kind != PieceKind::Pawn {
            return true;
        }
        let nearby_file = enemy_square.file().abs_diff(square.file()) <= 1;
        let ahead = match side {
            Color::White => enemy_square.rank() > square.rank(),
            Color::Black => enemy_square.rank() < square.rank(),
        };
        !(nearby_file && ahead)
    })
}

fn file_is_open(position: &Position, file: u8) -> bool {
    position
        .board()
        .pieces()
        .all(|(square, piece)| square.file() != file || piece.kind != PieceKind::Pawn)
}

fn material_balance(position: &Position, side: Color) -> i32 {
    position.board().pieces().fold(0, |score, (_, piece)| {
        let value = piece_value(piece.kind);
        if piece.color == side {
            score + value
        } else {
            score - value
        }
    })
}

fn material_ratio(position: &Position, side: Color) -> f32 {
    let own = position
        .board()
        .pieces()
        .filter(|(_, piece)| piece.color == side)
        .map(|(_, piece)| piece_value(piece.kind).max(0))
        .sum::<i32>();
    let enemy = position
        .board()
        .pieces()
        .filter(|(_, piece)| piece.color != side)
        .map(|(_, piece)| piece_value(piece.kind).max(0))
        .sum::<i32>();
    own as f32 / (own + enemy).max(1) as f32
}

fn material_simplicity(position: &Position) -> f32 {
    let pieces = position
        .board()
        .pieces()
        .filter(|(_, piece)| piece.kind != PieceKind::King)
        .count();
    1.0 - pieces as f32 / 126.0
}

fn enemy_activity(position: &Position, side: Color) -> f32 {
    let size = position.board().size();
    let (sum, count) = position
        .board()
        .pieces()
        .filter(|(_, piece)| piece.color != side && piece.kind != PieceKind::King)
        .fold((0.0, 0_u32), |(sum, count), (square, _)| {
            (
                sum + centrality(square, size.files(), size.ranks()),
                count + 1,
            )
        });
    sum / count.max(1) as f32
}

fn piece_name(kind: PieceKind) -> &'static str {
    match kind {
        PieceKind::Pawn => "pawn",
        PieceKind::Knight => "knight",
        PieceKind::Bishop => "bishop",
        PieceKind::Rook => "rook",
        PieceKind::Queen => "queen",
        PieceKind::King => "king",
        PieceKind::Archbishop => "archbishop",
        PieceKind::Chancellor => "chancellor",
        PieceKind::Cannon => "cannon",
        PieceKind::Elephant => "elephant",
        PieceKind::Camel => "camel",
        PieceKind::Giraffe => "giraffe",
        PieceKind::Archer => "archer",
        PieceKind::Machine => "machine",
        PieceKind::Amazon => "amazon",
        PieceKind::Lion => "lion",
        PieceKind::Buffalo => "buffalo",
        PieceKind::Centaur => "centaur",
        PieceKind::Admiral => "admiral",
        PieceKind::Missionary => "missionary",
        PieceKind::Eagle => "eagle",
        PieceKind::Rhinoceros => "rhinoceros",
        PieceKind::Prince => "prince",
        PieceKind::Sorceress => "sorceress",
        PieceKind::Duchess => "duchess",
        PieceKind::Troll => "troll",
    }
}

#[derive(Clone, Copy)]
enum Language {
    Russian,
    English,
}

fn render(plan: &StrategicPlan, language: Language) -> String {
    let kind = localized_kind(plan.kind, language);
    let method = localized_method(plan.method, language);
    let reason = localized_reason(plan.reason, language);
    let target = localized_target(&plan.target, language);
    let confidence = (plan.confidence * 100.0).round() as u8;
    match language {
        Language::Russian => format!(
            "Цель: {kind} {target}; средство: {method}; причина: {reason}. Горизонт: {} хода, уверенность: {confidence}%.",
            plan.horizon
        ),
        Language::English => format!(
            "Goal: {kind} {target}; method: {method}; reason: {reason}. Horizon: {} moves, confidence: {confidence}%.",
            plan.horizon
        ),
    }
}

fn localized_kind(kind: PlanKind, language: Language) -> &'static str {
    match (language, kind) {
        (Language::Russian, PlanKind::DevelopPiece) => "развить фигуру",
        (Language::Russian, PlanKind::ImprovePiece) => "улучшить фигуру",
        (Language::Russian, PlanKind::KingAttack) => "атаковать короля",
        (Language::Russian, PlanKind::KingSafety) => "укрепить короля",
        (Language::Russian, PlanKind::GainSpace) => "захватить пространство",
        (Language::Russian, PlanKind::PawnBreak) => "провести пешечный прорыв",
        (Language::Russian, PlanKind::CreatePasser) => "создать проходную",
        (Language::Russian, PlanKind::AdvancePasser) => "продвинуть проходную",
        (Language::Russian, PlanKind::UseOpenFile) => "использовать открытую линию",
        (Language::Russian, PlanKind::UseDiagonal) => "использовать диагональ",
        (Language::Russian, PlanKind::OccupyOutpost) => "занять форпост",
        (Language::Russian, PlanKind::RestrictPiece) => "ограничить фигуру",
        (Language::Russian, PlanKind::Trade) => "упростить позицию разменами",
        (Language::Russian, PlanKind::PreserveMaterial) => "сохранить активный материал",
        (Language::English, PlanKind::DevelopPiece) => "develop a piece",
        (Language::English, PlanKind::ImprovePiece) => "improve a piece",
        (Language::English, PlanKind::KingAttack) => "attack the king",
        (Language::English, PlanKind::KingSafety) => "improve king safety",
        (Language::English, PlanKind::GainSpace) => "gain space",
        (Language::English, PlanKind::PawnBreak) => "prepare a pawn break",
        (Language::English, PlanKind::CreatePasser) => "create a passed pawn",
        (Language::English, PlanKind::AdvancePasser) => "advance the passed pawn",
        (Language::English, PlanKind::UseOpenFile) => "use the open file",
        (Language::English, PlanKind::UseDiagonal) => "use the diagonal",
        (Language::English, PlanKind::OccupyOutpost) => "occupy an outpost",
        (Language::English, PlanKind::RestrictPiece) => "restrict an enemy piece",
        (Language::English, PlanKind::Trade) => "simplify with exchanges",
        (Language::English, PlanKind::PreserveMaterial) => "preserve active material",
    }
}

fn localized_method(method: PlanMethod, language: Language) -> &'static str {
    match (language, method) {
        (Language::Russian, PlanMethod::Activate) => "активизация",
        (Language::Russian, PlanMethod::Centralize) => "централизация",
        (Language::Russian, PlanMethod::BuildPressure) => "наращивание давления",
        (Language::Russian, PlanMethod::Reinforce) => "подвод защиты",
        (Language::Russian, PlanMethod::AdvancePawn) => "продвижение пешки",
        (Language::Russian, PlanMethod::OpenLine) => "вскрытие линии",
        (Language::Russian, PlanMethod::OccupySquare) => "занятие ключевого поля",
        (Language::Russian, PlanMethod::ExchangeDefender) => "размен защитника",
        (Language::Russian, PlanMethod::AvoidExchange) => "уклонение от разменов",
        (Language::English, PlanMethod::Activate) => "activation",
        (Language::English, PlanMethod::Centralize) => "centralization",
        (Language::English, PlanMethod::BuildPressure) => "build pressure",
        (Language::English, PlanMethod::Reinforce) => "bring defenders",
        (Language::English, PlanMethod::AdvancePawn) => "advance a pawn",
        (Language::English, PlanMethod::OpenLine) => "open a line",
        (Language::English, PlanMethod::OccupySquare) => "occupy a key square",
        (Language::English, PlanMethod::ExchangeDefender) => "exchange a defender",
        (Language::English, PlanMethod::AvoidExchange) => "avoid exchanges",
    }
}

fn localized_reason(reason: PlanReason, language: Language) -> &'static str {
    match (language, reason) {
        (Language::Russian, PlanReason::UndevelopedPiece) => "фигура ещё не развита",
        (Language::Russian, PlanReason::PoorlyPlacedPiece) => "фигура расположена неудачно",
        (Language::Russian, PlanReason::ExposedKing) => "король недостаточно защищён",
        (Language::Russian, PlanReason::WeakKingZone) => "в зоне короля есть слабые поля",
        (Language::Russian, PlanReason::SpaceDeficit) => "не хватает пространства",
        (Language::Russian, PlanReason::BlockedCenter) => "центр заблокирован",
        (Language::Russian, PlanReason::PassedPawnPotential) => "структура допускает проходную",
        (Language::Russian, PlanReason::ExistingPassedPawn) => "проходная уже создана",
        (Language::Russian, PlanReason::OpenLine) => "линия доступна для давления",
        (Language::Russian, PlanReason::WeakSquare) => "доступно устойчивое слабое поле",
        (Language::Russian, PlanReason::ActiveEnemyPiece) => "вражеская фигура слишком активна",
        (Language::Russian, PlanReason::MaterialAdvantage) => {
            "материальный перевес выгодно реализовать"
        }
        (Language::Russian, PlanReason::MaterialDisadvantage) => {
            "для контригры нужны активные фигуры"
        }
        (Language::English, PlanReason::UndevelopedPiece) => "a piece is still undeveloped",
        (Language::English, PlanReason::PoorlyPlacedPiece) => "a piece is poorly placed",
        (Language::English, PlanReason::ExposedKing) => "the king lacks protection",
        (Language::English, PlanReason::WeakKingZone) => "the king zone contains weak squares",
        (Language::English, PlanReason::SpaceDeficit) => "the position lacks space",
        (Language::English, PlanReason::BlockedCenter) => "the center is blocked",
        (Language::English, PlanReason::PassedPawnPotential) => "the structure can create a passer",
        (Language::English, PlanReason::ExistingPassedPawn) => "a passed pawn already exists",
        (Language::English, PlanReason::OpenLine) => "a line is available for pressure",
        (Language::English, PlanReason::WeakSquare) => "a stable weak square is available",
        (Language::English, PlanReason::ActiveEnemyPiece) => "an enemy piece is too active",
        (Language::English, PlanReason::MaterialAdvantage) => {
            "the material edge favors simplification"
        }
        (Language::English, PlanReason::MaterialDisadvantage) => {
            "active pieces are needed for counterplay"
        }
    }
}

fn localized_target(target: &PlanTarget, language: Language) -> String {
    match target {
        PlanTarget::Square(square) => format!("{}", square.engine()),
        PlanTarget::File(file) => format!(
            "{}-{}",
            char::from(b'a' + *file),
            if matches!(language, Language::Russian) {
                "линия"
            } else {
                "file"
            }
        ),
        PlanTarget::Diagonal(square) => format!("{} diagonal", square.engine()),
        PlanTarget::Piece(actor) => format!("{}@{}", actor.piece, actor.square.engine()),
        PlanTarget::Region(region) => match (language, region) {
            (Language::Russian, BoardRegion::QueenSide) => "на ферзевом фланге".to_owned(),
            (Language::Russian, BoardRegion::Center) => "в центре".to_owned(),
            (Language::Russian, BoardRegion::KingSide) => "на королевском фланге".to_owned(),
            (Language::Russian, BoardRegion::OwnCamp) => "в своём лагере".to_owned(),
            (Language::Russian, BoardRegion::EnemyCamp) => "в лагере соперника".to_owned(),
            (Language::English, BoardRegion::QueenSide) => "on the queenside".to_owned(),
            (Language::English, BoardRegion::Center) => "in the center".to_owned(),
            (Language::English, BoardRegion::KingSide) => "on the kingside".to_owned(),
            (Language::English, BoardRegion::OwnCamp) => "in the own camp".to_owned(),
            (Language::English, BoardRegion::EnemyCamp) => "in the enemy camp".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use capablanca_chess_plus::Variant;

    #[test]
    fn starting_position_has_applicable_bilingual_plans() {
        let position = Variant::TerachessII.starting_position();
        let plans = generate_plan_candidates(&position);
        assert!(plans.len() >= 6);
        assert!(plans.iter().all(|plan| plan.applicable(&position)));
        assert!(
            plans.iter().all(
                |plan| plan.text_ru().contains("Причина") || plan.text_ru().contains("причина")
            )
        );
        assert!(plans.iter().all(|plan| plan.text_en().contains("reason")));
    }

    #[test]
    fn plan_state_keeps_a_valid_plan_until_a_reason_to_change() {
        let position = Variant::TerachessII.starting_position();
        let plans = generate_plan_candidates(&position);
        let mut state = PlanState::default();
        let first = state.choose(&position, plans[0].clone()).clone();
        let mut weaker = plans[1].clone();
        weaker.confidence = first.confidence;
        assert_eq!(state.choose(&position, weaker), &first);
        state.record_safety_veto();
        state.record_safety_veto();
        assert!(state.current.is_none());
        assert_eq!(
            state.last_cancel_reason,
            Some(CancelCondition::SafetyVetoTwice)
        );
    }

    #[test]
    fn plan_json_round_trip_preserves_explanation_source() {
        let position = Variant::TerachessII.starting_position();
        let plan = generate_plan_candidates(&position).remove(0);
        let encoded = serde_json::to_string(&plan).unwrap();
        let decoded: StrategicPlan = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, plan);
        assert_eq!(decoded.text_ru(), plan.text_ru());
    }
}
