//! Общая операция «отдать кадр слоистому окну».
//!
//! И аватар, и радиальное меню рисуют кадр в памяти ([`Canvas`]) и отдают его
//! окну одним вызовом `UpdateLayeredWindow`. Попиксельная прозрачность — его
//! работа, не наша: поэтому окна не мигают и не показывают чёрных углов.

use crate::canvas::Canvas;

#[cfg(windows)]
pub fn blit(hwnd: windows::Win32::Foundation::HWND, canvas: &Canvas, x: i32, y: i32) {
    use windows::Win32::Foundation::{COLORREF, POINT, SIZE};
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
        AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
        HBITMAP, HGDIOBJ,
    };
    use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindow, ULW_ALPHA};

    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: canvas.width as i32,
                biHeight: -(canvas.height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap: HBITMAP = match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) => bitmap,
            Err(_) => {
                let _ = DeleteDC(dc);
                ReleaseDC(None, screen);
                return;
            }
        };
        let old = SelectObject(dc, HGDIOBJ(bitmap.0));
        if !bits.is_null() {
            let bytes = canvas.as_bytes();
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), bits as *mut u8, bytes.len());
        }

        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let size = SIZE { cx: canvas.width as i32, cy: canvas.height as i32 };
        let source = POINT { x: 0, y: 0 };
        let dest = POINT { x, y };
        let _ = UpdateLayeredWindow(
            hwnd,
            Some(screen),
            Some(&dest),
            Some(&size),
            Some(dc),
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );

        SelectObject(dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
    }
}
