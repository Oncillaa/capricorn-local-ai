//! Холст аватара: пиксели, картинки и текст с прозрачностью.
//!
//! Кадр собирается в память и целиком отдаётся окну через
//! `UpdateLayeredWindow`. Текст рисуется системным шрифтом в чёрно-белую
//! маску: цвет и прозрачность накладываем сами.

/// Один пиксель в порядке BGRA — так его ждёт Windows.
pub type Pixel = u32;

#[inline]
fn bgra(r: u8, g: u8, b: u8, a: u8) -> Pixel {
    (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// Картинка с прозрачностью (RGBA, не домноженная на альфу).
#[derive(Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height` пикселей, порядок RGBA.
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn empty(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0; (width * height * 4) as usize] }
    }

    /// Загрузить PNG/JPEG (или WAV-подобный мусор не заметим — просто `None`).
    pub fn load(path: &std::path::Path) -> Option<Self> {
        let file = std::fs::File::open(path).ok()?;
        let decoder = image::ImageReader::new(std::io::BufReader::new(file))
            .with_guessed_format()
            .ok()?;
        let rgba = decoder.decode().ok()?.to_rgba8();
        Some(Self {
            width: rgba.width(),
            height: rgba.height(),
            pixels: rgba.into_raw(),
        })
    }

    /// Изменить размер методом «ближайшего соседа»: быстро и без размытия.
    pub fn scaled(&self, width: u32, height: u32) -> Self {
        if width == self.width && height == self.height || width == 0 || height == 0 {
            return self.clone();
        }
        let mut out = Image::empty(width, height);
        for y in 0..height {
            let sy = (y as u64 * self.height as u64 / height as u64) as u32;
            for x in 0..width {
                let sx = (x as u64 * self.width as u64 / width as u64) as u32;
                let src = ((sy * self.width + sx) * 4) as usize;
                let dst = ((y * width + x) * 4) as usize;
                out.pixels[dst..dst + 4].copy_from_slice(&self.pixels[src..src + 4]);
            }
        }
        out
    }

    /// Вписать в высоту, сохранив пропорции.
    pub fn fit_height(&self, height: u32) -> Self {
        if self.height == 0 {
            return self.clone();
        }
        let width = (self.width as f64 * height as f64 / self.height as f64).round() as u32;
        self.scaled(width.max(1), height)
    }

    /// Умножить цвет на `tint`, не трогая альфу (цветные скины).
    pub fn tinted(&self, tint: [u8; 3]) -> Self {
        let mut out = self.clone();
        for chunk in out.pixels.chunks_exact_mut(4) {
            chunk[0] = ((chunk[0] as u16 * tint[0] as u16) / 255) as u8;
            chunk[1] = ((chunk[1] as u16 * tint[1] as u16) / 255) as u8;
            chunk[2] = ((chunk[2] as u16 * tint[2] as u16) / 255) as u8;
        }
        out
    }
}

/// Кадр окна: фон прозрачный, поверх — фигура и подпись.
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    /// BGRA, домноженный на альфу (требование `UpdateLayeredWindow`).
    pub pixels: Vec<Pixel>,
}

impl Canvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height, pixels: vec![0; (width * height) as usize] }
    }

    pub fn clear(&mut self) {
        self.pixels.fill(0);
    }

    /// Наложить картинку со сдвигом, умножив её прозрачность на `opacity` (0…1).
    pub fn draw_image(&mut self, img: &Image, left: i32, top: i32, opacity: f32) {
        let opacity = opacity.clamp(0.0, 1.0);
        if opacity <= 0.0 {
            return;
        }
        for y in 0..img.height as i32 {
            let dy = top + y;
            if dy < 0 || dy >= self.height as i32 {
                continue;
            }
            for x in 0..img.width as i32 {
                let dx = left + x;
                if dx < 0 || dx >= self.width as i32 {
                    continue;
                }
                let src = ((y as u32 * img.width + x as u32) * 4) as usize;
                let alpha = img.pixels[src + 3] as f32 / 255.0 * opacity;
                if alpha <= 0.004 {
                    continue;
                }
                let (r, g, b) = (img.pixels[src], img.pixels[src + 1], img.pixels[src + 2]);
                self.blend(dx as u32, dy as u32, r, g, b, alpha);
            }
        }
    }

    /// Наложить пиксель поверх (альфа-смешивание «источник поверх»).
    pub fn blend(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, alpha: f32) {
        if x >= self.width || y >= self.height {
            return;
        }
        let alpha = alpha.clamp(0.0, 1.0);
        let index = (y * self.width + x) as usize;
        let dst = self.pixels[index];
        let da = ((dst >> 24) & 0xFF) as f32 / 255.0;
        let out_a = alpha + da * (1.0 - alpha);
        if out_a <= 0.0 {
            self.pixels[index] = 0;
            return;
        }
        // Цвета храним уже домноженными на альфу.
        let mix = |src: u8, dst_part: u32| -> u32 {
            let src = src as f32 * alpha;
            let dst_v = (dst_part & 0xFF) as f32;
            ((src + dst_v * (1.0 - alpha)) / out_a).round().min(255.0) as u32
        };
        let r_out = mix(r, dst >> 16);
        let g_out = mix(g, dst >> 8);
        let b_out = mix(b, dst);
        let a_out = (out_a * 255.0).round() as u32;
        self.pixels[index] = (a_out.min(255) << 24)
            | ((r_out.min(255) * a_out / 255) << 16)
            | ((g_out.min(255) * a_out / 255) << 8)
            | (b_out.min(255) * a_out / 255);
    }

    /// Залить скруглённый прямоугольник (подложка под текст).
    pub fn round_rect(&mut self, left: i32, top: i32, width: i32, height: i32, radius: i32, color: [u8; 3], alpha: f32) {
        let radius = radius.max(0);
        for y in 0..height {
            for x in 0..width {
                if !inside_round_rect(x, y, width, height, radius) {
                    continue;
                }
                self.blend(
                    (left + x).max(0) as u32,
                    (top + y).max(0) as u32,
                    color[0],
                    color[1],
                    color[2],
                    alpha,
                );
            }
        }
    }

    /// Нарисовать маску текста (альфа 0…255) нужным цветом.
    pub fn draw_mask(&mut self, mask: &Mask, left: i32, top: i32, color: [u8; 3], opacity: f32) {
        for y in 0..mask.height as i32 {
            for x in 0..mask.width as i32 {
                let alpha = mask.values[(y as u32 * mask.width + x as u32) as usize] as f32 / 255.0 * opacity;
                if alpha <= 0.004 {
                    continue;
                }
                self.blend(
                    (left + x).max(0) as u32,
                    (top + y).max(0) as u32,
                    color[0],
                    color[1],
                    color[2],
                    alpha,
                );
            }
        }
    }

    /// Готовые байты BGRA для `UpdateLayeredWindow`.
    pub fn as_bytes(&self) -> &[u8] {
        // `Pixel` — это u32, а нужен непрерывный поток байт.
        unsafe {
            std::slice::from_raw_parts(
                self.pixels.as_ptr().cast::<u8>(),
                self.pixels.len() * std::mem::size_of::<Pixel>(),
            )
        }
    }
}

fn inside_round_rect(x: i32, y: i32, width: i32, height: i32, radius: i32) -> bool {
    if radius <= 0 {
        return true;
    }
    let (w, h, r) = (width - 1, height - 1, radius);
    let corners = [
        (r, r, x < r && y < r),
        (w - r, r, x > w - r && y < r),
        (r, h - r, x < r && y > h - r),
        (w - r, h - r, x > w - r && y > h - r),
    ];
    for (cx, cy, in_corner) in corners {
        if in_corner {
            let dx = (x - cx) as f32;
            let dy = (y - cy) as f32;
            return dx * dx + dy * dy <= (r as f32) * (r as f32);
        }
    }
    true
}

/// Чёрно-белая маска текста, отрисованная системным шрифтом.
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub values: Vec<u8>,
}

impl Mask {
    pub fn empty() -> Self {
        Self { width: 0, height: 0, values: Vec::new() }
    }
}

/// Цвет шрифта в COLORREF (0x00BBGGRR).
pub fn colorref(rgb: [u8; 3]) -> u32 {
    (rgb[0] as u32) | ((rgb[1] as u32) << 8) | ((rgb[2] as u32) << 16)
}

/// Пиксель в BGRA — для отладочных проверок.
pub fn make_pixel(r: u8, g: u8, b: u8, a: u8) -> Pixel {
    bgra(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blends_half_transparent_over_clear() {
        let mut canvas = Canvas::new(4, 4);
        canvas.blend(1, 1, 255, 0, 0, 0.5);
        let pixel = canvas.pixels[5];
        let alpha = (pixel >> 24) & 0xFF;
        assert!((alpha as i32 - 128).abs() <= 2);
    }

    #[test]
    fn draw_image_respects_bounds() {
        let mut canvas = Canvas::new(4, 4);
        let img = Image {
            width: 2,
            height: 2,
            pixels: vec![255; 16],
        };
        canvas.draw_image(&img, 3, 3, 1.0);
        // Внутрь попал только один пиксель.
        let opaque = canvas.pixels.iter().filter(|p| (**p >> 24) > 0).count();
        assert_eq!(opaque, 1);
    }

    #[test]
    fn scaling_keeps_aspect() {
        let img = Image::empty(100, 50);
        let fitted = img.fit_height(25);
        assert_eq!(fitted.height, 25);
        assert_eq!(fitted.width, 50);
    }

    #[test]
    fn tint_darkens_colors() {
        let img = Image { width: 1, height: 1, pixels: vec![200, 200, 200, 255] };
        let tinted = img.tinted([128, 0, 0]);
        assert_eq!(tinted.pixels[0], 100);
        assert_eq!(tinted.pixels[1], 0);
    }
}
