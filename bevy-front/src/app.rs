use bevy::{
    asset::{AssetMetaCheck, AssetPlugin},
    input_focus::tab_navigation::TabNavigationPlugin,
    prelude::*,
};
#[cfg(target_arch = "wasm32")]
use bevy::{
    log::{DEFAULT_FILTER, Level, LogPlugin},
    post_process::{
        PostProcessPlugin, bloom::BloomPlugin, effect_stack::EffectStackPlugin,
        msaa_writeback::MsaaWritebackPlugin,
    },
};
use bevy_panorbit_camera::PanOrbitCameraPlugin;

use crate::{
    ai::AiPlugin, audio::GameAudioPlugin, board::BoardPlugin, game::GamePlugin, hud::HudPlugin,
    input::InputPlugin, menu::GameMenuPlugin, multiplayer::MultiplayerPlugin, pieces::PiecesPlugin,
    promotion::PromotionPlugin, reflection::PlanarReflectionPlugin, scene::EnvironmentPlugin,
    skybox::SkyboxPlugin,
};

#[cfg(target_arch = "wasm32")]
pub(crate) const ASSET_ROOT: &str = match option_env!("CAPABLANCA_ASSET_ROOT") {
    Some(path) => path,
    None => "assets",
};

#[cfg(not(target_arch = "wasm32"))]
pub(crate) const ASSET_ROOT: &str = "assets";

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
    let default_plugins = DefaultPlugins
        .set(AssetPlugin {
            // Production web builds replace this with a content-addressed
            // directory (assets/<hash>). The URL may then be cached forever
            // without making updated assets stale after the next deployment.
            file_path: ASSET_ROOT.to_owned(),
            // The project does not use per-asset `.meta` files. On the web,
            // checking for them would issue one guaranteed 404 request for
            // every model, texture, sound, font, and shader.
            meta_check: AssetMetaCheck::Never,
            ..default()
        })
        .set(WindowPlugin {
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
        });

    #[cfg(target_arch = "wasm32")]
    let default_plugins = default_plugins
        .set(LogPlugin {
            level: Level::WARN,
            // These modules deliberately probe optional compute/storage
            // features and fall back on WebGL2. Their warnings describe the
            // selected compatibility path, not a broken render pipeline.
            filter: format!(
                "{DEFAULT_FILTER}\
                 bevy_core_pipeline::oit::resolve=error,\
                 bevy_core_pipeline::prepass::background_motion_vectors=error,\
                 bevy_pbr::ssao=error,\
                 bevy_pbr::atmosphere=error"
            ),
            ..default()
        })
        // The game uses bloom and vignette, but not depth of field or motion
        // blur. Avoid initializing their unsupported WebGL2 pipelines.
        .disable::<PostProcessPlugin>();

    app.add_plugins(default_plugins);

    #[cfg(target_arch = "wasm32")]
    app.add_plugins((MsaaWritebackPlugin, BloomPlugin, EffectStackPlugin));

    app.add_plugins((
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
