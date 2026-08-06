use bevy::prelude::*;

#[cfg(target_arch = "wasm32")]
const LOW_END_MODE_KEY: &str = "capablanca_chess.low_end_mode";

#[derive(Resource, Clone, Copy, Debug)]
pub(crate) struct GraphicsSettings {
    pub(crate) low_end_mode: bool,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self {
            low_end_mode: load_low_end_mode(),
        }
    }
}

impl GraphicsSettings {
    pub(crate) fn toggle_low_end_mode(&mut self) {
        self.low_end_mode = !self.low_end_mode;
        save_low_end_mode(self.low_end_mode);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_low_end_mode() -> bool {
    false
}

#[cfg(target_arch = "wasm32")]
fn load_low_end_mode() -> bool {
    web_sys::window()
        .and_then(|window| window.local_storage().ok())
        .flatten()
        .and_then(|storage| storage.get_item(LOW_END_MODE_KEY).ok())
        .flatten()
        .is_some_and(|value| value == "1")
}

#[cfg(not(target_arch = "wasm32"))]
fn save_low_end_mode(_enabled: bool) {}

#[cfg(target_arch = "wasm32")]
fn save_low_end_mode(enabled: bool) {
    let Some(storage) = web_sys::window()
        .and_then(|window| window.local_storage().ok())
        .flatten()
    else {
        return;
    };

    let _ = storage.set_item(LOW_END_MODE_KEY, if enabled { "1" } else { "0" });
}
