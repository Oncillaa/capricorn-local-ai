//! Капри 2.0 — общее ядро: настройки, персона, разбор команд.
//!
//! Крейт не зависит от ОС: всё, что общается с Windows, спрятано за
//! [`platform::Platform`].

pub mod ai;
pub mod appindex;
pub mod chess;
pub mod chat;
pub mod checkers;
pub mod config;
pub mod engine;
pub mod exec;
pub mod keys;
pub mod menu;
pub mod persona;
pub mod platform;
pub mod reminders;
pub mod reply;
pub mod save;
pub mod translate;
pub mod util;

pub use config::Config;
pub use engine::Engine;
pub use platform::{Platform, Track};
pub use reply::{action, Reply};
