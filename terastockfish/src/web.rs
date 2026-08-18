use capablanca_chess_plus::{Position, Variant};
use wasm_bindgen::prelude::*;

use crate::{SearchLimits, SearchOptions, Searcher, mate_distance};

/// Single-threaded TeraStockfish instance intended to live inside a browser
/// Web Worker. Keeping the searcher alive preserves its transposition table
/// and move-ordering history between moves without blocking Bevy's main thread.
#[wasm_bindgen]
pub struct WebEngine {
    searcher: Searcher,
}

#[wasm_bindgen]
impl WebEngine {
    #[wasm_bindgen(constructor)]
    #[must_use]
    pub fn new(hash_megabytes: usize) -> Self {
        Self {
            searcher: Searcher::new(SearchOptions {
                hash_megabytes: hash_megabytes.clamp(1, 256),
                threads: 1,
            }),
        }
    }

    pub fn clear_hash(&self) {
        self.searcher.clear_hash();
    }

    /// Analyze one Terachess II FEN with an explicit node budget. The compact
    /// UCI-shaped response lets the existing Bevy AI parser consume the native
    /// and browser engines identically.
    pub fn analyze(&mut self, fen: &str, nodes: u64) -> Result<String, JsValue> {
        let position = Position::from_fen(Variant::TerachessII.rules(), fen)
            .map_err(|error| JsValue::from_str(&error.to_string()))?;
        let result = self.searcher.analyze(
            &position,
            SearchLimits {
                max_depth: 191,
                max_nodes: Some(nodes.max(1)),
                ..SearchLimits::default()
            },
        );
        let score = mate_distance(result.score).map_or_else(
            || format!("cp {}", result.score),
            |distance| format!("mate {distance}"),
        );
        let best_move = result
            .best_move
            .map_or_else(|| "0000".to_owned(), |chess_move| chess_move.to_uci());
        Ok(format!(
            "info depth {} score {} nodes {}\nbestmove {}",
            result.completed_depth, score, result.nodes, best_move
        ))
    }
}
