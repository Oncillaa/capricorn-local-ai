//! Радиальное меню: восемь секторов поверх всех окон.
//!
//! Устроено так же, как окно аватара: слоистое окно без рамки, кадр рисуется
//! в памяти и отдаётся через `UpdateLayeredWindow` — прозрачность попиксельная,
//! поэтому видно только кольцо, а не квадрат.
//!
//! Фокус у игры не отбирается: окно создано с `WS_EX_NOACTIVATE`, а клавиши
//! ловит низкоуровневый хук, пока меню открыто (Esc и цифры 1–8). Хук живёт
//! ровно столько, сколько висит меню, — постоянно он не нужен.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;

use parking_lot::Mutex;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, GetCursorPos, KillTimer, RegisterClassExW,
    SetTimer, SetWindowPos, SetWindowsHookExW, ShowWindow, UnhookWindowsHookEx, KBDLLHOOKSTRUCT,
    MSLLHOOKSTRUCT, SW_HIDE, SW_SHOWNOACTIVATE, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, HHOOK,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW,
};

use crate::canvas::Canvas;
use crate::menu_ring::{self, Hit, SIZE};
use crate::UiRequest;

/// Таймер, который сам убирает забытое меню.
const TIMER_AUTO_CLOSE: usize = 0xC0DE;
const AUTO_CLOSE_MS: u32 = 20_000;
/// Таймер-«поводок»: опрос клавиш на случай, если хук не сработал.
const TIMER_KEYS: usize = 0xC0DF;
const KEYS_POLL_MS: u32 = 40;

/// Виртуальные коды, которые нас интересуют.
const VK_ESCAPE: u32 = 0x1B;
const VK_FIRST_DIGIT: u32 = 0x31; // «1»
const VK_FIRST_NUMPAD: u32 = 0x61; // Numpad 1

/// Страница кольца. Секторов на обеих ровно восемь, поэтому геометрия
/// (`menu_ring`) общая — меняются только подписи и то, что они делают.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    /// Основное кольцо: настройки, чат, игры, магазин, инструменты, уровни.
    Main,
    /// Подкольцо «Инструменты»: терминал, медиа, перевод, таймер и прочее.
    Tools,
}

impl Page {
    fn sectors(self) -> &'static [capri_core::menu::Sector] {
        match self {
            Page::Main => &capri_core::menu::SECTORS,
            Page::Tools => &capri_core::menu::TOOLS,
        }
    }
}

struct Menu {
    hwnd: HWND,
    /// Дескриптор модуля: нужен, чтобы Windows приняла низкоуровневый хук.
    instance: HINSTANCE,
    canvas: Canvas,
    hover: Option<Hit>,
    requests: Sender<UiRequest>,
    keyboard_hook: Option<HHOOK>,
    mouse_hook: Option<HHOOK>,
    monitors: Vec<capri_core::platform::MonitorInfo>,
    visible: bool,
    /// Левый верхний угол окна — чтобы понять, куда попал курсор.
    x: i32,
    y: i32,
    /// Какие клавиши были зажаты на прошлом опросе (нужны фронты).
    pressed: [bool; 9],
    /// Какая страница на экране сейчас.
    page: Page,
    /// С какой страницы открывать меню. Обычно основная; отладочный
    /// `CAPRI_MENU_PAGE=tools` открывает сразу подкольцо — так второе кольцо
    /// можно проверить снимком экрана, не попадая мышью по сектору.
    initial_page: Page,
}


static MENU: Mutex<Option<Menu>> = Mutex::new(None);
static CLASS_ATOM: AtomicIsize = AtomicIsize::new(0);

// Дескрипторы Win32 формально не `Send`, но окно и хуки созданы и
// используются в одном потоке (цикл сообщений аватара) — так же, как в
// `avatar::State`. Мьютекс здесь только для доступа из хуков.
unsafe impl Send for Menu {}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// С какой страницы открывать кольцо. Отладка: `CAPRI_MENU_PAGE=tools`.
fn initial_page_from_env() -> Page {
    match std::env::var("CAPRI_MENU_PAGE") {
        Ok(value) if value.trim().eq_ignore_ascii_case("tools") => Page::Tools,
        Ok(value) if !value.trim().is_empty() => {
            log::warn!("CAPRI_MENU_PAGE={value}: знаю только «tools», беру основное кольцо");
            Page::Main
        }
        _ => Page::Main,
    }
}

/// Создать скрытое окно меню. Зовётся один раз при запуске.
pub fn init(instance: HINSTANCE, requests: Sender<UiRequest>, monitors: Vec<capri_core::platform::MonitorInfo>) -> bool {
    unsafe {
        let class_name = wide("CapriRadialMenu");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            log::error!("Не смогла зарегистрировать класс окна меню");
            return false;
        }
        CLASS_ATOM.store(atom as isize, Ordering::Relaxed);

        let hwnd = match CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            windows::core::PCWSTR(class_name.as_ptr()),
            windows::core::PCWSTR(class_name.as_ptr()),
            WS_POPUP,
            0,
            0,
            SIZE,
            SIZE,
            None,
            None,
            Some(instance),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                log::error!("Не смогла создать окно меню: {error}");
                return false;
            }
        };

        // Клон канала для отладочного выбора сектора: `menu` забирает
        // `requests` себе, а отладочному пути нужен свой отправитель.
        let requests_for_pick = requests.clone();
        let mut menu = Menu {
            hwnd,
            instance,
            canvas: Canvas::new(SIZE as u32, SIZE as u32),
            hover: None,
            requests: requests.clone(),
            keyboard_hook: None,
            mouse_hook: None,
            monitors,
            visible: false,
            x: 0,
            y: 0,
            pressed: [false; 9],
            page: Page::Main,
            initial_page: initial_page_from_env(),
        };
        repaint(&mut menu);
        *MENU.lock() = Some(menu);

        // Отладка: `CAPRI_MENU_AT=1090,390` показывает меню в заданной точке,
        // не глядя на курсор. Так его можно проверить автоматически — снимком
        // экрана, где позиция не «уезжает» вместе с мышью.
        if let Ok(spec) = std::env::var("CAPRI_MENU_AT") {
            if let Some((x, y)) = spec.split_once(',').and_then(|(x, y)| {
                Some((x.trim().parse::<i32>().ok()?, y.trim().parse::<i32>().ok()?))
            }) {
                let mut guard = MENU.lock();
                if let Some(menu) = guard.as_mut() {
                    log::info!("CAPRI_MENU_AT={x},{y}: показываю меню при старте");
                    show_at_locked(menu, x, y);
                }
            }
        }

        // Отладка: `CAPRI_MENU_PICK=terminal` — выбрать сектор кольца сразу,
        // без мыши. Синтетические клики не доходят до нашего хука в тестовом
        // окружении, а проверить надо всю цепочку: сектор → ядро → панель.
        if let Ok(id) = std::env::var("CAPRI_MENU_PICK") {
            let id = id.trim().to_string();
            if !id.is_empty() {
                log::info!("CAPRI_MENU_PICK={id}: выбираю сектор без мыши");
                let _ = requests_for_pick.send(UiRequest::RadialAction(id));
            }
        }
        true
    }
}

/// Открыть меню у курсора или закрыть, если оно уже открыто.
pub fn toggle() {
    let mut guard = MENU.lock();
    let Some(menu) = guard.as_mut() else {
        log::warn!("Окно меню не создано — горячая клавиша не сработает");
        return;
    };
    if menu.visible {
        hide_locked(menu);
    } else {
        show_locked(menu);
    }
}

/// Закрыть, если открыто.
pub fn hide() {
    let mut guard = MENU.lock();
    if let Some(menu) = guard.as_mut() {
        hide_locked(menu);
    }
}

/// Открыто ли меню прямо сейчас.
pub fn is_visible() -> bool {
    MENU.lock().as_ref().is_some_and(|menu| menu.visible)
}

fn show_locked(menu: &mut Menu) {
    let cursor = cursor_position();
    let (left, top, right, bottom) = screen_area(&menu.monitors, cursor);

    // Ставим кольцо вокруг курсора, но не выпускаем за монитор.
    let half = SIZE / 2;
    let max_x = (right - SIZE).max(left);
    let max_y = (bottom - SIZE).max(top);
    let x = (cursor.x - half).clamp(left, max_x);
    let y = (cursor.y - half).clamp(top, max_y);
    show_at_locked(menu, x, y);
}

/// Показать меню так, чтобы его центр был в точке (`x`, `y`) — левый верхний
/// угол окна. Курсор при этом считается уже стоящим в центре кольца.
fn show_at_locked(menu: &mut Menu, x: i32, y: i32) {
    let half = SIZE / 2;
    menu.x = x;
    menu.y = y;
    // Каждое открытие начинается с основной страницы: запоминать, на чём
    // пользователь остановился в прошлый раз, — сюрприз, а не удобство.
    menu.page = menu.initial_page;

    // Курсор в центре окна — значит, под ним крестик «закрыть».
    let cursor = cursor_position();
    menu.hover = Some(menu_ring::hit(
        (cursor.x - x) as f32 - half as f32,
        (cursor.y - y) as f32 - half as f32,
    ));
    repaint(menu);

    unsafe {
        let _ = SetWindowPos(
            menu.hwnd,
            Some(HWND_TOPMOST),
            menu.x,
            menu.y,
            SIZE,
            SIZE,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = ShowWindow(menu.hwnd, SW_SHOWNOACTIVATE);
        // Хуки нужны только пока меню на экране.
        if menu.keyboard_hook.is_none() {
            menu.keyboard_hook = install_hook(menu.instance, WH_KEYBOARD_LL, Some(keyboard_hook));
        }
        if menu.mouse_hook.is_none() {
            menu.mouse_hook = install_hook(menu.instance, WH_MOUSE_LL, Some(mouse_hook));
        }
        SetTimer(Some(menu.hwnd), TIMER_AUTO_CLOSE, AUTO_CLOSE_MS, None);
        // Второй путь для клавиш: опрос. Хук может не сработать (игра с
        // античитом, чужой хук выше по цепочке), а опрос состояния клавиш
        // работает всегда, пока жив наш цикл сообщений.
        SetTimer(Some(menu.hwnd), TIMER_KEYS, KEYS_POLL_MS, None);
    }
    menu.visible = true;
    menu.pressed = [false; 9];
    log::info!("Радиальное меню открыто");
}

fn hide_locked(menu: &mut Menu) {
    if !menu.visible {
        return;
    }
    menu.visible = false;
    unsafe {
        let _ = KillTimer(Some(menu.hwnd), TIMER_AUTO_CLOSE);
        let _ = KillTimer(Some(menu.hwnd), TIMER_KEYS);
        let _ = ShowWindow(menu.hwnd, SW_HIDE);
    }
    if let Some(hook) = menu.keyboard_hook.take() {
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
    }
    if let Some(hook) = menu.mouse_hook.take() {
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
    }
}

fn repaint(menu: &mut Menu) {
    let labels = menu_ring::labels_of(menu.page.sectors());
    menu_ring::paint(&mut menu.canvas, menu.hover, &labels);
    crate::layered::blit(menu.hwnd, &menu.canvas, menu.x, menu.y);
}

/// Пересчитать подсветку под курсором. Нужно после смены страницы: под тем же
/// пикселем теперь другой пункт, и подсветка обязана переехать вместе с ним.
fn refresh_hover(menu: &mut Menu) {
    let cursor = cursor_position();
    let half = SIZE / 2;
    menu.hover = Some(menu_ring::hit(
        (cursor.x - menu.x) as f32 - half as f32,
        (cursor.y - menu.y) as f32 - half as f32,
    ));
}

fn cursor_position() -> POINT {
    let mut point = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut point);
    }
    point
}

/// Границы монитора, на котором стоит курсор.
fn screen_area(
    monitors: &[capri_core::platform::MonitorInfo],
    cursor: POINT,
) -> (i32, i32, i32, i32) {
    let hit = monitors.iter().find(|monitor| {
        cursor.x >= monitor.x
            && cursor.y >= monitor.y
            && cursor.x < monitor.x + monitor.width as i32
            && cursor.y < monitor.y + monitor.height as i32
    });
    let monitor = hit.or_else(|| monitors.iter().find(|m| m.primary)).or_else(|| monitors.first());
    match monitor {
        Some(monitor) => (
            monitor.x,
            monitor.y,
            monitor.x + monitor.width as i32,
            monitor.y + monitor.height as i32,
        ),
        None => (0, 0, 1920, 1080),
    }
}

fn install_hook(
    instance: HINSTANCE,
    kind: windows::Win32::UI::WindowsAndMessaging::WINDOWS_HOOK_ID,
    callback: windows::Win32::UI::WindowsAndMessaging::HOOKPROC,
) -> Option<HHOOK> {
    unsafe {
        // Для низкоуровневых хуков Windows ждёт дескриптор модуля: с пустым
        // хук ставится «успешно», но не вызывается.
        match SetWindowsHookExW(kind, callback, Some(instance), 0) {
            Ok(hook) => Some(hook),
            Err(error) => {
                log::warn!("Не смогла поставить хук клавиатуры/мыши: {error}");
                None
            }
        }
    }
}

/// Выбрать сектор по номеру: и «1…8», и цифры на боковой клавиатуре.
fn sector_by_key(vk: u32) -> Option<usize> {
    match vk {
        VK_FIRST_DIGIT..=0x38 => Some((vk - VK_FIRST_DIGIT) as usize),
        VK_FIRST_NUMPAD..=0x68 => Some((vk - VK_FIRST_NUMPAD) as usize),
        _ => None,
    }
}

/// Нажата ли клавиша прямо сейчас (без учёта того, кому она досталась).
fn key_down(vk: u32) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

/// Резервный путь для клавиш: тот же выбор, но по опросу состояния.
///
/// Нужен, потому что низкоуровневый хук — не гарантия: его может не пропустить
/// игра с античитом, а может и вовсе не пустить система. Опрос состояния клавиш
/// обходится без хуков и работает, пока работает наш цикл сообщений.
fn poll_keys_locked(menu: &mut Menu) {
    let mut pressed = [false; 9];
    pressed[0] = key_down(VK_ESCAPE);
    for (index, slot) in pressed.iter_mut().enumerate().skip(1) {
        let digit = VK_FIRST_DIGIT + index as u32 - 1;
        let numpad = VK_FIRST_NUMPAD + index as u32 - 1;
        *slot = key_down(digit) || key_down(numpad);
    }

    let previous = menu.pressed;
    menu.pressed = pressed;

    // Реагируем на нажатие, а не на удержание.
    if pressed[0] && !previous[0] {
        hide_locked(menu);
        return;
    }
    let chosen = pressed
        .iter()
        .enumerate()
        .skip(1)
        .find(|(index, down)| **down && !previous[*index])
        .map(|(index, _)| index - 1);
    if let Some(index) = chosen {
        choose(menu, index);
    }
}

/// Выбрать сектор текущей страницы.
///
/// Сектора-навигации не закрывают кольцо: «Инструменты» показывает второе
/// кольцо, «Назад» возвращает основное. Иначе за подкольцом пришлось бы снова
/// звать горячую клавишу — а инструменты живут именно там.
fn choose(menu: &mut Menu, index: usize) {
    let sectors = menu.page.sectors();
    let sector = &sectors[index % sectors.len()];
    match sector.target {
        capri_core::menu::Target::Submenu => {
            log::info!("Меню: «{}» → второе кольцо", sector.label);
            menu.page = Page::Tools;
            refresh_hover(menu);
            repaint(menu);
        }
        capri_core::menu::Target::Back => {
            log::info!("Меню: «{}» → основное кольцо", sector.label);
            menu.page = Page::Main;
            refresh_hover(menu);
            repaint(menu);
        }
        _ => {
            log::info!("Меню: выбрано «{}»", sector.label);
            let _ = menu.requests.send(UiRequest::RadialAction(sector.id.to_string()));
            hide_locked(menu);
        }
    }
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let message = wparam.0 as u32;
        if message == WM_KEYDOWN || message == WM_SYSKEYDOWN {
            let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let mut guard = MENU.lock();
            if let Some(menu) = guard.as_mut() {
                if menu.visible {
                    log::debug!("меню: клавиша vkCode={:#04X}", info.vkCode);
                    if info.vkCode == VK_ESCAPE {
                        hide_locked(menu);
                        return LRESULT(1);
                    }
                    if let Some(index) = sector_by_key(info.vkCode) {
                        choose(menu, index);
                        return LRESULT(1);
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && wparam.0 as u32 == WM_LBUTTONUP {
        let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
        let mut guard = MENU.lock();
        if let Some(menu) = guard.as_mut() {
            if menu.visible {
                // Клик мимо кольца — закрываем. По прозрачным углам окна клик
                // уходит в приложение под нами, своего сообщения мы не получим,
                // поэтому решает попадание, а не прямоугольник окна.
                let hover = menu_ring::hit(
                    (info.pt.x - menu.x) as f32 - (SIZE / 2) as f32,
                    (info.pt.y - menu.y) as f32 - (SIZE / 2) as f32,
                );
                // Мимо кольца и в «дырку» между кольцом и центром — закрываем:
                // там окно прозрачное, клик уходит в приложение под нами.
                if matches!(hover, Hit::Outside | Hit::Gap) {
                    hide_locked(menu);
                }
            }
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let x = (lparam.0 & 0xFFFF) as u16 as i16 as f32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
            let mut guard = MENU.lock();
            if let Some(menu) = guard.as_mut() {
                let hover = menu_ring::hit(x - (SIZE / 2) as f32, y - (SIZE / 2) as f32);
                if menu.hover != Some(hover) {
                    menu.hover = Some(hover);
                    repaint(menu);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            let mut guard = MENU.lock();
            if let Some(menu) = guard.as_mut() {
                match menu.hover {
                    Some(Hit::Sector(index)) => choose(menu, index),
                    Some(Hit::Center) => hide_locked(menu),
                    // Промах между центром и кольцом — просто игнорируем.
                    _ => {}
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            hide();
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_AUTO_CLOSE {
                hide();
            } else if wparam.0 == TIMER_KEYS {
                let mut guard = MENU.lock();
                if let Some(menu) = guard.as_mut() {
                    if menu.visible {
                        poll_keys_locked(menu);
                    }
                }
            }
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_pick_sectors_in_order() {
        assert_eq!(sector_by_key(u32::from(b'1')), Some(0));
        assert_eq!(sector_by_key(u32::from(b'8')), Some(7));
        // Остальные клавиши не трогаем.
        assert_eq!(sector_by_key(u32::from(b'9')), None);
        assert_eq!(sector_by_key(u32::from(b'0')), None);
        // Клавиши на боковой клавиатуре работают так же.
        assert_eq!(sector_by_key(0x61), Some(0));
        assert_eq!(sector_by_key(0x68), Some(7));
        // Остальные клавиши не трогаем.
        assert_eq!(sector_by_key(0x41), None);
    }

    #[test]
    fn pages_have_the_same_number_of_sectors_as_the_ring_draws() {
        // Кольцо рисует и разбирает попадания по константе `menu_ring::SECTORS`.
        // Если страница принесёт другое число пунктов, подписи разъедутся с
        // секторами, а цифра 8 перестанет попадать в сектор.
        assert_eq!(Page::Main.sectors().len(), menu_ring::SECTORS);
        assert_eq!(Page::Tools.sectors().len(), menu_ring::SECTORS);
    }

    #[test]
    fn every_page_sector_is_found_by_id() {
        for page in [Page::Main, Page::Tools] {
            for sector in page.sectors() {
                assert_eq!(
                    capri_core::menu::find(sector.id).map(|s| s.id),
                    Some(sector.id),
                    "ядро не знает сектор «{}»",
                    sector.id
                );
            }
        }
    }

    #[test]
    fn only_the_tools_page_holds_the_terminal() {
        // Пункт 6 фазы B: терминал открывается из подкольца «Инструменты».
        assert!(Page::Tools.sectors().iter().any(|s| s.id == "terminal"));
        assert!(!Page::Main.sectors().iter().any(|s| s.id == "terminal"));
    }

    #[test]
    fn navigation_sectors_live_where_expected() {
        // «Инструменты» — на основной странице, «Назад» — в подкольце. Если
        // поменять их местами, из подкольца нельзя будет выйти.
        assert!(Page::Main
            .sectors()
            .iter()
            .any(|s| s.target == capri_core::menu::Target::Submenu));
        assert!(Page::Tools
            .sectors()
            .iter()
            .any(|s| s.target == capri_core::menu::Target::Back));
        assert!(!Page::Tools
            .sectors()
            .iter()
            .any(|s| s.target == capri_core::menu::Target::Submenu));
    }

    #[test]
    fn area_falls_back_when_there_are_no_monitors() {
        let (left, top, right, bottom) = screen_area(&[], POINT { x: 10, y: 10 });
        assert_eq!((left, top), (0, 0));
        assert!(right > 0 && bottom > 0);
    }

    #[test]
    fn area_follows_the_cursor_monitor() {
        let primary = capri_core::platform::MonitorInfo {
            width: 1920,
            height: 1080,
            primary: true,
            ..Default::default()
        };
        let second = capri_core::platform::MonitorInfo {
            index: 1,
            x: 1920,
            width: 2560,
            height: 1440,
            ..Default::default()
        };
        let monitors = vec![primary, second];
        assert_eq!(screen_area(&monitors, POINT { x: 100, y: 100 }).2, 1920);
        assert_eq!(screen_area(&monitors, POINT { x: 3000, y: 100 }).2, 1920 + 2560);
        // Курсор за пределами всех мониторов — берём основной.
        assert_eq!(screen_area(&monitors, POINT { x: -500, y: -500 }).2, 1920);
    }
}
