//! Панели на tao + WebView2 (wry).

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use tao::dpi::{LogicalPosition, LogicalSize};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopWindowTarget};
use tao::platform::run_return::EventLoopExtRunReturn;
use tao::platform::windows::EventLoopBuilderExtWindows;
use tao::window::{Window, WindowBuilder, WindowId};
use wry::http::{header::CONTENT_TYPE, Response, StatusCode};
use wry::{WebView, WebViewBuilder};

use crate::{assets, Command, PanelEvent, PanelId};

/// Живая панель: окно и его веб-вью.
///
/// Панели не уничтожаются при закрытии — прячутся. Второе открытие тогда
/// мгновенное, и веб-вью не теряет своё состояние (позицию в шахматах,
/// прокрутку лога).
struct PanelWindow {
    window: Window,
    webview: WebView,
}

/// Крутить цикл панелей в текущем потоке до [`Command::Quit`].
pub fn run(commands: Receiver<Command>, events: Sender<PanelEvent>) -> i32 {
    let mut builder = EventLoopBuilder::<Command>::with_user_event();
    // Главный поток занят окном аватара, поэтому цикл панелей живёт в своём.
    builder.with_any_thread(true);
    // DPI-awareness уже сложилась вокруг окна аватара: включать её вторым
    // потоком поздно — аватар начнёт считаться в других единицах и «прыгнет».
    builder.with_dpi_aware(false);
    let mut event_loop = builder.build();

    // Будить цикл из чужого потока умеет только proxy, поэтому канал команд
    // перекладываем в события цикла отдельным потоком.
    let proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        while let Ok(command) = commands.recv() {
            if proxy.send_event(command).is_err() {
                break;
            }
        }
    });

    let mut panels: HashMap<PanelId, PanelWindow> = HashMap::new();
    let mut by_window: HashMap<WindowId, PanelId> = HashMap::new();

    event_loop.run_return(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::UserEvent(Command::Quit) => *control_flow = ControlFlow::Exit,

            Event::UserEvent(Command::Open(id)) => {
                if let Some(panel) = panels.get(&id) {
                    panel.window.set_visible(true);
                    panel.window.set_minimized(false);
                    panel.window.set_focus();
                    // Страница уже загружена и `ready` повторно не пришлёт, а
                    // оболочке нужно знать, что окно снова на экране: иначе
                    // медиа-виджет не получит состояние после переоткрытия.
                    let _ = events.send(PanelEvent {
                        panel: id,
                        action: "shown".into(),
                        data: serde_json::Value::Null,
                    });
                    return;
                }
                match open_panel(id, target, events.clone()) {
                    Ok(panel) => {
                        by_window.insert(panel.window.id(), id);
                        panels.insert(id, panel);
                        log::info!("Панель «{}» открыта", id.as_str());
                    }
                    Err(e) => {
                        log::error!("Панель «{}» не открылась: {e}", id.as_str());
                        let _ = events.send(PanelEvent {
                            panel: id,
                            action: "open-failed".into(),
                            data: serde_json::json!({ "error": e }),
                        });
                    }
                }
            }

            Event::UserEvent(Command::Close(id)) => {
                if let Some(panel) = panels.get(&id) {
                    panel.window.set_visible(false);
                }
            }

            Event::UserEvent(Command::Eval(id, js)) => {
                if let Some(panel) = panels.get(&id) {
                    if let Err(e) = panel.webview.evaluate_script(&js) {
                        log::warn!("Панель «{}»: скрипт не выполнился: {e}", id.as_str());
                    }
                } else {
                    // Нормальная гонка: состояние прилетело раньше окна.
                    // Страница сама попросит его заново через `ready`.
                    log::debug!("Панель «{}» ещё не открыта — пропускаю состояние", id.as_str());
                }
            }

            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. } => {
                // Крестик окна панель не убивает: прячем и живём дальше.
                if let Some(id) = by_window.get(&window_id).copied() {
                    if let Some(panel) = panels.get(&id) {
                        panel.window.set_visible(false);
                    }
                    let _ = events.send(PanelEvent {
                        panel: id,
                        action: "hidden".into(),
                        data: serde_json::Value::Null,
                    });
                }
            }

            _ => {}
        }
    })
}

/// Создать окно панели вместе с веб-вью.
fn open_panel<T: 'static>(
    id: PanelId,
    target: &EventLoopWindowTarget<T>,
    events: Sender<PanelEvent>,
) -> Result<PanelWindow, String> {
    let (width, height) = id.size();
    let window = WindowBuilder::new()
        .with_title(id.title())
        .with_inner_size(LogicalSize::new(width, height))
        .with_min_inner_size(LogicalSize::new(720.0_f64.min(width), 460.0))
        .with_resizable(true)
        .build(target)
        .map_err(|e| format!("окно не создалось: {e}"))?;

    // По центру основного монитора: на скриншотах окно настроек стоит ровно так.
    if let Some(monitor) = target.primary_monitor() {
        let size = monitor.size().to_logical::<f64>(monitor.scale_factor());
        let x = ((size.width - width) / 2.0).max(0.0);
        let y = ((size.height - height) / 2.0).max(0.0);
        window.set_outer_position(LogicalPosition::new(x, y));
    }

    let page = format!("capri://localhost/{}", id.page());
    let ipc = events.clone();
    let webview = WebViewBuilder::new()
        .with_custom_protocol("capri".into(), move |_webview_id, request| {
            serve(request.uri().path())
        })
        .with_url(page)
        .with_ipc_handler(move |request: wry::http::Request<String>| {
            handle_ipc(id, request.body(), &ipc);
        })
        // Тёмный фон обязателен: пока грузится страница, окно не должно
        // мигать белым.
        .with_background_color((10, 6, 18, 255))
        .with_devtools(cfg!(debug_assertions))
        .build(&window)
        .map_err(|e| {
            format!(
                "WebView2 не поднялся ({e}). Нужен компонент \
                 «Microsoft Edge WebView2 Runtime» — он есть в Windows 11 \
                 и в большинстве сборок Windows 10"
            )
        })?;

    window.set_visible(true);
    window.set_focus();

    Ok(PanelWindow { window, webview })
}

/// Отдать файл интерфейса по пути из `capri://localhost/...`.
fn serve(path: &str) -> Response<Cow<'static, [u8]>> {
    let (asset, status) = match assets::get(path) {
        Some(asset) => (asset, StatusCode::OK),
        None => (assets::not_found(), StatusCode::NOT_FOUND),
    };
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, asset.mime)
        .header("Cache-Control", "no-store")
        .body(Cow::Borrowed(asset.bytes))
        .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
}

/// Разобрать сообщение из страницы: `{"action": "...", "data": ...}`.
fn handle_ipc(panel: PanelId, body: &str, events: &Sender<PanelEvent>) {
    let value: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(e) => {
            log::warn!("Панель «{}»: неразборчивое сообщение ({e}): {body}", panel.as_str());
            return;
        }
    };
    let action = value
        .get("action")
        .and_then(|a| a.as_str())
        .unwrap_or_default()
        .to_string();
    if action.is_empty() {
        log::warn!("Панель «{}»: сообщение без action", panel.as_str());
        return;
    }
    let data = value.get("data").cloned().unwrap_or(serde_json::Value::Null);
    let _ = events.send(PanelEvent { panel, action, data });
}
