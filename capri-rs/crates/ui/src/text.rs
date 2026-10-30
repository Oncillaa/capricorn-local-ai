//! Рисование текста системным шрифтом в маску (GDI).

use crate::canvas::Mask;

/// Нарисовать текст в маску. `bold` — для заголовков, `max_width` — перенос.
#[cfg(windows)]
pub fn render(text: &str, size: i32, bold: bool, max_width: i32) -> Mask {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
        GetDC, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS,
        DT_CALCRECT, DT_WORDBREAK, FF_DONTCARE, FW_BOLD, FW_NORMAL, HGDIOBJ,
        OUT_TT_PRECIS, PROOF_QUALITY,
    };

    let size = size.clamp(8, 120);
    if text.trim().is_empty() {
        return Mask::empty();
    }

    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Mask::empty();
        }
        let dc = CreateCompatibleDC(Some(screen));
        if dc.is_invalid() {
            ReleaseDC(None, screen);
            return Mask::empty();
        }

        // Монохромная DIB-секция: 8 бит на пиксель, белое = текст.
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: 8,
            biHeight: -8, // сверху вниз
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(_) => {
                let _ = DeleteDC(dc);
                ReleaseDC(None, screen);
                return Mask::empty();
            }
        };
        let old_bitmap = SelectObject(dc, HGDIOBJ(bitmap.0));

        let font = CreateFontW(
            -size,
            0,
            0,
            0,
            if bold { FW_BOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            if bold { PROOF_QUALITY } else { CLEARTYPE_QUALITY },
            DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
            PCWSTR(wide("Segoe UI").as_ptr()),
        );
        let old_font = SelectObject(dc, HGDIOBJ(font.0));

        // Сначала узнаём нужный размер.
        let mut wide_text = wide(text);
        let mut rect = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: if max_width > 0 { max_width } else { 4000 },
            bottom: 0,
        };
        DrawTextW(dc, &mut wide_text, &mut rect, DT_CALCRECT | DT_WORDBREAK);
        let width = (rect.right - rect.left).max(1) + 2;
        let height = (rect.bottom - rect.top).max(size) + 2;

        // Пересоздаём секцию под точный размер.
        SelectObject(dc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);

        render_at(text, size, bold, width, height)
    }
}

#[cfg(windows)]
fn render_at(text: &str, size: i32, bold: bool, width: i32, height: i32) -> Mask {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::RECT;
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, CreateFontW, DeleteDC, DeleteObject, DrawTextW,
        GetDC, ReleaseDC, SelectObject, SetBkColor, SetTextColor, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS, DT_WORDBREAK,
        FF_DONTCARE, FW_BOLD, FW_NORMAL, HGDIOBJ, OUT_TT_PRECIS, PROOF_QUALITY,
    };

    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            ReleaseDC(None, screen);
            return Mask::empty();
        };
        let old_bitmap = SelectObject(dc, HGDIOBJ(bitmap.0));

        let font = CreateFontW(
            -size,
            0,
            0,
            0,
            if bold { FW_BOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_TT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            PROOF_QUALITY,
            DEFAULT_PITCH.0 as u32 | FF_DONTCARE.0 as u32,
            PCWSTR(wide("Segoe UI").as_ptr()),
        );
        let old_font = SelectObject(dc, HGDIOBJ(font.0));

        // Чёрный фон, белый текст — по яркости получим альфу.
        SetBkColor(dc, windows::Win32::Foundation::COLORREF(0));
        SetTextColor(dc, windows::Win32::Foundation::COLORREF(0xFFFFFF));

        let mut wide_text = wide(text);
        let mut rect = RECT { left: 1, top: 1, right: width - 1, bottom: height - 1 };
        DrawTextW(dc, &mut wide_text, &mut rect, DT_WORDBREAK);

        // Читаем пиксели из секции.
        let mut mask = Mask {
            width: width as u32,
            height: height as u32,
            values: vec![0; (width * height) as usize],
        };
        if !bits.is_null() {
            let pixels = std::slice::from_raw_parts(bits as *const u8, (width * height * 4) as usize);
            for i in 0..(width * height) as usize {
                // BGRA → берём синий канал как яркость (белый текст).
                mask.values[i] = pixels[i * 4];
            }
        }

        SelectObject(dc, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));
        SelectObject(dc, old_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        mask
    }
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Вне Windows текста на экране нет — возвращаем пустую маску.
#[cfg(not(windows))]
pub fn render(_text: &str, _size: i32, _bold: bool, _max_width: i32) -> Mask {
    Mask::empty()
}

/// Сколько строк занимает текст при данной ширине (для высоты окна).
pub fn line_count(text: &str, max_width: i32, font_size: i32) -> u32 {
    if text.trim().is_empty() {
        return 0;
    }
    let per_char = (font_size as f32 * 0.52).max(4.0);
    let per_line = (max_width as f32 / per_char).max(1.0);
    let mut lines = 0.0f32;
    for paragraph in text.split('\n') {
        let chars = paragraph.chars().count() as f32;
        lines += (chars / per_line).ceil().max(1.0);
    }
    lines.round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_wrapped_lines() {
        assert_eq!(line_count("", 300, 16), 0);
        assert_eq!(line_count("привет", 300, 16), 1);
        assert!(line_count(&"слово ".repeat(40), 300, 16) > 1);
    }

    #[test]
    fn empty_text_gives_empty_mask() {
        let mask = render("", 16, false, 300);
        assert_eq!(mask.width, 0);
    }
}
