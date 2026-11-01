//! Распознавание текста на картинке: что видит OCR Капри.
//!
//! Запуск: `cargo run -p capri-win --example ocr_demo`
//!
//! Пример полезен, когда «перевод не работает»: он показывает сразу и какие
//! языки установлены в системе, и что именно распознаётся. Текст рисуется
//! системным шрифтом через GDI, поэтому проверка идёт по тому же пути, что и
//! панель перевода: DIB → RGBA → WinRT OCR.

use windows::core::PCWSTR;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush, DeleteDC,
    DeleteObject, DrawTextW, FillRect, GetDC, GetDIBits, ReleaseDC, SelectObject, SetBkMode,
    SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CLEARTYPE_QUALITY, DEFAULT_CHARSET,
    DEFAULT_PITCH, DIB_RGB_COLORS, DT_LEFT, DT_TOP, FW_BOLD, HGDIOBJ, OUT_DEFAULT_PRECIS,
    CLIP_DEFAULT_PRECIS, TRANSPARENT,
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Нарисовать строки текста системным шрифтом на белом фоне.
fn render(lines: &[&str], width: u32, height: u32) -> image::RgbaImage {
    unsafe {
        let screen = GetDC(None);
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width as i32, height as i32);
        let old = SelectObject(mem, HGDIOBJ(bitmap.0));

        let mut rect = windows::Win32::Foundation::RECT {
            left: 0,
            top: 0,
            right: width as i32,
            bottom: height as i32,
        };
        let brush = CreateSolidBrush(windows::Win32::Foundation::COLORREF(0x00FF_FFFF));
        FillRect(mem, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));

        let font = CreateFontW(
            -44,
            0,
            0,
            0,
            FW_BOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            PCWSTR(wide("Segoe UI").as_ptr()),
        );
        let old_font = SelectObject(mem, HGDIOBJ(font.0));
        SetBkMode(mem, TRANSPARENT);
        SetTextColor(mem, windows::Win32::Foundation::COLORREF(0x000000));

        let mut y = 10;
        for line in lines {
            let mut text = wide(line);
            let len = text.len() - 1;
            rect.left = 20;
            rect.top = y;
            rect.right = width as i32 - 20;
            rect.bottom = height as i32;
            DrawTextW(mem, &mut text[..len], &mut rect, DT_LEFT | DT_TOP);
            y += 60;
        }

        SelectObject(mem, old_font);
        let _ = DeleteObject(HGDIOBJ(font.0));

        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width as i32;
        info.bmiHeader.biHeight = -(height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;

        let mut buffer = vec![0u8; (width * height * 4) as usize];
        let _ = GetDIBits(
            mem,
            bitmap,
            0,
            height as u32,
            Some(buffer.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );

        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        for px in buffer.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        image::RgbaImage::from_raw(width, height, buffer).unwrap_or_else(|| {
            image::RgbaImage::from_pixel(width, height, image::Rgba([255, 255, 255, 255]))
        })
    }
}

fn main() {
    println!("=== что умеет OCR на этой машине ===");
    match capri_win::ocr::capability() {
        Ok(capability) => {
            println!("языки: {:?}", capability.languages);
            println!("русский: {}", capability.has_russian());
            println!("максимальный размер картинки: {}", capability.max_dimension);
            println!("выбранный по умолчанию: {:?}", capability.pick(None));
            if !capability.has_russian() {
                println!(
                    "ПОДСКАЗКА: русского распознавания нет. Параметры → Время и язык → \
                     Язык и регион → добавить русский → Дополнительные компоненты → \
                     «Оптическое распознавание символов»."
                );
            }
        }
        Err(error) => println!("не смогла спросить: {error}"),
    }

    let image = render(&["Just an ordinary developer", "Привет, мир"], 900, 160);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("ocr_sample.png");
    match image.save(&path) {
        Ok(()) => println!("\nобразец: {}", path.display()),
        Err(error) => println!("\nне смогла сохранить образец: {error}"),
    }

    for (label, wanted) in [("русский", None), ("английский", Some("en-US"))] {
        println!("\n=== {label} ===");
        match capri_win::ocr::recognize(&image, wanted) {
            Ok(recognized) => {
                println!("язык: {}, строк: {}", recognized.language, recognized.lines.len());
                println!("--- текст ---\n{}", recognized.text());
            }
            Err(error) => println!("ошибка: {error}"),
        }
    }
}
