//! DeckCraft in the browser.
//!
//! Runs the same [`deckcraft_ui_egui::SlideApp`] as the desktop app through eframe's web
//! runner (wgpu: WebGPU where available, WebGL2 otherwise). Build with `trunk build --release`
//! from this directory (output in `dist/web`).
//!
//! Differences from the desktop app:
//! - no TCP control channel (browsers can't listen on sockets);
//! - File → Open and Insert → Pictures/Audio/Video use the browser file picker; bytes arrive
//!   asynchronously through `Services::inbox` (presentations open, media is inserted);
//! - Save and Export trigger a browser download;
//! - dropped files are read asynchronously by `web::WebShell` and delivered through the inbox;
//! - preferences are not persisted, except the interface language (`language`), which the
//!   ERPCons office site shares between its editors;
//! - no AutoRecover folder: where the page provides a draft store (`window.erpDrafts`, the
//!   ERPCons office site), unsaved presentations are kept there instead (`draft_bridge`);
//! - the optional PI ID account is the office site's session (`window.erpAccount`, `account`);
//! - with the office site's share sheet (`window.erpShare`), File › Share… hands it a copy
//!   (`share`), and dropped files report whether they opened (`window.erpIntakeResult`, `intake`).
//!
//! URL query flags: `?webgl` forces the WebGL2 backend instead of WebGPU; `?blank` starts with an
//! empty presentation instead of the sample deck; `?show` starts the slide show.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(any(target_arch = "wasm32", test))]
mod account;
#[cfg(target_arch = "wasm32")]
mod audio;
#[cfg(any(target_arch = "wasm32", test))]
mod clipboard;
#[cfg(target_arch = "wasm32")]
mod closed_hook;
#[cfg(target_arch = "wasm32")]
mod draft_bridge;
#[cfg(any(target_arch = "wasm32", test))]
mod draft_state;
#[cfg(any(target_arch = "wasm32", test))]
mod file_types;
#[cfg(any(target_arch = "wasm32", test))]
mod intake;
#[cfg(target_arch = "wasm32")]
mod language;
#[cfg(any(target_arch = "wasm32", test))]
mod save_errors;
#[cfg(target_arch = "wasm32")]
mod share;
#[cfg(any(target_arch = "wasm32", test))]
mod show_fullscreen;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("deckcraft-web only runs in the browser: build it with `trunk build --release` in apps/deckcraft-web");
}
