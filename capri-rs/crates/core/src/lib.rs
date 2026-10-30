//! Капри 2.0 — общее ядро: настройки, персона, разбор команд.
//!
//! Крейт не зависит от ОС: всё, что общается с Windows, спрятано за
//! [`platform::Platform`].

pub mod ai;
pub mod appindex;
pub mod config;
pub mod engine;
pub mod keys;
pub mod persona;
pub mod platform;
pub mod reply;
pub mod save;
pub mod util;

pub use config::Config;
pub use engine::Engine;
pub use platform::{Platform, Track};
pub use reply::{action, Reply};
