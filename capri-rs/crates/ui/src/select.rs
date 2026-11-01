//! Выделение области экрана для перевода.
//!
//! Поверх всего экрана показывается полупрозрачная шторка, пользователь тянет
//! по ней мышь, и получается прямоугольник — тот самый, что уйдёт в
//! `capture_region` и дальше в OCR.
//!
//! Почему отдельное окно, а не «просто попросить координаты»: без шторки
//! непонятно, что программа вообще что-то ждёт, а промах мышью выглядит как
//! «ничего не произошло». Шторка закрывает вопрос: пока она на экране, режим
//! выделения включён, и `Esc` из него выходит.
//!
//! Окно живёт в потоке, который вызвал [`select_region`]: цикл сообщений
//! блокирует этот поток до конца выделения. Поэтому звать её нужно из рабочего
//! потока, а не из того, что рисует аватар.

/// Что получилось из выделения.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    /// Пользователь выделил область.
    Region { x: i32, y: i32, width: i32, height: i32 },
    /// Пользователь отказался (`Esc`, правая кнопка, слишком маленькая рамка).
    Cancelled,
}

/// Меньше этого выделение считается случайным кликом.
const MIN_SIDE: i32 = 4;

/// Спросить у пользователя область экрана.
///
/// Функция блокирует вызвавший поток до конца выделения. `None` — шторку не
/// удалось создать (например, сеанс без доступа к рабочему столу); это не
/// отмена пользователем, и панель должна показать именно сбой.
pub fn select_region() -> Option<Pick> {
    #[cfg(windows)]
    {
        imp::select_region()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Закрыть шторку, если она открыта. Безопасно звать из любого потока.
pub fn cancel() {
    #[cfg(windows)]
    imp::cancel();
}

#[cfg(not(windows))]
pub fn cancel() {}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::OnceLock;

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, VK_ESCAPE};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW,
        GetSystemMetrics, GetWindowLongPtrW, LoadCursorW, PostMessageW, PostQuitMessage,
        RegisterClassW,
        SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TranslateMessage, CREATESTRUCTW,
        CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, IDC_CROSS, MSG, SM_CXVIRTUALSCREEN,
        SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_SHOW, WM_CLOSE, WM_CREATE,
        WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN,
        WM_SYSKEYDOWN, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    };

    use super::{Pick, MIN_SIDE};
    use crate::canvas::Canvas;
    use crate::layered;

    /// Окно шторки: нужно, чтобы закрыть её из другого потока.
    static OVERLAY: AtomicIsize = AtomicIsize::new(0);
    /// Класс окна регистрируется один раз на процесс.
    static CLASS: OnceLock<()> = OnceLock::new();

    /// Имя класса окна — общее для регистрации и создания.
    const CLASS_NAME: &str = "CapriRegionOverlay";

    /// Данные, которые живут всё время показа окна.
    struct State {
        /// Начало рамки (экранные координаты).
        start: POINT,
        /// Текущий угол рамки (экранные координаты).
        current: POINT,
        /// Тянем ли мышь прямо сейчас.
        dragging: bool,
        /// Где стоит окно: экранные координаты его левого верхнего угла.
        origin: POINT,
        /// Итог выделения. `None` — окно ещё открыто.
        pick: Option<Pick>,
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Границы виртуального экрана: (`x`, `y`, ширина, высота).
    ///
    /// Виртуальный экран — объединение всех мониторов. Шторка накрывает его
    /// целиком: иначе на втором мониторе выделять было бы нечего.
    fn virtual_screen() -> (i32, i32, i32, i32) {
        unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN),
                GetSystemMetrics(SM_CYVIRTUALSCREEN),
            )
        }
    }

    /// Прямоугольник выделения в экранных координатах.
    fn current_rect(state: &State) -> RECT {
        RECT {
            left: state.start.x.min(state.current.x),
            top: state.start.y.min(state.current.y),
            right: state.start.x.max(state.current.x),
            bottom: state.start.y.max(state.current.y),
        }
    }

    /// Перерисовать шторку: затемнение, вырезанная область под курсором, рамка.
    ///
    /// Затемнение — 45 % чёрного. Сильнее — не видно, что выделяешь; слабее —
    /// непонятно, что режим вообще включён.
    fn repaint(hwnd: HWND, state: &State, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        let mut canvas = Canvas::new(width, height);
        for y in 0..height {
            for x in 0..width {
                canvas.blend(x, y, 0, 0, 0, 0.45);
            }
        }

        if state.dragging {
            let rect = current_rect(state);
            let left = rect.left - state.origin.x;
            let top = rect.top - state.origin.y;
            let rect_width = rect.right - rect.left;
            let rect_height = rect.bottom - rect.top;

            // Внутри рамки — снова прозрачно: человек должен видеть исходный
            // текст, иначе непонятно, что попадёт в перевод.
            for y in top.max(0)..(top + rect_height).min(height as i32) {
                for x in left.max(0)..(left + rect_width).min(width as i32) {
                    canvas.pixels[(y as u32 * width + x as u32) as usize] = 0;
                }
            }

            // Рамка из двух линий: белую видно на тёмном фоне, синюю — на белом.
            draw_rect_outline(&mut canvas, left - 1, top - 1, rect_width + 2, rect_height + 2, [255, 255, 255]);
            draw_rect_outline(&mut canvas, left - 2, top - 2, rect_width + 4, rect_height + 4, [40, 160, 255]);
        }

        layered::blit(hwnd, &canvas, state.origin.x, state.origin.y);
    }

    /// Обвести прямоугольник по контуру, не заливая его.
    fn draw_rect_outline(
        canvas: &mut Canvas,
        left: i32,
        top: i32,
        width: i32,
        height: i32,
        color: [u8; 3],
    ) {
        let right = left + width;
        let bottom = top + height;
        for x in left..=right {
            canvas.blend(x.max(0) as u32, top.max(0) as u32, color[0], color[1], color[2], 1.0);
            canvas.blend(x.max(0) as u32, bottom.max(0) as u32, color[0], color[1], color[2], 1.0);
        }
        for y in top..=bottom {
            canvas.blend(left.max(0) as u32, y.max(0) as u32, color[0], color[1], color[2], 1.0);
            canvas.blend(right.max(0) as u32, y.max(0) as u32, color[0], color[1], color[2], 1.0);
        }
    }

    /// Точка из `lparam` сообщения мыши (младшее слово — x, старшее — y).
    ///
    /// Координаты там знаковые 16-битные, поэтому при отрицательном смещении
    /// монитора они приходят «большими» беззнаковыми числами.
    fn lparam_point(lparam: LPARAM) -> (i32, i32) {
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        (x, y)
    }

    /// Перевести клиентскую точку в экранную.
    fn to_screen(state: &State, x: i32, y: i32) -> POINT {
        POINT { x: state.origin.x + x, y: state.origin.y + y }
    }

    /// Запомнить итог и закрыть окно.
    ///
    /// Кроме самого окна надо разбудить цикл сообщений: он ждёт `GetMessageW`
    /// без фильтра по окну, а `DestroyWindow` не кладёт в очередь `WM_QUIT` —
    /// без `PostQuitMessage` функция выделения не вернула бы результат никогда.
    fn finish(hwnd: HWND, state: *mut State, pick: Pick) {
        if !state.is_null() {
            unsafe {
                (*state).pick = Some(pick);
            }
        }
        unsafe {
            let _ = DestroyWindow(hwnd);
            PostQuitMessage(0);
        }
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;

        match message {
            WM_CREATE => {
                let create = lparam.0 as *const CREATESTRUCTW;
                if !create.is_null() {
                    let state = (*create).lpCreateParams as *mut State;
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if state_ptr.is_null() {
                    return LRESULT(0);
                }
                let state = &mut *state_ptr;
                let (x, y) = lparam_point(lparam);
                state.start = to_screen(state, x, y);
                state.current = state.start;
                state.dragging = true;
                SetCapture(hwnd);
                let (_, _, width, height) = virtual_screen();
                repaint(hwnd, state, width as u32, height as u32);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if state_ptr.is_null() {
                    return LRESULT(0);
                }
                let state = &mut *state_ptr;
                if state.dragging {
                    let (x, y) = lparam_point(lparam);
                    state.current = to_screen(state, x, y);
                    let (_, _, width, height) = virtual_screen();
                    repaint(hwnd, state, width as u32, height as u32);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if state_ptr.is_null() {
                    return LRESULT(0);
                }
                let state = &mut *state_ptr;
                if !state.dragging {
                    return LRESULT(0);
                }
                state.dragging = false;
                let _ = ReleaseCapture();

                // Финальную точку берём у самой мыши, а не из последнего
                // `WM_MOUSEMOVE`: сообщение о движении может не дойти (очередь
                // переполнена, окно под нагрузкой), и тогда рамка «схлопывается»
                // в точку — человек протянул область, а получил отмену. Позиция
                // курсора в момент отпускания верна всегда.
                let mut cursor = POINT::default();
                if GetCursorPos(&mut cursor).is_ok() {
                    state.current = cursor;
                }

                let rect = current_rect(state);
                let picked_width = rect.right - rect.left;
                let picked_height = rect.bottom - rect.top;
                // Случайное шевеление мышью — не выделение: пусть человек
                // повторит, чем получить перевод одной буквы. Тот же порог, что
                // у `capture_region`: меньше 4×4 распознавать нечего.
                let pick = if picked_width >= MIN_SIDE && picked_height >= MIN_SIDE {
                    Pick::Region {
                        x: rect.left,
                        y: rect.top,
                        width: picked_width,
                        height: picked_height,
                    }
                } else {
                    Pick::Cancelled
                };
                finish(hwnd, state_ptr, pick);
                LRESULT(0)
            }
            WM_RBUTTONDOWN => {
                finish(hwnd, state_ptr, Pick::Cancelled);
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                if wparam.0 == VK_ESCAPE.0 as usize {
                    finish(hwnd, state_ptr, Pick::Cancelled);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // Окно могли закрыть и помимо наших обработчиков (например, при
                // выходе из программы) — тогда выделения не будет. Цикл
                // сообщений тоже надо разбудить, иначе он останется ждать.
                if !state_ptr.is_null() && (*state_ptr).pick.is_none() {
                    (*state_ptr).pick = Some(Pick::Cancelled);
                }
                OVERLAY.store(0, Ordering::SeqCst);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    fn ensure_class() -> Result<(), String> {
        if CLASS.get().is_some() {
            return Ok(());
        }
        unsafe {
            let instance =
                GetModuleHandleW(None).map_err(|e| format!("не смогла получить модуль: {e}"))?;
            let class_name = wide(CLASS_NAME);
            let cursor = LoadCursorW(None, IDC_CROSS).unwrap_or_default();
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: cursor,
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                return Err("класс окна выделения не зарегистрировался".into());
            }
            let _ = CLASS.set(());
        }
        Ok(())
    }

    pub fn select_region() -> Option<Pick> {
        ensure_class().ok()?;
        let (x, y, width, height) = virtual_screen();
        if width <= 0 || height <= 0 {
            return None;
        }

        // Состояние живёт на стеке этого потока: окно закрывается раньше, чем
        // функция вернёт управление, поэтому указатель не переживёт её.
        let mut state = State {
            start: POINT { x: 0, y: 0 },
            current: POINT { x: 0, y: 0 },
            dragging: false,
            origin: POINT { x, y },
            pick: None,
        };

        unsafe {
            let instance = GetModuleHandleW(None).ok()?;
            let class_name = wide(CLASS_NAME);
            let title = wide("Capri — выделение области");
            let hwnd = CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_POPUP,
                x,
                y,
                width,
                height,
                None,
                None,
                Some(instance.into()),
                Some(&mut state as *mut State as *mut std::ffi::c_void),
            )
            .ok()?;

            OVERLAY.store(hwnd.0 as isize, Ordering::SeqCst);

            repaint(hwnd, &state, width as u32, height as u32);
            let _ = ShowWindow(hwnd, SW_SHOW);
            // Фокус нужен для `Esc`: без него клавиша уйдёт в окно под шторкой.
            let _ = SetForegroundWindow(hwnd);

            let mut message = MSG::default();
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }

            state.pick
        }
    }

    pub fn cancel() {
        let raw = OVERLAY.swap(0, Ordering::SeqCst);
        if raw != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(raw as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_and_region_are_different() {
        assert_ne!(Pick::Cancelled, Pick::Region { x: 0, y: 0, width: 10, height: 10 });
    }

    #[test]
    fn pick_stores_region_as_given() {
        let pick = Pick::Region { x: 100, y: 200, width: 300, height: 40 };
        match pick {
            Pick::Region { x, y, width, height } => {
                assert_eq!((x, y, width, height), (100, 200, 300, 40));
            }
            Pick::Cancelled => panic!("ожидалась область"),
        }
    }

    #[test]
    fn tiny_side_is_rejected() {
        // Порог должен совпадать с тем, что считает `capture_region` пригодным.
        assert_eq!(MIN_SIDE, 4);
    }
}
