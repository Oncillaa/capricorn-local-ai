//! Вшитые в бинарник страницы панелей.
//!
//! Рядом с `capri.exe` ничего раскладывать не нужно: `web/` попадает внутрь
//! через `include_bytes!`, а отдаётся протоколом `capri://`.
//!
//! Почему протокол, а не `file://`: у `file://` в WebView2 нет нормальных
//! MIME-типов и ломаются ES-модули, а ещё он открывает странице доступ к диску
//! целиком. Здесь страница видит ровно эти файлы и ничего больше.

/// Один файл интерфейса.
pub struct Asset {
    pub bytes: &'static [u8],
    pub mime: &'static str,
}

macro_rules! asset {
    ($path:literal, $mime:expr) => {
        Asset { bytes: include_bytes!(concat!("../web/", $path)), mime: $mime }
    };
}

const HTML: &str = "text/html; charset=utf-8";
const CSS: &str = "text/css; charset=utf-8";
const JS: &str = "text/javascript; charset=utf-8";
const PNG: &str = "image/png";

/// Найти файл по пути из запроса (`/app.css`, `app.css`).
pub fn get(path: &str) -> Option<Asset> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "settings.html" } else { path };
    Some(match path {
        "settings.html" => asset!("settings.html", HTML),
        "shop.html" => asset!("shop.html", HTML),
        "levels.html" => asset!("levels.html", HTML),
        "media.html" => asset!("media.html", HTML),
        "chess.html" => asset!("chess.html", HTML),
        "chat.html" => asset!("chat.html", HTML),
        "terminal.html" => asset!("terminal.html", HTML),
        "translate.html" => asset!("translate.html", HTML),
        "wip.html" => asset!("wip.html", HTML),
        "app.css" => asset!("app.css", CSS),
        "app.js" => asset!("app.js", JS),
        "bridge.js" => asset!("bridge.js", JS),
        "shop.js" => asset!("shop.js", JS),
        "levels.js" => asset!("levels.js", JS),
        "media.js" => asset!("media.js", JS),
        "chess.js" => asset!("chess.js", JS),
        "chat.js" => asset!("chat.js", JS),
        "terminal.js" => asset!("terminal.js", JS),
        "translate.js" => asset!("translate.js", JS),
        "capri.png" => asset!("capri.png", PNG),
        _ => return None,
    })
}

/// Что отдать, если файла нет.
pub fn not_found() -> Asset {
    asset!("404.html", HTML)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_known_pages() {
        assert!(get("settings.html").is_some());
        assert!(get("/app.css").is_some());
        assert!(get("/").map(|a| a.mime.contains("html")).unwrap_or(false));
        assert!(get("nope.html").is_none());
    }

    #[test]
    fn shop_and_levels_are_real_pages() {
        // Раньше все они отдавали заглушку `wip.html` — теперь это живые экраны.
        let wip = get("wip.html").map(|a| a.bytes).unwrap_or(&[]);
        for page in [
            "shop.html",
            "levels.html",
            "media.html",
            "chess.html",
            "chat.html",
            "terminal.html",
            "translate.html",
        ] {
            let asset = get(page).unwrap_or_else(|| panic!("нет страницы {page}"));
            assert_ne!(asset.bytes, wip, "{page} всё ещё заглушка");
            assert!(asset.mime.contains("html"));
        }
        // Общие скрипты и арт персонажа.
        assert!(get("bridge.js").is_some());
        assert!(get("shop.js").is_some());
        assert!(get("levels.js").is_some());
        assert!(get("media.js").is_some());
        assert!(get("chess.js").is_some());
        assert!(get("chat.js").is_some());
        assert!(get("terminal.js").is_some());
        assert!(get("translate.js").is_some());
        assert!(get("capri.png").map(|a| a.mime == "image/png").unwrap_or(false));
    }

    #[test]
    fn translate_page_is_not_the_stub() {
        // Перевод был последним экраном на заглушке. Тест держит это состояние:
        // если страницу снова подменят на `wip.html`, он упадёт.
        let page = get("translate.html").expect("страница перевода должна быть");
        let stub = get("wip.html").map(|a| a.bytes).unwrap_or(&[]);
        assert_ne!(page.bytes, stub, "перевод снова отдаёт заглушку");

        // В странице должны быть свои скрипт и стили, иначе она не заработает.
        let html = std::str::from_utf8(page.bytes).unwrap_or_default();
        assert!(html.contains("translate.js"), "страница не подключает свой скрипт");
        assert!(html.contains("app.css"), "страница не подключает общие стили");
    }

    #[test]
    fn translate_settings_are_a_real_pane() {
        // Раздел «Перевод» в настройках был заглушкой (`data-stub`). Теперь у
        // него своя разметка — иначе провайдера негде было бы выбрать.
        let page = get("settings.html").expect("страница настроек должна быть");
        let html = std::str::from_utf8(page.bytes).unwrap_or_default();
        assert!(
            !html.contains(r#"data-pane="translate" data-stub"#),
            "раздел перевода снова стал заглушкой"
        );
        assert!(html.contains(r#"data-pane="translate""#), "нет раздела перевода");
        assert!(html.contains(r#"id="translate-to""#), "нет выбора языка перевода");
        assert!(html.contains(r#"id="translate-url""#), "нет поля адреса LibreTranslate");

        // И страница должна уметь с этим работать.
        let script = get("app.js").expect("общий скрипт должен быть");
        let js = std::str::from_utf8(script.bytes).unwrap_or_default();
        assert!(js.contains("translate-to"), "скрипт не знает про выбор языка");
        assert!(js.contains("translate-url"), "скрипт не знает про адрес сервера");
    }

    #[test]
    fn every_panel_has_its_page() {
        // Ни одна панель не должна открываться в пустоту.
        for panel in crate::PanelId::ALL {
            let page = panel.page();
            assert!(get(page).is_some(), "нет страницы {page} для «{}»", panel.as_str());
        }
    }
}
