//! Реализация [capri_core::platform::Platform] для Windows.
//!
//! Каждая возможность спрятана в отдельный модуль; всё, что можно сделать
//! без COM и без ожидания, делается напрямую через Win32, остальное —
//! через PowerShell (с кэшированием, чтобы не тормозить разбор команд).

pub mod audio;
pub mod input;
pub mod launch;
pub mod media;
pub mod platform;
pub mod screenshot;
pub mod system;
pub mod util;
pub mod windows_api;

pub use platform::WindowsPlatform;
