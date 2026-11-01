//! Спрайты Капри: позы, кадры анимаций, картинка для прогулки.

use std::path::{Path, PathBuf};

use crate::canvas::Image;

/// Ширина окна при размере 100 % (как `W` в `avatar.py`).
pub const WIDTH: u32 = 340;
/// Высота фигуры при размере 100 %.
pub const FIG_HEIGHT: u32 = 380;
/// Высота окна с местом под подпись при размере 100 %.
pub const HEIGHT: u32 = 500;
/// Сколько пикселей окна отведено под подпись (остальное — фигура).
pub const CAPTION_HEIGHT: u32 = HEIGHT - FIG_HEIGHT;
/// Уже этого окно не делаем: подпись перестанет влезать.
pub const MIN_WIDTH: u32 = 220;

/// Геометрия окна для выбранного размера.
///
/// 100 % — ровно как в картинках (окно 340×500, фигура 380). Меньше — фигура
/// и окно сжимаются пропорционально, но по ширине остаётся место под подпись.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    /// Высота фигуры: во столько пикселей вписываем кадр.
    pub fig_height: u32,
}

impl Geometry {
    pub fn for_scale(percent: u8) -> Self {
        let percent = capri_core::config::clamp_scale(percent) as f64 / 100.0;
        let fig_height = ((FIG_HEIGHT as f64 * percent).round() as u32).max(1);
        let width = ((WIDTH as f64 * percent).round() as u32).max(MIN_WIDTH);
        Self { width, height: fig_height + CAPTION_HEIGHT, fig_height }
    }
}

impl Default for Geometry {
    fn default() -> Self {
        Self::for_scale(100)
    }
}

pub struct Sprites {
    /// Основные позы: `capri.png`, `каприии.png`, …
    pub poses: Vec<Image>,
    /// Кадры анимации «машет» (60 кадров).
    pub wave: Vec<Image>,
    /// Кадры анимации «думает» (32 кадра).
    pub eyes: Vec<Image>,
    /// Картинка для прогулки по экрану.
    pub roam: Option<Image>,
}

impl Sprites {
    /// Загрузить всё, что найдётся. Чего нет — просто пропускаем.
    pub fn load(assets_dir: &Path, base_dir: &Path) -> Self {
        let mut poses: Vec<Image> = Vec::new();
        for folder in [assets_dir.to_path_buf(), base_dir.join("assets"), base_dir.join("assets").join("poses")]
        {
            if !folder.is_dir() {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            let mut files: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    matches!(
                        p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
                        Some("png" | "jpg" | "jpeg" | "webp")
                    )
                })
                .collect();
            files.sort();
            for file in files {
                if let Some(image) = Image::load(&file) {
                    poses.push(image.fit_height(FIG_HEIGHT));
                }
            }
            if !poses.is_empty() {
                break;
            }
        }

        let wave = load_frames(&assets_dir.join("anim").join("capri"));
        let eyes = load_frames(&assets_dir.join("anim").join("eyes"));
        let roam = ["capri_roam.png", "walk.png"]
            .iter()
            .map(|name| assets_dir.join(name))
            .chain(std::iter::once(assets_dir.join("poses").join("capri_roam.png")))
            .find_map(|path| Image::load(&path).map(|img| img.fit_height(FIG_HEIGHT)));

        if poses.is_empty() {
            log::warn!("Не нашла ни одной картинки Капри в {}", assets_dir.display());
        }

        Self { poses, wave, eyes, roam }
    }

    pub fn is_empty(&self) -> bool {
        self.poses.is_empty() && self.wave.is_empty()
    }

    /// Кадр для «главного» вида: первая поза или первый кадр анимации.
    pub fn default_frame(&self) -> Option<&Image> {
        self.poses.first().or_else(|| self.wave.first())
    }
}

/// Загрузить кадры анимации по именам `0000.png`, `0001.png`, …
fn load_frames(dir: &Path) -> Vec<Image> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
        .collect();
    files.sort();
    files
        .into_iter()
        .filter_map(|path| Image::load(&path).map(|img| img.fit_height(FIG_HEIGHT)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_assets_do_not_panic() {
        let sprites = Sprites::load(Path::new("нет-такой-папки"), Path::new("нет-такой-папки"));
        assert!(sprites.is_empty());
        assert!(sprites.default_frame().is_none());
    }

    #[test]
    fn full_scale_is_the_original_size() {
        let full = Geometry::for_scale(100);
        assert_eq!(full.width, WIDTH);
        assert_eq!(full.height, HEIGHT);
        assert_eq!(full.fig_height, FIG_HEIGHT);
    }

    #[test]
    fn smaller_scale_shrinks_figure_but_keeps_caption_room() {
        let half = Geometry::for_scale(50);
        assert_eq!(half.fig_height, 190);
        // Место под подпись не сжимается — иначе текст обрежется.
        assert_eq!(half.height, 190 + CAPTION_HEIGHT);
        // По ширине держим минимум: подпись должна влезать.
        assert_eq!(half.width, MIN_WIDTH);
    }

    #[test]
    fn tiniest_scale_still_gives_a_visible_window() {
        let tiny = Geometry::for_scale(1);
        assert_eq!(tiny.fig_height, 4);
        assert_eq!(tiny.width, MIN_WIDTH);
        // Даже на минимуме окно не вырождается в ноль.
        assert!(tiny.height >= CAPTION_HEIGHT);
    }
}
