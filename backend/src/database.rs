use capablanca_chess_plus::{Color as EngineSide, Game, Variant as EngineVariant};
use multiplayer_protocol::{
    MAX_GAME_ID_LEN, MAX_MOVE_LEN, Side, SidePreference, SyncReason, Variant,
};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use thiserror::Error;
use uuid::Uuid;

const GAME_ID_LEN: usize = 10;
const GAME_ID_ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
const CREATE_RETRIES: usize = 8;

#[derive(Clone)]
pub(crate) struct Repository {
    pool: PgPool,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthenticatedPlayer {
    pub(crate) game_id: String,
    pub(crate) side: Side,
    pub(crate) player_token: String,
}

#[derive(Debug)]
pub(crate) struct ReadyGame {
    pub(crate) auth: AuthenticatedPlayer,
    pub(crate) variant: Variant,
    pub(crate) revision: u64,
    pub(crate) history: Vec<String>,
}

#[derive(Debug)]
pub(crate) enum MoveResult {
    Accepted {
        revision: u64,
        uci: String,
    },
    Sync {
        revision: u64,
        history: Vec<String>,
        reason: SyncReason,
    },
}

#[derive(Debug, Error)]
pub(crate) enum RepositoryError {
    #[error("{message}")]
    Client { code: &'static str, message: String },
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error("stored game {game_id} is inconsistent: {detail}")]
    Corrupt { game_id: String, detail: String },
}

#[derive(FromRow)]
struct RoomRow {
    game_id: String,
    variant: String,
    white_token_hash: Option<Vec<u8>>,
    black_token_hash: Option<Vec<u8>>,
    revision: i64,
}

impl Repository {
    pub(crate) const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) async fn create_game(
        &self,
        variant: Variant,
        preference: SidePreference,
    ) -> Result<ReadyGame, RepositoryError> {
        let side = resolve_side(preference);
        let player_token = generate_token();
        let token_hash = hash_token(&player_token);
        let variant_name = variant_name(variant);

        for _ in 0..CREATE_RETRIES {
            let game_id = generate_game_id();
            let (white_hash, black_hash) = match side {
                Side::White => (Some(token_hash.clone()), None),
                Side::Black => (None, Some(token_hash.clone())),
            };
            let result = sqlx::query(
                "INSERT INTO games
                    (game_id, variant, white_token_hash, black_token_hash, revision)
                 VALUES ($1, $2, $3, $4, 0)
                 ON CONFLICT (game_id) DO NOTHING",
            )
            .bind(&game_id)
            .bind(variant_name)
            .bind(white_hash)
            .bind(black_hash)
            .execute(&self.pool)
            .await?;
            if result.rows_affected() == 1 {
                return Ok(ReadyGame {
                    auth: AuthenticatedPlayer {
                        game_id,
                        side,
                        player_token,
                    },
                    variant,
                    revision: 0,
                    history: Vec::new(),
                });
            }
        }

        Err(client_error(
            "game_id_exhausted",
            "Could not allocate a game id. Please try again.",
        ))
    }

    pub(crate) async fn join_game(
        &self,
        game_id: &str,
        player_token: Option<&str>,
    ) -> Result<ReadyGame, RepositoryError> {
        let game_id = normalize_game_id(game_id)?;
        let mut transaction = self.pool.begin().await?;
        let mut room = sqlx::query_as::<_, RoomRow>(
            "SELECT game_id, variant, white_token_hash, black_token_hash, revision
             FROM games WHERE game_id = $1 FOR UPDATE",
        )
        .bind(&game_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or_else(|| client_error("game_not_found", "No game exists with that id."))?;

        let (side, player_token) = if let Some(token) = player_token {
            (authenticate(&room, token)?, token.to_owned())
        } else {
            let side = match (
                room.white_token_hash.is_none(),
                room.black_token_hash.is_none(),
            ) {
                (true, false) => Side::White,
                (false, true) => Side::Black,
                (true, true) => {
                    return Err(RepositoryError::Corrupt {
                        game_id,
                        detail: "neither player slot is claimed".to_owned(),
                    });
                }
                (false, false) => {
                    return Err(client_error(
                        "game_full",
                        "Both colors are already claimed. A saved player token is required.",
                    ));
                }
            };
            let token = generate_token();
            let token_hash = hash_token(&token);
            match side {
                Side::White => {
                    sqlx::query("UPDATE games SET white_token_hash = $2 WHERE game_id = $1")
                        .bind(&room.game_id)
                        .bind(&token_hash)
                        .execute(&mut *transaction)
                        .await?;
                    room.white_token_hash = Some(token_hash);
                }
                Side::Black => {
                    sqlx::query("UPDATE games SET black_token_hash = $2 WHERE game_id = $1")
                        .bind(&room.game_id)
                        .bind(&token_hash)
                        .execute(&mut *transaction)
                        .await?;
                    room.black_token_hash = Some(token_hash);
                }
            }
            (side, token)
        };

        let history = fetch_history(&mut transaction, &room.game_id).await?;
        verify_revision(&room, &history)?;
        let variant = parse_variant(&room.variant, &room.game_id)?;
        let revision = revision_to_u64(room.revision, &room.game_id)?;
        transaction.commit().await?;
        Ok(ReadyGame {
            auth: AuthenticatedPlayer {
                game_id: room.game_id,
                side,
                player_token,
            },
            variant,
            revision,
            history,
        })
    }

    pub(crate) async fn play_move(
        &self,
        auth: &AuthenticatedPlayer,
        client_revision: u64,
        uci: &str,
    ) -> Result<MoveResult, RepositoryError> {
        if uci.is_empty() || uci.len() > MAX_MOVE_LEN || !uci.is_ascii() {
            return Err(client_error("invalid_move", "Malformed UCI move."));
        }

        let mut transaction = self.pool.begin().await?;
        let room = locked_room(&mut transaction, &auth.game_id).await?;
        authenticate_as(&room, auth.side, &auth.player_token)?;
        let history = fetch_history(&mut transaction, &room.game_id).await?;
        verify_revision(&room, &history)?;
        let revision = revision_to_u64(room.revision, &room.game_id)?;
        if revision != client_revision {
            transaction.commit().await?;
            return Ok(MoveResult::Sync {
                revision,
                history,
                reason: SyncReason::RevisionMismatch,
            });
        }

        let variant = parse_variant(&room.variant, &room.game_id)?;
        let mut game = replay_game(variant, &history, &room.game_id)?;
        if side_from_engine(game.position().side_to_move()) != auth.side {
            return Err(client_error(
                "not_your_turn",
                "It is the other player's turn.",
            ));
        }
        let chess_move = game
            .position()
            .parse_uci_move(&uci.to_ascii_lowercase())
            .map_err(|_| client_error("illegal_move", "The move is not legal in this position."))?;
        game.play(chess_move)
            .map_err(|_| client_error("illegal_move", "The move is not legal in this position."))?;
        let accepted_uci = chess_move.to_uci();
        let next_revision = revision
            .checked_add(1)
            .ok_or_else(|| client_error("revision_overflow", "Game revision overflow."))?;
        let next_revision_i64 = i64::try_from(next_revision)
            .map_err(|_| client_error("revision_overflow", "Game revision overflow."))?;

        sqlx::query("INSERT INTO game_moves (game_id, revision, uci) VALUES ($1, $2, $3)")
            .bind(&room.game_id)
            .bind(next_revision_i64)
            .bind(&accepted_uci)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("UPDATE games SET revision = $2, updated_at = now() WHERE game_id = $1")
            .bind(&room.game_id)
            .bind(next_revision_i64)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(MoveResult::Accepted {
            revision: next_revision,
            uci: accepted_uci,
        })
    }

    pub(crate) async fn resync(
        &self,
        auth: &AuthenticatedPlayer,
    ) -> Result<(u64, Vec<String>), RepositoryError> {
        let mut transaction = self.pool.begin().await?;
        let room = locked_room(&mut transaction, &auth.game_id).await?;
        authenticate_as(&room, auth.side, &auth.player_token)?;
        let history = fetch_history(&mut transaction, &room.game_id).await?;
        verify_revision(&room, &history)?;
        let revision = revision_to_u64(room.revision, &room.game_id)?;
        transaction.commit().await?;
        Ok((revision, history))
    }
}

async fn locked_room(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    game_id: &str,
) -> Result<RoomRow, RepositoryError> {
    sqlx::query_as::<_, RoomRow>(
        "SELECT game_id, variant, white_token_hash, black_token_hash, revision
         FROM games WHERE game_id = $1 FOR UPDATE",
    )
    .bind(game_id)
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| client_error("game_not_found", "The game no longer exists."))
}

async fn fetch_history(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    game_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT uci FROM game_moves WHERE game_id = $1 ORDER BY revision ASC",
    )
    .bind(game_id)
    .fetch_all(&mut **transaction)
    .await
}

fn verify_revision(room: &RoomRow, history: &[String]) -> Result<(), RepositoryError> {
    let revision = revision_to_u64(room.revision, &room.game_id)?;
    if usize::try_from(revision).ok() != Some(history.len()) {
        return Err(RepositoryError::Corrupt {
            game_id: room.game_id.clone(),
            detail: format!(
                "revision {revision} does not match {} stored moves",
                history.len()
            ),
        });
    }
    Ok(())
}

fn replay_game(
    variant: Variant,
    history: &[String],
    game_id: &str,
) -> Result<Game, RepositoryError> {
    let mut game = Game::new(engine_variant(variant).starting_position());
    for (index, uci) in history.iter().enumerate() {
        let chess_move =
            game.position()
                .parse_uci_move(uci)
                .map_err(|error| RepositoryError::Corrupt {
                    game_id: game_id.to_owned(),
                    detail: format!("move {} ({uci}) cannot be parsed: {error}", index + 1),
                })?;
        game.play(chess_move)
            .map_err(|error| RepositoryError::Corrupt {
                game_id: game_id.to_owned(),
                detail: format!("move {} ({uci}) cannot be replayed: {error}", index + 1),
            })?;
    }
    Ok(game)
}

fn authenticate(room: &RoomRow, token: &str) -> Result<Side, RepositoryError> {
    let hash = hash_token(token);
    match (
        room.white_token_hash.as_deref() == Some(hash.as_slice()),
        room.black_token_hash.as_deref() == Some(hash.as_slice()),
    ) {
        (true, false) => Ok(Side::White),
        (false, true) => Ok(Side::Black),
        _ => Err(client_error(
            "invalid_player_token",
            "The saved player token is invalid for this game.",
        )),
    }
}

fn authenticate_as(room: &RoomRow, side: Side, token: &str) -> Result<(), RepositoryError> {
    if authenticate(room, token)? == side {
        Ok(())
    } else {
        Err(client_error(
            "invalid_player_token",
            "The token does not own this color.",
        ))
    }
}

fn normalize_game_id(value: &str) -> Result<String, RepositoryError> {
    let value = value.trim().to_ascii_uppercase();
    if value.is_empty()
        || value.len() > MAX_GAME_ID_LEN
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(client_error("invalid_game_id", "Malformed game id."));
    }
    Ok(value)
}

fn generate_game_id() -> String {
    let random = Uuid::new_v4();
    random
        .as_bytes()
        .iter()
        .take(GAME_ID_LEN)
        .map(|byte| GAME_ID_ALPHABET[usize::from(*byte) % GAME_ID_ALPHABET.len()] as char)
        .collect()
}

fn generate_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn hash_token(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

fn resolve_side(preference: SidePreference) -> Side {
    match preference {
        SidePreference::White => Side::White,
        SidePreference::Black => Side::Black,
        SidePreference::Random if Uuid::new_v4().as_bytes()[0] & 1 == 0 => Side::White,
        SidePreference::Random => Side::Black,
    }
}

fn variant_name(variant: Variant) -> &'static str {
    match variant {
        Variant::Capablanca => "capablanca",
        Variant::Gothic => "gothic",
        Variant::Embassy => "embassy",
        Variant::Schoolbook => "schoolbook",
        Variant::Bird => "bird",
        Variant::Carrera => "carrera",
        Variant::Grand => "grand",
    }
}

fn parse_variant(value: &str, game_id: &str) -> Result<Variant, RepositoryError> {
    match value {
        "capablanca" => Ok(Variant::Capablanca),
        "gothic" => Ok(Variant::Gothic),
        "embassy" => Ok(Variant::Embassy),
        "schoolbook" => Ok(Variant::Schoolbook),
        "bird" => Ok(Variant::Bird),
        "carrera" => Ok(Variant::Carrera),
        "grand" => Ok(Variant::Grand),
        _ => Err(RepositoryError::Corrupt {
            game_id: game_id.to_owned(),
            detail: format!("unknown variant {value:?}"),
        }),
    }
}

fn engine_variant(variant: Variant) -> EngineVariant {
    match variant {
        Variant::Capablanca => EngineVariant::Capablanca,
        Variant::Gothic => EngineVariant::Gothic,
        Variant::Embassy => EngineVariant::Embassy,
        Variant::Schoolbook => EngineVariant::Schoolbook,
        Variant::Bird => EngineVariant::Bird,
        Variant::Carrera => EngineVariant::Carrera,
        Variant::Grand => EngineVariant::Grand,
    }
}

fn side_from_engine(side: EngineSide) -> Side {
    match side {
        EngineSide::White => Side::White,
        EngineSide::Black => Side::Black,
    }
}

fn revision_to_u64(revision: i64, game_id: &str) -> Result<u64, RepositoryError> {
    u64::try_from(revision).map_err(|_| RepositoryError::Corrupt {
        game_id: game_id.to_owned(),
        detail: format!("negative revision {revision}"),
    })
}

fn client_error(code: &'static str, message: impl Into<String>) -> RepositoryError {
    RepositoryError::Client {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_ids_are_short_unambiguous_codes() {
        for _ in 0..32 {
            let id = generate_game_id();
            assert_eq!(id.len(), GAME_ID_LEN);
            assert!(id.bytes().all(|byte| GAME_ID_ALPHABET.contains(&byte)));
        }
    }

    #[test]
    fn token_hash_does_not_reveal_the_token() {
        let token = generate_token();
        let hash = hash_token(&token);
        assert_eq!(hash.len(), 32);
        assert_ne!(hash, token.as_bytes());
    }

    #[test]
    fn stored_history_is_revalidated_by_the_shared_engine() {
        let game = replay_game(
            Variant::Gothic,
            &["a2a3".to_owned(), "a7a6".to_owned()],
            "TESTROOM",
        )
        .unwrap();
        assert_eq!(game.position().side_to_move(), EngineSide::White);

        let error = replay_game(Variant::Gothic, &["a2a5".to_owned()], "TESTROOM").unwrap_err();
        assert!(matches!(error, RepositoryError::Corrupt { .. }));
    }
}
