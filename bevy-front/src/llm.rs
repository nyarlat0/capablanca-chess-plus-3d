use crate::{
    app::FrontendSet,
    game::{ChessMatch, Controller, apply_move, is_playable},
    menu::{GameMenuState, GameMode},
    pieces::PieceAnimationState,
};
use bevy::{
    ecs::hierarchy::ChildSpawnerCommands,
    input_focus::tab_navigation::TabIndex,
    prelude::*,
    text::{EditableText, TextCursorStyle},
};
use std::sync::{Mutex, mpsc};

pub(crate) struct LlmPlugin;
impl Plugin for LlmPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LlmSession>()
            .add_systems(Startup, setup_status)
            .add_systems(Update, sync_controls.in_set(FrontendSet::Menu))
            .add_systems(Update, update_match.in_set(FrontendSet::AiStart))
            .add_systems(Update, update_status.in_set(FrontendSet::Hud));
    }
}

enum Event {
    Rejected(usize, String),
    Finished(
        llm_match::Match,
        Result<capablanca_chess_plus::Move, String>,
    ),
}
struct Pending {
    generation: u64,
    events: Mutex<mpsc::Receiver<Event>>,
    _cancel: tokio::sync::oneshot::Sender<()>,
}

#[derive(Resource)]
pub(crate) struct LlmSession {
    endpoint: String,
    config: Option<llm_match::Config>,
    state: Option<llm_match::Match>,
    pending: Option<Pending>,
    failed: bool,
    status: String,
}
impl Default for LlmSession {
    fn default() -> Self {
        let endpoint = llm_match::Config::load()
            .map(|c| c.endpoint)
            .unwrap_or_default();
        Self {
            endpoint,
            config: None,
            state: None,
            pending: None,
            failed: false,
            status: String::new(),
        }
    }
}
impl LlmSession {
    pub(crate) fn reset(&mut self) {
        self.pending = None; // dropping sender cancels the old async request
        self.state = None;
        self.config = None;
        self.failed = false;
        self.status.clear();
    }
}

#[derive(Component)]
struct Controls;
#[derive(Component)]
struct Endpoint;
#[derive(Component)]
struct ProfileLabel;
#[derive(Component)]
struct StatusButton;
#[derive(Component)]
struct StatusPanel;
#[derive(Component)]
struct StatusText;

pub(crate) fn spawn_controls(parent: &mut ChildSpawnerCommands, font: &Handle<Font>) {
    let mut input = EditableText {
        allow_newlines: false,
        max_characters: Some(512),
        ..default()
    };
    input.editor_mut().set_text(
        &llm_match::Config::load()
            .map(|c| c.endpoint)
            .unwrap_or_default(),
    );
    parent
        .spawn((
            Controls,
            Node {
                display: Display::None,
                width: percent(100),
                flex_direction: FlexDirection::Column,
                row_gap: px(5),
                ..default()
            },
        ))
        .with_children(|p| {
            let profile = llm_match::Config::load()
                .map(|c| format!("PROFILE: {}", c.profile_name))
                .unwrap_or_else(|e| format!("Profile configuration: {e}"));
            p.spawn((
                ProfileLabel,
                Text::new(profile),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(12.),
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.7, 0.8)),
            ));
            p.spawn((
                Text::new("KOBOLDCPP URL"),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(12.),
                    ..default()
                },
            ));
            p.spawn((
                Endpoint,
                TabIndex(0),
                input,
                Node {
                    width: percent(100),
                    min_height: px(32),
                    padding: UiRect::all(px(6)),
                    overflow: Overflow::clip_x(),
                    ..default()
                },
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(16.),
                    ..default()
                },
                TextLayout::no_wrap(),
                TextCursorStyle {
                    color: Color::srgb(1., 0.3, 0.6),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.15, 0.10, 0.18, 0.9)),
            ));
        });
}

fn sync_controls(
    menu: Res<GameMenuState>,
    mut session: ResMut<LlmSession>,
    mut controls: Query<&mut Node, With<Controls>>,
    inputs: Query<&EditableText, (Changed<EditableText>, With<Endpoint>)>,
) {
    for mut node in &mut controls {
        node.display = if menu.selected_mode == GameMode::Llm {
            Display::Flex
        } else {
            Display::None
        };
    }
    for input in &inputs {
        session.endpoint = input.value().to_string();
    }
}

fn setup_status(mut commands: Commands, assets: Res<AssetServer>) {
    // The frontend disables Bevy's default_font feature: every label needs
    // an explicit font, otherwise the retry control renders without text.
    let font: Handle<Font> = assets.load("fonts/FiraSans-Bold.ttf");
    commands
        .spawn((
            StatusPanel,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(12),
                left: percent(5),
                max_width: percent(90),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Start,
                row_gap: px(10),
                padding: UiRect::all(px(10)),
                border_radius: BorderRadius::all(px(8)),
                display: Display::None,
                ..default()
            },
            GlobalZIndex(20),
            BackgroundColor(Color::srgba(0.08, 0.05, 0.10, 0.9)),
        ))
        .with_children(|p| {
            p.spawn((
                StatusText,
                Text::default(),
                TextFont {
                    font: font.clone().into(),
                    font_size: FontSize::Px(14.),
                    ..default()
                },
                TextColor(Color::srgb(0.97, 0.95, 0.98)),
            ));
            p.spawn((
                StatusButton,
                Button,
                TabIndex(0),
                Node {
                    display: Display::None,
                    min_height: px(36),
                    padding: UiRect::axes(px(18), px(8)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(8)),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.85, 0.10, 0.39)),
                BorderColor::all(Color::srgb(1.0, 0.42, 0.65)),
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new("Retry"),
                    TextFont {
                        font: font.into(),
                        font_size: FontSize::Px(16.),
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Pickable::IGNORE,
                ));
            });
        });
}

fn update_status(
    menu: Res<GameMenuState>,
    mut session: ResMut<LlmSession>,
    mut panels: Query<&mut Node, (With<StatusPanel>, Without<StatusButton>)>,
    mut buttons: Query<
        (&mut Node, Ref<Interaction>, &mut BackgroundColor),
        (With<StatusButton>, Without<StatusPanel>),
    >,
    mut labels: Query<&mut Text, (With<StatusText>, Without<ProfileLabel>)>,
    mut profiles: Query<&mut Text, (With<ProfileLabel>, Without<StatusText>)>,
) {
    if let Some(config) = &session.config {
        for mut text in &mut profiles {
            text.0 = format!("PROFILE: {}", config.profile_name);
        }
    }
    let visible = !menu.open && menu.active_mode == GameMode::Llm && !session.status.is_empty();
    for mut node in &mut panels {
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
    }
    for (mut node, interaction, mut background) in &mut buttons {
        node.display = if visible && session.failed {
            Display::Flex
        } else {
            Display::None
        };
        background.0 = match *interaction {
            Interaction::Pressed => Color::srgb(0.66, 0.06, 0.29),
            Interaction::Hovered => Color::srgb(1.0, 0.25, 0.55),
            Interaction::None => Color::srgb(0.85, 0.10, 0.39),
        };
        if visible
            && interaction.is_changed()
            && *interaction == Interaction::Pressed
            && session.failed
        {
            session.failed = false;
            session.status = "Retrying…".into();
            node.display = Display::None;
        }
    }
    for mut text in &mut labels {
        text.0 = session.status.clone();
    }
}

fn update_match(
    menu: Res<GameMenuState>,
    animation: Res<PieceAnimationState>,
    mut chess: ResMut<ChessMatch>,
    mut session: ResMut<LlmSession>,
) {
    if menu.open || menu.active_mode != GameMode::Llm {
        session.pending = None;
        return;
    }
    if session
        .pending
        .as_ref()
        .is_some_and(|p| p.generation != chess.generation)
    {
        session.reset();
    }
    let mut events = Vec::new();
    let mut disconnected = false;
    if let Some(pending) = &session.pending {
        let receiver = pending.events.lock().unwrap();
        loop {
            match receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
    }
    for event in events {
        match event {
            Event::Rejected(attempt, answer) => {
                let format = session
                    .config
                    .as_ref()
                    .map(|c| c.representation.output_format)
                    .unwrap_or_default();
                if let Some(state) = &mut session.state {
                    let _ = state.accept_answer(&answer, format);
                }
                session.status =
                    format!("LLM: rejected attempt {attempt}, requesting another move…");
            }
            Event::Finished(state, result) => {
                session.pending = None;
                session.state = Some(state);
                match result {
                    Ok(mv) => {
                        apply_move(&mut chess, mv, None);
                        session.status.clear();
                    }
                    Err(error) => {
                        session.failed = true;
                        session.status = error;
                    }
                }
            }
        }
    }
    if disconnected && session.pending.is_some() {
        session.pending = None;
        session.failed = true;
        session.status = "LLM worker disconnected before returning a move".into();
    }
    if session.pending.is_some() || session.failed {
        return;
    }
    if session.config.is_none() {
        match llm_match::Config::load().and_then(|mut c| {
            c.endpoint = llm_match::normalize_endpoint(&session.endpoint)?;
            Ok(c)
        }) {
            Ok(config) => {
                info!(
                    "LLM representation profile: {} | {}",
                    config.profile_name,
                    config.representation.summary()
                );
                session.config = Some(config);
            }
            Err(e) => {
                session.failed = true;
                session.status = format!("LLM configuration: {e:#}");
                return;
            }
        }
    }
    let state = session
        .state
        .get_or_insert_with(|| llm_match::Match::new(chess.variant, menu.active_side.opposite()));
    if state.uci_history().len() > chess.move_history.len()
        || state
            .uci_history()
            .iter()
            .zip(&chess.move_history)
            .any(|(accepted, actual)| accepted != actual)
    {
        session.failed = true;
        session.status = "LLM history mismatch; start a new game".into();
        return;
    }
    // Only human moves can have been applied outside this session. Accepted
    // model moves were committed to both games together in Finished above.
    for mv in &chess.move_history[state.uci_history().len()..] {
        if let Err(error) = state.accept_human(mv) {
            session.failed = true;
            session.status = format!("LLM history mismatch: {error}");
            return;
        }
    }
    // Record even a game-ending human move, but never generate before its
    // animation (including captured pieces) has finished.
    if !animation.is_settled(chess.generation)
        || chess.pending_promotion.is_some()
        || !is_playable(chess.game.outcome())
        || chess.controllers[chess.game.position().side_to_move().index()] != Controller::Llm
    {
        return;
    }
    if state.game().position().to_fen() != chess.game.position().to_fen() {
        session.failed = true;
        session.status = "LLM position mismatch; start a new game".into();
        return;
    }
    let mut state = state.clone();
    let config = session
        .config
        .as_ref()
        .expect("initialized match configuration")
        .clone();
    let (tx, rx) = mpsc::channel();
    let (cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    session.pending = Some(Pending {
        generation: chess.generation,
        events: Mutex::new(rx),
        _cancel: cancel,
    });
    session.status = "LLM is choosing a move…".into();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let result = match runtime {
            Ok(runtime) => runtime.block_on(async {
                tokio::select! {
                    _ = cancelled => None,
                    result = llm_match::generate_move(&mut state, &config, |attempt, answer| { let _ = tx.send(Event::Rejected(attempt, answer.to_owned())); }) => Some(result.map_err(|e| format!("KoboldCPP: {e:#}"))),
                }
            }),
            Err(error) => Some(Err(error.to_string())),
        };
        if let Some(result) = result {
            let _ = tx.send(Event::Finished(state, result));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use capablanca_chess_plus::{Color as Side, Variant};

    fn world() -> World {
        let mut world = World::new();
        world.insert_resource(GameMenuState {
            open: false,
            active_mode: GameMode::Llm,
            active_side: Side::White,
            ..default()
        });
        let mut chess = ChessMatch::default();
        chess.controllers = [Controller::Human, Controller::Llm];
        let mv = chess.game.position().parse_uci_move("e2e4").unwrap();
        apply_move(&mut chess, mv, None);
        world.insert_resource(chess);
        world.insert_resource(PieceAnimationState::default()); // not settled: no HTTP
        world.insert_resource(LlmSession::default());
        world
    }

    #[test]
    fn llm_records_human_move_while_animation_is_still_running() {
        let mut world = world();
        world.run_system_once(update_match).unwrap();
        let session = world.resource::<LlmSession>();
        assert_eq!(session.state.as_ref().unwrap().uci_history()[0], "e2e4");
        assert_eq!(
            session.state.as_ref().unwrap().history()[0].content,
            "White Pawn e2-e4"
        );
        assert!(session.pending.is_none());
        // Run again: semantic text must NOT be compared to ChessMatch UCI.
        world.run_system_once(update_match).unwrap();
        assert!(!world.resource::<LlmSession>().failed);
    }

    #[test]
    fn llm_semantic_history_remains_synchronized_after_model_and_human_moves() {
        let mut world = world();
        world.run_system_once(update_match).unwrap();
        let mut state = world.resource::<LlmSession>().state.clone().unwrap();
        let mv = state.accept_model("e7e5").unwrap();
        let generation = world.resource::<ChessMatch>().generation;
        let (tx, rx) = mpsc::channel();
        tx.send(Event::Finished(state, Ok(mv))).unwrap();
        let (cancel, _rx) = tokio::sync::oneshot::channel();
        world.resource_mut::<LlmSession>().pending = Some(Pending {
            generation,
            events: Mutex::new(rx),
            _cancel: cancel,
        });
        world.run_system_once(update_match).unwrap();
        let mut chess = world.resource_mut::<ChessMatch>();
        let mv = chess.game.position().parse_uci_move("d2d4").unwrap();
        apply_move(&mut chess, mv, None);
        world.run_system_once(update_match).unwrap();
        let session = world.resource::<LlmSession>();
        assert!(!session.failed);
        let state = session.state.as_ref().unwrap();
        assert_eq!(
            state.uci_history(),
            world.resource::<ChessMatch>().move_history
        );
        assert_eq!(state.history()[1].content, "Black Pawn e7-e5");
        assert_eq!(state.history()[2].content, "White Pawn d2-d4");
    }

    #[test]
    fn llm_numeric_history_and_rejections_keep_canonical_frontend_sync() {
        let mut world = world();
        world.run_system_once(update_match).unwrap();
        world.resource_mut::<LlmSession>().config =
            Some(llm_match::Config::load_with_profile(Some("numeric-last-move")).unwrap());
        let generation = world.resource::<ChessMatch>().generation;
        let (tx, rx) = mpsc::channel();
        let bad = "Black Pawn: (5,7) -> (5,4)";
        tx.send(Event::Rejected(1, bad.into())).unwrap();
        let (cancel, _rx) = tokio::sync::oneshot::channel();
        world.resource_mut::<LlmSession>().pending = Some(Pending {
            generation,
            events: Mutex::new(rx),
            _cancel: cancel,
        });
        world.run_system_once(update_match).unwrap();
        let mut state = world.resource::<LlmSession>().state.clone().unwrap();
        assert_eq!(state.wrong_moves(), [bad]);
        assert_eq!(state.uci_history(), ["e2e4"]);
        let mv = state
            .accept_answer(
                "Black Pawn: (5,7) -> (5,5)",
                llm_match::OutputFormat::Numeric,
            )
            .unwrap();
        tx.send(Event::Finished(state, Ok(mv))).unwrap();
        world.run_system_once(update_match).unwrap();
        world.run_system_once(update_match).unwrap();
        let session = world.resource::<LlmSession>();
        assert!(!session.failed);
        assert_eq!(
            session.state.as_ref().unwrap().uci_history(),
            world.resource::<ChessMatch>().move_history
        );
        assert_eq!(
            session.state.as_ref().unwrap().numeric_history()[1].content,
            "Black Pawn: (5,7) -> (5,5)"
        );
    }

    #[test]
    fn llm_url_field_has_working_input_tab_setup() {
        let mut world = World::new();
        world
            .commands()
            .spawn_empty()
            .with_children(|p| spawn_controls(p, &Handle::default()));
        world.flush();
        let mut query = world.query_filtered::<(&TabIndex, &EditableText), With<Endpoint>>();
        let (tab, input) = query.single(&world).unwrap();
        assert_eq!(tab.0, 0);
        assert!(!input.allow_newlines);
    }

    #[test]
    fn retry_button_is_visible_for_failure_and_hides_after_click() {
        let mut world = world();
        {
            let mut session = world.resource_mut::<LlmSession>();
            session.failed = true;
            session.status = "KoboldCPP unavailable".into();
        }
        let panel = world.spawn((StatusPanel, Node::default())).id();
        let button = world
            .spawn((
                StatusButton,
                Button,
                Node::default(),
                BackgroundColor::default(),
            ))
            .id();
        let label = world.spawn((StatusText, Text::default())).id();
        world.run_system_once(update_status).unwrap();
        assert_eq!(world.get::<Node>(panel).unwrap().display, Display::Flex);
        assert_eq!(world.get::<Node>(button).unwrap().display, Display::Flex);
        assert_eq!(
            world.get::<BackgroundColor>(button).unwrap().0,
            Color::srgb(0.85, 0.10, 0.39)
        );
        assert_eq!(world.get::<Text>(label).unwrap().0, "KoboldCPP unavailable");
        *world.get_mut::<Interaction>(button).unwrap() = Interaction::Pressed;
        world.run_system_once(update_status).unwrap();
        assert!(!world.resource::<LlmSession>().failed);
        assert_eq!(world.get::<Node>(button).unwrap().display, Display::None);
        assert_eq!(world.get::<Text>(label).unwrap().0, "Retrying…");
        world.resource_mut::<GameMenuState>().open = true;
        world.run_system_once(update_status).unwrap();
        assert_eq!(world.get::<Node>(panel).unwrap().display, Display::None);
    }

    #[test]
    fn llm_discards_response_from_old_generation() {
        let mut world = world();
        let before = world.resource::<ChessMatch>().game.position().to_fen();
        let generation = world.resource::<ChessMatch>().generation;
        let mut old = llm_match::Match::new(Variant::Gothic, Side::Black);
        old.accept_human("e2e4").unwrap();
        let mv = old.accept_model("e7e5").unwrap();
        let (tx, rx) = mpsc::channel();
        tx.send(Event::Finished(old, Ok(mv))).unwrap();
        let (cancel, _rx) = tokio::sync::oneshot::channel();
        world.resource_mut::<LlmSession>().pending = Some(Pending {
            generation: generation.wrapping_sub(1),
            events: Mutex::new(rx),
            _cancel: cancel,
        });
        world.run_system_once(update_match).unwrap();
        assert_eq!(
            world.resource::<ChessMatch>().game.position().to_fen(),
            before
        );
        assert_eq!(
            world
                .resource::<LlmSession>()
                .state
                .as_ref()
                .unwrap()
                .history()
                .len(),
            1
        );
    }

    #[test]
    fn llm_open_menu_cancels_client_request() {
        let mut session = LlmSession::default();
        let (_tx, rx) = mpsc::channel();
        let (cancel, mut cancelled) = tokio::sync::oneshot::channel();
        session.pending = Some(Pending {
            generation: 1,
            events: Mutex::new(rx),
            _cancel: cancel,
        });
        let mut world = world();
        world.insert_resource(session);
        world.resource_mut::<GameMenuState>().open = true;
        world.run_system_once(update_match).unwrap();
        assert!(world.resource::<LlmSession>().pending.is_none());
        assert!(matches!(
            cancelled.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Closed)
        ));
    }
}
