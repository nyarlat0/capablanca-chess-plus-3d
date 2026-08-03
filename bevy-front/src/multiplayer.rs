use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCamera;
use capablanca_chess_plus::{Color as EngineSide, Variant as EngineVariant};
use ewebsock::{Options, WsEvent, WsMessage, WsReceiver, WsSender};
use multiplayer_protocol::{
    ClientMessage, PROTOCOL_VERSION, ServerMessage, Side, SidePreference, Variant,
};

use crate::{
    app::FrontendSet,
    game::{ChessMatch, Controller, MoveRequest, apply_move, restart_match},
    menu::{GameMenuState, GameMode},
    pieces::PieceAnimationState,
    scene::{CameraAutoTurn, start_camera_turn},
};

const RECONNECT_SECONDS: f32 = 2.0;
const MAX_INCOMING_FRAME_BYTES: usize = 1024 * 1024;
#[cfg(target_arch = "wasm32")]
const TOKEN_STORAGE_PREFIX: &str = "capablanca_chess.player_token.";

pub(crate) struct MultiplayerPlugin;

impl Plugin for MultiplayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MultiplayerState>()
            .init_non_send::<MultiplayerClient>()
            .add_message::<MultiplayerCommand>()
            .add_message::<MultiplayerRoomReady>()
            .add_systems(
                Update,
                (handle_multiplayer_commands, poll_multiplayer)
                    .chain()
                    .in_set(FrontendSet::Multiplayer),
            )
            .add_systems(
                Update,
                dispatch_move_requests.in_set(FrontendSet::MoveDispatch),
            );
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum MultiplayerPhase {
    #[default]
    Offline,
    Connecting,
    Active,
    Reconnecting,
    Error,
}

#[derive(Resource)]
pub(crate) struct MultiplayerState {
    pub(crate) phase: MultiplayerPhase,
    pub(crate) game_id: Option<String>,
    pub(crate) side: Option<EngineSide>,
    pub(crate) revision: u64,
    accepted_revision: u64,
    pub(crate) opponent_connected: bool,
    pub(crate) pending_move: Option<String>,
    pub(crate) status: String,
    player_token: Option<String>,
    accepted_moves: VecDeque<(u64, String)>,
}

impl Default for MultiplayerState {
    fn default() -> Self {
        Self {
            phase: MultiplayerPhase::Offline,
            game_id: None,
            side: None,
            revision: 0,
            accepted_revision: 0,
            opponent_connected: false,
            pending_move: None,
            status: "Enter a game id to join, or leave it empty to create one.".to_owned(),
            player_token: None,
            accepted_moves: VecDeque::new(),
        }
    }
}

#[derive(Message, Clone, Debug)]
pub(crate) enum MultiplayerCommand {
    Create {
        variant: EngineVariant,
        side: crate::menu::SideChoice,
    },
    Join {
        game_id: String,
    },
    Disconnect,
}

#[derive(Message, Clone, Copy, Debug)]
pub(crate) struct MultiplayerRoomReady {
    pub(crate) created: bool,
}

#[derive(Default)]
struct MultiplayerClient {
    sender: Option<WsSender>,
    receiver: Option<WsReceiver>,
    handshake: Option<ClientMessage>,
    reconnect_timer: Option<Timer>,
    tokens: HashMap<String, String>,
}

impl MultiplayerClient {
    fn close(&mut self) {
        if let Some(sender) = &mut self.sender {
            sender.close();
        }
        self.sender = None;
        self.receiver = None;
        self.handshake = None;
        self.reconnect_timer = None;
    }

    fn connect(&mut self, handshake: ClientMessage) -> Result<(), String> {
        self.close();
        let options = Options {
            max_incoming_frame_size: MAX_INCOMING_FRAME_BYTES,
            ..default()
        };
        let (sender, receiver) = ewebsock::connect(websocket_url(), options)
            .map_err(|error| format!("Could not open WebSocket: {error}"))?;
        self.sender = Some(sender);
        self.receiver = Some(receiver);
        self.handshake = Some(handshake);
        Ok(())
    }

    fn send(&mut self, message: &ClientMessage) -> Result<(), String> {
        let sender = self
            .sender
            .as_mut()
            .ok_or_else(|| "The multiplayer socket is not connected.".to_owned())?;
        let json = serde_json::to_string(message)
            .map_err(|error| format!("Could not serialize multiplayer message: {error}"))?;
        sender.send(WsMessage::Text(json));
        Ok(())
    }

    fn save_token(&mut self, game_id: &str, token: &str) {
        self.tokens.insert(game_id.to_owned(), token.to_owned());
        save_browser_token(game_id, token);
    }

    fn load_token(&self, game_id: &str) -> Option<String> {
        self.tokens
            .get(game_id)
            .cloned()
            .or_else(|| load_browser_token(game_id))
    }
}

fn handle_multiplayer_commands(
    mut commands: MessageReader<MultiplayerCommand>,
    mut client: NonSendMut<MultiplayerClient>,
    mut state: ResMut<MultiplayerState>,
) {
    for command in commands.read() {
        match command {
            MultiplayerCommand::Create { variant, side } => {
                *state = MultiplayerState {
                    phase: MultiplayerPhase::Connecting,
                    status: "Creating game…".to_owned(),
                    ..default()
                };
                let handshake = ClientMessage::CreateGame {
                    protocol: PROTOCOL_VERSION,
                    variant: variant_to_wire(*variant),
                    side: side_to_preference(*side),
                };
                if let Err(error) = client.connect(handshake) {
                    state.phase = MultiplayerPhase::Error;
                    state.status = error;
                }
            }
            MultiplayerCommand::Join { game_id } => {
                let game_id = game_id.trim().to_ascii_uppercase();
                *state = MultiplayerState {
                    phase: MultiplayerPhase::Connecting,
                    game_id: Some(game_id.clone()),
                    status: format!("Joining {game_id}…"),
                    ..default()
                };
                let handshake = ClientMessage::JoinGame {
                    protocol: PROTOCOL_VERSION,
                    game_id: game_id.clone(),
                    player_token: client.load_token(&game_id),
                };
                if let Err(error) = client.connect(handshake) {
                    state.phase = MultiplayerPhase::Error;
                    state.status = error;
                }
            }
            MultiplayerCommand::Disconnect => {
                client.close();
                *state = MultiplayerState::default();
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn poll_multiplayer(
    time: Res<Time>,
    mut client: NonSendMut<MultiplayerClient>,
    mut state: ResMut<MultiplayerState>,
    mut chess_match: ResMut<ChessMatch>,
    animation: Res<PieceAnimationState>,
    mut menu: ResMut<GameMenuState>,
    mut auto_turn: ResMut<CameraAutoTurn>,
    mut camera: Single<&mut PanOrbitCamera>,
    mut room_ready: MessageWriter<MultiplayerRoomReady>,
) {
    if let Some(timer) = &mut client.reconnect_timer {
        timer.tick(time.delta());
        if timer.is_finished()
            && let (Some(game_id), Some(player_token)) =
                (state.game_id.clone(), state.player_token.clone())
        {
            let handshake = ClientMessage::JoinGame {
                protocol: PROTOCOL_VERSION,
                game_id,
                player_token: Some(player_token),
            };
            if let Err(error) = client.connect(handshake) {
                state.status = error;
                client.reconnect_timer =
                    Some(Timer::from_seconds(RECONNECT_SECONDS, TimerMode::Once));
            }
        }
    }

    let mut events = Vec::new();
    if let Some(receiver) = &mut client.receiver {
        while let Some(event) = receiver.try_recv() {
            events.push(event);
        }
    }
    for event in events {
        match event {
            WsEvent::Opened => {
                if let Some(handshake) = client.handshake.take()
                    && let Err(error) = client.send(&handshake)
                {
                    state.phase = MultiplayerPhase::Error;
                    state.status = error;
                }
            }
            WsEvent::Message(WsMessage::Text(text)) => {
                match serde_json::from_str::<ServerMessage>(&text) {
                    Ok(message) => handle_server_message(
                        message,
                        &mut client,
                        &mut state,
                        &mut chess_match,
                        &mut menu,
                        &mut auto_turn,
                        &mut camera,
                        &mut room_ready,
                    ),
                    Err(error) => {
                        state.status = format!("Invalid server message: {error}");
                        request_resync(&mut client, &state);
                    }
                }
            }
            WsEvent::Message(_) => {}
            WsEvent::Error(error) => {
                client.sender = None;
                client.receiver = None;
                if state.phase == MultiplayerPhase::Active
                    || state.phase == MultiplayerPhase::Reconnecting
                {
                    state.phase = MultiplayerPhase::Reconnecting;
                    state.status = format!("Connection lost ({error}). Reconnecting…");
                    client.reconnect_timer =
                        Some(Timer::from_seconds(RECONNECT_SECONDS, TimerMode::Once));
                } else {
                    state.phase = MultiplayerPhase::Error;
                    state.status = format!("WebSocket error: {error}");
                }
            }
            WsEvent::Closed => {
                client.sender = None;
                client.receiver = None;
                if state.phase == MultiplayerPhase::Active
                    || state.phase == MultiplayerPhase::Reconnecting
                {
                    state.phase = MultiplayerPhase::Reconnecting;
                    state.status = "Connection lost. Reconnecting…".to_owned();
                    client.reconnect_timer =
                        Some(Timer::from_seconds(RECONNECT_SECONDS, TimerMode::Once));
                } else if state.phase == MultiplayerPhase::Connecting {
                    state.phase = MultiplayerPhase::Error;
                    state.status = "The multiplayer server closed the connection.".to_owned();
                }
            }
        }
    }

    apply_next_accepted_move(&mut client, &mut state, &mut chess_match, &animation);
}

#[allow(clippy::too_many_arguments)]
fn handle_server_message(
    message: ServerMessage,
    client: &mut MultiplayerClient,
    state: &mut MultiplayerState,
    chess_match: &mut ChessMatch,
    menu: &mut GameMenuState,
    auto_turn: &mut CameraAutoTurn,
    camera: &mut PanOrbitCamera,
    room_ready: &mut MessageWriter<MultiplayerRoomReady>,
) {
    match message {
        ServerMessage::GameReady {
            created,
            game_id,
            player_token,
            side,
            variant,
            revision,
            history,
            opponent_connected,
        } => {
            let engine_side = side_from_wire(side);
            let engine_variant = variant_from_wire(variant);
            if let Err(error) =
                load_server_history(chess_match, engine_variant, engine_side, revision, &history)
            {
                state.phase = MultiplayerPhase::Error;
                state.status = error;
                return;
            }
            client.save_token(&game_id, &player_token);
            client.reconnect_timer = None;
            state.phase = MultiplayerPhase::Active;
            state.game_id = Some(game_id.clone());
            state.player_token = Some(player_token);
            state.side = Some(engine_side);
            state.revision = revision;
            state.accepted_revision = revision;
            state.opponent_connected = opponent_connected;
            state.pending_move = None;
            state.accepted_moves.clear();
            state.status = room_status(&game_id, opponent_connected);

            menu.active_mode = GameMode::Multiplayer;
            menu.active_side = engine_side;
            menu.selected_mode = GameMode::Multiplayer;
            menu.selected_variant = engine_variant;
            menu.multiplayer_game_id = game_id;
            menu.open = false;
            start_camera_turn(camera, auto_turn, engine_side);
            room_ready.write(MultiplayerRoomReady { created });
        }
        ServerMessage::MoveAccepted { revision, uci } => {
            if revision <= state.revision {
                return;
            }
            if revision != state.accepted_revision.saturating_add(1) {
                state.status = "Move stream is out of sync. Requesting history…".to_owned();
                request_resync(client, state);
                return;
            }
            state.accepted_revision = revision;
            if state.pending_move.as_deref() == Some(uci.as_str()) {
                state.pending_move = None;
            }
            state.accepted_moves.push_back((revision, uci));
        }
        ServerMessage::OpponentConnection { connected } => {
            state.opponent_connected = connected;
            if let Some(game_id) = &state.game_id {
                state.status = room_status(game_id, connected);
            }
        }
        ServerMessage::Sync {
            revision,
            history,
            reason: _,
        } => {
            let (Some(side), variant) = (state.side, chess_match.variant) else {
                return;
            };
            match load_server_history(chess_match, variant, side, revision, &history) {
                Ok(()) => {
                    state.revision = revision;
                    state.accepted_revision = revision;
                    state.pending_move = None;
                    state.accepted_moves.clear();
                    state.status = state.game_id.as_ref().map_or_else(
                        || "Synchronized.".to_owned(),
                        |game_id| room_status(game_id, state.opponent_connected),
                    );
                }
                Err(error) => {
                    state.phase = MultiplayerPhase::Error;
                    state.status = error;
                }
            }
        }
        ServerMessage::Error { code, message } => {
            state.pending_move = None;
            state.status = message;
            if state.phase == MultiplayerPhase::Connecting
                || matches!(
                    code.as_str(),
                    "game_not_found"
                        | "game_full"
                        | "invalid_player_token"
                        | "protocol_mismatch"
                        | "connection_replaced"
                )
            {
                state.phase = MultiplayerPhase::Error;
            }
        }
    }
}

fn apply_next_accepted_move(
    client: &mut MultiplayerClient,
    state: &mut MultiplayerState,
    chess_match: &mut ChessMatch,
    animation: &PieceAnimationState,
) {
    if !animation.is_settled(chess_match.generation) {
        return;
    }
    let Some((revision, uci)) = state.accepted_moves.pop_front() else {
        return;
    };
    if revision != state.revision.saturating_add(1) {
        request_resync(client, state);
        return;
    }
    let chess_move = match chess_match.game.position().parse_uci_move(&uci) {
        Ok(chess_move) => chess_move,
        Err(_) => {
            state.status = "Confirmed move does not fit the local board. Resynchronizing…".into();
            request_resync(client, state);
            return;
        }
    };
    apply_move(chess_match, chess_move, None);
    state.revision = revision;
    if let Some(game_id) = &state.game_id {
        state.status = room_status(game_id, state.opponent_connected);
    }
}

fn dispatch_move_requests(
    mut requests: MessageReader<MoveRequest>,
    menu: Res<GameMenuState>,
    mut client: NonSendMut<MultiplayerClient>,
    mut multiplayer: ResMut<MultiplayerState>,
    mut chess_match: ResMut<ChessMatch>,
) {
    for MoveRequest(chess_move) in requests.read().copied() {
        if menu.active_mode != GameMode::Multiplayer {
            apply_move(&mut chess_match, chess_move, None);
            continue;
        }
        if multiplayer.phase != MultiplayerPhase::Active {
            multiplayer.status = "The multiplayer connection is not ready.".to_owned();
            continue;
        }
        if multiplayer.pending_move.is_some() || !multiplayer.accepted_moves.is_empty() {
            multiplayer.status = "Waiting for the server to confirm the previous move.".to_owned();
            continue;
        }
        let side_to_move = chess_match.game.position().side_to_move();
        if multiplayer.side != Some(side_to_move) {
            multiplayer.status = "Waiting for the other player.".to_owned();
            continue;
        }
        let uci = chess_move.to_uci();
        let message = ClientMessage::PlayMove {
            revision: multiplayer.revision,
            uci: uci.clone(),
        };
        match client.send(&message) {
            Ok(()) => {
                multiplayer.pending_move = Some(uci);
                multiplayer.status = "Move sent. Waiting for server confirmation…".to_owned();
                chess_match.selected = None;
                chess_match.pending_promotion = None;
            }
            Err(error) => multiplayer.status = error,
        }
    }
}

fn load_server_history(
    chess_match: &mut ChessMatch,
    variant: EngineVariant,
    player_side: EngineSide,
    revision: u64,
    history: &[String],
) -> Result<(), String> {
    if usize::try_from(revision).ok() != Some(history.len()) {
        return Err(format!(
            "Server history has {} moves but revision is {revision}.",
            history.len()
        ));
    }
    restart_match(chess_match, variant);
    chess_match.controllers = [Controller::Remote, Controller::Remote];
    chess_match.controllers[player_side.index()] = Controller::Human;
    for (index, uci) in history.iter().enumerate() {
        let chess_move = chess_match
            .game
            .position()
            .parse_uci_move(uci)
            .map_err(|error| format!("Invalid server move {} ({uci}): {error}", index + 1))?;
        apply_move(chess_match, chess_move, None);
    }
    chess_match.animate_last_move = false;
    for captured in &mut chess_match.captured_pieces {
        captured.generation = u64::MAX;
    }
    chess_match.selected = None;
    chess_match.pending_promotion = None;
    Ok(())
}

fn request_resync(client: &mut MultiplayerClient, state: &MultiplayerState) {
    let _ = client.send(&ClientMessage::Resync {
        revision: state.revision,
    });
}

fn room_status(game_id: &str, opponent_connected: bool) -> String {
    if opponent_connected {
        format!("Game {game_id} · opponent connected")
    } else {
        format!("Game {game_id} · waiting for opponent")
    }
}

fn side_to_preference(side: crate::menu::SideChoice) -> SidePreference {
    match side {
        crate::menu::SideChoice::Random => SidePreference::Random,
        crate::menu::SideChoice::White => SidePreference::White,
        crate::menu::SideChoice::Black => SidePreference::Black,
    }
}

fn side_from_wire(side: Side) -> EngineSide {
    match side {
        Side::White => EngineSide::White,
        Side::Black => EngineSide::Black,
    }
}

fn variant_to_wire(variant: EngineVariant) -> Variant {
    match variant {
        EngineVariant::Capablanca => Variant::Capablanca,
        EngineVariant::Gothic => Variant::Gothic,
        EngineVariant::Embassy => Variant::Embassy,
        EngineVariant::Schoolbook => Variant::Schoolbook,
        EngineVariant::Bird => Variant::Bird,
        EngineVariant::Carrera => Variant::Carrera,
        EngineVariant::Grand => Variant::Grand,
    }
}

fn variant_from_wire(variant: Variant) -> EngineVariant {
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

#[cfg(not(target_arch = "wasm32"))]
fn websocket_url() -> String {
    std::env::var("CAPABLANCA_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:8080/ws".to_owned())
}

#[cfg(target_arch = "wasm32")]
fn websocket_url() -> String {
    if let Some(configured) = option_env!("CAPABLANCA_WS_URL") {
        return configured.to_owned();
    }
    let location = web_sys::window().expect("browser window exists").location();
    let scheme = if location.protocol().as_deref() == Ok("https:") {
        "wss"
    } else {
        "ws"
    };
    let host = location
        .host()
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned());
    format!("{scheme}://{host}/ws")
}

#[cfg(not(target_arch = "wasm32"))]
fn save_browser_token(_game_id: &str, _token: &str) {}

#[cfg(target_arch = "wasm32")]
fn save_browser_token(game_id: &str, token: &str) {
    let Some(storage) = web_sys::window()
        .and_then(|window| window.local_storage().ok())
        .flatten()
    else {
        return;
    };
    let _ = storage.set_item(&format!("{TOKEN_STORAGE_PREFIX}{game_id}"), token);
}

#[cfg(not(target_arch = "wasm32"))]
fn load_browser_token(_game_id: &str) -> Option<String> {
    None
}

#[cfg(target_arch = "wasm32")]
fn load_browser_token(game_id: &str) -> Option<String> {
    web_sys::window()
        .and_then(|window| window.local_storage().ok())
        .flatten()
        .and_then(|storage| {
            storage
                .get_item(&format!("{TOKEN_STORAGE_PREFIX}{game_id}"))
                .ok()
                .flatten()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_history_restores_the_position_without_animation() {
        let mut chess_match = ChessMatch::default();
        load_server_history(
            &mut chess_match,
            EngineVariant::Gothic,
            EngineSide::Black,
            2,
            &["a2a3".to_owned(), "a7a6".to_owned()],
        )
        .unwrap();
        assert_eq!(chess_match.controllers[0], Controller::Remote);
        assert_eq!(chess_match.controllers[1], Controller::Human);
        assert!(!chess_match.animate_last_move);
        assert_eq!(chess_match.last_move.unwrap().to_uci(), "a7a6");
    }
}
