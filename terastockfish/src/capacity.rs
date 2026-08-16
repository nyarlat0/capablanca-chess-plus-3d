use capablanca_chess_plus::Square;

pub const MAX_BOARD_FILES: usize = 18;
pub const MAX_BOARD_RANKS: usize = 18;
pub const MAX_BOARD_SQUARES: usize = MAX_BOARD_FILES * MAX_BOARD_RANKS;

#[must_use]
pub(crate) const fn square_index(square: Square) -> usize {
    square.rank() as usize * MAX_BOARD_FILES + square.file() as usize
}
