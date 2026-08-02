use bevy::prelude::*;
use bevy_panorbit_camera::PanOrbitCameraPlugin;

use crate::{
    ai::AiPlugin, board::BoardPlugin, game::GamePlugin, hud::HudPlugin, input::InputPlugin,
    menu::GameMenuPlugin, pieces::PiecesPlugin, promotion::PromotionPlugin,
    reflection::PlanarReflectionPlugin, scene::EnvironmentPlugin, skybox::SkyboxPlugin,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, SystemSet)]
pub(crate) enum FrontendSet {
    Menu,
    Input,
    AiPoll,
    AiStart,
    Camera,
    BoardSync,
    PieceSync,
    Highlights,
    Animation,
    Hud,
}

pub struct FrontendPlugin;

impl Plugin for FrontendPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            Update,
            (
                FrontendSet::Menu,
                FrontendSet::Input,
                FrontendSet::AiPoll,
                FrontendSet::AiStart,
                FrontendSet::Camera,
                FrontendSet::BoardSync,
                FrontendSet::PieceSync,
                FrontendSet::Highlights,
                FrontendSet::Animation,
                FrontendSet::Hud,
            )
                .chain(),
        )
        .add_plugins((
            GamePlugin,
            GameMenuPlugin,
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
            ..default()
        }),
        ..default()
    }))
    .add_plugins((MeshPickingPlugin, PanOrbitCameraPlugin, FrontendPlugin));
    app
}

pub fn run() {
    build_app().run();
}
