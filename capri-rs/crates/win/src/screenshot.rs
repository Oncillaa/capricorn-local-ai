//! Скриншот экрана.

use std::path::PathBuf;

/// Прямоугольник в экранных координатах: левый верхний угол и размеры.
///
/// Координаты — виртуального экрана, того же, что у [`capture_region`]: при
/// нескольких мониторах левый верхний угол главного может быть не `(0, 0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Region {
    /// Область по двум углам — так её удобнее собирать из перетаскивания, где
    /// пользователь может вести мышь в любую сторону.
    pub fn from_corners(x1: i32, y1: i32, x2: i32, y2: i32) -> Self {
        Self {
            x: x1.min(x2),
            y: y1.min(y2),
            width: (x2 - x1).abs(),
            height: (y2 - y1).abs(),
        }
    }

    /// Слишком маленькая область — это случайный клик, а не выделение.
    ///
    /// 4×4 пикселя: меньше распознавать нечего, а случайное движение мыши при
    /// клике даёт ровно такую «рамку» в один-два пикселя.
    pub fn is_usable(&self) -> bool {
        self.width >= 4 && self.height >= 4
    }
}

#[cfg(windows)]
mod imp {
    use std::path::PathBuf;

    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
        ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
        SRCCOPY,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    };

    use super::Region;

    /// Снять с экрана прямоугольник и вернуть его как RGBA-картинку.
    ///
    /// Координаты — экранные. Область, вылезшую за пределы виртуального экрана,
    /// подрезаем: `BitBlt` за границей вернул бы чёрные поля, а OCR по ним —
    /// мусорные слова.
    pub fn capture_region(region: Region) -> Result<image::RgbaImage, String> {
        unsafe {
            let screen_x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let screen_y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let screen_w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let screen_h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if screen_w <= 0 || screen_h <= 0 {
                return Err("не смогла определить размер экрана".into());
            }
            if !region.is_usable() {
                return Err("область слишком маленькая — выдели побольше".into());
            }

            // Пересечение запрошенного прямоугольника с экраном.
            let left = region.x.max(screen_x);
            let top = region.y.max(screen_y);
            let right = (region.x + region.width).min(screen_x + screen_w);
            let bottom = (region.y + region.height).min(screen_y + screen_h);
            let w = right - left;
            let h = bottom - top;
            if w <= 0 || h <= 0 {
                return Err("область за пределами экрана".into());
            }

            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("не смогла получить контекст экрана".into());
            }
            let mem = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            if bitmap.is_invalid() {
                ReleaseDC(None, screen);
                return Err("не смогла создать картинку".into());
            }
            let old = SelectObject(mem, HGDIOBJ(bitmap.0));

            let mut ok = BitBlt(mem, 0, 0, w, h, Some(screen), left, top, SRCCOPY).is_ok();

            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h; // сверху вниз
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0;

            let mut buffer = vec![0u8; (w * h * 4) as usize];
            let lines = GetDIBits(
                mem,
                bitmap,
                0,
                h as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                &mut info,
                DIB_RGB_COLORS,
            );
            if lines == 0 {
                ok = false;
            }

            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);

            if !ok {
                return Err("не смогла снять область экрана".into());
            }

            // BGRA → RGBA
            for px in buffer.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            image::RgbaImage::from_raw(w as u32, h as u32, buffer)
                .ok_or_else(|| "не смогла собрать картинку".to_string())
        }
    }

    pub fn capture(path: &PathBuf) -> Result<PathBuf, String> {
        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if w <= 0 || h <= 0 {
                return Err("не смогла определить размер экрана".into());
            }

            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("не смогла получить контекст экрана".into());
            }
            let mem = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            if bitmap.is_invalid() {
                ReleaseDC(None, screen);
                return Err("не смогла создать картинку".into());
            }
            let old = SelectObject(mem, HGDIOBJ(bitmap.0));

            let mut ok = BitBlt(mem, 0, 0, w, h, Some(screen), x, y, SRCCOPY).is_ok();

            let mut info = BITMAPINFO::default();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = w;
            info.bmiHeader.biHeight = -h; // сверху вниз
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB.0;

            let mut buffer = vec![0u8; (w * h * 4) as usize];
            let lines = GetDIBits(
                mem,
                bitmap,
                0,
                h as u32,
                Some(buffer.as_mut_ptr() as *mut _),
                &mut info,
                DIB_RGB_COLORS,
            );
            if lines == 0 {
                ok = false;
            }

            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);

            if !ok {
                return Err("скриншот не получился".into());
            }

            // BGRA → RGBA
            for px in buffer.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            let img = image::RgbaImage::from_raw(w as u32, h as u32, buffer)
                .ok_or_else(|| "не смогла собрать картинку".to_string())?;

            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            img.save(path)
                .map_err(|e| format!("не смогла сохранить скриншот: {e}"))?;
            Ok(path.clone())
        }
    }
}

/// Папка для скриншотов: «Изображения\Capri».
pub fn screenshot_dir() -> PathBuf {
    dirs::picture_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Capri")
}

/// Снять с экрана прямоугольник и вернуть его как RGBA-картинку.
///
/// Нужна панели перевода: OCR работает по картинке в памяти, а не по файлу —
/// сохранять промежуточный PNG на диск незачем.
pub fn capture_region(region: Region) -> Result<image::RgbaImage, String> {
    #[cfg(windows)]
    {
        return imp::capture_region(region);
    }
    #[cfg(not(windows))]
    {
        let _ = region;
        Err("только для Windows".into())
    }
}

/// Сделать скриншот всего экрана и сохранить в «Изображения\Capri».
pub fn capture_screen() -> Result<PathBuf, String> {
    let path = screenshot_dir().join(format!("скриншот_{}.png", stamp()));
    #[cfg(windows)]
    {
        return imp::capture(&path);
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err("только для Windows".into())
    }
}

/// Метка времени для имён файлов: `2026-10-30_18-04-12`.
pub fn stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}_{:02}-{:02}-{:02}",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

/// Гражданская дата из числа дней с 1970-01-01 (алгоритм Говарда Хиннанта).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_math() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_000), (2022, 1, 8));
    }
}
