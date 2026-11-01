//! Реализация [capri_core::platform::Platform] для Windows.
//!
//! Каждая возможность спрятана в отдельный модуль; всё, что можно сделать
//! без COM и без ожидания, делается напрямую через Win32, остальное —
//! через PowerShell (с кэшированием, чтобы не тормозить разбор команд).

pub mod audio;
pub mod audio_loop;
pub mod exec;
pub mod input;
pub mod launch;
pub mod media;
pub mod monitors;
pub mod ocr;
pub mod platform;
pub mod screenshot;
pub mod single;
pub mod system;
pub mod util;
pub mod windows_api;

pub use platform::WindowsPlatform;
pub use single::{already_running_notice, summon_running_instance, InstanceGuard};
