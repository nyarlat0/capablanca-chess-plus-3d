use bevy::input_focus::tab_navigation::TabNavigationPlugin;
use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCameraPlugin;

use crate::{
    ai::AiPlugin, audio::GameAudioPlugin, board::BoardPlugin, game::GamePlugin, hud::HudPlugin,
    input::InputPlugin, menu::GameMenuPlugin, multiplayer::MultiplayerPlugin, pieces::PiecesPlugin,
    promotion::PromotionPlugin, reflection::PlanarReflectionPlugin, scene::EnvironmentPlugin,
    skybox::SkyboxPlugin,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub(crate) enum FrontendSet {
    Menu,
    Multiplayer,
    Input,
    MoveDispatch,
    AiPoll,
    AiStart,
    Camera,
    BoardSync,
    PieceSync,
    Highlights,
    Animation,
    Audio,
    Hud,
}

pub struct FrontendPlugin;

impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            Update,
            (
                FrontendSet::Menu,
                FrontendSet::Multiplayer,
                FrontendSet::Input,
                FrontendSet::MoveDispatch,
                FrontendSet::AiPoll,
                FrontendSet::AiStart,
                FrontendSet::Camera,
                FrontendSet::BoardSync,
                FrontendSet::PieceSync,
                FrontendSet::Highlights,
                FrontendSet::Animation,
                FrontendSet::Audio,
                FrontendSet::Hud,
            )
                .chain(),
        )
        .add_plugins((
            GamePlugin,
            GameAudioPlugin,
            GameMenuPlugin,
            MultiplayerPlugin,
            AiPlugin,
            InputPlugin,
            EnvironmentPlugin,
            SkyboxPlugin,
            PlanarReflectionPlugin,
            BoardPlugin,
            PiecesPlugin,
            PromotionPlugin,
            HudPlugin,
        ));
    }
}

pub fn build_app() -> App {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "Capablanca Chess Plus 3D".into(),
            name: Some("capablanca-chess-plus-3d".into()),
            resolution: (1280, 800).into(),
            // Keep mobile pinch and two-finger drag inside the game instead of
            // letting the browser zoom or scroll the surrounding page.
            prevent_default_event_handling: true,
            ..default()
        }),
        ..default()
    }))
    .add_plugins((
        MeshPickingPlugin,
        TabNavigationPlugin,
        PanOrbitCameraPlugin,
        FrontendPlugin,
    ));
    app
}

pub fn run() {
    build_app().run();
}
