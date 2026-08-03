use actix_web::{HttpRequest, HttpResponse, web};
use actix_ws::{AggregatedMessage, Session};
use multiplayer_protocol::{ClientMessage, PROTOCOL_VERSION, ServerMessage, SyncReason};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::{
    database::{AuthenticatedPlayer, MoveResult, Repository, RepositoryError},
    hub::{ConnectionHub, Outbound},
};

const MAX_FRAME_BYTES: usize = 16 * 1024;

#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) repository: Repository,
    pub(crate) hub: ConnectionHub,
}

pub(crate) async fn websocket(
    request: HttpRequest,
    body: web::Payload,
    state: web::Data<AppState>,
) -> actix_web::Result<HttpResponse> {
    let (response, session, stream) = actix_ws::handle(&request, body)?;
    actix_web::rt::spawn(connection_task(
        session,
        stream
            .max_frame_size(MAX_FRAME_BYTES)
            .aggregate_continuations(),
        state.get_ref().clone(),
    ));
    Ok(response)
}

async fn connection_task(
    mut session: Session,
    mut stream: actix_ws::AggregatedMessageStream,
    state: AppState,
) {
    let connection_id = Uuid::new_v4();
    let (outbound_sender, mut outbound_receiver) = mpsc::unbounded_channel();
    let mut authenticated: Option<AuthenticatedPlayer> = None;

    loop {
        tokio::select! {
            outbound = outbound_receiver.recv() => match outbound {
                Some(Outbound::Message(message)) => {
                    if send_message(&mut session, &message).await.is_err() {
                        break;
                    }
                }
                Some(Outbound::Close) | None => break,
            },
            incoming = stream.recv() => match incoming {
                Some(Ok(AggregatedMessage::Text(text))) => {
                    match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(message) => handle_client_message(
                            &state,
                            connection_id,
                            &outbound_sender,
                            &mut authenticated,
                            message,
                        ).await,
                        Err(_) => send_error(
                            &outbound_sender,
                            "invalid_json",
                            "The WebSocket message is not valid protocol JSON.",
                        ),
                    }
                }
                Some(Ok(AggregatedMessage::Ping(bytes))) => {
                    if session.pong(&bytes).await.is_err() {
                        break;
                    }
                }
                Some(Ok(AggregatedMessage::Pong(_))) => {}
                Some(Ok(AggregatedMessage::Close(_))) | None => break,
                Some(Ok(AggregatedMessage::Binary(_))) => send_error(
                    &outbound_sender,
                    "binary_unsupported",
                    "Only JSON text messages are supported.",
                ),
                Some(Err(error)) => {
                    tracing::debug!(%connection_id, %error, "websocket protocol error");
                    break;
                }
            }
        }
    }

    if let Some(auth) = authenticated {
        state
            .hub
            .unregister(&auth.game_id, auth.side, connection_id)
            .await;
    }
    let _ = session.close(None).await;
}

async fn handle_client_message(
    state: &AppState,
    connection_id: Uuid,
    sender: &mpsc::UnboundedSender<Outbound>,
    authenticated: &mut Option<AuthenticatedPlayer>,
    message: ClientMessage,
) {
    match message {
        ClientMessage::CreateGame {
            protocol,
            variant,
            side,
        } => {
            if !check_handshake(protocol, authenticated, sender) {
                return;
            }
            match state.repository.create_game(variant, side).await {
                Ok(ready) => {
                    let opponent_connected = state
                        .hub
                        .register(
                            &ready.auth.game_id,
                            ready.auth.side,
                            connection_id,
                            sender.clone(),
                        )
                        .await;
                    send_local(
                        sender,
                        ServerMessage::GameReady {
                            created: true,
                            game_id: ready.auth.game_id.clone(),
                            player_token: ready.auth.player_token.clone(),
                            side: ready.auth.side,
                            variant: ready.variant,
                            revision: ready.revision,
                            history: ready.history,
                            opponent_connected,
                        },
                    );
                    *authenticated = Some(ready.auth);
                }
                Err(error) => send_repository_error(sender, error),
            }
        }
        ClientMessage::JoinGame {
            protocol,
            game_id,
            player_token,
        } => {
            if !check_handshake(protocol, authenticated, sender) {
                return;
            }
            match state
                .repository
                .join_game(&game_id, player_token.as_deref())
                .await
            {
                Ok(ready) => {
                    let opponent_connected = state
                        .hub
                        .register(
                            &ready.auth.game_id,
                            ready.auth.side,
                            connection_id,
                            sender.clone(),
                        )
                        .await;
                    state
                        .hub
                        .notify_opponent(
                            &ready.auth.game_id,
                            ready.auth.side,
                            ServerMessage::OpponentConnection { connected: true },
                        )
                        .await;
                    send_local(
                        sender,
                        ServerMessage::GameReady {
                            created: false,
                            game_id: ready.auth.game_id.clone(),
                            player_token: ready.auth.player_token.clone(),
                            side: ready.auth.side,
                            variant: ready.variant,
                            revision: ready.revision,
                            history: ready.history,
                            opponent_connected,
                        },
                    );
                    *authenticated = Some(ready.auth);
                }
                Err(error) => send_repository_error(sender, error),
            }
        }
        ClientMessage::PlayMove { revision, uci } => {
            let Some(auth) = authenticated.as_ref() else {
                send_error(sender, "not_authenticated", "Create or join a game first.");
                return;
            };
            match state.repository.play_move(auth, revision, &uci).await {
                Ok(MoveResult::Accepted { revision, uci }) => {
                    state
                        .hub
                        .broadcast(&auth.game_id, ServerMessage::MoveAccepted { revision, uci })
                        .await;
                }
                Ok(MoveResult::Sync {
                    revision,
                    history,
                    reason,
                }) => send_local(
                    sender,
                    ServerMessage::Sync {
                        revision,
                        history,
                        reason,
                    },
                ),
                Err(error) => send_repository_error(sender, error),
            }
        }
        ClientMessage::Resync { revision: _ } => {
            let Some(auth) = authenticated.as_ref() else {
                send_error(sender, "not_authenticated", "Create or join a game first.");
                return;
            };
            match state.repository.resync(auth).await {
                Ok((revision, history)) => send_local(
                    sender,
                    ServerMessage::Sync {
                        revision,
                        history,
                        reason: SyncReason::Requested,
                    },
                ),
                Err(error) => send_repository_error(sender, error),
            }
        }
    }
}

fn check_handshake(
    protocol: u16,
    authenticated: &Option<AuthenticatedPlayer>,
    sender: &mpsc::UnboundedSender<Outbound>,
) -> bool {
    if protocol != PROTOCOL_VERSION {
        send_error(
            sender,
            "protocol_mismatch",
            "Client and server protocol versions do not match.",
        );
        return false;
    }
    if authenticated.is_some() {
        send_error(
            sender,
            "already_authenticated",
            "This WebSocket already belongs to a game.",
        );
        return false;
    }
    true
}

fn send_repository_error(sender: &mpsc::UnboundedSender<Outbound>, error: RepositoryError) {
    match error {
        RepositoryError::Client { code, message } => send_error(sender, code, &message),
        internal => {
            tracing::error!(error = %internal, "multiplayer repository failure");
            send_error(
                sender,
                "server_error",
                "The server could not process the request.",
            );
        }
    }
}

fn send_error(sender: &mpsc::UnboundedSender<Outbound>, code: &str, message: &str) {
    send_local(
        sender,
        ServerMessage::Error {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    );
}

fn send_local(sender: &mpsc::UnboundedSender<Outbound>, message: ServerMessage) {
    let _ = sender.send(Outbound::Message(message));
}

async fn send_message(
    session: &mut Session,
    message: &ServerMessage,
) -> Result<(), actix_ws::Closed> {
    let json = serde_json::to_string(message).expect("server protocol messages are serializable");
    session.text(json).await
}
