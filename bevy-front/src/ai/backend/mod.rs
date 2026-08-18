#[derive(Debug)]
pub(super) enum BackendEvent {
    Line(String),
    Error(String),
}

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
mod tera_native;
#[cfg(target_arch = "wasm32")]
mod tera_web;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
pub(super) use native::Backend as FairyBackend;
#[cfg(not(target_arch = "wasm32"))]
pub(super) use tera_native::Backend as TeraBackend;
#[cfg(target_arch = "wasm32")]
pub(super) use tera_web::Backend as TeraBackend;
#[cfg(target_arch = "wasm32")]
pub(super) use web::Backend as FairyBackend;
