//! Слоистое окно аватара: прозрачный фон, подпись, перетаскивание.
//!
//! Окно рисуется целиком в память и отдаётся системе одним вызовом
//! `UpdateLayeredWindow` — так работает попиксельная прозрачность без
//! мерцания и без мерцающих углов.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;

use parking_lot::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION,
    NOTIFYICONDATAW, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW,
    GetSystemMetrics, GetWindowThreadProcessId, KillTimer, LoadIconW, PostMessageW, PostQuitMessage,
    RegisterClassExW, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, TrackPopupMenu,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, GWL_EXSTYLE,
    IDI_APPLICATION, MF_SEPARATOR, MF_STRING, MSG, SM_CXSCREEN, SM_CYSCREEN,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE, SW_SHOW, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, WM_APP, WM_HOTKEY, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NULL, WM_RBUTTONUP, WM_TIMER, WS_EX_LAYERED, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, SetWindowLongPtrW,
};

use crate::canvas::{Canvas, Mask};
use crate::sprite::{Geometry, Sprites};
use crate::text;
use crate::view::ViewSettings;
use crate::{CharacterSetting, UiRequest};

/// Сообщение «обновить картинку» — шлём сами себе из таймера.
const WM_CAPRI_TICK: u32 = WM_APP + 1;
const TIMER_ID: usize = 1;
/// Таймер второй попытки поставить иконку в трей.
const TIMER_TRAY: usize = 3;
/// Сколько раз и с каким шагом пробуем поставить иконку.
///
/// `Shell_NotifyIconW(NIM_ADD)` отвечает отказом, если область уведомлений ещё
/// не готова: Капри стартует вместе с системой, а `explorer.exe` в этот момент
/// может только подниматься. Одна попытка на старте — это «иконки нет до
/// перезапуска», поэтому пробуем несколько раз, а не сдаёмся сразу.
const TRAY_ATTEMPTS: usize = 5;
const TRAY_RETRY_MS: u32 = 2_000;
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
    /// Размер окна и фигуры (ползунок «Размер» в настройках).
    geometry: Geometry,
    /// Живость и выбранный монитор.
    view: ViewSettings,
    /// Все мониторы: границы прогулки и выбор экрана.
    monitors: Vec<capri_core::platform::MonitorInfo>,
    /// Экран, на котором живём сейчас.
    area: Area,
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

/// Сколько попыток поставить иконку в трей уже сделано.
static TRAY_TRIES: AtomicUsize = AtomicUsize::new(0);

/// Дескриптор модуля: нужен повторной попытке иконки из `wndproc`, а его
/// вызывают из C, куда ничего не передать.
static MODULE_HINSTANCE: AtomicIsize = AtomicIsize::new(0);

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
pub fn run(
    sprites: Sprites,
    requests: Sender<UiRequest>,
    view: ViewSettings,
    monitors: Vec<capri_core::platform::MonitorInfo>,
    hotkey_spec: String,
) -> i32 {
    unsafe {
        let instance = match GetModuleHandleW(None) {
            Ok(handle) => handle,
            Err(e) => {
                log::error!("Не получила дескриптор модуля: {e}");
                return 1;
            }
        };

        let class_name = wide(crate::AVATAR_CLASS);
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

        let geometry = Geometry::for_scale(view.scale);
        let area = area_of(&monitors, view.monitor);
        // Как и раньше — не вплотную к краю, а с небольшим отступом.
        let corner = corner_position("bottom-right", area, geometry);
        let x = (corner.0 - 24).max(area.x);
        let y = (corner.1 - 48).max(area.y);

        let hwnd = match CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WS_POPUP,
            x,
            y,
            geometry.width as i32,
            geometry.height as i32,
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

        // Клоны для меню и горячей клавиши: остальное уедет в состояние окна.
        let requests_for_window = requests.clone();
        let monitors_for_menu = monitors.clone();
        {
            let mut state = State {
                hwnd,
                canvas: Canvas::new(geometry.width, geometry.height),
                sprites,
                geometry,
                view,
                monitors,
                area,
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

        add_tray_icon_with_retry(hwnd, windows::Win32::Foundation::HINSTANCE(instance.0));

        // Радиальное меню: своё скрытое окно, которое ждёт горячей клавиши.
        let menu_ready = crate::radial::init(
            windows::Win32::Foundation::HINSTANCE(instance.0),
            requests_for_window.clone(),
            monitors_for_menu,
        );
        if menu_ready {
            register_hotkey(hwnd, &hotkey_spec, &requests_for_window);
        } else {
            log::warn!("Радиальное меню недоступно, горячую клавишу не ставлю");
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            geometry.width as i32,
            geometry.height as i32,
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
            // Идём к случайной точке неподалёку, не выходя за свой монитор.
            let span = (state.area.width - state.geometry.width as i32).max(1);
            let target = (state.x + (rand_offset() * 300.0) as i32)
                .clamp(state.area.x, state.area.x + span);
            let speed = state.view.liveliness.walk_speed();
            state.roam_target = Some(target);
            state.vx = if target > state.x { speed } else { -speed };
        }
        crate::UiEvent::Hop => {
            let span_x = (state.area.width - state.geometry.width as i32).max(1);
            let span_y = (state.area.height - state.geometry.height as i32).max(1);
            state.x = (state.x + (rand_offset() * 400.0) as i32)
                .clamp(state.area.x, state.area.x + span_x);
            state.y = (state.y + (rand_offset() * 200.0) as i32)
                .clamp(state.area.y, state.area.y + span_y);
            state.roam_target = None;
            state.vx = 0;
            apply_position(state);
        }
        crate::UiEvent::MoveTo(where_) => {
            let (x, y) = corner_position(&where_, state.area, state.geometry);
            state.x = x;
            state.y = y;
            state.roam_target = None;
            state.vx = 0;
            apply_position(state);
        }
        crate::UiEvent::SetSkin(color) => {
            state.skin = color;
        }
        crate::UiEvent::Character(setting) => apply_character(state, setting),
        crate::UiEvent::Hotkey(spec) => register_hotkey(state.hwnd, &spec, &state.requests),
    }
    paint(state);
}

/// Поставить горячую клавишу на окно. Пустая строка — «клавиша не нужна».
fn register_hotkey(hwnd: HWND, spec: &str, requests: &Sender<UiRequest>) {
    crate::hotkey::unregister(hwnd);
    if spec.trim().is_empty() {
        log::info!("Горячая клавиша меню выключена");
        return;
    }
    let parsed = match crate::hotkey::parse(spec) {
        Ok(parsed) => parsed,
        Err(why) => {
            let text = format!("Горячая клавиша меню: {why}");
            log::warn!("{text}");
            let _ = requests.send(UiRequest::HotkeyError(text));
            return;
        }
    };
    match crate::hotkey::register(hwnd, parsed) {
        Ok(()) => log::info!("Горячая клавиша меню: {}", crate::hotkey::describe(parsed)),
        Err(why) => {
            log::warn!("{why}");
            let _ = requests.send(UiRequest::HotkeyError(why));
        }
    }
}

/// Прямоугольник экрана, на котором живёт Капри (физические пиксели).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Area {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

/// Экран для выбранного монитора.
///
/// Если такого номера нет (монитор отключили) — берём основной, если и его
/// нет — весь экран по системным метрикам: без экрана окно жить не может.
fn area_of(monitors: &[capri_core::platform::MonitorInfo], index: u32) -> Area {
    let found = monitors
        .iter()
        .find(|m| m.index == index)
        .or_else(|| monitors.iter().find(|m| m.primary))
        .or_else(|| monitors.first());
    match found {
        Some(monitor) if monitor.width > 0 && monitor.height > 0 => Area {
            x: monitor.x,
            y: monitor.y,
            width: monitor.width as i32,
            height: monitor.height as i32,
        },
        _ => Area {
            x: 0,
            y: 0,
            width: unsafe { GetSystemMetrics(SM_CXSCREEN) },
            height: unsafe { GetSystemMetrics(SM_CYSCREEN) },
        },
    }
}

/// Не дать окну уехать за пределы своего монитора.
fn clamp_into_area(state: &mut State) {
    let span_x = (state.area.width - state.geometry.width as i32).max(0);
    let span_y = (state.area.height - state.geometry.height as i32).max(0);
    state.x = state.x.clamp(state.area.x, state.area.x + span_x);
    state.y = state.y.clamp(state.area.y, state.area.y + span_y);
}

/// Применить новый размер, живость или монитор к живому окну.
fn apply_character(state: &mut State, setting: CharacterSetting) {
    match setting {
        CharacterSetting::Scale(percent) => {
            let previous = state.geometry;
            state.view = state.view.with_scale(percent);
            state.geometry = Geometry::for_scale(state.view.scale);
            // Растём и сжимаемся от правого нижнего угла: Капри должна
            // остаться там, где её поставил пользователь.
            state.x += previous.width as i32 - state.geometry.width as i32;
            state.y += previous.height as i32 - state.geometry.height as i32;
            clamp_into_area(state);
            apply_position(state);
        }
        CharacterSetting::Liveliness(value) => {
            state.view = state.view.with_liveliness(&value);
        }
        CharacterSetting::Monitor(index) => {
            state.view = state.view.with_monitor(index);
            state.area = area_of(&state.monitors, index);
            let (x, y) = corner_position("right", state.area, state.geometry);
            state.x = x;
            state.y = y;
            state.roam_target = None;
            state.vx = 0;
            apply_position(state);
        }
    }
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

fn corner_position(where_: &str, area: Area, geometry: Geometry) -> (i32, i32) {
    let span_x = (area.width - geometry.width as i32).max(0);
    let span_y = (area.height - geometry.height as i32).max(0);
    let right = area.x + span_x;
    let bottom = area.y + span_y;
    let center_x = area.x + span_x / 2;
    let center_y = area.y + span_y / 2;
    match where_ {
        "left" => (area.x, bottom),
        "right" => (right, bottom),
        "top" => (right, area.y),
        "bottom" => (right, bottom),
        "center" => (center_x, center_y),
        "left-top" => (area.x, area.y),
        "left-bottom" => (area.x, bottom),
        "right-top" => (right, area.y),
        "right-bottom" | "bottom-right" => (right, bottom),
        "top-left" => (area.x, area.y),
        "top-right" => (right, area.y),
        "bottom-left" => (area.x, bottom),
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
    let geometry = state.geometry;
    let width = geometry.width;
    if state.canvas.width != width || state.canvas.height != geometry.height {
        state.canvas = Canvas::new(width, geometry.height);
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
        // Кадры хранятся в исходном размере: под текущий масштаб подгоняем
        // только тот, который сейчас показываем.
        let image = image.fit_height(geometry.fig_height);
        let image = match skin {
            Some(tint) => image.tinted(tint),
            None => image,
        };
        let left = (width as i32 - image.width as i32) / 2;
        let top = (geometry.fig_height as i32 - image.height as i32).max(0) + 4;
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
            let box_y = geometry.fig_height as i32 + 6;
            canvas.round_rect(box_x, box_y, box_w, box_h, 10, [10, 8, 18], 0.82 * opacity);
            canvas.draw_mask(&mask, box_x + padding, box_y + padding / 2, [120, 240, 255], opacity);
        }
    }

    blit(state);
}

/// Отправить пиксели в окно.
fn blit(state: &mut State) {
    crate::layered::blit(state.hwnd, &state.canvas, state.x, state.y);
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

    // Анимация. «Живость» растягивает кадры: спокойная Капри двигается
    // медленнее и плавнее, живая — быстрее.
    let slowdown = state.view.liveliness.frame_slowdown();
    if let Some(anim) = state.animation.as_mut() {
        anim.counter += 1;
        if anim.counter >= (anim.hold * slowdown).max(1) {
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
const MENU_PANEL: usize = 1009;
/// Открыть кольцо меню без клавиатуры — на случай, если горячую клавишу
/// заняла другая программа.
const MENU_RING: usize = 1010;
/// Терминал: панель с потоковым выводом команд. Пункт в трее нужен потому, что
/// из кольца он живёт в подкольце «Инструменты», а сюда ведёт один клик.
const MENU_TERMINAL: usize = 1011;

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
        let items: [(usize, &str); 11] = [
            (MENU_CALL, "Позвать Капри"),
            (MENU_RING, "Кольцо меню (Ctrl+Alt+C)"),
            (MENU_PANEL, "Настройки…"),
            (MENU_TERMINAL, "Терминал…"),
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
            MENU_PANEL => {
                let _ = requests.send(UiRequest::OpenPanel("settings".into()));
            }
            MENU_TERMINAL => {
                let _ = requests.send(UiRequest::OpenPanel("terminal".into()));
            }
            MENU_RING => {
                crate::radial::toggle();
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

/// Поставить иконку в трей, повторяя попытку, если область уведомлений ещё
/// не готова.
///
/// Отказ `NIM_ADD` на старте — обычное дело, когда система только загрузилась:
/// `explorer.exe` поднимается позже Капри. Раньше одна неудачная попытка
/// означала «иконки нет до перезапуска», и в логе оставалось предупреждение.
/// Теперь первый отказ лишь запускает таймер: до [`TRAY_ATTEMPTS`] попыток с
/// шагом [`TRAY_RETRY_MS`]. Успех — пишем в лог; исчерпали попытки — честно
/// предупреждаем один раз, что иконки не будет.
fn add_tray_icon_with_retry(hwnd: HWND, instance: windows::Win32::Foundation::HINSTANCE) {
    MODULE_HINSTANCE.store(instance.0 as isize, Ordering::Relaxed);
    TRAY_TRIES.store(1, Ordering::Relaxed);
    if add_tray_icon(hwnd, instance) {
        return;
    }
    log::info!(
        "Иконка в трее: область уведомлений не готова, попробую ещё {} раз(а)",
        TRAY_ATTEMPTS - 1
    );
    unsafe {
        SetTimer(Some(hwnd), TIMER_TRAY, TRAY_RETRY_MS, None);
    }
}

/// Следующая попытка по таймеру. Возвращает `true`, если пора остановиться.
fn retry_tray_icon(hwnd: HWND, instance: windows::Win32::Foundation::HINSTANCE) -> bool {
    let tries = TRAY_TRIES.fetch_add(1, Ordering::Relaxed) + 1;
    if add_tray_icon(hwnd, instance) {
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_TRAY);
        }
        return true;
    }
    if tries >= TRAY_ATTEMPTS {
        log::warn!(
            "Иконка в трее: Shell_NotifyIconW отказал {tries} раз(а) подряд — иконки не будет. \
             Меню доступно по Ctrl+Alt+C."
        );
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_TRAY);
        }
        return true;
    }
    false
}

fn add_tray_icon(hwnd: HWND, instance: windows::Win32::Foundation::HINSTANCE) -> bool {
    unsafe {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_APP + 2,
            ..Default::default()
        };
        // Своё лицо лучше стандартной заглушки: в трее Капри должна быть
        // узнаваемой, а не «какое-то приложение».
        if let Some(icon) = tray_icon() {
            data.hIcon = icon;
        } else if let Ok(icon) = LoadIconW(Some(instance), PCWSTR(1 as *const u16)) {
            data.hIcon = icon;
        } else if let Ok(icon) = LoadIconW(None, IDI_APPLICATION) {
            data.hIcon = icon;
        }
        let tip = wide("Капри");
        let len = tip.len().min(data.szTip.len());
        data.szTip[..len].copy_from_slice(&tip[..len]);
        // Результат пишем в лог: «иконки нет» надо замечать сразу, а не искать
        // глазами по трею.
        if Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
            // Версия 4 — иначе всплывающие подсказки и нажатия ведут себя по
            // правилам двадцатилетней давности.
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
            log::info!("Иконка в трее: поставлена");
            true
        } else {
            // Без кода ошибки «отказал» ничего не объясняет: причиной может
            // быть и неготовый explorer, и пустой hIcon, и неверный cbSize.
            let code = windows::Win32::Foundation::GetLastError().0;
            log::warn!(
                "Иконка в трее: Shell_NotifyIconW отказал (код {code}, \
                 hIcon={:?}, cbSize={})",
                data.hIcon,
                data.cbSize
            );
            // Неудачная попытка иконку не принимает — уничтожаем её сами,
            // иначе каждая из пяти попыток оставляет утечку в HANDLE.
            if !data.hIcon.is_invalid() {
                let _ = DestroyIcon(data.hIcon);
            }
            false
        }
    }
}

/// Иконка Капри для трея: 32×32 лицо, нарисованное в памяти.
///
/// Ресурсной иконки в exe нет и не будет — фигура собирается кодом. Поэтому
/// рисуем её теми же двумя цветами, что и аватар: фиолетовый диск и зелёные
/// глаза в пиксель.
fn tray_icon() -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    use windows::Win32::Graphics::Gdi::{
        CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
        DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};

    const SIZE: usize = 32;
    // Палитра аватара: фиолетовое тело, зелёная кайма и глаза.
    const BODY: (u8, u8, u8) = (104, 40, 208);
    const EDGE: (u8, u8, u8) = (56, 255, 20);
    const DARK: (u8, u8, u8) = (26, 16, 44);

    let mut pixels = vec![0u8; SIZE * SIZE * 4];
    let center = (SIZE as f32 - 1.0) / 2.0;
    let radius = SIZE as f32 / 2.0 - 1.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = (dx * dx + dy * dy).sqrt() / radius;
            if distance > 1.0 {
                continue;
            }
            let (r, g, b) = if distance > 0.86 {
                EDGE
            } else if distance > 0.78 {
                DARK
            } else {
                BODY
            };
            let index = (y * SIZE + x) * 4;
            pixels[index] = b;
            pixels[index + 1] = g;
            pixels[index + 2] = r;
            pixels[index + 3] = 255;
        }
    }

    // Глаза и улыбка: два пикселя на глаз — как в спрайте Капри.
    let mut put = |x: usize, y: usize, color: (u8, u8, u8)| {
        if x >= SIZE || y >= SIZE {
            return;
        }
        let index = (y * SIZE + x) * 4;
        pixels[index] = color.2;
        pixels[index + 1] = color.1;
        pixels[index + 2] = color.0;
        pixels[index + 3] = 255;
    };
    for (x, y) in [(10, 12), (11, 12), (10, 13), (11, 13)] {
        put(x, y, EDGE);
    }
    for (x, y) in [(20, 12), (21, 12), (20, 13), (21, 13)] {
        put(x, y, EDGE);
    }
    for (x, y) in [(11, 20), (12, 21), (13, 22), (14, 22), (15, 22), (16, 22), (17, 22), (18, 21), (19, 20)] {
        put(x, y, EDGE);
    }

    unsafe {
        // Для иконки с прозрачностью нужен именно DIB-раздел: `CreateBitmap`
        // с 32 битами альфу не переносит, и `CreateIconIndirect` собирает
        // иконку по маске. Маска у нас нулевая («показывать всё»), поэтому
        // получался сплошной тёмный квадрат вместо круга.
        let header = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE as i32,
                biHeight: -(SIZE as i32), // минус — строки сверху вниз
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let color = match CreateDIBSection(None, &header, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) if !bits.is_null() => bitmap,
            _ => {
                log::warn!("Иконка в трее: цветная картинка не создалась");
                return None;
            }
        };
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u8>(), pixels.len());

        // Маска из нулей — «показывать всё»: прозрачность задаёт альфа.
        let zero_mask = vec![0u8; SIZE * SIZE / 8];
        let mask = CreateBitmap(
            SIZE as i32,
            SIZE as i32,
            1,
            1,
            Some(zero_mask.as_ptr().cast()),
        );
        let info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&info).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        if icon.is_none() {
            log::warn!("Иконка в трее: из картинки не собралась");
        }
        icon
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
            } else if wparam.0 == TIMER_TRAY {
                // Иконка в трей: повторная попытка, если область уведомлений
                // была не готова на старте.
                let instance = windows::Win32::Foundation::HINSTANCE(
                    MODULE_HINSTANCE.load(Ordering::Relaxed) as *mut _,
                );
                let _ = retry_tray_icon(hwnd, instance);
            }
            LRESULT(0)
        }
        WM_CAPRI_TICK => {
            tick();
            LRESULT(0)
        }
        WM_HOTKEY => {
            // Горячая клавиша пришла сюда же, где живёт цикл сообщений:
            // отдельный поток для неё не нужен.
            if wparam.0 as i32 == crate::hotkey::HOTKEY_ID {
                crate::radial::toggle();
            }
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
                    // Клик без движения — позвать Капри и засчитать поглаживание.
                    let _ = state.requests.send(UiRequest::Call);
                    let _ = state.requests.send(UiRequest::Petted);
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
        // Второй экземпляр попросил показаться — показываемся и отзываемся.
        crate::WM_CAPRI_SUMMON => {
            let guard = STATE.lock();
            if let Some(state) = guard.as_ref() {
                let _ = state.requests.send(UiRequest::Call);
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
        let area = Area { x: 0, y: 0, width: 1000, height: 800 };
        // Окно во весь рост: 340×500.
        let full = Geometry::for_scale(100);
        assert_eq!(corner_position("left", area, full), (0, 300));
        assert_eq!(corner_position("right", area, full), (660, 300));
        assert_eq!(corner_position("center", area, full), (330, 150));
        assert_eq!(corner_position("top-left", area, full), (0, 0));
    }

    #[test]
    fn corners_follow_the_chosen_monitor() {
        // Второй монитор справа: его левый край — за пределами основного.
        let area = Area { x: 1920, y: -100, width: 1920, height: 1080 };
        let full = Geometry::for_scale(100);
        assert_eq!(corner_position("left-top", area, full), (1920, -100));
        assert_eq!(corner_position("right-bottom", area, full), (3500, 480));
    }

    #[test]
    fn window_stays_on_screen_when_scaled() {
        let area = Area { x: 0, y: 0, width: 800, height: 600 };
        // Маленькое окно помещается в правый нижний угол …
        let small = Geometry::for_scale(50);
        let (x, y) = corner_position("right", area, small);
        assert_eq!((x, y), (800 - small.width as i32, 600 - small.height as i32));
        // … и целиком остаётся на экране.
        assert!(x + small.width as i32 <= area.width);
        assert!(y + small.height as i32 <= area.height);
    }

    #[test]
    fn unknown_monitor_falls_back_to_primary() {
        let mut primary = capri_core::platform::MonitorInfo::default();
        primary.index = 0;
        primary.width = 2560;
        primary.height = 1440;
        primary.primary = true;
        let mut second = capri_core::platform::MonitorInfo::default();
        second.index = 1;
        second.x = 2560;
        second.width = 1920;
        second.height = 1080;

        let monitors = vec![primary.clone(), second.clone()];
        assert_eq!(area_of(&monitors, 1).x, 2560);
        // Монитор отключили, а в настройках остался его номер — берём основной.
        let fallback = area_of(&monitors, 7);
        assert_eq!(fallback.width, 2560);
        assert_eq!(fallback.x, 0);
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

    /// Окно без Windows: для проверки арифметики масштаба и границ хватает.
    fn test_state(geometry: Geometry, area: Area) -> State {
        let (requests, _unused) = std::sync::mpsc::channel();
        State {
            hwnd: HWND(std::ptr::null_mut()),
            canvas: Canvas::new(geometry.width, geometry.height),
            sprites: Sprites { poses: Vec::new(), wave: Vec::new(), eyes: Vec::new(), roam: None },
            geometry,
            view: ViewSettings::default(),
            monitors: Vec::new(),
            area,
            vx: 0,
            animation: None,
            opacity: 1.0,
            target_opacity: 1.0,
            subtitle: String::new(),
            skin: None,
            x: 0,
            y: 0,
            dragging: false,
            drag_offset: (0, 0),
            drag_moved: false,
            requests,
            hidden: false,
            roam_target: None,
        }
    }

    #[test]
    fn shrinking_keeps_the_bottom_right_corner() {
        let area = Area { x: 0, y: 0, width: 1000, height: 800 };
        let mut state = test_state(Geometry::for_scale(100), area);
        // Стоим в правом нижнем углу.
        state.x = area.width - state.geometry.width as i32;
        state.y = area.height - state.geometry.height as i32;
        apply_character(&mut state, CharacterSetting::Scale(50));
        assert_eq!(state.geometry, Geometry::for_scale(50));
        // Угол на месте: окно сжалось к нему, а не убежало в центр.
        assert_eq!(state.x + state.geometry.width as i32, area.width);
        assert_eq!(state.y + state.geometry.height as i32, area.height);
    }

    #[test]
    fn growing_at_the_edge_stays_on_screen() {
        let area = Area { x: 0, y: 0, width: 800, height: 600 };
        let mut state = test_state(Geometry::for_scale(50), area);
        state.x = area.width - state.geometry.width as i32;
        state.y = area.height - state.geometry.height as i32;
        apply_character(&mut state, CharacterSetting::Scale(100));
        assert!(state.x >= 0 && state.y >= 0);
        assert!(state.x + state.geometry.width as i32 <= area.width);
        assert!(state.y + state.geometry.height as i32 <= area.height);
    }

    #[test]
    fn liveliness_and_monitor_apply_immediately() {
        let mut primary = capri_core::platform::MonitorInfo::default();
        primary.width = 1920;
        primary.height = 1080;
        primary.primary = true;
        let mut second = capri_core::platform::MonitorInfo::default();
        second.index = 1;
        second.x = 1920;
        second.width = 2560;
        second.height = 1440;

        let area = Area { x: 0, y: 0, width: 1920, height: 1080 };
        let mut state = test_state(Geometry::for_scale(100), area);
        state.monitors = vec![primary, second];

        apply_character(&mut state, CharacterSetting::Liveliness("calm".into()));
        assert_eq!(state.view.liveliness, crate::view::Liveliness::Calm);

        apply_character(&mut state, CharacterSetting::Monitor(1));
        assert_eq!(state.view.monitor, 1);
        assert_eq!(state.area.x, 1920);
        assert_eq!(state.area.width, 2560);
        // Окно переехало на второй монитор.
        assert!(state.x >= 1920);
    }
}

