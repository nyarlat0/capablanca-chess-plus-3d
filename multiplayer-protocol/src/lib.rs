use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_GAME_ID_LEN: usize = 12;
pub const MAX_MOVE_LEN: usize = 8;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    White,
    Black,
}

impl Side {
    #[must_use]
    pub const fn opposite(self) -> Self {
        match self {
            Self::White => Self::Black,
            Self::Black => Self::White,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SidePreference {
    Random,
    White,
    Black,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    Capablanca,
    Gothic,
    Embassy,
    Schoolbook,
    Bird,
    Carrera,
    Grand,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    CreateGame {
        protocol: u16,
        variant: Variant,
        side: SidePreference,
    },
    JoinGame {
        protocol: u16,
        game_id: String,
        player_token: Option<String>,
    },
    PlayMove {
        revision: u64,
        uci: String,
    },
    Resync {
        revision: u64,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncReason {
    Requested,
    RevisionMismatch,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    GameReady {
        created: bool,
        game_id: String,
        player_token: String,
        side: Side,
        variant: Variant,
        revision: u64,
        history: Vec<String>,
        opponent_connected: bool,
    },
    MoveAccepted {
        revision: u64,
        uci: String,
    },
    OpponentConnection {
        connected: bool,
    },
    Sync {
        revision: u64,
        history: Vec<String>,
        reason: SyncReason,
    },
    Error {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_payload_contains_only_revision_and_move() {
        let value = serde_json::to_value(ClientMessage::PlayMove {
            revision: 7,
            uci: "a2a4".to_owned(),
        })
        .unwrap();
        assert_eq!(value["type"], "play_move");
        assert_eq!(value["revision"], 7);
        assert_eq!(value["uci"], "a2a4");
        assert_eq!(value.as_object().unwrap().len(), 3);
    }
}
