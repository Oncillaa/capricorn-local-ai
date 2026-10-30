//! Слух и голос Капри: распознавание речи (Vosk/Google), синтез (Edge TTS
//! с запасным голосом Windows), воспроизведение и эффекты.
//!
//! Всё, что требует нативных библиотек (`libvosk.dll`) или сети, работает
//! «мягко»: при сбое возвращается понятная русская ошибка, а не паника.

pub mod audio;
pub mod edge;
pub mod google;
pub mod listener;
pub mod speaker;

pub use listener::{ListenerHandle, ListenerOptions};
pub use speaker::Speaker;

/// Где живёт кэш озвучки: `%LOCALAPPDATA%\Capri`.
pub fn cache_root() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("LOCALAPPDATA") {
        if !dir.is_empty() {
            return std::path::PathBuf::from(dir).join("Capri");
        }
    }
    std::env::temp_dir().join("Capri")
}
