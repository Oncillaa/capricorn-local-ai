//! Разбор и выполнение команд: перенос `actions.py`.
//!
//! Порядок работы: [`Engine::handle`] нормализует фразу, разбирается с
//! ожидающими подтверждениями («да»/«нет») и выбором из похожих программ,
//! пробует свои ответы из настроек, режет фразу на цепочку шагов
//! («открой телеграм и напиши привет») и, наконец, прогоняет таблицу правил.
//! Если ничего не совпало — `None`, и решает нейросеть.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use regex::Regex;

use crate::ai::{AiChat, LocalAi};
use crate::appindex::AppIndex;
use crate::config::{Config, Map};
use crate::keys::{describe as describe_keys, parse_keys};
use crate::persona::line;
use crate::platform::{
    KeyStroke, MediaCommand, Platform, PowerMode, ScrollDirection, TabCommand,
};
use crate::reply::{action, Reply};
use crate::util::{norm, parse_number, translit, stem};

pub const CAPRI_URL: &str = "https://github.com/kystt29/capricorn";

/// Процессы, которые нельзя трогать никогда.
const PROTECTED: &[&str] = &[
    "explorer", "python", "pythonw", "svchost", "csrss", "winlogon", "dwm", "lsass", "services",
    "system", "smss", "wininit", "fontdrvhost", "sihost", "taskhostw", "ctfmon", "runtimebroker",
    "searchhost", "startmenuexperiencehost", "shellexperiencehost", "textinputhost", "ollama",
    "conhost", "audiodg", "capri", "system idle process", "idle",
];

const TYPE_VERBS: &str = "напиши|напечатай|введи|набери|вбей|впиши|пиши";
const NOT_TYPING: &str = r"(стих\w*|стишок|рассказ\w*|истори\w*|сказк\w*|шутк\w*|анекдот\w*|песн\w*|сочинени\w*|эссе)\b";

static REMEMBER_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(запомни|сохрани|добавь)( мне| себе)? (этот |эту |это |текущ\w+ |открыт\w+ )?\
         (сайт\w*|страниц\w*|вкладк\w*|ссылк\w*|программ\w*|приложени\w*|игр\w*)\s*(?P<name>.*)$",
    )
    .expect("regex")
});

static FORGET_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(забудь|удали|убери)( мой| мою| из памяти)? (сайт\w*|страниц\w*|ссылк\w*|программ\w*|приложени\w*|игр\w*) (?P<name>.+)$",
    )
    .expect("regex")
});

static UPDATES_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(есть |проверь |проверить |какие |когда |а )?(новые )?обновлени\w*( есть| вышли| для тебя| капри)?$|\
         (есть|проверь|проверить|вышл\w*|появил\w*|скачать|где|какие|нет ли)\b.*\bобновлени|\
         (что|как)( там| же| у тебя| у нас)? (по|с) обновлени|\
         обнови (себя|капри)|обновись|\
         ^(есть |вышла |появилась |где )?(нов\w*|последн\w*) верси\w*( капри| тебя)?( вышла| есть| появилась)?$|\
         (скачать|где) (\S+ )?(нов\w*|последн\w*) верси\w*( капри| тебя)?$|\
         (твой|свой|твою|свою) (гитхаб|страниц\w*|репозитори\w*)|гитхаб капри|где тебя (скачать|взять)",
    )
    .expect("regex")
});

static REPLY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^((когда|если) (я )?(говорю|скажу|произнесу|спрошу)|на (фразу|слова|вопрос))\s+(?P<p>.+?)\s*,?\s+\
         (ты )?(отвечай|ответь|говори|скажи)( мне)?\s+(?P<r>.+)$",
    )
    .expect("regex")
});

static FORGET_REPLY_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(забудь|удали|убери) (свой |мой |этот )?ответ (на|для) (фразу |слова |вопрос )?(?P<p>.+)$")
        .expect("regex")
});

static BROWSER_SUFFIX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\s+(в|через)\s+(браузер\w*\s+)?(?P<b>(гугл )?хром\w*|chrome|яндекс\w*|yandex|опер\w*|opera|\
         (фаер|файр|файер)фокс\w*|мозил+\w*|firefox|(майкрософт )?эдж\w*|edge|брейв\w*|brave|вивальди|vivaldi)\
         (\s+браузер\w*)?$",
    )
    .expect("regex")
});

static STOP_SCROLL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(стоп|хватит|стой|остановись|достаточно|перестань|харэ|все|всё)\b").expect("regex"));

/// Ответ «да» на подтверждение (выключить, перезагрузить).
static YES_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(да|давай|подтверждаю|конечно|ага)\b").expect("regex"));

/// Ответ «нет» на подтверждение.
static NO_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(нет|не надо|отмена|стоп)\b").expect("regex"));

/// Отказ от выбора из списка программ.
static PICK_NO_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(никак\w*|ни одно|ни то|отмена|не надо|нет|ничего)\b").expect("regex")
});

/// Фраза похожа на вопрос — тогда честнее поискать в браузере.
static QUESTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)^(что|кто|как|какой|какая|какое|какие|почему|зачем|сколько|где|когда|куда|откуда|чем|чей|расскажи|объясни|скажи)\b",
    )
    .expect("regex")
});

/// Просьба про музыку — тогда ищем на YouTube, а не в общей выдаче.
static SONG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)песн|трек").expect("regex"));

/// Пользователь говорит про то же окно, что и раньше.
static THIS_WINDOW_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(окно|это|это окно)$").expect("regex"));

/// Фраза уже заканчивается точкой, вопросом или многоточием.
static SENTENCE_END_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[.!?…]$").expect("regex"));

/// «… на ютуб» в конце запроса.
static YT_TAIL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\s+(на|в) (ютуб\w*|youtube)$").expect("regex"));

/// «на ютуб …» в начале запроса.
static YT_HEAD_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^(на|в) (ютуб\w*|youtube)\s*").expect("regex"));

/// «закрой хром принудительно» — принудительное закрытие.
static FORCE_SUFFIX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\s+(принудительно|насильно|жестко|совсем)$").expect("regex"));

/// Что делать, когда пользователь выбирает из нескольких вариантов.
#[derive(Clone)]
enum Choice {
    Launch { name: String, target: String },
    Kill { name: String },
}

enum Pending {
    Power(PowerMode),
}

type Handler = fn(&mut Engine, &str, Option<&regex::Captures>) -> Option<Reply>;

pub struct Engine {
    cfg: Arc<RwLock<Config>>,
    platform: Arc<dyn Platform>,
    base_dir: PathBuf,

    sites: Map,
    apps: Map,
    aliases: Map,
    replies: Map,

    index: Arc<AppIndex>,
    ai: AiChat,
    local: Option<LocalAi>,

    pending: Option<Pending>,
    choice: Option<Vec<Choice>>,
    browser_once: Option<String>,

    autoscroll_stop: Arc<AtomicBool>,
    autoscroll_running: Arc<AtomicBool>,
    media_paused: Option<String>,
    youtube_opened_at: Option<Instant>,
    quit_requested: bool,

    routes: Vec<(Regex, Handler)>,
}

impl Engine {
    pub fn new(cfg: Arc<RwLock<Config>>, platform: Arc<dyn Platform>, base_dir: PathBuf) -> Self {
        let snapshot = cfg.read().clone();
        let index = Arc::new(AppIndex::new());
        index.build_async(Arc::clone(&platform));

        let mut engine = Self {
            cfg,
            platform,
            base_dir,
            sites: Map::new(),
            apps: Map::new(),
            aliases: Map::new(),
            replies: Map::new(),
            index,
            ai: AiChat::new(&snapshot),
            local: if snapshot.local_ai { Some(LocalAi::new(&snapshot)) } else { None },
            pending: None,
            choice: None,
            browser_once: None,
            autoscroll_stop: Arc::new(AtomicBool::new(true)),
            autoscroll_running: Arc::new(AtomicBool::new(false)),
            media_paused: None,
            youtube_opened_at: None,
            quit_requested: false,
            routes: Vec::new(),
        };
        engine.set_lists();
        engine.routes = build_routes();
        engine
    }

    // ---------------------------------------------------------------- состояние
    /// Перечитать сайты/программы/ответы из настроек (вызывается при смене config.json).
    pub fn set_lists(&mut self) {
        let cfg = self.cfg.read().clone();
        self.sites = cfg.sites.iter().map(|(k, v)| (norm(k), v.clone())).collect();
        self.apps = cfg.apps.iter().map(|(k, v)| (norm(k), v.clone())).collect();
        self.aliases = cfg.app_aliases.iter().map(|(k, v)| (norm(k), v.clone())).collect();
        self.replies = cfg
            .replies
            .iter()
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|(k, v)| (norm(k), v.clone()))
            .collect();
    }

    pub fn autoscroll_active(&self) -> bool {
        self.autoscroll_running.load(Ordering::Relaxed)
    }

    /// Остановить фоновые действия. `true` — что-то действительно остановили.
    pub fn stop_background(&self) -> bool {
        if self.autoscroll_running.swap(false, Ordering::Relaxed) {
            self.autoscroll_stop.store(true, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    pub fn index(&self) -> &Arc<AppIndex> {
        &self.index
    }

    // ---------------------------------------------------------------- вход

    pub fn handle(&mut self, cmd: &str) -> Option<Reply> {
        let c = norm(cmd);

        // «да»/«нет» в ответ на подтверждение.
        if let Some(pending) = self.pending.take() {
            if YES_RE.is_match(&c) {
                return Some(match pending {
                    Pending::Power(mode) => {
                        let noun = if matches!(mode, PowerMode::Restart) { "перезагрузка" } else { "выключение" };
                        let _ = self.platform.power(mode);
                        Reply::say(line("power_do", &[("what_noun", noun)]))
                    }
                });
            }
            if NO_RE.is_match(&c) {
                return Some(Reply::say(line("cancel", &[])));
            }
            // Не ответ — значит, новая команда, продолжаем как обычно.
        }

        // Любая новая фраза «стоп» гасит автопрокрутку.
        if self.stop_background() && STOP_SCROLL_RE.is_match(&c) {
            return Some(Reply::say(line("scroll_stop", &[])));
        }

        // Выбор из нескольких программ.
        if let Some(options) = self.choice.take() {
            if let Some(pick) = resolve_choice(&c, &options) {
                return Some(match pick {
                    Choice::Launch { name, target } => {
                        match self.platform.launch(&target) {
                            Ok(()) => Reply::say(line("launch", &[("t", &name)])),
                            Err(_) => Reply::say(line("app_missing", &[("t", &name)])),
                        }
                    }
                    Choice::Kill { name } => self.kill_by_name(&name, false),
                });
            }
            if PICK_NO_RE.is_match(&c) {
                return Some(Reply::say(line("cancel", &[])));
            }
        }

        // Свои ответы — раньше команд и нейросети.
        if let Some(custom) = self.custom_reply(&c) {
            return Some(custom);
        }

        // «запомни … как X и Y» — название целиком, а не цепочка.
        let whole = [&*REMEMBER_RE, &*FORGET_RE, &*REPLY_RE, &*FORGET_REPLY_RE]
            .iter()
            .any(|re| re.is_match(&c));
        if !whole {
            if let Some(steps) = split_chain(&c) {
                return Some(self.run_chain(&steps));
            }
        }
        self.handle_one(&c)
    }

    /// Одна команда (без разбора на цепочку).
    pub fn handle_one(&mut self, c: &str) -> Option<Reply> {
        // «открой ютуб в хроме» — браузер только для этой фразы.
        let mut browser_once = None;
        let mut missing_browser = None;
        let mut text = c.to_string();
        if let Some(caps) = BROWSER_SUFFIX.captures(c) {
            if let Some(b) = caps.name("b") {
                let word = b.as_str().to_string();
                if self.platform.find_browser(&word).is_some() {
                    browser_once = Some(word);
                } else {
                    missing_browser = Some(word);
                }
                text = c[..caps.get(0).unwrap().start()].trim().to_string();
            }
        }
        self.browser_once = browser_once;
        let mut reply = self.dispatch(&text);
        self.browser_once = None;

        if let (Some(r), Some(word)) = (reply.as_mut(), missing_browser) {
            let prefix = line("browser_missing", &[("b", &word)]);
            r.text = format!("{prefix} {}", r.text);
        }
        reply
    }

    fn dispatch(&mut self, c: &str) -> Option<Reply> {
        if let Some(custom) = self.custom_reply(c) {
            return Some(custom);
        }
        // Индексы, а не ссылки: обработчик берёт `&mut self`.
        let count = self.routes.len();
        for i in 0..count {
            let matched = self.routes[i].0.captures(c);
            let Some(caps) = matched else { continue };
            let handler = self.routes[i].1;
            if let Some(reply) = handler(self, c, Some(&caps)) {
                return Some(reply);
            }
        }
        None
    }

    /// Вопрос без ответа: поиск в Google или честное «не знаю».
    pub fn fallback(&mut self, cmd: &str, force_search: bool) -> Reply {
        let c = norm(cmd);
        if force_search || QUESTION_RE.is_match(&c) {
            if !self.cfg.read().search_when_unsure {
                return Reply::say(line("dont_know", &[]));
            }
            self.web(&format!("https://www.google.com/search?q={}", url_encode(cmd)));
            return Reply::say(line("search_q", &[]));
        }
        Reply::say(line("unknown", &[]))
    }

    // ---------------------------------------------------------------- свои ответы

    fn custom_reply(&self, c: &str) -> Option<Reply> {
        if self.replies.is_empty() {
            return None;
        }
        if let Some(text) = self.replies.get(c) {
            return Some(Reply::say(text.clone()));
        }
        if let Some(close) = crate::util::closest(c, self.replies.keys().map(|s| s.as_str()), 0.85) {
            return self.replies.get(&close).map(|t| Reply::say(t.clone()));
        }
        // Фраза целиком внутри команды и лишнего немного: «ну как дела» → «как дела».
        let mut phrases: Vec<&String> = self.replies.keys().collect();
        phrases.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
        for phrase in phrases {
            if c.chars().count().saturating_sub(phrase.chars().count()) <= 12 {
                let pattern = format!(r"(^|\s){}(\s|$)", regex::escape(phrase));
                if Regex::new(&pattern).map(|re| re.is_match(c)).unwrap_or(false) {
                    return self.replies.get(phrase).map(|t| Reply::say(t.clone()));
                }
            }
        }
        None
    }

    // ---------------------------------------------------------------- помощники

    /// Открыть сайт: в браузере из фразы, из настроек или в обычном.
    pub fn web(&self, url: &str) {
        let browser = self
            .browser_once
            .clone()
            .filter(|b| !b.is_empty())
            .or_else(|| Some(self.cfg.read().browser.clone()).filter(|b| !b.is_empty()));
        if let Err(e) = self.platform.open_url(url, browser.as_deref()) {
            log::warn!("Не смогла открыть {url}: {e}");
        }
    }

    fn launch_target(&self, target: &str) -> bool {
        match self.platform.launch(target) {
            Ok(()) => true,
            Err(e) => {
                log::info!("{target} не запустился: {e}");
                false
            }
        }
    }

    /// Запомнить/забыть сайт или программу (пишет в config.json).
    fn remember(&mut self, section: &str, name: &str, target: Option<&str>) -> Option<String> {
        let name = norm(name);
        let err = self
            .cfg
            .read()
            .set_entry(section, &name, target)
            .err();
        if err.is_some() {
            return err;
        }
        {
            let mut cfg = self.cfg.write();
            let book = match section {
                "sites" => &mut cfg.sites,
                "apps" => &mut cfg.apps,
                _ => &mut cfg.replies,
            };
            match target {
                Some(v) => {
                    book.insert(name.clone(), v.to_string());
                }
                None => {
                    book.remove(&name);
                }
            }
        }
        self.set_lists();
        None
    }

    fn kill_by_name(&self, name: &str, force: bool) -> Reply {
        let killed = self.platform.kill_process(name, force);
        if killed == 0 {
            Reply::say(line("not_running", &[("t", name)]))
        } else {
            Reply::say(line("closed", &[("names", name)]))
        }
    }

    fn media_pause_state(&mut self, value: Option<String>) {
        self.media_paused = value;
    }

    fn say_media_info(&self) -> Reply {
        match self.platform.now_playing() {
            Some(track) if !track.title.is_empty() => {
                let text = if track.artist.is_empty() {
                    track.title.clone()
                } else {
                    format!("{} — {}", track.artist, track.title)
                };
                Reply::say(line("media_info", &[("t", &text)]))
            }
            _ => Reply::say(line("media_none", &[])),
        }
    }

    /// Разбор «напиши привет и отправь».
    fn parse_type(step: &str) -> Option<(String, Option<bool>)> {
        let re = Regex::new(&format!(r"(?i)^({TYPE_VERBS})\s+(?P<t>.+)$")).ok()?;
        let caps = re.captures(step.trim())?;
        let verb = caps.get(1)?.as_str().to_lowercase();
        let mut text = caps.name("t")?.as_str().trim().to_string();
        if Regex::new(&format!("(?i){NOT_TYPING}")).ok()?.is_match(&text) {
            return None;
        }
        // «напиши» — печатает и отправляет; «напечатай/введи/набери» — только печатает.
        let mut send = if matches!(verb.as_str(), "напиши" | "пиши" | "впиши") {
            None
        } else {
            Some(false)
        };
        let end = Regex::new(r"(?i)\s+(и\s+)?(отправь|отправить|нажми энтер|нажми enter|и энтер)$").ok()?;
        if let Some(m) = end.find(&text) {
            text = text[..m.start()].trim().to_string();
            send = Some(true);
        }
        let lead = Regex::new(r"(?i)^(текст|сообщение|фразу|слово)\s+").ok()?;
        let text = lead.replace(&text, "").to_string();
        Some((text, send))
    }

    /// Цепочка шагов: «открой телеграм чат мама напиши привет».
    fn run_chain(&mut self, steps: &[String]) -> Reply {
        log::info!("Цепочка: {steps:?}");
        let mut said: Vec<String> = Vec::new();
        let mut context: Option<String> = None;

        for step in steps {
            let s = norm(step);
            if let Some((text, send)) = Self::parse_type(&s) {
                if let Some(ctx) = &context {
                    if self.platform.window_by_title(ctx).is_none() {
                        said.push(line("focus_fail", &[]));
                        return Reply::say(said.join(" "));
                    }
                }
                if let Err(e) = self.platform.type_text(&text) {
                    log::warn!("Печать не удалась: {e}");
                }
                let press = send.unwrap_or_else(|| self.cfg.read().type_press_enter);
                if press {
                    std::thread::sleep(Duration::from_millis(120));
                    let _ = self.platform.press_key(crate::keys::VK_ENTER);
                }
                said.push(line(if press { "sent" } else { "typed" }, &[("t", &text)]));
                continue;
            }

            match self.handle_one(&s) {
                Some(reply) => {
                    if reply.action_is("ask") || reply.action_is("fail") {
                        said.push(reply.text);
                        return Reply::say(said.join(" "));
                    }
                    if !reply.text.is_empty() {
                        said.push(reply.text.clone());
                    }
                    if reply.action_is(action::QUIT) || reply.action_is(action::HIDE) {
                        return reply;
                    }
                    // Запоминаем окно, с которым теперь работаем.
                    if s.starts_with("открой") || s.starts_with("запусти") || s.starts_with("зайди") {
                        if s.contains("телеграм") || s.contains("telegram") {
                            context = Some("Telegram".to_string());
                            std::thread::sleep(Duration::from_millis(600));
                        } else if s.contains("ютуб") || s.contains("youtube") {
                            context = Some("YouTube".to_string());
                            std::thread::sleep(Duration::from_millis(2500));
                        } else if s.contains("браузер") {
                            context = Some("".to_string());
                            std::thread::sleep(Duration::from_millis(2500));
                        }
                    }
                }
                None => said.push(line("chain_skip", &[("t", step)])),
            }
        }

        if said.is_empty() {
            Reply::say(line("done", &[]))
        } else {
            Reply::say(said.join(" "))
        }
    }

    fn scroll_steps(&self, dir: ScrollDirection, steps: u32) -> Result<(), String> {
        self.platform.scroll(dir, steps)
    }
}

// -------------------------------------------------------------------- таблица правил

fn build_routes() -> Vec<(Regex, Handler)> {
    fn re(pattern: &str) -> Regex {
        match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => panic!("плохое правило «{pattern}»: {e}"),
        }
    }

    vec![
        (re(r"(?i)^(выход|выключись|отключись|заверши работу|завершить работу)$"), h_quit),
        (
            re(r"(?i)^(спрячься|скройся|исчезни|пока|отбой|свободна)\b|^(уйди|уходи|убирайся|свали|сгинь|испарись|скрой себя|убери себя)\b|(уйди|уходи|убирайся|свали|пропади|исчезни) (с|из) (экрана|монитора|глаз)"),
            h_hide,
        ),
        (REMEMBER_RE.clone(), h_remember),
        (FORGET_RE.clone(), h_forget),
        (UPDATES_RE.clone(), h_updates),
        (REPLY_RE.clone(), h_reply_cmd),
        (FORGET_REPLY_RE.clone(), h_forget_reply),
        (re(r"(?i)помаши|помахай|махни|помаши ручкой|поздоровайся"), h_wave),
        (re(r"(?i)погуляй|прогуляйся|пройдись|прогулк"), h_roam),
        (
            re(r"(?i)что ты (умеешь|можешь)|как\w* (у тебя )?команд|(назови|перечисли|скажи) (\S+ )?команд|список команд|помощь$"),
            h_help,
        ),
        (
            re(r"(?i)(перемест|подвин|сдвин|переди|встань|иди|отойди|переберись)\w*\s.*(угол|влево|вправо|лев|прав|верх|низ|центр|середин|другую сторону)"),
            h_move,
        ),
        (re(r"(?i)(выключи|перезагрузи|перезапусти) (компьютер|пк|комп|систему)"), h_power),
        (re(r"(?i)заблокируй (компьютер|пк|комп|экран)"), h_lock),
        (
            re(r"(?i)громкост|громче|тише|(вкл|выкл|отключ|убер|верни|включ)\w* звук|звук (на|в) |без звука|замуть"),
            h_volume,
        ),
        (re(r"(?i)яркост|ярче|темнее"), h_brightness),
        (
            re(r"(?i)(проанализ|анализ|диагност|провер|просканир|сканир)\w*.*(пк|компьютер|комп|систем|ошибк)"),
            h_analyze,
        ),
        (
            re(r"(?i)(какой|какое) (сейчас )?(микрофон|устройство ввода)|что за микрофон|(через )?какой микрофон"),
            h_mic_info,
        ),
        (
            re(r"(?i)(смени|поменяй|переключи|следующий|другой) (\S+ )?(микрофон|устройство ввода)"),
            h_mic_next,
        ),
        (
            re(r"(?i)\b(прокрут|пролист|листай|листани|полистай|скролл|скроль|спустись|поднимись|крути (вниз|вверх))\w*|^(в |на |до )?(самый |самую |самого )?(низ|верх|конец страницы|начало страницы|низа|верха)$|^(еще |ещё )?(ниже|выше)$"),
            h_scroll,
        ),
        (
            re(r"(?i)^(поставь |поставить )?(на )?пауз\w*( музык\w*| видео| трек| песн\w*| ролик)?$|^(останови|приостанови|тормозни|стопни)( музык\w*| видео| трек| песн\w*| воспроизведение| ролик)?$|^(выключи|стоп) (музык\w*|видео|трек|песн\w*|ролик)$"),
            h_media_pause,
        ),
        (
            re(r"(?i)^(продолжи|продолжай|играй|воспроизведи|сними( с)? пауз\w*|сними)( музык\w*| видео| трек| песн\w*| воспроизведение| ролик)?$|^(включи|запусти) (дальше|обратно|снова)$|^(включи|запусти) (музык\w*|видео|трек|песн\w*) (обратно|снова|дальше)$"),
            h_media_play,
        ),
        (
            re(r"(?i)^(следующ\w*|дальше|переключи|скипни|пропусти)( трек| песн\w*| видео| композици\w*| ролик)?$|^(включи|давай) следующ\w*( трек| песн\w*| видео| ролик)?$"),
            h_media_next,
        ),
        (
            re(r"(?i)^(предыдущ\w*|прошл\w*)( трек| песн\w*| видео| композици\w*| ролик)?$|^(включи|верни) (предыдущ|прошл)\w*( трек| песн\w*| видео| ролик)?$"),
            h_media_prev,
        ),
        (re(r"(?i)что (сейчас )?(играет|звучит|за (песн|трек|музык|видео)\w*)"), h_media_info),
        (
            re(r"(?i)^(перемотай|перемотать|промотай|мотни|отмотай|прокрути|верни\w*|вернись|пропусти|скипни|перемотка|поставь|включи)( видео| музык\w*| трек| песн\w*| ролик)?( на| в| к| с)?( тайминг\w*| отметк\w*| момент\w*| время| позици\w*)?\s*(\d|полминут|полтор|начал|сначала|заново|вперед|назад|(\S+ ){0,2}(секунд|минут|час))"),
            h_media_seek,
        ),
        (
            re(r"(?i)^(нажми на поиск|нажми поиск|перейди в поиск|в поиск|поиск|поставь курсор в поиск)$"),
            h_search_field,
        ),
        (re(r"(?i)^(нажми|нажать|зажми|жми|набери сочетание) (?P<k>.+)$"), h_press),
        (re(&format!(r"(?i)^({TYPE_VERBS})\s+\S")), h_type_cmd),
        (
            re(r"(?i)^(открой |зайди в |перейди в |выбери |найди )?(в )?(телеграм\w*|telegram|телег\w*|тг )?\s*((?P<ord>перв\w+|втор\w+|трет\w+|четверт\w+|пят\w+|шест\w+|седьм\w+|восьм\w+|девят\w+|десят\w+) )?(чат\w*|chat|диалог\w*|переписк\w*|контакт\w*|беседу|беседа|группу|канал)( с| со)?( (?P<name>.+))?$"),
            h_chat,
        ),
        (
            re(r"(?i)^(открой|зайди в|перейди в|напиши в|запусти) (телеграм\w*|telegram|телег\w*|тг) (.+)$"),
            h_chat_named,
        ),
        (re(r"(?i)сверни (все|всё)|покажи рабочий стол"), h_minimize_all),
        (re(r"(?i)\bсверни\b"), h_minimize),
        (re(r"(?i)(закрой|закрыть|убери) (эту |текущую |\S+ )?вкладк"), h_tab),
        (
            re(r"(?i)нов(ую|ая) вкладк|\b(верни|восстанови) (закрыт\w* )?вкладк|(следующ|предыдущ)\w* вкладк"),
            h_tab,
        ),
        (re(r"(?i)^(создай|сделай|создать) (?P<what>.*(папк|файл|документ|таблиц).*)$"), h_create),
        (
            re(r"(?i)^(найди|покажи|открой|загугли|погугли|поищи)( мне)?( в интернете| в гугле)? (фото\w*|фотк\w*|картинк\w*|изображени\w*|пикч\w*|обои)( (с|со|про|о|об|на тему))? (?P<q>.+)$"),
            h_images,
        ),
        (
            re(r"(?i)^(найди|поищи|загугли|погугли|расскажи|узнай|посмотри)( мне)?( в интернете| в гугле| в сети)? (информаци\w*|инфу|инфо|что-нибудь|что нибудь|всё|все|сведения|данные|статью)( (о|об|про|по))? (?P<q>.+)$"),
            h_info,
        ),
        (
            re(r"(?i)^(открой|включи|покажи|найди|запусти|поставь|врубай|вруби)( мне)?( какое-нибудь| какой-нибудь| какое нибудь| любое| любой| одно| одно)? (?P<kind>видео\w*|видос\w*|ролик\w*|клип\w*|песн\w*|трек\w*|музык\w*)( на ютуб\w*| в ютуб\w*)?( (с|со|про|о|об|где|на тему))? (?P<q>.+)$"),
            h_video,
        ),
        (
            re(r"(?i)^(открой|включи|покажи|запусти|поставь)( мне)? (на|в) (ютуб\w*|youtube) (?P<q>.+)$"),
            h_video,
        ),
        (
            re(r"(?i)^(найди|поищи|загугли|погугли|ищи)( в интернете| в гугле| в google)? (?P<q>.+)$"),
            h_search,
        ),
        (
            re(r"(?i)^(открой|запусти|включи|зайди на|перейди на|покажи) (?P<t>.+)$"),
            h_open,
        ),
        (
            re(r"(?i)^((сними|снять) (задачу|процесс\w*)|(заверши|завершить|убей|убить|останови|закрой) (все )?(процесс\w*|задачу|задачи)|кильни|килльни|грохни|прибей) (?P<t>.+)$"),
            h_kill_process,
        ),
        (
            re(r"(?i)что (сейчас )?(грузит|нагружает|жрет|ест|тормозит)|(какие|какой) процесс\w* (грузят|нагружают|жрут|тормозят)|самые (тяжелые|прожорливые) процессы"),
            h_top_processes,
        ),
        (re(r"(?i)^(закрой|заверши|выключи|убей) (?P<t>.+)$"), h_close),
        (re(r"(?i)который час|сколько времени|сколько сейчас времени|время сейчас"), h_time),
        (re(r"(?i)какое (сегодня )?число|какая (сегодня )?дата|какой сегодня день"), h_date),
        (re(r"(?i)скриншот|скрин экрана|делай скрин|сделай скрин"), h_screenshot),
    ]
}

// -------------------------------------------------------------------- обработчики

fn h_quit(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    Some(Reply::act(line("quit", &[]), action::QUIT))
}

fn h_hide(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    Some(Reply::act(line("hide", &[]), action::HIDE))
}

fn h_wave(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    Some(Reply::act(line("wave", &[]), action::WAVE))
}

fn h_roam(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    Some(Reply::act(line("roam", &[]), action::ROAM))
}

fn h_help(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    Some(Reply::say(line("help", &[])))
}

fn h_move(_e: &mut Engine, c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let has = |p: &str| Regex::new(p).map(|re| re.is_match(c)).unwrap_or(false);
    let where_ = if has(r"(?i)другую сторону|в другую") {
        "other".to_string()
    } else if has(r"(?i)центр|середин") {
        "center".to_string()
    } else {
        let h = if has(r"(?i)лев|влево") {
            "left"
        } else if has(r"(?i)прав|вправо") {
            "right"
        } else {
            ""
        };
        let v = if has(r"(?i)верх|наверх") {
            "top"
        } else if has(r"(?i)низ|вниз") {
            "bottom"
        } else {
            ""
        };
        let parts: Vec<&str> = [v, h].into_iter().filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            "other".to_string()
        } else {
            parts.join("-")
        }
    };
    Some(Reply::act(line("move", &[]), format!("{}{}", action::MOVE, where_)))
}

fn h_power(e: &mut Engine, c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let restart = c.starts_with("перезаг") || c.starts_with("перезап") || c.contains("перезагруз")
        || c.contains("перезапуст");
    let verb = if restart { "перезагрузить" } else { "выключить" };
    let verb_cap = format!("{}{}", verb.chars().next()?.to_uppercase(), &verb[verb.chars().next()?.len_utf8()..]);
    e.pending = Some(Pending::Power(if restart { PowerMode::Restart } else { PowerMode::Shutdown }));
    Some(Reply::say(line("power_ask", &[("what", verb), ("what_cap", &verb_cap)])))
}

fn h_lock(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    match e.platform.lock_workstation() {
        Ok(()) => Some(Reply::say(line("lock", &[]))),
        Err(err) => Some(Reply::say(line("error", &[("e", &err)]))),
    }
}

fn h_volume(e: &mut Engine, c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let has = |p: &str| Regex::new(p).map(|re| re.is_match(c)).unwrap_or(false);

    if has(r"(?i)(выключи|выключить|отключи|убери|убрать)\w* звук|без звука|замуть") {
        let _ = e.platform.set_mute(true);
        return Some(Reply::say(line("mute", &[])));
    }
    if has(r"(?i)(включи|включить|верни)\w* звук") && parse_number(c).is_none() {
        let _ = e.platform.set_mute(false);
        return Some(Reply::say(line("unmute", &[])));
    }

    let n = parse_number(c);
    let cur = e.platform.get_volume().unwrap_or(50) as i64;
    let new = if has(r"(?i)громче|прибав|увелич|повыс|выше") {
        cur + n.unwrap_or(10)
    } else if has(r"(?i)тише|убав|уменьш|пониз|ниже") {
        cur - n.unwrap_or(10)
    } else if let Some(value) = n {
        value
    } else {
        return Some(Reply::say(line("vol_now", &[("n", &cur.to_string())])));
    };
    let new = new.clamp(0, 100) as u8;
    match e.platform.set_volume(new) {
        Ok(()) => Some(Reply::say(line("vol_set", &[("n", &new.to_string())]))),
        Err(err) => Some(Reply::say(line("error", &[("e", &err)]))),
    }
}

fn h_brightness(e: &mut Engine, c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let has = |p: &str| Regex::new(p).map(|re| re.is_match(c)).unwrap_or(false);
    let Some(cur) = e.platform.get_brightness() else {
        return Some(Reply::say(line("bright_fail", &[])));
    };
    let cur = cur as i64;
    let n = parse_number(c);
    let new = if has(r"(?i)ярче|прибав|увелич|повыс|выше") {
        cur + n.unwrap_or(15)
    } else if has(r"(?i)темнее|убав|уменьш|пониз|ниже") {
        cur - n.unwrap_or(15)
    } else if let Some(value) = n {
        value
    } else {
        return Some(Reply::say(line("bright_now", &[("n", &cur.to_string())])));
    };
    let new = new.clamp(0, 100) as u8;
    match e.platform.set_brightness(new) {
        Ok(()) => Some(Reply::say(line("bright_set", &[("n", &new.to_string())]))),
        Err(_) => Some(Reply::say(line("bright_reject", &[]))),
    }
}

fn h_analyze(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let report = e.platform.analyze_pc();
    let folder = e.base_dir.join("reports");
    let _ = std::fs::create_dir_all(&folder);
    let file = folder.join(format!("report_{}.txt", report_stamp()));
    let text = render_report(&report);
    let _ = std::fs::write(&file, &text);
    e.launch_target(&file.to_string_lossy());

    // Пересказ нейросетью, если она есть.
    if e.cfg.read().enable_ai {
        if let Some(spoken) = e.ai.summarize_report(&text) {
            return Some(Reply::say(spoken));
        }
    }
    if let Some(local) = &e.local {
        if let Some(spoken) = local.summarize_report(&text) {
            return Some(Reply::say(spoken));
        }
    }
    if report.problems.is_empty() {
        return Some(Reply::say(line("scan_ok", &[])));
    }
    let head = report.problems.iter().take(4).cloned().collect::<Vec<_>>().join("; ");
    Some(Reply::say(line(
        "scan_bad",
        &[("n", &report.problems.len().to_string()), ("head", &head)],
    )))
}

fn h_mic_info(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    Some(Reply::act("", action::MIC_INFO))
}

fn h_mic_next(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    Some(Reply::act("", action::MIC_NEXT))
}

fn h_scroll(e: &mut Engine, c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let has = |p: &str| Regex::new(p).map(|re| re.is_match(c)).unwrap_or(false);
    let up = has(r"(?i)\b(вверх|наверх|выше|верх\w*|начал\w*|поднимись|назад)\b");
    let down = has(r"(?i)\b(вниз|ниже|низ\w*|конец|дальше|спустись|вперед)\b");
    let auto = has(r"(?i)\b(медленно|плавно|автоматически|сама|постепенно|прокручивай|листай сама)\b")
        || has(r"(?i)^(листай|полистай|прокручивай|скроль|скролль)( медленно| плавно)?$");

    if !(up || down || auto) || has(r"(?i)секунд|минут|\bчас|\d[:.]\d\d|полминут") {
        return None; // «прокрути на 30 секунд» — это перемотка видео
    }
    let direction = if up && !down { ScrollDirection::Up } else { ScrollDirection::Down };
    e.stop_background();

    if auto {
        let fast = has(r"(?i)быстр");
        e.autoscroll_running.store(true, Ordering::Relaxed);
        e.autoscroll_stop.store(false, Ordering::Relaxed);
        let stop = Arc::clone(&e.autoscroll_stop);
        let running = Arc::clone(&e.autoscroll_running);
        let platform = Arc::clone(&e.platform);
        let interval = if fast { 250 } else { 600 };
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(300);
            while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                let _ = platform.scroll(direction, 1);
                std::thread::sleep(Duration::from_millis(interval));
            }
            running.store(false, Ordering::Relaxed);
        });
        return Some(Reply::say(line("scroll_auto", &[])));
    }

    if has(r"(?i)\b(самый|самую|самого|до конца|в конец|в начало|до низа|до верха|конец страницы|начало страницы)\b|^(в |на |до )?(низ|верх|низа|верха)$")
    {
        let target = if matches!(direction, ScrollDirection::Up) {
            ScrollDirection::Top
        } else {
            ScrollDirection::Bottom
        };
        let _ = e.scroll_steps(target, 1);
        return Some(Reply::say(line(
            if matches!(direction, ScrollDirection::Up) { "scroll_top" } else { "scroll_end" },
            &[],
        )));
    }

    let notches = if has(r"(?i)\b(немного|чуть|чуть-чуть|слегка|капельку)\b|^(еще )?(ниже|выше)$") {
        3
    } else if let Some(n) = parse_number(c) {
        if has(r"(?i)экран|страниц|раз") {
            (n.clamp(1, 20) * 6) as u32
        } else {
            (n.clamp(1, 30)) as u32
        }
    } else {
        6
    };
    let _ = e.scroll_steps(direction, notches);
    Some(Reply::say(line("scroll", &[])))
}

fn h_media_pause(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    match e.platform.media(MediaCommand::Pause) {
        Ok(()) => {
            e.media_pause_state(Some("playing".into()));
            Some(Reply::say(line("media_paused", &[])))
        }
        Err(_) => Some(Reply::say(line("media_none", &[]))),
    }
}

fn h_media_play(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    match e.platform.media(MediaCommand::Play) {
        Ok(()) => Some(Reply::say(line("media_play", &[]))),
        Err(_) => Some(Reply::say(line("media_none", &[]))),
    }
}

fn h_media_next(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    match e.platform.media(MediaCommand::Next) {
        Ok(()) => {
            std::thread::sleep(Duration::from_millis(700));
            match e.platform.now_playing() {
                Some(track) if !track.title.is_empty() => {
                    Some(Reply::say(line("media_next", &[("t", &track.title)])))
                }
                _ => Some(Reply::say(line("done", &[]))),
            }
        }
        Err(_) => Some(Reply::say(line("media_none", &[]))),
    }
}

fn h_media_prev(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    match e.platform.media(MediaCommand::Prev) {
        Ok(()) => Some(Reply::say(line("done", &[]))),
        Err(_) => Some(Reply::say(line("media_none", &[]))),
    }
}

fn h_media_info(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    Some(e.say_media_info())
}

fn h_media_seek(e: &mut Engine, c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let has = |p: &str| Regex::new(p).map(|re| re.is_match(c)).unwrap_or(false);
    let back = has(r"(?i)\bназад\b|\bотмотай|\bверни|\bвернись");
    let fwd = has(r"(?i)\bвперед|\bпромотай|\bпропусти|\bскипни|\bмотни");

    let cleaned = Regex::new(r"(?i)^(поставь|включи|начни|запусти|давай|играй|воспроизведи|перемотай|перемотать|промотай|мотни|отмотай|прокрути|верни\w*|вернись|пропусти|скипни|перемотка)\s+")
        .ok()
        .map(|re| re.replace(c, "").to_string())
        .unwrap_or_else(|| c.to_string());
    let cleaned = Regex::new(r"(?i)\b(вперед|назад|на|с|со|в|к|видео|музык\w*|трек|песн\w*|ролик|момент\w*|отметк\w*|тайминг)\b")
        .ok()
        .map(|re| re.replace_all(&cleaned, " ").to_string())
        .unwrap_or(cleaned);

    let mut seconds = parse_time(&cleaned);
    let to_start = has(r"(?i)начал|сначала|заново");
    let absolute = to_start || (!back && !fwd);

    if seconds.is_none() {
        if !(back || fwd) {
            return None;
        }
        seconds = Some(10.0); // «перемотай вперёд» без числа — на 10 секунд
    }
    let secs = seconds.unwrap_or(10.0);

    let result = if absolute {
        e.platform.media_seek(secs)
    } else {
        let current = e.platform.now_playing().and_then(|t| t.position).unwrap_or(0.0);
        let target = if back { (current - secs).max(0.0) } else { current + secs };
        e.platform.media_seek(target)
    };

    match result {
        Ok(()) => Some(Reply::say(line("media_seek", &[("t", &format_time(secs))]))),
        Err(_) => Some(Reply::say(line("media_seek_fail", &[]))),
    }
}

fn h_type_cmd(e: &mut Engine, c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let (text, send) = Engine::parse_type(c)?;
    // На YouTube «напиши X» — это поиск X, а не печать в страницу.
    let recent_yt = e
        .youtube_opened_at
        .map(|t| t.elapsed() < Duration::from_secs(120))
        .unwrap_or(false);
    let foreground = e.platform.foreground_window();
    let on_youtube = foreground
        .as_ref()
        .map(|w| w.title.to_lowercase().contains("youtube"))
        .unwrap_or(false);

    if on_youtube || recent_yt {
        e.web(&format!("https://www.youtube.com/results?search_query={}", url_encode(&text)));
        return Some(Reply::say(line("yt", &[("q", &text)])));
    }

    let press = send.unwrap_or_else(|| e.cfg.read().type_press_enter);
    if let Err(err) = e.platform.type_text(&text) {
        return Some(Reply::say(line("error", &[("e", &err)])));
    }
    if press {
        std::thread::sleep(Duration::from_millis(120));
        let _ = e.platform.press_key(crate::keys::VK_ENTER);
    }
    Some(Reply::say(line(if press { "sent" } else { "typed" }, &[("t", &text)])))
}

fn h_search_field(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let _ = e.platform.send_keys(&[KeyStroke::new(crate::keys::VK_CTRL), KeyStroke::new('F' as u16)]);
    Some(Reply::say(line("search_ready", &[])))
}

fn h_chat(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let name = m
        .and_then(|c| c.name("name"))
        .map(|n| n.as_str().to_string())
        .map(|n| stem(&n))
        .filter(|n| !n.is_empty());
    let Some(name) = name else {
        // «чат» без имени — просто открыть Telegram.
        return match e.platform.telegram_send("", "", false) {
            Ok(()) => Some(Reply::say(line("chat_open", &[("t", "Telegram")]))),
            Err(_) => Some(Reply::say(line("app_missing", &[("t", "Telegram")]))),
        };
    };
    match e.platform.telegram_send(&name, "", false) {
        Ok(()) => Some(Reply::say(line("chat_open", &[("t", &name)]))),
        Err(_) => Some(Reply::act(
            line("chat_not_found", &[("t", &name)]),
            "fail",
        )),
    }
}

fn h_chat_named(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let name = m.and_then(|c| c.get(3)).map(|n| n.as_str().to_string())?;
    // «открой телеграм папа» — но не «открой телеграм и напиши».
    if name.starts_with("и ") || name.starts_with("напиши") {
        return None;
    }
    match e.platform.telegram_send(&name, "", false) {
        Ok(()) => Some(Reply::say(line("chat_open", &[("t", &name)]))),
        Err(_) => Some(Reply::act(
            line("chat_not_found", &[("t", &name)]),
            "fail",
        )),
    }
}

fn h_minimize_all(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let _ = e.platform.minimize_all();
    Some(Reply::say(line("done", &[])))
}

fn h_minimize(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    if let Some(window) = e.platform.foreground_window() {
        let _ = e.platform.minimize_window(window.handle);
    }
    Some(Reply::say(line("done", &[])))
}

fn h_tab(e: &mut Engine, c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let has = |p: &str| Regex::new(p).map(|re| re.is_match(c)).unwrap_or(false);
    if has(r"(?i)нов(ую|ая) вкладк") {
        let _ = e.platform.tab_command(TabCommand::New);
        return Some(Reply::say(line("tab_new", &[])));
    }
    if has(r"(?i)\b(верни|восстанови)") {
        let _ = e.platform.tab_command(TabCommand::ReopenClosed);
        return Some(Reply::say(line("tab_restore", &[])));
    }
    if has(r"(?i)следующ") {
        let _ = e.platform.tab_command(TabCommand::Next);
        return Some(Reply::say(line("done", &[])));
    }
    if has(r"(?i)предыдущ") {
        let _ = e.platform.tab_command(TabCommand::Prev);
        return Some(Reply::say(line("done", &[])));
    }
    let n = parse_number(c).unwrap_or(1).clamp(1, 20);
    for _ in 0..n {
        let _ = e.platform.tab_command(TabCommand::Close);
        std::thread::sleep(Duration::from_millis(120));
    }
    Some(Reply::say(line(
        if n > 1 { "tab_closed" } else { "tab_closed_one" },
        &[("n", &n.to_string())],
    )))
}

fn h_press(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let text = m.and_then(|c| c.name("k")).map(|k| k.as_str().to_string())?;
    let mut times = 1i64;
    let mut keys_text = text.clone();
    if let Some(re) = Regex::new(r"(?i)\s(\S+) раз\w*$").ok() {
        if let Some(caps) = re.captures(&text) {
            if let Some(n) = caps.get(1).and_then(|g| parse_number(g.as_str())) {
                times = n.clamp(1, 30);
                keys_text = text[..caps.get(0).unwrap().start()].to_string();
            }
        }
    }
    match parse_keys(&keys_text) {
        Ok(keys) => {
            for _ in 0..times {
                if let Err(err) = e.platform.send_keys(&keys) {
                    return Some(Reply::say(line("error", &[("e", &err)])));
                }
                std::thread::sleep(Duration::from_millis(80));
            }
            Some(Reply::say(line("pressed", &[("t", &describe_keys(&keys))])))
        }
        Err(unknown) => Some(Reply::say(line("key_unknown", &[("t", &unknown)]))),
    }
}

fn h_create(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let what = m.and_then(|c| c.name("what")).map(|w| w.as_str().to_string())?;

    // Куда создавать.
    let mut place = e.platform.known_folder("desktop");
    let mut place_name = "на рабочем столе".to_string();
    let mut rest = what.clone();
    if let Ok(re) = Regex::new(
        r"(?i)\s(на|в|во)\s+(рабоч\w+ стол\w*|документ\w*|загрузк\w*|фото\w*|изображени\w*|картинк\w*|музык\w*|видео|папк\w* \S+)$",
    ) {
        if let Some(caps) = re.captures(&what) {
            let spot_word = caps.get(2).map(|g| g.as_str().to_string()).unwrap_or_default();
            let folder_key = match spot_word.as_str() {
                s if s.starts_with("рабоч") => Some("desktop"),
                s if s.starts_with("документ") => Some("documents"),
                s if s.starts_with("загрузк") => Some("downloads"),
                s if s.starts_with("фото") || s.starts_with("изображени") || s.starts_with("картинк") => Some("pictures"),
                s if s.starts_with("музык") => Some("music"),
                s if s.starts_with("видео") => Some("videos"),
                s if s.starts_with("папк") => {
                    let name = s.trim_start_matches("папк").trim();
                    return match e.platform.search_file(name) {
                        Some(path) => {
                            place = Some(path.clone());
                            place_name = format!("в папке «{}»", path.file_name().unwrap_or_default().to_string_lossy());
                            rest = what[..caps.get(0).unwrap().start()].to_string();
                            create_in(e, &place, &place_name, &rest)
                        }
                        None => Some(Reply::say(line("folder_missing", &[("t", name)]))),
                    };
                }
                _ => None,
            };
            if let Some(key) = folder_key {
                place = e.platform.known_folder(key);
            }
            rest = what[..caps.get(0).unwrap().start()].to_string();
        }
    }
    create_in(e, &place, &place_name, &rest)
}

/// Создать папку/файл по разобранному описанию.
fn create_in(
    _e: &mut Engine,
    place: &Option<PathBuf>,
    place_name: &str,
    what: &str,
) -> Option<Reply> {
    let Some(dir) = place.clone() else {
        return Some(Reply::say(line("folder_missing", &[("t", "папка")])));
    };
    let Ok(re) = Regex::new(
        r"(?i)^(нов\w+ )?(?P<kind>папк\w*|текстов\w+ файл\w*|текстов\w+ документ\w*|документ\w*|файл\w*|таблиц\w*)( (с )?(именем|названием))?\s*(?P<name>.*)$",
    ) else {
        return None;
    };
    let caps = re.captures(what.trim())?;
    let kind = caps.name("kind")?.as_str().to_lowercase();
    let mut name = caps.name("name").map(|n| n.as_str().trim().to_string()).unwrap_or_default();

    // «файл заметки точка txt»
    let mut ext = String::new();
    if let Ok(dot) = Regex::new(r"(?i)\s*(точка|dot|\.)\s*(?P<e>[a-zа-я0-9]+)$") {
        if let Some(dot_caps) = dot.captures(&name) {
            ext = format!(".{}", translit(dot_caps.name("e").map(|g| g.as_str()).unwrap_or("txt")));
            name = name[..dot_caps.get(0).unwrap().start()].trim().to_string();
        }
    }

    let default = if kind.starts_with("папк") {
        "Новая папка".to_string()
    } else if kind.starts_with("таблиц") {
        if ext.is_empty() {
            ext = ".xlsx".into();
        }
        "Новая таблица".to_string()
    } else if kind.starts_with("документ") && !kind.starts_with("текст") {
        if ext.is_empty() {
            ext = ".docx".into();
        }
        "Новый документ".to_string()
    } else {
        if ext.is_empty() {
            ext = ".txt".into();
        }
        "Новый файл".to_string()
    };

    let clean: String = name.chars().filter(|c| !matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')).collect();
    let mut name = clean.trim().to_string();
    if name.is_empty() {
        name = default;
    }
    let mut name = name.trim().to_string();
    if let Some(first) = name.chars().next() {
        name.replace_range(0..first.len_utf8(), &first.to_uppercase().to_string());
    }

    let mut path = dir.join(format!("{name}{ext}"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{name} ({n}){ext}"));
        n += 1;
    }

    let created = if kind.starts_with("папк") {
        std::fs::create_dir_all(&path).is_ok()
    } else {
        std::fs::OpenOptions::new().write(true).create_new(true).open(&path).is_ok()
    };
    let shown = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    if created {
        Some(Reply::say(line("created", &[("t", &shown), ("where", place_name)])))
    } else {
        Some(Reply::say(line("error", &[("e", "не смогла создать файл")])))
    }
}

fn h_images(e: &mut Engine, c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let mut q = m.and_then(|c| c.name("q")).map(|q| q.as_str().trim().to_string())?;
    if c.contains("обои") {
        q = format!("обои {q}");
    }
    e.web(&format!("https://www.google.com/search?tbm=isch&q={}", url_encode(&q)));
    Some(Reply::say(line("images", &[("q", &q)])))
}

fn h_info(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let q = m.and_then(|c| c.name("q")).map(|q| q.as_str().trim().to_string())?;
    e.web(&format!("https://www.google.com/search?q={}", url_encode(&q)));
    match wiki_summary(&q) {
        Some(summary) => Some(Reply::say(line("info", &[("t", &summary)]))),
        None => Some(Reply::say(line("web", &[("q", &q)]))),
    }
}

fn h_video(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let caps = m?;
    let mut q = caps.name("q").map(|q| q.as_str().trim().to_string())?;
    let kind = caps.name("kind").map(|k| k.as_str().to_string()).unwrap_or_default();
    q = YT_TAIL_RE.replace(&q, "").to_string();
    q = YT_HEAD_RE.replace(&q, "").to_string();
    if q.is_empty() || matches!(q.as_str(), "на" | "в" | "с" | "про" | "о") {
        q = "интересное видео".to_string();
    }
    let search = if SONG_RE.is_match(&kind) {
        format!("{q} песня")
    } else {
        q.clone()
    };

    match youtube_first(&search) {
        Some((id, title)) => {
            e.web(&format!("https://www.youtube.com/watch?v={id}"));
            e.youtube_opened_at = Some(Instant::now());
            let mut title = title.split_whitespace().collect::<Vec<_>>().join(" ");
            if title.chars().count() > 70 {
                title = format!("{}…", title.chars().take(67).collect::<String>());
            }
            Some(Reply::say(line(
                if title.is_empty() { "video_untitled" } else { "video" },
                &[("t", &title)],
            )))
        }
        None => {
            e.web(&format!("https://www.youtube.com/results?search_query={}", url_encode(&search)));
            e.youtube_opened_at = Some(Instant::now());
            Some(Reply::say(line("yt", &[("q", &q)])))
        }
    }
}

fn h_search(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let mut q = m.and_then(|c| c.name("q")).map(|q| q.as_str().trim().to_string())?;
    if let Ok(re) = Regex::new(r"(?i)(на|в) (ютубе|ютуб|youtube)") {
        if let Some(found) = re.find(&q) {
            q = q.replace(found.as_str(), "").trim().to_string();
            e.web(&format!("https://www.youtube.com/results?search_query={}", url_encode(&q)));
            return Some(Reply::say(line("yt", &[("q", &q)])));
        }
    }
    e.web(&format!("https://www.google.com/search?q={}", url_encode(&q)));
    Some(Reply::say(line("web", &[("q", &q)])))
}

fn h_open(e: &mut Engine, c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let raw = m.and_then(|c| c.name("t")).map(|t| t.as_str().to_string())?;
    if c.contains("ютуб") || c.contains("youtube") {
        e.youtube_opened_at = Some(Instant::now());
    }
    let mut t = norm(&raw);
    for prefix in ["мне ", "пожалуйста ", "программу ", "приложение "] {
        if let Some(stripped) = t.strip_prefix(prefix) {
            t = stripped.to_string();
        }
    }
    if let Some(stripped) = t.strip_suffix(" пожалуйста") {
        t = stripped.trim().to_string();
    }
    if t == "звук" {
        return None;
    }

    let site_mode = t.starts_with("сайт ");
    if site_mode {
        t = t.trim_start_matches("сайт ").trim().to_string();
    }

    // Папки.
    if !site_mode {
        let explicit = Regex::new(r"(?i)^(папк\w*|каталог\w*|директори\w*)\s+(?P<n>.+)$")
            .ok()
            .and_then(|re| re.captures(&t))
            .and_then(|caps| caps.name("n").map(|n| n.as_str().to_string()));
        let query = explicit.clone().unwrap_or_else(|| t.clone());
        if let Some(folder) = find_folder(e, &query, explicit.is_some()) {
            e.launch_target(&folder.to_string_lossy());
            let shown = folder
                .file_name()
                .map(|f| f.to_string_lossy().to_string())
                .unwrap_or_else(|| query.clone());
            return Some(Reply::say(line("folder_open", &[("t", &shown)])));
        }
        if let Some(name) = explicit {
            return Some(Reply::say(line("folder_missing", &[("t", &name)])));
        }
    }

    // Свои программы и сайты из настроек.
    if !site_mode {
        if let Some(target) = e.apps.get(&t).cloned() {
            let ok = e.launch_target(&target);
            return Some(Reply::say(line(
                if ok { "open" } else { "app_missing" },
                &[("t", &t)],
            )));
        }
    }
    if let Some(url) = e.sites.get(&t).cloned() {
        e.web(&url);
        return Some(Reply::say(line("open", &[("t", &t)])));
    }
    if let Some(close) = crate::util::closest(&t, e.sites.keys().map(|s| s.as_str()), 0.82) {
        if let Some(url) = e.sites.get(&close).cloned() {
            e.web(&url);
            return Some(Reply::say(line("open", &[("t", &close)])));
        }
    }

    // Похоже на адрес сайта?
    let domain = Regex::new(r"(?i)\s*(точка|dot)\s*")
        .ok()
        .map(|re| re.replace_all(&t, ".").to_string())
        .unwrap_or_else(|| t.clone())
        .replace(' ', "");
    if Regex::new(r"(?i)^[\w\-]+(\.[\w\-]+)*\.[a-zа-я]{2,}$")
        .map(|re| re.is_match(&domain))
        .unwrap_or(false)
    {
        e.web(&format!("https://{domain}"));
        return Some(Reply::say(line("open", &[("t", &domain)])));
    }

    // Установленные программы.
    if !site_mode {
        let options = e.index.candidates(&t, &e.aliases, 3);
        if options.is_empty() {
            return Some(Reply::say(line("app_missing", &[("t", &t)])));
        }
        let top = options[0].clone();
        let similar: Vec<_> = options[1..]
            .iter()
            .filter(|o| top.0 - o.0 < 0.1)
            .cloned()
            .collect();

        if top.0 >= 1.0 || similar.is_empty() {
            if e.launch_target(&top.2) {
                return Some(Reply::say(line("launch", &[("t", &top.1)])));
            }
            // Ярлык не открылся — пробуем похожие ярлыки того же приложения.
            for (name, target) in e.index.similar(&top.1) {
                if e.launch_target(&target) {
                    return Some(Reply::say(line("launch", &[("t", &name)])));
                }
            }
            return Some(Reply::say(line("app_missing", &[("t", &t)])));
        }

        // Несколько похожих — спрашиваем, какое открыть.
        let mut choices: Vec<Choice> = vec![Choice::Launch {
            name: top.1.clone(),
            target: top.2.clone(),
        }];
        for (_, name, target) in similar.into_iter().take(2) {
            choices.push(Choice::Launch { name, target });
        }
        let listed = choices
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let name = match ch {
                    Choice::Launch { name, .. } => name.clone(),
                    Choice::Kill { name } => name.clone(),
                };
                format!("{} — {}", ordinal_ru(i), capitalize(&name))
            })
            .collect::<Vec<_>>()
            .join(", ");
        e.choice = Some(choices);
        return Some(Reply::act(line("choose_app", &[("t", &listed)]), "ask"));
    }

    e.web(&format!("https://www.google.com/search?q={}", url_encode(&t)));
    Some(Reply::say(line("site_unknown", &[("t", &t)])))
}

fn h_kill_process(e: &mut Engine, c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let raw = m.and_then(|c| c.name("t")).map(|t| t.as_str().to_string())?;
    let mut t = raw.trim().to_string();
    if let Ok(re) = Regex::new(r"(?i)^(все |всех )?(процесс\w*|задач\w*)\s+") {
        t = re.replace(&t, "").to_string();
    }
    let force = c.contains("принудительн") || c.contains("насильн") || c.contains("жестк") || c.contains("совсем");
    close_target(e, &t, force)
}

fn h_close(e: &mut Engine, c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let t = m.and_then(|c| c.name("t")).map(|t| t.as_str().to_string())?;
    let force = c.contains("принудительн") || c.contains("насильн") || c.contains("жестк");
    close_target(e, &t, force)
}

/// Закрыть программу или окно по названию.
fn close_target(e: &mut Engine, raw: &str, force: bool) -> Option<Reply> {
    let mut t = raw.trim().to_string();
    t = FORCE_SUFFIX_RE.replace(&t, "").trim().to_string();

    // «закрой окно» — Alt+F4.
    if THIS_WINDOW_RE.is_match(&t) {
        let _ = e.platform.send_keys(&[KeyStroke::new(crate::keys::VK_ALT), KeyStroke::new(0x73)]);
        return Some(Reply::say(line("closed_window", &[])));
    }

    let q = norm(&t);
    let mut variants: Vec<String> = vec![q.clone(), translit(&q)];
    if let Some(alias) = e.aliases.get(&q) {
        variants.push(norm(alias));
    }
    if let Some(target) = e.apps.get(&q) {
        if let Some(stem_name) = Path::new(target)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
        {
            variants.push(stem_name);
        }
    }
    for (_, name, _) in e.index.candidates(&q, &e.aliases, 2) {
        if let Some(first) = name.split_whitespace().next() {
            variants.push(first.to_string());
        }
    }
    variants = variants
        .into_iter()
        .map(|v| v.replace(' ', ""))
        .filter(|v| v.chars().count() >= 3)
        .collect();

    let processes = e.platform.processes();
    let mut scored: Vec<(f64, String)> = Vec::new();
    for proc in &processes {
        let name = proc.name.to_lowercase().replace(".exe", "");
        if name.is_empty() || PROTECTED.contains(&name.as_str()) {
            continue;
        }
        let mut best: f64 = 0.0;
        for v in &variants {
            let score = if name == *v {
                1.0
            } else if name.starts_with(v.as_str())
                || (name.chars().count() >= 4 && v.starts_with(&name))
            {
                0.9
            } else {
                let mut s = crate::util::similarity(v, &name);
                if crate::util::sounds_like(v, &name) {
                    s = s.max(0.86);
                }
                s
            };
            best = best.max(score);
        }
        if best >= 0.85 {
            match scored.iter_mut().find(|(_, n)| *n == name) {
                Some(entry) => entry.0 = entry.0.max(best),
                None => scored.push((best, name)),
            }
        }
    }

    if scored.is_empty() {
        return Some(Reply::say(line("not_running", &[("t", &t)])));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let top = scored[0].clone();
    let close: Vec<_> = scored[1..]
        .iter()
        .filter(|s| top.0 - s.0 < 0.1)
        .cloned()
        .collect();

    if top.0 < 1.0 && !close.is_empty() {
        let mut choices: Vec<Choice> = vec![Choice::Kill { name: top.1.clone() }];
        for (_, name) in close.into_iter().take(2) {
            choices.push(Choice::Kill { name });
        }
        let listed = choices
            .iter()
            .enumerate()
            .map(|(i, ch)| {
                let name = match ch {
                    Choice::Kill { name } => name.clone(),
                    Choice::Launch { name, .. } => name.clone(),
                };
                format!("{} — {name}", ordinal_ru(i))
            })
            .collect::<Vec<_>>()
            .join(", ");
        e.choice = Some(choices);
        return Some(Reply::act(line("choose_close", &[("t", &listed)]), "ask"));
    }

    Some(e.kill_by_name(&top.1, force))
}

fn h_top_processes(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let procs = e.platform.processes();
    let mut by_cpu: Vec<_> = procs
        .iter()
        .filter(|p| !PROTECTED.contains(&p.name.to_lowercase().as_str()))
        .cloned()
        .collect();
    by_cpu.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
    let mut by_mem = procs;
    by_mem.sort_by(|a, b| b.mem_mb.partial_cmp(&a.mem_mb).unwrap_or(std::cmp::Ordering::Equal));

    let cpu_text = by_cpu
        .iter()
        .take(3)
        .filter(|p| p.cpu >= 1.0)
        .map(|p| format!("{} {:.0} процентов", p.name, p.cpu))
        .collect::<Vec<_>>()
        .join(", ");
    let cpu_text = if cpu_text.is_empty() { "ничего особо не грузит".to_string() } else { cpu_text };
    let mem_text = by_mem
        .iter()
        .take(3)
        .map(|p| {
            if p.mem_mb >= 1024.0 {
                format!("{} {:.1} гигабайта", p.name, p.mem_mb / 1024.0)
            } else {
                format!("{} {:.0} мегабайт", p.name, p.mem_mb)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    Some(Reply::say(line("top_procs", &[("cpu", &cpu_text), ("mem", &mem_text)])))
}

fn h_time(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    let now = chrono::Local::now();
    Some(Reply::say(line("time", &[("t", &now.format("%H:%M").to_string())])))
}

fn h_date(_e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    const MONTHS: [&str; 12] = [
        "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября",
        "октября", "ноября", "декабря",
    ];
    const DAYS: [&str; 7] = [
        "понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье",
    ];
    use chrono::{Datelike, Local};
    let today = Local::now().date_naive();
    let text = format!(
        "{}, {} {}",
        DAYS[today.weekday().num_days_from_monday() as usize],
        today.day(),
        MONTHS[today.month0() as usize]
    );
    Some(Reply::say(line("date", &[("t", &text)])))
}

fn h_screenshot(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    match e.platform.screenshot() {
        Ok(path) => {
            log::info!("Скриншот: {}", path.display());
            Some(Reply::say(line("screenshot", &[])))
        }
        Err(err) => Some(Reply::say(line("error", &[("e", &err)]))),
    }
}

// -------------------------------------------------------------------- запоминание

fn h_remember(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let caps = m?;
    let what = caps.get(1)?.as_str().to_string();
    let name = caps.name("name").map(|n| n.as_str().trim().to_string()).unwrap_or_default();
    let name = Regex::new(r"(?i)^(как|под названием|под именем|с названием|назови)\s+")
        .ok()
        .map(|re| re.replace(&name, "").to_string())
        .unwrap_or(name)
        .trim_matches(|c: char| c == '«' || c == '»' || c == '"' || c == ' ')
        .to_string();
    let is_site = Regex::new(r"(?i)сайт|страниц|вкладк|ссылк")
        .map(|re| re.is_match(&what))
        .unwrap_or(false);

    if name.is_empty() {
        return Some(Reply::say(line(
            "remember_noname",
            &[("what", if is_site { "этот сайт" } else { "эту программу" })],
        )));
    }

    let foreground = e.platform.foreground_window();
    if is_site {
        let Some(window) = foreground else {
            return Some(Reply::say(line("remember_nosite", &[])));
        };
        let Some(url) = e.platform.window_url(window.handle) else {
            return Some(Reply::say(line("remember_nosite", &[])));
        };
        let err = e.remember("sites", &name, Some(&url));
        if let Some(err) = err {
            return Some(Reply::say(line("remember_fail", &[("e", &err)])));
        }
        let short = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("www.")
            .split('/')
            .next()
            .unwrap_or(&url)
            .to_string();
        return Some(Reply::say(line("remember_site", &[("t", &name), ("url", &short)])));
    }

    let Some(window) = foreground else {
        return Some(Reply::say(line("remember_noapp", &[])));
    };
    let Some(exe) = window.exe else {
        return Some(Reply::say(line("remember_noapp", &[])));
    };
    let base = Path::new(&exe)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if base.is_empty() || PROTECTED.contains(&base.as_str()) || base == "applicationframehost" {
        return Some(Reply::say(line("remember_noapp", &[])));
    }
    if let Some(err) = e.remember("apps", &name, Some(&exe)) {
        return Some(Reply::say(line("remember_fail", &[("e", &err)])));
    }
    Some(Reply::say(line("remember_app", &[("t", &name), ("app", &base)])))
}

fn h_forget(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let caps = m?;
    let what = caps.get(1)?.as_str().to_string();
    let name = norm(caps.name("name")?.as_str());
    let site = Regex::new(r"(?i)сайт|страниц|вкладк|ссылк")
        .map(|re| re.is_match(&what))
        .unwrap_or(false);
    let exists = if site { e.sites.contains_key(&name) } else { e.apps.contains_key(&name) };
    if !exists {
        return Some(Reply::say(line("forget_missing", &[("t", &name)])));
    }
    let section = if site { "sites" } else { "apps" };
    match e.remember(section, &name, None) {
        Some(err) => Some(Reply::say(line("remember_fail", &[("e", &err)]))),
        None => Some(Reply::say(line("forget_ok", &[("t", &name)]))),
    }
}

fn h_updates(e: &mut Engine, _c: &str, _m: Option<&regex::Captures>) -> Option<Reply> {
    e.web(CAPRI_URL);
    Some(Reply::say(line("updates", &[])))
}

fn h_reply_cmd(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let caps = m?;
    let phrase = caps.name("p")?.as_str().trim_matches(|c: char| c == ' ' || c == ',').to_string();
    let mut text = caps.name("r")?.as_str().trim_matches(|c: char| c == ' ' || c == ',').to_string();
    if let Some(first) = text.chars().next() {
        text.replace_range(0..first.len_utf8(), &first.to_uppercase().to_string());
    }
    if !SENTENCE_END_RE.is_match(&text) {
        text.push('.');
    }
    match e.remember("replies", &phrase, Some(&text)) {
        Some(err) => Some(Reply::say(line("remember_fail", &[("e", &err)]))),
        None => Some(Reply::say(line("reply_saved", &[("p", &phrase), ("r", &text)]))),
    }
}

fn h_forget_reply(e: &mut Engine, _c: &str, m: Option<&regex::Captures>) -> Option<Reply> {
    let phrase = norm(m?.name("p")?.as_str());
    let key = if e.replies.contains_key(&phrase) {
        Some(phrase.clone())
    } else {
        crate::util::closest(&phrase, e.replies.keys().map(|s| s.as_str()), 0.85)
    };
    let Some(key) = key else {
        return Some(Reply::say(line("forget_missing", &[("t", &phrase)])));
    };
    match e.remember("replies", &key, None) {
        Some(err) => Some(Reply::say(line("remember_fail", &[("e", &err)]))),
        None => Some(Reply::say(line("forget_ok", &[("t", &key)]))),
    }
}

// -------------------------------------------------------------------- утилиты

fn ordinal_ru(i: usize) -> &'static str {
    match i {
        0 => "первое",
        1 => "второе",
        _ => "третье",
    }
}

fn capitalize(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(first) = out.chars().next() {
        out.replace_range(0..first.len_utf8(), &first.to_uppercase().to_string());
    }
    out
}

/// Найти папку по названию: известные папки, диски, потом поиск по диску.
fn find_folder(e: &Engine, name: &str, search: bool) -> Option<PathBuf> {
    let key = norm(name);
    // «диск д» / «диск c».
    if let Some(caps) = Regex::new(r"(?i)^(диск|раздел) (?P<l>.+)$").ok().and_then(|re| re.captures(&key)) {
        let letter = caps.name("l")?.as_str().chars().next()?;
        let path = PathBuf::from(format!("{}:\\", letter.to_ascii_uppercase()));
        if path.is_dir() {
            return Some(path);
        }
    }
    for known in ["pictures", "documents", "downloads", "desktop", "music", "videos"] {
        if let Some(path) = e.platform.known_folder(known) {
            let folder_name = norm(&path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default());
            if folder_name == key || crate::util::similarity(&folder_name, &key) > 0.8 {
                return Some(path);
            }
        }
    }
    if let Some(path) = e.platform.known_folder(&key) {
        return Some(path);
    }
    if let Some(path) = e.platform.search_file(name) {
        if path.is_dir() {
            return Some(path);
        }
    }
    if search {
        return e.platform.search_file(name);
    }
    None
}

/// «314» → 3:14, «полминуты» → 30 с, «на 2 минуты 30 секунд» → 150 с.
fn parse_time(text: &str) -> Option<f64> {
    let has = |p: &str| Regex::new(p).map(|re| re.is_match(text)).unwrap_or(false);
    if has(r"(?i)полминут") {
        return Some(30.0);
    }
    if has(r"(?i)полтор") {
        return Some(90.0);
    }
    let cleaned = norm(text);
    // «3:14» или «3.14»
    if let Ok(re) = Regex::new(r"(?P<a>\d{1,3})[:.](\d{2})") {
        if let Some(caps) = re.captures(&cleaned) {
            let minutes: f64 = caps.get(1)?.as_str().parse().ok()?;
            let seconds: f64 = caps.get(2)?.as_str().parse().ok()?;
            return Some(minutes * 60.0 + seconds);
        }
    }
    // «2 минуты 30 секунд» / «45 секунд» / «7 минут»
    let mut total = 0.0;
    let mut found = false;
    if let Ok(re) = Regex::new(r"(?i)(\d+)\s*(час\w*|минут\w*|секунд\w*)") {
        for caps in re.captures_iter(&cleaned) {
            let value: f64 = caps.get(1)?.as_str().parse().ok()?;
            let unit = caps.get(2)?.as_str();
            if unit.starts_with("час") {
                total += value * 3600.0;
            } else if unit.starts_with("минут") {
                total += value * 60.0;
            } else {
                total += value;
            }
            found = true;
        }
    }
    if found {
        return Some(total);
    }
    // «на 314» без двоеточия — это 3:14.
    let digits: String = cleaned.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 3 {
        let n: f64 = digits.parse().ok()?;
        return Some((n / 100.0).trunc() * 60.0 + (n % 100.0));
    }
    if digits.is_empty() {
        return None;
    }
    Some(digits.parse().ok()?)
}

fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h} ч {m} мин")
    } else if m > 0 && s > 0 {
        format!("{m} мин {s} с")
    } else if m > 0 {
        format!("{m} мин")
    } else {
        format!("{s} с")
    }
}

/// Кодирование в URL (без внешних крейтов).
pub fn url_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*byte as char),
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn report_stamp() -> String {
    chrono::Local::now().format("%Y%m%d_%H%M%S").to_string()
}

fn render_report(report: &crate::platform::PcReport) -> String {
    let mut text = String::from("Капри — отчёт о компьютере\n==========================\n\n");
    text.push_str(&format!("Процессор: {} (загрузка {:.0}%)\n", report.cpu_name, report.cpu_load));
    text.push_str(&format!(
        "Память: {:.1} из {:.1} ГБ\n",
        report.ram_used_gb, report.ram_total_gb
    ));
    text.push_str(&format!("Видеокарта: {}\n", report.gpu_name));
    text.push_str(&format!("Время работы: {:.1} ч\n\n", report.uptime_hours));
    text.push_str("Диски:\n");
    for (name, free, total) in &report.disks {
        text.push_str(&format!("  {name} — свободно {free:.1} из {total:.1} ГБ\n"));
    }
    text.push_str("\nПроблемы:\n");
    if report.problems.is_empty() {
        text.push_str("  не найдено\n");
    } else {
        for problem in &report.problems {
            text.push_str(&format!("  • {problem}\n"));
        }
    }
    text.push_str("\nСоветы:\n");
    if report.advice.is_empty() {
        text.push_str("  всё в порядке\n");
    } else {
        for advice in &report.advice {
            text.push_str(&format!("  • {advice}\n"));
        }
    }
    text
}

fn resolve_choice(c: &str, options: &[Choice]) -> Option<Choice> {
    let ordinals = [
        (r"(?i)перв\w*|один|одну|1", 0usize),
        (r"(?i)втор\w*|два|две|2", 1),
        (r"(?i)трет\w*|три|3", 2),
        (r"(?i)последн\w*", options.len().saturating_sub(1)),
    ];
    for (pattern, idx) in ordinals {
        if idx < options.len()
            && Regex::new(&format!(r"(^|\s)({pattern})(\s|$)")).map(|re| re.is_match(c)).unwrap_or(false)
        {
            return Some(options[idx].clone());
        }
    }
    // По названию.
    let words = Regex::new(r"(?i)^(открой|запусти|давай|вот|это|ну)\s+")
        .ok()
        .map(|re| re.replace(c, "").to_string())
        .unwrap_or_else(|| c.to_string());
    let mut best = 0.0;
    let mut best_choice = None;
    for choice in options {
        let name = match choice {
            Choice::Launch { name, .. } | Choice::Kill { name } => name.clone(),
        };
        let mut score = crate::util::similarity(&words, &name);
        if !words.is_empty() && name.contains(&words) {
            score = 1.0;
        }
        let first = name.split_whitespace().next().unwrap_or(&name);
        if crate::util::sounds_like(&words, first) {
            score = score.max(0.86);
        }
        if score > best {
            best = score;
            best_choice = Some(choice.clone());
        }
    }
    if best >= 0.6 {
        best_choice
    } else {
        None
    }
}

/// «открой телеграм первый чат напиши привет» → шаги цепочки.
/// `None` — шаг один, цепочки нет.
pub fn split_chain(cmd: &str) -> Option<Vec<String>> {
    let text = cmd.trim();
    if text.is_empty() {
        return None;
    }
    let verbs = Regex::new(&format!(r"(?i)\b({TYPE_VERBS})\b")).ok()?;
    let (head, tail) = match verbs.find(text) {
        Some(m) => (text[..m.start()].trim().to_string(), text[m.start()..].trim().to_string()),
        None => (text.to_string(), String::new()),
    };

    const START_WORDS: &[&str] = &[
        "открой", "откройте", "запусти", "зайди", "перейди", "нажми", "найди", "поищи", "прокрути",
        "пролистай", "листай", "полистай", "скролль", "промотай", "закрой", "сверни", "включи",
        "выключи", "поставь", "введи", "набери", "покажи", "выбери", "отправь",
    ];
    const ORDINALS: &[&str] = &[
        "первый", "первую", "второй", "вторую", "третий", "третью", "четвертый", "четвёртый",
        "пятый", "шестой", "седьмой", "восьмой", "девятый", "десятый",
    ];
    const CHAT_WORDS: &[&str] = &["чат", "chat", "диалог", "переписк", "контакт"];

    let is_chat_word = |w: &str| CHAT_WORDS.iter().any(|c| w.starts_with(c));
    let connectors = Regex::new(r"(?i)\s+(?:и|потом|затем|после этого)\s+").ok()?;

    let mut parts: Vec<String> = Vec::new();
    for chunk in connectors.split(&head) {
        let chunk = chunk.trim().trim_matches(|c: char| c == ',' || c == '.').trim();
        if chunk.is_empty() {
            continue;
        }
        // Режем чанк на границах шагов: «открой телеграм первый чат» → два шага.
        let words: Vec<&str> = chunk.split_whitespace().collect();
        let mut current: Vec<&str> = Vec::new();
        for (i, word) in words.iter().enumerate() {
            let lower = word.to_lowercase();
            let next = words.get(i + 1).map(|n| n.to_lowercase()).unwrap_or_default();
            let starts_ordinal_chat = ORDINALS.contains(&lower.as_str()) && is_chat_word(&next);
            let is_start = START_WORDS.contains(&lower.as_str())
                || starts_ordinal_chat
                || (is_chat_word(&lower) && i > 0 && lower != "в");
            if is_start && !current.is_empty() {
                parts.push(current.join(" "));
                current.clear();
            }
            current.push(word);
        }
        if !current.is_empty() {
            parts.push(current.join(" "));
        }
    }
    if !tail.is_empty() {
        parts.push(tail);
    }

    // Одинокое «третий» приклеиваем к следующему шагу: «третий» + «чат с мамой».
    let mut merged: Vec<String> = Vec::new();
    for part in parts {
        let single_ordinal = {
            let words: Vec<&str> = part.split_whitespace().collect();
            words.len() == 1 && ORDINALS.contains(&words[0].to_lowercase().as_str())
        };
        if single_ordinal {
            merged.push(part);
            continue;
        }
        match merged.last() {
            Some(last)
                if last.split_whitespace().count() == 1
                    && ORDINALS.contains(&last.to_lowercase().as_str()) =>
            {
                if let Some(last) = merged.last_mut() {
                    last.push(' ');
                    last.push_str(&part);
                }
            }
            _ => merged.push(part),
        }
    }

    if merged.len() >= 2 {
        Some(merged)
    } else {
        None
    }
}

/// Первое видео на YouTube по запросу (id, название).
fn youtube_first(query: &str) -> Option<(String, String)> {
    let url = format!("https://www.youtube.com/results?search_query={}", url_encode(query));
    let mut response = ureq::get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Capri/2.0")
        .header("Accept-Language", "ru-RU,ru;q=0.9")
        .config()
        .timeout_global(Some(Duration::from_secs(12)))
        .build()
        .call()
        .ok()?;
    let html = response.body_mut().read_to_string().ok()?;

    let id_re = Regex::new(r#"\"videoId\":\"([A-Za-z0-9_-]{11})\""#).ok()?;
    let id = id_re.captures(&html)?.get(1)?.as_str().to_string();

    // Название: первый «title» с текстом рядом с этим id, иначе — первый title.
    let title_re = Regex::new(r#""title":\{"runs":\[\{"text":"([^"]{3,200})""#).ok()?;
    let title = title_re
        .captures(&html)
        .and_then(|c| c.get(1).map(|t| t.as_str().to_string()))
        .map(|t| unescape_json(&t))
        .unwrap_or_default();
    Some((id, title))
}

/// Начало статьи из Википедии, если она есть.
fn wiki_summary(query: &str) -> Option<String> {
    let title = url_encode(&query.replace(' ', "_"));
    let url = format!("https://ru.wikipedia.org/api/rest_v1/page/summary/{title}");
    let mut response = ureq::get(&url)
        .header("User-Agent", "Capri/2.0 (голосовой ассистент)")
        .config()
        .timeout_global(Some(Duration::from_secs(6)))
        .build()
        .call()
        .ok()?;
    let value: serde_json::Value = response.body_mut().read_json().ok()?;
    let extract = value.get("extract")?.as_str()?.to_string();
    // Первые два предложения — дальше слушать невозможно.
    let mut sentences = 0;
    let mut out = String::new();
    for ch in extract.chars() {
        out.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            sentences += 1;
            if sentences >= 2 {
                break;
            }
        }
    }
    let out = out.trim().to_string();
    if out.chars().count() < 20 {
        None
    } else {
        Some(out)
    }
}

fn unescape_json(text: &str) -> String {
    text.replace("\\u0026", "&")
        .replace("\\\"", "\"")
        .replace("\\n", " ")
        .replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_chain_of_steps() {
        let steps = split_chain("открой телеграм первый чат напиши привет").unwrap();
        assert_eq!(steps[0], "открой телеграм");
        assert!(steps.last().unwrap().starts_with("напиши"));
    }

    #[test]
    fn single_command_is_not_a_chain() {
        assert!(split_chain("открой ютуб").is_none());
    }

    #[test]
    fn parses_type_steps() {
        let (text, send) = Engine::parse_type("напиши привет").unwrap();
        assert_eq!(text, "привет");
        assert_eq!(send, None);
        let (text, send) = Engine::parse_type("напечатай привет").unwrap();
        assert_eq!(text, "привет");
        assert_eq!(send, Some(false));
        assert!(Engine::parse_type("напиши стих про осень").is_none());
    }

    #[test]
    fn parses_media_times() {
        assert_eq!(parse_time("3:14"), Some(194.0));
        assert_eq!(parse_time("45 секунд"), Some(45.0));
        assert_eq!(parse_time("2 минуты 30 секунд"), Some(150.0));
    }

    #[test]
    fn encodes_urls() {
        assert_eq!(url_encode("привет мир"), "%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82+%D0%BC%D0%B8%D1%80");
    }
}
