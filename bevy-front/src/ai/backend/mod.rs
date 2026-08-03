#[derive(Debug)]
pub(super) enum BackendEvent {
    Line(String),
    Error(String),
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::Backend;
#[cfg(target_arch = "wasm32")]
pub(super) use web::Backend;
