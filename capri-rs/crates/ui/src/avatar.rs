//! Слоистое окно аватара: прозрачный фон, подпись, перетаскивание.
//!
//! Окно рисуется целиком в память и отдаётся системе одним вызовом
//! `UpdateLayeredWindow` — так работает попиксельная прозрачность без
//! мерцания и без мерцающих углов.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::Sender;

use parking_lot::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HGDIOBJ, HBITMAP,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW,
    GetSystemMetrics, GetWindowThreadProcessId, LoadIconW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TrackPopupMenu,
    TranslateMessage, UpdateLayeredWindow, CS_HREDRAW, CS_VREDRAW, GWL_EXSTYLE,
    IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG, SM_CXSCREEN, SM_CYSCREEN,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, ULW_ALPHA, WM_APP, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NULL, WM_RBUTTONUP, WM_TIMER, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, SetWindowLongPtrW,
};

use crate::canvas::{Canvas, Mask};
use crate::sprite::{Sprites, FIG_HEIGHT, HEIGHT, WIDTH};
use crate::text;
use crate::UiRequest;

/// Сообщение «обновить картинку» — шлём сами себе из таймера.
const WM_CAPRI_TICK: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;
/// Как часто перерисовываем (мс).
const TICK_MS: u32 = 40;
/// На сколько пикселей надо сдвинуть мышь, чтобы это считалось
/// перетаскиванием, а не кликом.
const DRAG_THRESHOLD: i32 = 4;
/// Шрифт подписи.
const SUBTITLE_SIZE: i32 = 17;

/// Состояние окна. Живёт в статике: обработчик сообщений — обычная функция C.
struct State {
    hwnd: HWND,
    canvas: Canvas,
    sprites: Sprites,
    /// Куда двигаться по X (для прогулки).
    vx: i32,
    /// Какая анимация играет и на каком кадре.
    animation: Option<Anim>,
    /// Масштаб прозрачности при появлении/исчезании.
    opacity: f32,
    target_opacity: f32,
    /// Подпись под фигурой.
    subtitle: String,
    /// Цвет скина.
    skin: Option<[u8; 3]>,
    /// Текущая позиция окна.
    x: i32,
    y: i32,
    /// Тащим ли окно мышью.
    dragging: bool,
    drag_offset: (i32, i32),
    /// Сдвинулась ли мышь с момента нажатия. Нужно, чтобы отличить клик
    /// («позвать Капри») от перетаскивания.
    drag_moved: bool,
    /// Меню в трее.
    requests: Sender<UiRequest>,
    /// Скрыто ли окно.
    hidden: bool,
    /// Позиция для прогулки: цель по X.
    roam_target: Option<i32>,
}

struct Anim {
    frames: Frames,
    index: usize,
    /// Сколько тиков на кадр.
    hold: u32,
    counter: u32,
}

#[derive(Clone, Copy, PartialEq)]
enum Frames {
    Wave,
    Eyes,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static QUIT: AtomicBool = AtomicBool::new(false);
static CLASS_ATOM: AtomicIsize = AtomicIsize::new(0);

/// Открыто ли меню прямо сейчас.
///
/// `TrackPopupMenu` не возвращает управление, пока меню не закрыто: он крутит
/// собственный цикл сообщений и внутри него вызывает наш же `wndproc`. Туда
/// приходит `WM_TIMER`, `tick()` берёт `STATE` — и если мьютекс всё ещё
/// держит тот, кто открыл меню, поток блокирует сам себя навсегда.
/// Поэтому: мьютекс отпускаем до открытия меню, а это флаг не даёт открыть
/// второе меню поверх первого.
static MENU_OPEN: AtomicBool = AtomicBool::new(false);

/// Пропуск в меню: кто взял, тот и закрывает (в том числе при панике).
struct MenuGuard;

impl MenuGuard {
    fn try_enter() -> Option<Self> {
        if MENU_OPEN.swap(true, Ordering::AcqRel) {
            None
        } else {
            Some(Self)
        }
    }
}

impl Drop for MenuGuard {
    fn drop(&mut self) {
        MENU_OPEN.store(false, Ordering::Release);
    }
}

// Указатель окна Windows трогаем только под мьютексом и только из
// потока-владельца окна, поэтому такая передача безопасна.
unsafe impl Send for State {}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Запустить окно. Возвращает код выхода. Блокирует поток.
pub fn run(sprites: Sprites, requests: Sender<UiRequest>) -> i32 {
    unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(handle) => handle,
            Err(e) => {
                log::error!("Не получила дескриптор модуля: {e}");
                return 1;
            }
        };

        let class_name = wide("CapriAvatarWindow");
        let wc = windows::Win32::UI::WindowsAndMessaging::WNDCLASSEXW {
            cbSize: std::mem::size_of::<windows::Win32::UI::WindowsAndMessaging::WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            log::error!("Не смогла зарегистрировать класс окна");
            return 1;
        }
        CLASS_ATOM.store(atom as isize, Ordering::Relaxed);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);
        let x = screen_w - WIDTH as i32 - 24;
        let y = screen_h - HEIGHT as i32 - 48;

        let hwnd = match CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WS_POPUP,
            x,
            y,
            WIDTH as i32,
            HEIGHT as i32,
            None,
            None,
            Some(instance.into()),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(e) => {
                log::error!("Не смогла создать окно: {e}");
                return 1;
            }
        };

        {
            let mut state = State {
                hwnd,
                canvas: Canvas::new(WIDTH, HEIGHT),
                sprites,
                vx: 0,
                animation: None,
                opacity: 0.0,
                target_opacity: 1.0,
                subtitle: String::new(),
                skin: None,
                x,
                y,
                dragging: false,
                drag_offset: (0, 0),
                drag_moved: false,
                requests,
                hidden: false,
                roam_target: None,
            };
            // Первый кадр рисуем сразу, чтобы окно не мигало пустотой.
            paint(&mut state);
            *STATE.lock() = Some(state);
        }

        add_tray_icon(hwnd, windows::Win32::Foundation::HINSTANCE(instance.0));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            WIDTH as i32,
            HEIGHT as i32,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
        SetTimer(Some(hwnd), TIMER_ID, TICK_MS, None);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        remove_tray_icon(hwnd);
        let _ = DestroyWindow(hwnd);
        message.wParam.0 as i32
    }
}

/// Применить событие из ядра к окну.
pub fn push_event(event: crate::UiEvent) {
    let mut guard = STATE.lock();
    let Some(state) = guard.as_mut() else { return };
    match event {
        crate::UiEvent::Summon => {
            state.hidden = false;
            state.target_opacity = 1.0;
            if state.opacity <= 0.01 {
                state.opacity = 0.0;
            }
            unsafe {
                let _ = ShowWindow(state.hwnd, SW_SHOW);
            }
        }
        crate::UiEvent::Vanish => {
            state.hidden = true;
            state.target_opacity = 0.0;
        }
        crate::UiEvent::Mode(mode, _secs) => {
            state.animation = match mode.as_str() {
                "thinking" | "speaking" => Some(Anim {
                    frames: Frames::Eyes,
                    index: 0,
                    hold: 2,
                    counter: 0,
                }),
                _ => None,
            };
        }
        crate::UiEvent::Subtitle(text) => {
            state.subtitle = text;
        }
        crate::UiEvent::PlayAnim(kind) => {
            state.animation = Some(Anim {
                frames: if kind == "eyes" { Frames::Eyes } else { Frames::Wave },
                index: 0,
                hold: if kind == "eyes" { 2 } else { 1 },
                counter: 0,
            });
        }
        crate::UiEvent::Roam => {
            // Идём к случайной точке неподалёку.
            let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
            let span = (screen_w - WIDTH as i32).max(1);
            let target = (state.x + (rand_offset() * 300.0) as i32).clamp(0, span);
            state.roam_target = Some(target);
            state.vx = if target > state.x { 6 } else { -6 };
        }
        crate::UiEvent::Hop => {
            let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
            let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
            let target_x = (state.x + (rand_offset() * 400.0) as i32)
                .clamp(0, (screen_w - WIDTH as i32).max(1));
            let target_y = (state.y + (rand_offset() * 200.0) as i32)
                .clamp(0, (screen_h - HEIGHT as i32).max(1));
            state.x = target_x;
            state.y = target_y;
            state.roam_target = None;
            state.vx = 0;
            apply_position(state);
        }
        crate::UiEvent::MoveTo(where_) => {
            let screen_w = unsafe { GetSystemMetrics(SM_CXSCREEN) };
            let screen_h = unsafe { GetSystemMetrics(SM_CYSCREEN) };
            let (x, y) = corner_position(&where_, screen_w, screen_h);
            state.x = x;
            state.y = y;
            state.roam_target = None;
            state.vx = 0;
            apply_position(state);
        }
        crate::UiEvent::SetSkin(color) => {
            state.skin = color;
        }
    }
    paint(state);
}

/// Переместить окно на новые координаты.
fn apply_position(state: &mut State) {
    unsafe {
        let _ = SetWindowPos(
            state.hwnd,
            None,
            state.x,
            state.y,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOSIZE,
        );
    }
}

fn corner_position(where_: &str, screen_w: i32, screen_h: i32) -> (i32, i32) {
    let right = (screen_w - WIDTH as i32).max(0);
    let bottom = (screen_h - HEIGHT as i32).max(0);
    let center_x = right / 2;
    let center_y = bottom / 2;
    match where_ {
        "left" => (0, bottom),
        "right" => (right, bottom),
        "top" => (right, 0),
        "bottom" => (right, bottom),
        "center" => (center_x, center_y),
        "left-top" => (0, 0),
        "left-bottom" => (0, bottom),
        "right-top" => (right, 0),
        "right-bottom" => (right, bottom),
        "top-left" => (0, 0),
        "top-right" => (right, 0),
        "bottom-left" => (0, bottom),
        "bottom-right" => (right, bottom),
        _ => (right, bottom),
    }
}

fn rand_offset() -> f32 {
    // Простое псевдослучайное число от времени: для прогулок хватает.
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 2000) as f32 / 1000.0 - 1.0
}

/// Откуда взять текущий кадр.
#[derive(Clone, Copy, PartialEq)]
enum Choice {
    Pose,
    Wave(usize),
    Eyes(usize),
    Roam,
}

/// Что показать в этом кадре (без заимствований — их берём в `paint`).
fn frame_choice(state: &State) -> Choice {
    if let Some(anim) = state.animation.as_ref() {
        match anim.frames {
            Frames::Wave if !state.sprites.wave.is_empty() => {
                return Choice::Wave(anim.index.min(state.sprites.wave.len() - 1));
            }
            Frames::Eyes if !state.sprites.eyes.is_empty() => {
                return Choice::Eyes(anim.index.min(state.sprites.eyes.len() - 1));
            }
            _ => {}
        }
    }
    if state.roam_target.is_some() && state.sprites.roam.is_some() {
        return Choice::Roam;
    }
    Choice::Pose
}

/// Собрать кадр и отдать его окну.
fn paint(state: &mut State) {
    let width = WIDTH;
    if state.canvas.width != width || state.canvas.height != HEIGHT {
        state.canvas = Canvas::new(width, HEIGHT);
    }

    let choice = frame_choice(state);
    let skin = state.skin;
    let opacity = state.opacity;
    let subtitle = state.subtitle.clone();

    // Берём поля по отдельности: иначе нельзя одновременно читать спрайты
    // и писать в холст.
    let State { canvas, sprites, .. } = state;
    canvas.clear();

    let frame = match choice {
        Choice::Pose => sprites.default_frame(),
        Choice::Wave(index) => sprites.wave.get(index),
        Choice::Eyes(index) => sprites.eyes.get(index),
        Choice::Roam => sprites.roam.as_ref().or_else(|| sprites.default_frame()),
    };

    if let Some(image) = frame {
        let image = match skin {
            Some(tint) => image.tinted(tint),
            None => image.clone(),
        };
        let left = (width as i32 - image.width as i32) / 2;
        let top = (FIG_HEIGHT as i32 - image.height as i32).max(0) + 4;
        canvas.draw_image(&image, left, top, opacity);
    }

    // Подпись под фигурой.
    if !subtitle.trim().is_empty() && opacity > 0.02 {
        let max_width = width as i32 - 24;
        let mask: Mask = text::render(&subtitle, SUBTITLE_SIZE, false, max_width);
        if mask.width > 0 {
            let padding = 10;
            let box_w = mask.width as i32 + padding * 2;
            let box_h = mask.height as i32 + padding;
            let box_x = (width as i32 - box_w) / 2;
            let box_y = FIG_HEIGHT as i32 + 6;
            canvas.round_rect(box_x, box_y, box_w, box_h, 10, [10, 8, 18], 0.82 * opacity);
            canvas.draw_mask(&mask, box_x + padding, box_y + padding / 2, [120, 240, 255], opacity);
        }
    }

    blit(state);
}

/// Отправить пиксели в окно.
fn blit(state: &mut State) {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut info = BITMAPINFO::default();
        info.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: state.canvas.width as i32,
            biHeight: -(state.canvas.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let bitmap: HBITMAP =
            match CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(bitmap) => bitmap,
                Err(_) => {
                    let _ = DeleteDC(dc);
                    ReleaseDC(None, screen);
                    return;
                }
            };
        let old = SelectObject(dc, HGDIOBJ(bitmap.0));
        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(state.canvas.as_bytes().as_ptr(), bits as *mut u8, state.canvas.as_bytes().len());
        }

        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let size = SIZE { cx: state.canvas.width as i32, cy: state.canvas.height as i32 };
        let source = POINT { x: 0, y: 0 };
        let dest = POINT { x: state.x, y: state.y };
        let _ = UpdateLayeredWindow(
            state.hwnd,
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

fn tick() {
    let mut guard = STATE.lock();
    let Some(state) = guard.as_mut() else { return };

    let mut needs_paint = false;

    // Плавное появление и исчезание.
    if (state.opacity - state.target_opacity).abs() > 0.01 {
        let step = 0.08;
        state.opacity += (state.target_opacity - state.opacity).clamp(-step, step);
        needs_paint = true;
    } else if state.opacity != state.target_opacity {
        state.opacity = state.target_opacity;
        needs_paint = true;
    }
    if state.hidden && state.opacity <= 0.01 {
        unsafe {
            let _ = ShowWindow(state.hwnd, SW_HIDE);
        }
    }

    // Анимация.
    if let Some(anim) = state.animation.as_mut() {
        anim.counter += 1;
        if anim.counter >= anim.hold {
            anim.counter = 0;
            anim.index += 1;
            let len = match anim.frames {
                Frames::Wave => state.sprites.wave.len(),
                Frames::Eyes => state.sprites.eyes.len(),
            };
            if anim.index >= len {
                anim.index = 0;
                // «Машет» проигрываем один раз.
                if anim.frames == Frames::Wave {
                    state.animation = None;
                }
            }
            needs_paint = true;
        }
    }

    // Прогулка.
    if let Some(target) = state.roam_target {
        if (state.x - target).abs() <= state.vx.abs().max(1) {
            state.x = target;
            state.roam_target = None;
            state.vx = 0;
            needs_paint = true;
        } else {
            state.x += state.vx;
        }
        apply_position(state);
    }

    if needs_paint {
        paint(state);
    }
}

/// Считать ли сдвиг мыши перетаскиванием, а не кликом.
fn counts_as_drag(dx: i32, dy: i32) -> bool {
    dx.abs() > DRAG_THRESHOLD || dy.abs() > DRAG_THRESHOLD
}

/// Сделать окно активным по-настоящему.
///
/// Обычный `SetForegroundWindow` из фонового процесса Windows просто
/// игнорирует — окно остаётся неактивным, а всплывающее меню в таком
/// состоянии не получает ни Esc, ни клика мимо и висит открытым.
/// Приём из документации: временно присоединиться к потоку текущего
/// активного окна, тогда система считает нас «своими».
unsafe fn force_foreground(hwnd: HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    if GetForegroundWindow() == hwnd {
        return;
    }
    let target_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
    let own_thread = GetCurrentThreadId();
    if target_thread == 0 || target_thread == own_thread {
        let _ = SetForegroundWindow(hwnd);
        return;
    }
    if AttachThreadInput(target_thread, own_thread, true).as_bool() {
        let _ = SetForegroundWindow(hwnd);
        let _ = AttachThreadInput(target_thread, own_thread, false);
    }
}

// ------------------------------------------------------------------ меню и трей

const MENU_CALL: usize = 1001;
const MENU_MIC_NEXT: usize = 1002;
const MENU_VOL_UP: usize = 1003;
const MENU_VOL_DOWN: usize = 1004;
const MENU_INTERRUPT: usize = 1005;
const MENU_CONFIG: usize = 1006;
const MENU_RELOAD: usize = 1007;
const MENU_QUIT: usize = 1008;

/// Что нужно меню: окно-владелец и канал команд.
///
/// Берём под мьютексом на одно мгновение и сразу отпускаем: держать замок
/// во время `TrackPopupMenu` нельзя (см. `MENU_OPEN`).
fn menu_target() -> Option<(HWND, Sender<UiRequest>)> {
    let guard = STATE.lock();
    let state = guard.as_ref()?;
    Some((state.hwnd, state.requests.clone()))
}

/// Показать меню. Мьютекс `STATE` здесь уже отпущен.
fn show_menu(hwnd: HWND, requests: &Sender<UiRequest>) {
    // Второе меню поверх первого не открываем — это лишний вложенный цикл.
    let Some(_guard) = MenuGuard::try_enter() else {
        return;
    };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let items: [(usize, &str); 8] = [
            (MENU_CALL, "Позвать Капри"),
            (MENU_INTERRUPT, "Замолчать"),
            (MENU_MIC_NEXT, "Следующий микрофон"),
            (MENU_VOL_UP, "Громче"),
            (MENU_VOL_DOWN, "Тише"),
            (MENU_RELOAD, "Перечитать настройки"),
            (MENU_CONFIG, "Открыть config.json"),
            (MENU_QUIT, "Выход"),
        ];
        for (id, label) in items {
            let text = wide(label);
            let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(text.as_ptr()));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);

        // Меню закроется по клику мимо и поймает Esc только если его окно-
        // владелец активно. Нашему окну это запрещает WS_EX_NOACTIVATE,
        // поэтому на время меню флаг снимаем и сразу возвращаем обратно.
        let base_style = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
        let previous = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, base_style.0 as isize);
        // Запоминаем, у кого был фокус: после меню вернём его обратно,
        // чтобы Капри не отбирала активность у рабочего окна.
        let previous_focus = GetForegroundWindow();
        force_foreground(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            None,
            hwnd,
            None,
        );
        // Штатный приём: без этого меню может остаться висеть, если его
        // закрыли снаружи.
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        // Возвращаем «не активировать», чтобы окно не забирало фокус.
        let restore = if previous == 0 {
            (base_style | WS_EX_NOACTIVATE).0 as isize
        } else {
            previous
        };
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, restore);
        if !previous_focus.0.is_null() && previous_focus != hwnd {
            let _ = SetForegroundWindow(previous_focus);
        }
        let _ = DestroyMenu(menu);

        match chosen.0 as usize {
            MENU_CALL => {
                let _ = requests.send(UiRequest::Call);
            }
            MENU_INTERRUPT => {
                let _ = requests.send(UiRequest::Interrupt);
            }
            MENU_MIC_NEXT => {
                let _ = requests.send(UiRequest::MicNext);
            }
            MENU_VOL_UP => {
                let _ = requests.send(UiRequest::SetVolume(80));
            }
            MENU_VOL_DOWN => {
                let _ = requests.send(UiRequest::SetVolume(35));
            }
            MENU_RELOAD => {
                let _ = requests.send(UiRequest::Reload);
            }
            MENU_CONFIG => {
                let _ = requests.send(UiRequest::OpenConfig);
            }
            MENU_QUIT => {
                let _ = requests.send(UiRequest::Quit);
                QUIT.store(true, Ordering::Relaxed);
                PostQuitMessage(0);
            }
            _ => {}
        }
    }
}

fn add_tray_icon(hwnd: HWND, instance: windows::Win32::Foundation::HINSTANCE) {
    unsafe {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_APP + 2,
            ..Default::default()
        };
        if let Ok(icon) = LoadIconW(Some(instance), PCWSTR(1 as *const u16)) {
            data.hIcon = icon;
        } else if let Ok(icon) = LoadIconW(None, IDI_APPLICATION) {
            data.hIcon = icon;
        }
        let tip = wide("Капри");
        let len = tip.len().min(data.szTip.len());
        data.szTip[..len].copy_from_slice(&tip[..len]);
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
    }
}

fn remove_tray_icon(hwnd: HWND) {
    unsafe {
        let data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            ..Default::default()
        };
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

// ------------------------------------------------------------------ сообщения

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            if wparam.0 == TIMER_ID {
                tick();
            }
            LRESULT(0)
        }
        WM_CAPRI_TICK => {
            tick();
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let mut guard = STATE.lock();
            if let Some(state) = guard.as_mut() {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                state.dragging = true;
                state.drag_moved = false;
                state.drag_offset = (point.x - state.x, point.y - state.y);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let mut guard = STATE.lock();
            if let Some(state) = guard.as_mut() {
                if state.dragging {
                    let mut point = POINT::default();
                    let _ = GetCursorPos(&mut point);
                    let dx = point.x - state.drag_offset.0 - state.x;
                    let dy = point.y - state.drag_offset.1 - state.y;
                    // Дрожание руки за перетаскивание не считаем.
                    if counts_as_drag(dx, dy) {
                        state.drag_moved = true;
                    }
                    state.x = point.x - state.drag_offset.0;
                    state.y = point.y - state.drag_offset.1;
                    state.roam_target = None;
                    apply_position(state);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let mut guard = STATE.lock();
            if let Some(state) = guard.as_mut() {
                let moved = state.drag_moved;
                state.dragging = false;
                state.drag_moved = false;
                if !moved {
                    // Клик без движения — позвать Капри.
                    let _ = state.requests.send(UiRequest::Call);
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            // Меню открываем, уже отпустив STATE: внутри TrackPopupMenu
            // приходит WM_TIMER, а tick() снова берёт мьютекс.
            if let Some((hwnd, requests)) = menu_target() {
                show_menu(hwnd, &requests);
            }
            LRESULT(0)
        }
        // Клик по иконке в трее.
        m if m == WM_APP + 2 => {
            let event = (lparam.0 as u32) & 0xFFFF;
            if event == WM_RBUTTONUP {
                if let Some((hwnd, requests)) = menu_target() {
                    show_menu(hwnd, &requests);
                }
            } else if event == WM_LBUTTONUP {
                let guard = STATE.lock();
                if let Some(state) = guard.as_ref() {
                    let _ = state.requests.send(UiRequest::Call);
                }
            }
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_window_in_corners() {
        assert_eq!(corner_position("left", 1000, 800), (0, 300));
        assert_eq!(corner_position("right", 1000, 800), (660, 300));
        assert_eq!(corner_position("center", 1000, 800), (330, 150));
        assert_eq!(corner_position("top-left", 1000, 800), (0, 0));
    }

    #[test]
    fn random_offset_stays_small() {
        for _ in 0..20 {
            let value = rand_offset();
            assert!((-1.0..=1.0).contains(&value));
        }
    }

    #[test]
    fn small_jitter_is_a_click_not_a_drag() {
        // Ровно нажал и отпустил — это клик, Капри должна отозваться.
        assert!(!counts_as_drag(0, 0));
        // Рука дрогнула на пару пикселей — всё ещё клик.
        assert!(!counts_as_drag(2, -3));
        assert!(!counts_as_drag(-DRAG_THRESHOLD, DRAG_THRESHOLD));
        // Настоящее перетаскивание.
        assert!(counts_as_drag(DRAG_THRESHOLD + 1, 0));
        assert!(counts_as_drag(0, -DRAG_THRESHOLD - 1));
    }

    #[test]
    fn menu_cannot_be_opened_twice() {
        // Вложенное меню — это второй модальный цикл, его быть не должно.
        let first = MenuGuard::try_enter();
        assert!(first.is_some(), "первое меню должно открываться");
        assert!(MenuGuard::try_enter().is_none(), "второе меню открываться не должно");
        drop(first);
        assert!(MenuGuard::try_enter().is_some(), "после закрытия меню снова можно");
    }
}

