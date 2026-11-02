//! Капри 2.0 — голосовой помощник с характером.
//!
//! Запуск: `capri.exe` рядом с `config.json`, `assets/` и `models/`.
//! Подробности — в `README.md`.

// В релизе — без чёрного окна консоли: это настольный питомец, а не утилита.
// В отладочной сборке консоль остаётся, иначе не видно паник и `println!`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capri;
mod logger;
mod presence;
mod wake;

use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use capri_core::persona;
use capri_core::platform::MonitorInfo;
use capri_core::{Config, Platform};
use capri_ui::{CharacterSetting, UiEvent, UiOptions, UiRequest};
use capri_win::WindowsPlatform;
use parking_lot::RwLock;

use crate::capri::{Capri, Deps};
use crate::logger as log;

/// Сколько уровней вверх смотрим в поисках `config.json`.
const MAX_UP: usize = 5;

/// Имя мьютекса «одна Капри на пользователя».
///
/// Без `Global\`: два разных пользователя могут держать свои копии, а один и
/// тот же — нет.
const SINGLE_INSTANCE_NAME: &str = "Capri2.SingleInstance";

fn main() -> std::process::ExitCode {
    let console = std::env::args().any(|arg| arg == "--console");
    if console {
        // Собраны как оконное приложение — просим консоль у родителя,
        // иначе `--console` печатал бы в никуда.
        attach_console();
    }

    let base_dir = find_base_dir();
    log::init(&base_dir, console);

    // Всё, что упало, — в лог по-русски, а не в никуда.
    std::panic::set_hook(Box::new(panic_hook));

    log::write_line(
        "INFO",
        &format!(
            "Капри 2.0 запускается, каталог: {}, лог: {}",
            base_dir.display(),
            log::log_path().display()
        ),
    );

    // Второй экземпляр не нужен: два окна, две иконки в трее и одна горячая
    // клавиша на двоих. Пусть лучше скажет, что уже работает.
    let _guard = match capri_win::InstanceGuard::acquire(SINGLE_INSTANCE_NAME) {
        Some(guard) => guard,
        None => {
            log::write_line("WARN", "Капри уже запущена — зову первый экземпляр");
            // Обычно человек просто потерял окно: показываем его, а окно с
            // предупреждением оставляем на случай, когда окна не нашлось.
            if !capri_win::summon_running_instance(
                capri_ui::AVATAR_CLASS,
                capri_ui::WM_CAPRI_SUMMON,
            ) {
                log::write_line("WARN", "Окно первого экземпляра не нашлось");
                capri_win::already_running_notice(
                    "Капри",
                    "Я уже тут, только окна не видно.\n\nЗагляни в трей — я там.",
                );
            }
            return std::process::ExitCode::from(0);
        }
    };

    let code = run(base_dir);
    log::write_line("INFO", &format!("Капри 2.0 завершилась, код {code}"));
    std::process::ExitCode::from(code as u8)
}

/// Взять консоль у запустившего процесса — для режима `--console`.
fn attach_console() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
        unsafe {
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}

fn panic_hook(info: &PanicHookInfo) {
    let where_ = info
        .location()
        .map(|l| format!("{}:{}", l.file(), l.line()))
        .unwrap_or_else(|| "неизвестно".into());
    log::write_line("ERROR", &format!("Непредвиденная ошибка в {where_}: {info}"));
}

/// Каталог, где лежат `config.json`, `assets/`, `models/`.
///
/// Порядок: папка программы, её родители (до [`MAX_UP`] уровней — так
/// `target\release\capri.exe` находит настройки в корне проекта), затем
/// текущий каталог. И двойной клик, и запуск из терминала дают одно и то же.
fn find_base_dir() -> PathBuf {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()));

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = &exe_dir {
        candidates.push(dir.clone());
        let mut up = dir.parent();
        for _ in 0..MAX_UP {
            match up {
                Some(path) => {
                    candidates.push(path.to_path_buf());
                    up = path.parent();
                }
                None => break,
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }

    for candidate in &candidates {
        if candidate.join("config.json").is_file() {
            return candidate.clone();
        }
    }
    // Файла настроек ещё нет — годится каталог с ресурсами.
    for candidate in &candidates {
        if candidate.join("assets").is_dir() {
            return candidate.clone();
        }
    }
    // Совсем пусто — берём папку программы (или текущую).
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| PathBuf::from("."))
}

fn run(base_dir: PathBuf) -> i32 {
    let config_path = base_dir.join("config.json");
    let config = Config::load(&config_path);
    if let Some(warning) = &config.load_warning {
        log::write_line("WARN", &format!("config.json: {warning}"));
    }
    if !config_path.is_file() {
        log::write_line(
            "WARN",
            "config.json не найден — работаю со значениями по умолчанию",
        );
        if let Err(e) = config.save() {
            log::write_line("WARN", &format!("Не смогла создать config.json: {e}"));
        }
    }

    // Обращение к пользователю знает вся персона.
    persona::set_user(config.user_name.clone(), &config.user_gender);
    // Характер можно переписать в persona.json — он лежит рядом с config.json.
    persona::load_persona(&base_dir);

    let platform: Arc<dyn Platform> = Arc::new(WindowsPlatform::new(base_dir.clone()));
    let cfg = Arc::new(RwLock::new(config));
    // Список мониторов читаем один раз: он нужен и окну (на каком экране
    // жить), и панели настроек (пункт «Экран»).
    let monitors = platform.monitors();
    let speaker = Arc::new(capri_voice::Speaker::new(
        &cfg.read(),
        &capri_voice::cache_root(),
        // Промежуточный WAV для голоса Windows пишем рядом с программой:
        // SAPI отказывается работать с путями в AppData (0x80070057).
        Some(&base_dir),
    ));

    let (events_tx, events_rx) = channel::<UiEvent>();
    let (requests_tx, requests_rx) = channel::<UiRequest>();

    // Эквалайзер медиа-виджета: фоновый поток читает WASAPI loopback.
    // Стартуем до панелей, чтобы к открытию виджета полосы уже были живыми.
    capri_win::audio_loop::start();

    // Discord Rich Presence: отдельный поток, который сам решает, что и когда
    // отправить. Выключен в конфиге или пустой Application ID — не запускаем
    // вовсе, чтобы не тратить канал на заведомо безнадёжное подключение.
    start_presence(&cfg, platform.clone());

    // Панели интерфейса (WebView2) — свой поток и свой цикл событий:
    // главный поток занят окном аватара.
    let (panels_tx, panels_rx) = channel::<capri_panels::Command>();
    let (panel_events_tx, panel_events_rx) = channel::<capri_panels::PanelEvent>();
    let panels_quit = panels_tx.clone();
    // Отдельный канал запросов — для отладочного автооткрытия панели.
    let requests_for_panel = requests_tx.clone();

    std::thread::spawn(move || {
        let code = capri_panels::run(panels_rx, panel_events_tx);
        log::write_line("INFO", &format!("Панели завершились, код {code}"));
    });

    {
        let cfg = Arc::clone(&cfg);
        let base_dir = base_dir.clone();
        let monitors = monitors.clone();
        let panels_tx = panels_tx.clone();
        let requests_tx = requests_tx.clone();
        let events_tx = events_tx.clone();
        std::thread::spawn(move || {
            panel_dispatch(
                panel_events_rx,
                &cfg,
                &base_dir,
                &monitors,
                &panels_tx,
                &requests_tx,
                &events_tx,
            );
        });
    }

    let mic_level = Arc::new(AtomicU32::new(0f32.to_bits()));
    let current_mic = Arc::new(RwLock::new(None::<String>));

    // Медиа-виджет: пока он открыт, оболочка сама опрашивает плеер и шлёт
    // странице состояние и полосы эквалайзера. Обложку кэшируем — читать её
    // из SMTC каждые полсекунды дорого и незачем: она меняется вместе с треком.
    {
        let cfg = Arc::clone(&cfg);
        let panels_tx = panels_tx.clone();
        // Плеер нужен и Discord RPC, и виджету — берём ещё одну ссылку.
        let media = Arc::new(capri_win::WindowsPlatform::new(base_dir.clone()));
        std::thread::spawn(move || media_feeder(media, panels_tx, cfg));
    }

    let deps = Deps {
        base_dir: base_dir.clone(),
        cfg: Arc::clone(&cfg),
        platform,
        speaker,
        events: events_tx,
        mic_level: Arc::clone(&mic_level),
        current_mic: Arc::clone(&current_mic),
        panels: panels_tx,
    };

    let (capri, jobs, _bridge) = Capri::start(deps, requests_rx);

    // Отладка интерфейса: `CAPRI_PANEL=settings` открывает панель сразу при
    // старте — удобно проверять её, не ловя правый клик по аватарке.
    // Путь тот же, что и из меню: запрос окна → ядро → панели.
    if let Ok(name) = std::env::var("CAPRI_PANEL") {
        if !name.trim().is_empty() {
            log::write_line("INFO", &format!("CAPRI_PANEL={name}: открываю панель при старте"));
            let _ = requests_for_panel.send(UiRequest::OpenPanel(name));
        }
    }

    // Отладка масштаба: `CAPRI_SCALE=55` делает то же, что ползунок «Размер»,
    // но без мыши — так окно можно проверить в автоматическом прогоне.
    if let Ok(value) = std::env::var("CAPRI_SCALE") {
        if let Ok(percent) = value.trim().parse::<u8>() {
            log::write_line("INFO", &format!("CAPRI_SCALE={percent}: меняю размер при старте"));
            let _ = requests_for_panel.send(UiRequest::Character(CharacterSetting::Scale(percent)));
        }
    }

    // Отладка команд: `CAPRI_ASK=Капри, напомни через минуту …` проходит ровно
    // тот же путь, что и голос: окно → ядро → разбор → действие. Нужно, чтобы
    // проверять поведение без микрофона — иначе голосовые умения не проверить
    // автоматически вообще никак.
    if let Ok(text) = std::env::var("CAPRI_ASK") {
        if !text.trim().is_empty() {
            log::write_line("INFO", &format!("CAPRI_ASK: {text}"));
            let _ = requests_for_panel.send(UiRequest::Command(text));
        }
    }

    // Отладка кольца: `CAPRI_RADIAL=shot` — то же, что клик по сектору.
    // Без этого проверить секторы можно только мышью по кольцу, а оно
    // закрывается при потере фокуса.
    if let Ok(id) = std::env::var("CAPRI_RADIAL") {
        if !id.trim().is_empty() {
            log::write_line("INFO", &format!("CAPRI_RADIAL: {}", id.trim()));
            let _ = requests_for_panel.send(UiRequest::RadialAction(id.trim().to_string()));
        }
    }

    // Ядро работает в своём потоке, окно — в главном: Windows требует,
    // чтобы окно жило именно в том потоке, который его создал. О завершении
    // сообщает канал: так главный поток может подождать с пределом, а не
    // навсегда.
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let mut capri = capri;
        let code = capri.run(jobs);
        let _ = done_tx.send(code);
        code
    });

    let options = UiOptions {
        assets_dir: base_dir.join("assets"),
        base_dir,
        mic_level,
        current_mic,
        monitors: monitors.clone(),
    };

    let code = capri_ui::run(options, Arc::clone(&cfg), events_rx, requests_tx);

    // Окно закрылось — просим ядро закончить. Без этого просьба о выходе
    // терялась: окно исчезало, а процесс оставался в диспетчере задач.
    let _ = requests_for_panel.send(UiRequest::Quit);

    // Окно закрылось — гасим и цикл панелей, чтобы не держать поток.
    let _ = panels_quit.send(capri_panels::Command::Quit);

    // Окно закрылось — просим ядро закончить и не держим поток вечно.
    log::write_line("INFO", "Окно закрыто");
    // Ждём, но не бесконечно: если ядро застряло в речи или в сети, всё равно
    // выходим. Иначе процесс остаётся висеть в диспетчере задач и держит имя
    // единственного экземпляра — следующий запуск молча ничего не делает.
    match done_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(_) => {
            if worker.join().is_err() {
                log::write_line("WARN", "Поток ядра завершился с ошибкой");
            }
        }
        Err(_) => {
            log::write_line(
                "WARN",
                "Ядро не остановилось за пять секунд — выхожу принудительно",
            );
        }
    }
    code
}

/// Запустить трансляцию «что слушает Капри» в Discord.
///
/// Модуль сам ничего не знает про медиаплеер: ему передают функцию, которая
/// отвечает на вопрос «что показывать прямо сейчас». Поэтому Discord RPC
/// можно проверить тестами без Discord, а медиа — без Discord RPC.
fn start_presence(cfg: &Arc<RwLock<Config>>, platform: Arc<dyn Platform>) {
    let (app_id, enabled, level) = {
        let config = cfg.read();
        (
            config.discord_app_id.trim().to_string(),
            config.discord_enabled,
            capri_core::save::SaveManager::new(
                config.path.parent().unwrap_or(std::path::Path::new(".")),
            )
            .load()
            .level,
        )
    };

    if !enabled {
        log::write_line("INFO", "Discord Rich Presence выключен в настройках");
        return;
    }
    if app_id.is_empty() {
        log::write_line(
            "WARN",
            "Discord Rich Presence включён, но discord_app_id пуст — \
             впиши Application ID из dev-портала Discord",
        );
        return;
    }

    log::write_line("INFO", &format!("Discord Rich Presence: приложение {app_id}"));
    presence::spawn(app_id, move || {
        let track = platform.now_playing()?;
        let now = chrono::Utc::now().timestamp();
        let activity = presence::activity_for(&track, level, now);
        if activity.details.is_empty() && activity.state.is_empty() {
            None
        } else {
            Some(activity)
        }
    });
}

/// Шахматная партия, которая живёт между вызовами страницы.
///
/// Отдельный модуль, а не поле в диспетчере: страница присылает ходы
/// по одному, а бот думает в своём потоке — без общей сессии пришлось бы
/// тащить позицию через каналы.
mod chess_session {
    use capri_core::chess::{self, Bot, Game, Level, Outcome, Side};

    /// Что показывает страница.
    pub struct Session {
        game: Game,
        level: Level,
        /// Цвет, которым играет человек.
        my_side: Side,
        /// Фраза Капри под доской.
        comment: String,
        /// Идёт ли сейчас ход бота.
        thinking: bool,
        /// Награда за текущую партию уже начислена.
        ///
        /// Страница перерисовывается на каждое действие, поэтому без этой
        /// отметки XP за победу начислялся бы при каждом обновлении.
        pub rewarded: bool,
    }

    impl Default for Session {
        fn default() -> Self {
            Self::new(Level::Normal, Side::White)
        }
    }

    impl Session {
        pub fn new(level: Level, my_side: Side) -> Self {
            let mut session = Self {
                game: Game::new(),
                level,
                my_side,
                comment: String::new(),
                thinking: false,
                rewarded: false,
            };
            session.comment = session.opening_line();
            session
        }

        pub fn level(&self) -> Level {
            self.level
        }

        pub fn set_level(&mut self, level: Level) {
            self.level = level;
        }

        pub fn my_side(&self) -> Side {
            self.my_side
        }

        pub fn set_my_side(&mut self, side: Side) {
            self.my_side = side;
        }

        pub fn game(&self) -> &Game {
            &self.game
        }

        /// Подменить партию — только для тестов.
        ///
        /// Обычным кодом не нужно: позиция всегда приходит из ходов.
        #[cfg(test)]
        pub fn set_game_for_test(&mut self, game: Game) {
            self.game = game;
        }

        /// Ход человека по тексту `e2e4`.
        pub fn play_mine(&mut self, from: &str, to: &str) -> Result<(), String> {
            if self.game.outcome().is_some() {
                return Err("партия уже закончена".into());
            }
            if self.game.turn() != self.my_side {
                return Err("сейчас не твой ход".into());
            }

            let text = format!("{from}{to}");
            let was = self.game.material();
            self.game.play_text(&text)?;
            self.comment = self.react(was);
            Ok(())
        }

        /// Ход бота, если сейчас его очередь.
        ///
        /// Возвращает `None`, если ходить не надо (не его ход или партия
        /// закончена) — так вызывающий код не обязан знать правила.
        pub fn bot_reply(&mut self) -> Option<Result<String, String>> {
            if self.game.outcome().is_some() || self.game.turn() == self.my_side {
                return None;
            }
            let board = self.game.board().clone();
            let mv = Bot::new(self.level).best_move(&board)?;

            let was = self.game.material();
            let text = chess::move_text(mv);
            if let Err(why) = self.game.play(mv) {
                return Some(Err(why));
            }
            self.comment = self.react(was);
            Some(Ok(text))
        }

        /// Отменить ход. Для удобства отменяем пару «мой + ответ бота»,
        /// чтобы игрок снова оказался в своей позиции.
        pub fn undo(&mut self) -> bool {
            if !self.game.undo() {
                return false;
            }
            // Если теперь ход бота — значит, мы отменили только его ответ,
            // и надо отменить ещё и свой ход.
            if self.game.turn() != self.my_side && self.game.undo() {
                // Отменили оба.
            }
            true
        }

        /// Новая партия с текущими настройками.
        pub fn restart(&mut self) {
            self.game = Game::new();
            self.comment = self.opening_line();
            self.thinking = false;
            self.rewarded = false;
        }

        /// Что показать странице.
        ///
        /// `progress` — снимок сохранения: баланс ECS и счёт партий. Без него
        /// страница не может показать ни «165 ECS» в шапке, ни строку
        /// «Счёт против Капри» под доской.
        pub fn state(&self, progress: &capri_core::save::SaveData) -> serde_json::Value {
            let (white, black) = self.game.material();
            let outcome = self.game.outcome();
            serde_json::json!({
                "squares": self.game.squares(),
                "moves": self.game.moves_json(),
                "turn": self.game.turn().as_str(),
                "my_side": self.my_side.as_str(),
                "level": self.level.as_str(),
                "in_check": self.game.in_check(),
                "outcome": outcome.map(Outcome::label),
                "winner": self.game.winner().map(Side::as_str),
                "material": { "white": white, "black": black },
                "ply": self.game.ply(),
                "last_move": self.game.last_move().map(chess::move_text),
                "history": self.history(),
                "thinking": self.thinking,
                "comment": self.comment,
                "money": progress.money,
                "wins": progress.wins,
                "games": progress.games,
                "draws": progress.draws,
            })
        }

        /// Ходы в человеческом виде для списка справа.
        fn history(&self) -> Vec<String> {
            // `Game` не отдаёт историю наружу целиком — проигрываем её через
            // позицию заново было бы дорого. Поэтому берём то, что копится
            // в самом `Game`: пара «откуда-куда» на каждый полуход.
            self.game.history_text()
        }

        fn opening_line(&self) -> String {
            match self.my_side {
                Side::White => "Твой ход. Я подожду — мне не впервой.".to_string(),
                Side::Black => "Я начинаю. Смотри внимательно.".to_string(),
            }
        }

        /// Короткая реакция на ход — без ИИ, чтобы не тормозить игру.
        ///
        /// Комментарий собирается по факту, а не угадывается: взятие, шах,
        /// мат. Так фраза всегда уместна, даже если ИИ недоступен.
        fn react(&self, was: (i32, i32)) -> String {
            if self.game.outcome().is_some() {
                return match self.game.winner() {
                    Some(side) if side == self.my_side => "Ты победил. Я запомню этот мат.".into(),
                    Some(_) => "Мат. В следующий раз я буду добрее.".into(),
                    None => "Ничья. Достойно.".into(),
                };
            }
            if self.game.in_check() {
                return "Шах. Интересно, как ты выкрутишься.".into();
            }

            let captured = chess::captured_value(was, self.game.material());
            if captured >= 300 {
                // Ходил бот или человек — зависит от того, чей ход теперь.
                return if self.game.turn() == self.my_side {
                    "Забрала твою фигуру — спасибо, было вкусно.".into()
                } else {
                    "Одной фигурой меньше. Я это записала.".into()
                };
            }
            if captured > 0 {
                return "Пешку забираю.".into();
            }
            "Думаю.".into()
        }
    }
}

/// Партия в шашки: своя сессия рядом с шахматной.
///
/// Две сессии живут независимо и обе уезжают странице при каждом действии:
/// поэтому переключение вкладок «ШАХМАТЫ / ШАШКИ / ОЧКО» ничего не сбрасывает.
mod checkers_session {
    use capri_core::checkers::{self, Bot, Game, Level, Move, Outcome, Side, Square};

    pub struct Session {
        game: Game,
        level: Level,
        /// Цвет, которым играет человек.
        my_side: Side,
        /// Фраза Капри под доской.
        comment: String,
        /// Идёт ли ход бота.
        thinking: bool,
        /// Награда за эту партию уже начислена.
        pub rewarded: bool,
        /// Сыгранные ходы — для списка справа.
        history: Vec<String>,
    }

    impl Default for Session {
        fn default() -> Self {
            Self::new(Level::Normal, Side::White)
        }
    }

    impl Session {
        pub fn new(level: Level, my_side: Side) -> Self {
            let mut session = Self {
                game: Game::new(),
                level,
                my_side,
                comment: String::new(),
                thinking: false,
                rewarded: false,
                history: Vec::new(),
            };
            session.comment = session.opening_line();
            session
        }

        pub fn level(&self) -> Level {
            self.level
        }

        pub fn set_level(&mut self, level: Level) {
            self.level = level;
        }

        pub fn my_side(&self) -> Side {
            self.my_side
        }

        pub fn set_my_side(&mut self, side: Side) {
            self.my_side = side;
        }

        pub fn game(&self) -> &Game {
            &self.game
        }

        /// Ход человека. Точку приземления выбирает он, серию взятий
        /// достраивает движок: по правилам прерывать её нельзя.
        pub fn play_mine(&mut self, from: &str, to: &str) -> Result<(), String> {
            if self.game.outcome().is_some() {
                return Err("партия уже закончена".into());
            }
            if self.game.side() != self.my_side {
                return Err("сейчас не твой ход".into());
            }

            let from_square =
                Square::parse(from).ok_or_else(|| format!("не поняла поле «{from}»"))?;
            let to_square = Square::parse(to).ok_or_else(|| format!("не поняла поле «{to}»"))?;

            let found = self
                .game
                .legal_moves()
                .into_iter()
                .find(|mv| mv.from == from_square && mv.to == to_square)
                .ok_or_else(|| "так не сходить".to_string())?;

            self.game.apply(&found)?;
            self.history.push(checkers::move_text(&found));
            self.comment = self.react(&found);
            Ok(())
        }

        /// Ход бота, если сейчас его очередь.
        pub fn bot_reply(&mut self) -> Option<Result<String, String>> {
            if self.game.outcome().is_some() || self.game.side() == self.my_side {
                return None;
            }
            let mv = Bot::new(self.level).best_move(&self.game)?;
            let text = checkers::move_text(&mv);
            if let Err(why) = self.game.apply(&mv) {
                return Some(Err(why));
            }
            self.history.push(text.clone());
            self.comment = self.react(&mv);
            Some(Ok(text))
        }

        /// Новая партия с текущими настройками.
        pub fn restart(&mut self) {
            self.game = Game::new();
            self.history.clear();
            self.comment = self.opening_line();
            self.thinking = false;
            self.rewarded = false;
        }

        /// Что показать странице.
        pub fn state(&self, progress: &capri_core::save::SaveData) -> serde_json::Value {
            let board = self.game.state_json();
            serde_json::json!({
                "cells": board["cells"],
                "moves": board["moves"],
                "turn": board["side"],
                "outcome": board["outcome"],
                "winner": self.winner(),
                "ply": board["ply"],
                "white": board["white"],
                "black": board["black"],
                "my_side": self.my_side.as_str(),
                "level": self.level.as_str(),
                "thinking": self.thinking,
                "comment": self.comment,
                "history": self.history,
                "money": progress.money,
                "wins": progress.wins,
                "games": progress.games,
                "draws": progress.draws,
            })
        }

        /// Кто победил: `"white"`, `"black"` или `null` при ничьей.
        fn winner(&self) -> Option<&'static str> {
            match self.game.outcome() {
                Some(Outcome::WhiteWins) => Some("white"),
                Some(Outcome::BlackWins) => Some("black"),
                Some(Outcome::Draw) | None => None,
            }
        }

        fn opening_line(&self) -> String {
            match self.my_side {
                Side::White => "Твои шашки снизу. Бей первой — я отвечу.".to_string(),
                Side::Black => "Я начинаю. Смотри, куда я пойду.".to_string(),
            }
        }

        /// Реакция на ход — по факту, без ИИ: бой, дамка, конец партии.
        fn react(&self, mv: &Move) -> String {
            match self.game.outcome() {
                Some(Outcome::Draw) => return "Ничья. Ничья так ничья.".to_string(),
                Some(Outcome::WhiteWins) | Some(Outcome::BlackWins) => {
                    let i_won = match self.game.outcome() {
                        Some(Outcome::WhiteWins) => self.my_side == Side::White,
                        Some(Outcome::BlackWins) => self.my_side == Side::Black,
                        _ => false,
                    };
                    return if i_won {
                        "Ты выиграл. Красиво закрыл.".to_string()
                    } else {
                        "Партия моя. В следующий раз будь внимательнее.".to_string()
                    };
                }
                None => {}
            }

            if mv.captured.len() >= 2 {
                return format!("Серия из {} — вот это по-нашему.", mv.captured.len());
            }
            if !mv.captured.is_empty() {
                return "Забираю шашку. Спасибо.".to_string();
            }
            // Простая, дошедшая до последней горизонтали, стала дамкой.
            let promoted = self
                .game
                .piece_at(mv.to)
                .map(|piece| piece.king)
                .unwrap_or(false);
            if promoted {
                return "Дамка! Теперь я далеко бью.".to_string();
            }
            "Твой ход.".to_string()
        }
    }
}

/// Шахматные действия страницы.
fn chess_action(
    action: &str,
    data: &serde_json::Value,
    session: &std::sync::Mutex<chess_session::Session>,
    base_dir: &PathBuf,
    panels: &Sender<capri_panels::Command>,
) {
    use capri_core::chess::{Level, Outcome, Side};
    use capri_core::save::GameOutcome;

    let Ok(mut session) = session.lock() else {
        log::write_line("WARN", "Шахматы: сессия заблокирована");
        return;
    };

    match action {
        // Страница только открылась: отдаём текущую партию как есть.
        // Если человек играет чёрными, бот обязан сделать первый ход.
        "chess-ready" => {
            if session.game().ply() == 0 && session.my_side() == Side::Black {
                if let Some(Err(why)) = session.bot_reply() {
                    log::write_line("WARN", &format!("Шахматы: первый ход бота — {why}"));
                }
            }
        }
        "chess-new" => {
            if let Some(level) = data.get("level").and_then(|v| v.as_str()).and_then(Level::parse) {
                session.set_level(level);
            }
            if let Some(side) = data.get("side").and_then(|v| v.as_str()).and_then(Side::parse) {
                session.set_my_side(side);
            }
            session.restart();
            // Играя чёрными, человек ждёт первого хода бота — делаем его сразу.
            if session.my_side() == Side::Black {
                if let Some(Err(why)) = session.bot_reply() {
                    log::write_line("WARN", &format!("Шахматы: первый ход бота — {why}"));
                }
            }
            log::write_line(
                "INFO",
                &format!("Шахматы: новая партия, уровень «{}»", session.level().label()),
            );
        }
        "chess-move" => {
            let from = data.get("from").and_then(|v| v.as_str()).unwrap_or("");
            let to = data.get("to").and_then(|v| v.as_str()).unwrap_or("");
            match session.play_mine(from, to) {
                Ok(()) => {
                    // Сразу отвечаем ботом — партия не должна ждать клика.
                    if let Some(Err(why)) = session.bot_reply() {
                        log::write_line("WARN", &format!("Шахматы: бот не сходил — {why}"));
                    }
                }
                Err(why) => log::write_line("WARN", &format!("Шахматы: ход отклонён — {why}")),
            }
        }
        "chess-undo" => {
            if !session.undo() {
                log::write_line("INFO", "Шахматы: отменять нечего");
            }
        }
        "chess-level" => {
            if let Some(level) = data.get("level").and_then(|v| v.as_str()).and_then(Level::parse) {
                session.set_level(level);
            }
        }
        "chess-side" => {
            if let Some(side) = data.get("side").and_then(|v| v.as_str()).and_then(Side::parse) {
                session.set_my_side(side);
            }
        }
        _ => {}
    }

    // Партия кончилась — начисляем награду один раз.
    // `game_over_rewarded` не даёт выдать XP повторно, когда страница
    // перерисуется ещё раз (например, человек переключит уровень).
    if session.game().outcome().is_some() && !session.rewarded {
        session.rewarded = true;
        // Ничья — отдельный исход: раньше она попадала в «поражение».
        let outcome = match session.game().outcome() {
            Some(Outcome::Checkmate) => {
                if session.game().winner() == Some(session.my_side()) {
                    GameOutcome::Win
                } else {
                    GameOutcome::Loss
                }
            }
            Some(Outcome::Draw) => GameOutcome::Draw,
            None => GameOutcome::Loss,
        };
        let word = match outcome {
            GameOutcome::Win => "победа",
            GameOutcome::Draw => "ничья",
            GameOutcome::Loss => "поражение",
        };
        let path = capri_core::save::SaveManager::new(base_dir);
        let mut save = path.load();
        let level = save.on_game(outcome);
        match path.save(&save) {
            Ok(()) => log::write_line(
                "INFO",
                &format!(
                    "Шахматы: {word} — партия записана ({} побед, {} ничьих из {}), уровень {}",
                    save.wins, save.draws, save.games, level
                ),
            ),
            Err(why) => log::write_line("WARN", &format!("Шахматы: прогресс не сохранился — {why}")),
        }
    }
    // Новая партия снимает отметку о награде.
    if session.game().outcome().is_none() {
        session.rewarded = false;
    }

    // Баланс и счёт партий живут в сохранении, а не в сессии шахмат:
    // страница показывает их в шапке и под доской.
    let progress = capri_core::save::SaveManager::new(base_dir).load();
    let state = session.state(&progress);
    // Отдаём целый объект, а не `{ chess: ... }` с рукописным ключом: так
    // полезная нагрузка — честный JSON, её можно разобрать и проверить в тесте.
    let payload = serde_json::json!({ "chess": state });
    let js = format!("window.capri.setState({payload});");
    let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Chess, js));
}

/// Действия страницы по шашкам.
///
/// Устроено как у шахмат: партия живёт в ядре, страница только показывает и
/// присылает «откуда-куда». Бот отвечает сразу, в том же потоке.
fn checkers_action(
    action: &str,
    data: &serde_json::Value,
    session: &std::sync::Mutex<checkers_session::Session>,
    base_dir: &PathBuf,
    panels: &Sender<capri_panels::Command>,
) {
    use capri_core::checkers::{Level, Outcome, Side};
    use capri_core::save::GameOutcome;

    let Ok(mut session) = session.lock() else {
        log::write_line("WARN", "Шашки: сессия заблокирована");
        return;
    };

    match action {
        "checkers-ready" | "checkers-new" => {
            if action == "checkers-new" {
                if let Some(level) = data.get("level").and_then(|v| v.as_str()).and_then(Level::parse)
                {
                    session.set_level(level);
                }
                if let Some(side) = data.get("side").and_then(|v| v.as_str()).and_then(Side::parse) {
                    session.set_my_side(side);
                }
                session.restart();
                log::write_line(
                    "INFO",
                    &format!("Шашки: новая партия, уровень «{}»", session.level().as_str()),
                );
            }
            // Играя чёрными, человек ждёт первого хода бота.
            if session.game().ply() == 0 && session.my_side() == Side::Black {
                if let Some(Err(why)) = session.bot_reply() {
                    log::write_line("WARN", &format!("Шашки: первый ход бота — {why}"));
                }
            }
        }
        "checkers-move" => {
            let from = data.get("from").and_then(|v| v.as_str()).unwrap_or("");
            let to = data.get("to").and_then(|v| v.as_str()).unwrap_or("");
            match session.play_mine(from, to) {
                Ok(()) => {
                    if let Some(Err(why)) = session.bot_reply() {
                        log::write_line("WARN", &format!("Шашки: бот не сходил — {why}"));
                    }
                }
                Err(why) => log::write_line("WARN", &format!("Шашки: ход отклонён — {why}")),
            }
        }
        "checkers-level" => {
            if let Some(level) = data.get("level").and_then(|v| v.as_str()).and_then(Level::parse) {
                session.set_level(level);
            }
        }
        "checkers-side" => {
            if let Some(side) = data.get("side").and_then(|v| v.as_str()).and_then(Side::parse) {
                session.set_my_side(side);
            }
        }
        _ => {}
    }

    // Партия кончилась — награда один раз, как в шахматах.
    if session.game().outcome().is_some() && !session.rewarded {
        session.rewarded = true;
        let outcome = match session.game().outcome() {
            Some(Outcome::WhiteWins) | Some(Outcome::BlackWins) => {
                let winner_is_white = session.game().outcome() == Some(Outcome::WhiteWins);
                let i_won = (session.my_side() == Side::White) == winner_is_white;
                if i_won {
                    GameOutcome::Win
                } else {
                    GameOutcome::Loss
                }
            }
            Some(Outcome::Draw) => GameOutcome::Draw,
            None => GameOutcome::Loss,
        };
        let word = match outcome {
            GameOutcome::Win => "победа",
            GameOutcome::Draw => "ничья",
            GameOutcome::Loss => "поражение",
        };
        let manager = capri_core::save::SaveManager::new(base_dir);
        let mut save = manager.load();
        let level = save.on_game(outcome);
        match manager.save(&save) {
            Ok(()) => log::write_line(
                "INFO",
                &format!(
                    "Шашки: {word} — партия записана ({} побед, {} ничьих из {}), уровень {}",
                    save.wins, save.draws, save.games, level
                ),
            ),
            Err(why) => log::write_line("WARN", &format!("Шашки: прогресс не сохранился — {why}")),
        }
    }
    if session.game().outcome().is_none() {
        session.rewarded = false;
    }

    let progress = capri_core::save::SaveManager::new(base_dir).load();
    let state = session.state(&progress);
    let payload = serde_json::json!({ "checkers": state });
    let js = format!("window.capri.setState({payload});");
    let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Chess, js));
}

/// Чат с Капри: реплики, ответы модели и история на диске.
///
/// Страница присылает голый текст; здесь он попадает в историю, уходит
/// модели и ответ возвращается тем же `setState`. Блокировать поток панелей
/// на время запроса нельзя — поэтому ответ ждём в отдельном потоке, а
/// страница тем временем показывает «печатает…».
fn chat_action(
    action: &str,
    data: &serde_json::Value,
    cfg: &Arc<RwLock<Config>>,
    base_dir: &PathBuf,
    panels: &Sender<capri_panels::Command>,
) {
    use capri_core::chat::{self, Message, Store};

    let store = Store::new(base_dir);

    // Общая отправка состояния странице.
    let push = |thinking: bool, history: &chat::History, model: &str, provider: &str| {
        let payload = serde_json::json!({
            "chat": {
                "messages": history.to_json(),
                "model": model,
                "provider": provider,
                "available": !model.is_empty(),
                "thinking": thinking,
            }
        });
        let js = format!("window.capri.setState({payload});");
        let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Chat, js));
    };

    match action {
        "chat-ready" => {
            let history = store.load();
            let snapshot = cfg.read().clone();
            let (model, provider) = chat_model_label(&snapshot);
            push(false, &history, &model, &provider);
        }
        "chat-clear" => {
            let history = chat::History::new();
            match store.save(&history) {
                Ok(()) => log::write_line("INFO", "Чат: история очищена"),
                Err(why) => log::write_line("WARN", &format!("Чат: {why}")),
            }
            let snapshot = cfg.read().clone();
            let (model, provider) = chat_model_label(&snapshot);
            push(false, &history, &model, &provider);
        }
        "chat-send" => {
            let text = data.get("text").and_then(|v| v.as_str()).unwrap_or_default().trim().to_string();
            if text.is_empty() {
                return;
            }

            let mut history = store.load();
            history.push(Message::user(&text, chat::now_hhmm()));
            // Свою реплику показываем сразу, не ожидая ответа.
            let snapshot = cfg.read().clone();
            let (model, provider) = chat_model_label(&snapshot);
            let _ = store.save(&history);
            push(true, &history, &model, &provider);

            // Ответ модели — в фоне: запрос к Ollama может идти секундами,
            // а поток панелей обязан оставаться отзывчивым.
            let panels_bg = panels.clone();
            let history_for_ai = history.tail(20).to_vec();
            let base = base_dir.clone();
            std::thread::spawn(move || {
                let answer = ask_chat_model(&snapshot, &history_for_ai, &text);
                let store = Store::new(&base);
                let mut history = store.load();
                let reply = match answer {
                    Some(text) => text,
                    None => {
                        // Модель молчит — честно говорим об этом, а не
                        // притворяемся, что ответ есть.
                        "Не могу ответить: мозги не настроены или недоступны. \
                         Загляни в настройки — там видно, какая модель включена."
                            .to_string()
                    }
                };
                history.push(Message::capri(&reply, chat::now_hhmm()));
                if let Err(why) = store.save(&history) {
                    log::write_line("WARN", &format!("Чат: {why}"));
                }
                let payload = serde_json::json!({
                    "chat": {
                        "messages": history.to_json(),
                        "model": model,
                        "provider": provider,
                        "available": !model.is_empty(),
                        "thinking": false,
                    }
                });
                let js = format!("window.capri.setState({payload});");
                let _ = panels_bg.send(capri_panels::Command::Eval(capri_panels::PanelId::Chat, js));
            });
        }
        _ => {}
    }
}

/// Сколько ждём команду, прежде чем остановить её сами.
const TERMINAL_TIMEOUT_SECS: u64 = 120;

/// Флаг «останови текущую команду».
///
/// Одновременно идёт не больше одной команды — страница блокирует ввод на
/// время работы, — поэтому хватает одного флага: перед запуском сбрасываем.
fn terminal_cancel_flag() -> Arc<std::sync::atomic::AtomicBool> {
    static CANCEL: std::sync::OnceLock<Arc<std::sync::atomic::AtomicBool>> = std::sync::OnceLock::new();
    Arc::clone(CANCEL.get_or_init(|| Arc::new(std::sync::atomic::AtomicBool::new(false))))
}

/// Терминал: запуск команды и построчная отдача вывода.
///
/// Сама команда живёт в отдельной нити, а вывод уходит странице по мере
/// появления через `window.capri.line(...)`. Блокировать поток панелей
/// нельзя: пока идёт `npm install`, окно должно оставаться живым.
fn terminal_action(
    action: &str,
    data: &serde_json::Value,
    panels: &Sender<capri_panels::Command>,
) {
    use std::sync::atomic::Ordering;

    match action {
        "terminal-cancel" => {
            terminal_cancel_flag().store(true, Ordering::Relaxed);
        }
        "terminal-run" => {
            let command = data
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            if command.is_empty() {
                return;
            }

            let cancel = terminal_cancel_flag();
            cancel.store(false, Ordering::Relaxed);
            log::write_line("INFO", &format!("Терминал: {}", capri_core::exec::describe(&command)));

            let panels_bg = panels.clone();
            std::thread::spawn(move || {
                // Принимаем события процесса и сразу переправляем их странице.
                let (tx, rx) = channel();
                let forward = {
                    let panels = panels_bg.clone();
                    std::thread::spawn(move || {
                        // Строки копим, чтобы в конце посчитать «6 passing».
                        let mut seen: Vec<String> = Vec::new();
                        for event in rx {
                            match event {
                                capri_win::exec::Event::Line { text, kind, err } => {
                                    seen.push(text.clone());
                                    let payload = serde_json::json!({
                                        "text": text, "kind": kind, "err": err,
                                    });
                                    let js = format!("window.capri.line({payload});");
                                    let _ = panels.send(capri_panels::Command::Eval(
                                        capri_panels::PanelId::Terminal,
                                        js,
                                    ));
                                }
                                capri_win::exec::Event::Done { code, killed } => {
                                    let summary = capri_core::exec::summarize(&seen);
                                    let payload = serde_json::json!({
                                        "done": true,
                                        "code": code,
                                        "killed": killed,
                                        "summary": if summary.is_empty() {
                                            serde_json::Value::Null
                                        } else {
                                            summary.to_json()
                                        },
                                    });
                                    let js = format!("window.capri.line({payload});");
                                    let _ = panels.send(capri_panels::Command::Eval(
                                        capri_panels::PanelId::Terminal,
                                        js,
                                    ));
                                }
                            }
                        }
                        let _ = panels;
                    })
                };

                let code =
                    capri_win::exec::run_stream(&command, tx, cancel, TERMINAL_TIMEOUT_SECS);
                // `tx` уехал в `run_stream` и умер там — очередь закроется,
                // и нить переправки закончится сама.
                let _ = forward.join();
                log::write_line(
                    "INFO",
                    &format!("Терминал: команда завершена, код {code:?}"),
                );
            });
        }
        _ => {}
    }
}

/// Кнопки панели перевода.
///
/// Выделение области блокирует поток на всё время, пока человек тянет мышь,
/// поэтому работает в отдельной нити: иначе панели (в этом же потоке)
/// замерли бы вместе со шторкой, и результат было бы некому показать.
///
/// Порядок: шторка → снимок области → OCR → перевод → страница.
fn translate_action(
    action: &str,
    data: &serde_json::Value,
    cfg: &Arc<RwLock<Config>>,
    base_dir: &PathBuf,
    panels: &Sender<capri_panels::Command>,
) {
    match action {
        // Выделение области: работаем в своей нити.
        "translate-pick" => {
            let panels = panels.clone();
            let cfg = Arc::clone(cfg);
            let base_dir = base_dir.clone();
            std::thread::spawn(move || {
                translate_pick(&cfg, &base_dir, &panels);
            });
        }
        // «Ещё раз»: текст уже есть, нужен только новый перевод (мимо кэша).
        "translate-again" => {
            let text = data
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .trim()
                .to_string();
            if text.is_empty() {
                return;
            }
            let panels = panels.clone();
            let cfg = Arc::clone(cfg);
            let base_dir = base_dir.clone();
            std::thread::spawn(move || {
                let translator = make_translator(&cfg, &base_dir);
                let target = cfg.read().translate_to.clone();
                translate_text(Some("en"), &text, &target, &translator, &panels);
            });
        }
        // Копирование перевода в буфер обмена.
        "translate-copy" => {
            let text = data
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            if text.is_empty() {
                return;
            }
            // Буфер живёт в ядре (`Platform`), поэтому копируем его запросом.
            match capri_win::system::clipboard_set(&text) {
                Ok(()) => log::write_line("INFO", "Перевод: скопировано в буфер обмена"),
                Err(why) => log::write_line("WARN", &format!("Перевод: буфер обмена — {why}")),
            }
        }
        _ => {}
    }
}

/// Собрать переводчик по текущим настройкам.
fn make_translator(cfg: &Arc<RwLock<Config>>, base_dir: &PathBuf) -> capri_core::translate::Translator {
    let cfg = cfg.read();
    // Кэш лежит рядом с программой: там же, где config.json и логи.
    let dir = capri_core::translate::cache_dir(base_dir);
    capri_core::translate::Translator::new(&cfg, Some(dir))
}

/// Подпись провайдера для шапки панели перевода.
///
/// Отдельная функция, потому что `panel_state` держит `cfg.read()` и не может
/// создавать переводчика: тот тоже читает конфиг и упёрся бы в блокировку
/// (`parking_lot::RwLock` не рекурсивный).
fn translate_provider_label(config: &Config, base_dir: &PathBuf) -> String {
    let dir = capri_core::translate::cache_dir(base_dir);
    let translator = capri_core::translate::Translator::new(config, Some(dir));
    translator.backend_label()
}

/// Показать странице этап работы — чтобы «долго» не выглядело как «сломалось».
fn translate_stage(panels: &Sender<capri_panels::Command>, text: &str) {
    let payload = serde_json::json!({ "stage": text });
    let js = format!("window.capri.translate({payload});");
    let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Translate, js));
}

/// Показать странице ошибку.
fn translate_fail(panels: &Sender<capri_panels::Command>, why: &str) {
    let payload = serde_json::json!({ "error": why });
    let js = format!("window.capri.translate({payload});");
    let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Translate, js));
}

/// Выделить область, распознать и перевести.
fn translate_pick(
    cfg: &Arc<RwLock<Config>>,
    base_dir: &PathBuf,
    panels: &Sender<capri_panels::Command>,
) {
    translate_stage(panels, "выдели область на экране");

    // Отладка: `CAPRI_TRANSLATE_RECT=100,200,400,80` задаёт область без
    // перетаскивания мыши. Нужно для автоматической проверки: шторку не
    // провести синтетическими кликами, а весь остальной путь (снимок → OCR →
    // перевод → страница) проверить надо.
    if let Some(rect) = debug_rect() {
        log::write_line(
            "INFO",
            &format!(
                "Перевод: область из CAPRI_TRANSLATE_RECT = {}, {}, {}x{}",
                rect.x, rect.y, rect.width, rect.height
            ),
        );
        translate_with_region(cfg, base_dir, panels, rect);
        return;
    }

    let pick = match capri_ui::select::select_region() {
        Some(pick) => pick,
        None => {
            translate_fail(panels, "не смогла показать окно выделения");
            return;
        }
    };

    let (x, y, width, height) = match pick {
        capri_ui::select::Pick::Region { x, y, width, height } => (x, y, width, height),
        capri_ui::select::Pick::Cancelled => {
            // Отмена — не ошибка: человек передумал. Возвращаем экран в покой,
            // но текст перевода не стираем: он мог быть с прошлого раза.
            translate_stage(panels, "отменено");
            log::write_line("DEBUG", "Перевод: выделение отменено");
            let payload = serde_json::json!({ "cancelled": true });
            let js = format!("window.capri.translate({payload});");
            let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Translate, js));
            return;
        }
    };

    translate_stage(panels, "снимаю область…");
    translate_with_region(
        cfg,
        base_dir,
        panels,
        capri_win::screenshot::Region { x, y, width, height },
    );
}

/// Область из `CAPRI_TRANSLATE_RECT` для отладочного прогона.
fn debug_rect() -> Option<capri_win::screenshot::Region> {
    let raw = std::env::var("CAPRI_TRANSLATE_RECT").ok()?;
    let parts: Vec<i32> = raw
        .split(',')
        .filter_map(|part| part.trim().parse::<i32>().ok())
        .collect();
    if parts.len() != 4 {
        return None;
    }
    Some(capri_win::screenshot::Region {
        x: parts[0],
        y: parts[1],
        width: parts[2],
        height: parts[3],
    })
}

/// Снять область, распознать и перевести — без участия мыши.
fn translate_with_region(
    cfg: &Arc<RwLock<Config>>,
    base_dir: &PathBuf,
    panels: &Sender<capri_panels::Command>,
    region: capri_win::screenshot::Region,
) {
    let image = match capri_win::screenshot::capture_region(region) {
        Ok(image) => image,
        Err(why) => {
            log::write_line("WARN", &format!("Перевод: снимок области не удался — {why}"));
            translate_fail(panels, &format!("не смогла снять область: {why}"));
            return;
        }
    };

    translate_stage(panels, "распознаю текст…");
    let recognized = match capri_win::ocr::recognize(&image, None) {
        Ok(recognized) => recognized,
        Err(why) => {
            log::write_line("WARN", &format!("Перевод: OCR не сработал — {why}"));
            translate_fail(panels, &why);
            return;
        }
    };

    if recognized.is_empty() {
        log::write_line("INFO", "Перевод: на области не нашлось текста");
        translate_fail(
            panels,
            "на выделенной области не нашлось текста. Попробуй обвести текст поплотнее.",
        );
        return;
    }

    let text = recognized.text();
    log::write_line(
        "INFO",
        &format!(
            "Перевод: распознано {} символов ({}x{}), язык {}",
            recognized.char_count(),
            region.width,
            region.height,
            recognized.language
        ),
    );

    let translator = make_translator(cfg, base_dir);
    let target = cfg.read().translate_to.clone();
    let from = capri_core::translate::from_tag(&recognized.language);
    translate_text(Some(from), &text, &target, &translator, panels);
}

/// Перевести текст и отдать результат странице.
fn translate_text(
    from: Option<&str>,
    text: &str,
    target: &str,
    translator: &capri_core::translate::Translator,
    panels: &Sender<capri_panels::Command>,
) {
    translate_stage(panels, "перевожу…");
    match translator.translate(text, from, target) {
        Ok(result) => {
            log::write_line(
                "INFO",
                &format!(
                    "Перевод: {} → {} через {} ({} символов{})",
                    result.from,
                    result.to,
                    result.provider,
                    result.text.chars().count(),
                    if result.cached { ", из кэша" } else { "" }
                ),
            );
            let payload = serde_json::json!({
                "source": text,
                "text": result.text,
                "from_name": capri_core::translate::language(result.from).name,
                "to_name": capri_core::translate::language(result.to).name,
                "provider": result.provider,
                "cached": result.cached,
            });
            let js = format!("window.capri.translate({payload});");
            let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Translate, js));
        }
        Err(why) => {
            log::write_line("WARN", &format!("Перевод: не получилось — {why}"));
            translate_fail(panels, &why);
        }
    }
}

/// Откуда берётся ответ: подпись для шапки чата.
////// Возвращает (модель, провайдер). Пустая модель означает «мозгов нет» —
/// страница покажет это честно.
fn chat_model_label(cfg: &Config) -> (String, String) {
    let local = capri_core::ai::LocalAi::new(cfg);
    if local.available() {
        let model = local.effective_model().unwrap_or_else(|| cfg.local_model.clone());
        // Пишем не просто «локальная», а кто именно ответил: LM Studio или
        // Ollama. Без этого человек не понимает, к какому серверу мы вообще
        // подключились, и не может проверить, «та ли» модель.
        let provider = local.describe().unwrap_or_else(|| "локальная".to_string());
        return (model, provider);
    }
    let chat = capri_core::ai::AiChat::new(cfg);
    if chat.available() {
        return (cfg.model.clone(), "Claude".to_string());
    }
    (String::new(), "нет".to_string())
}

/// Спросить модель, передав хвост разговора как контекст.
fn ask_chat_model(
    cfg: &Config,
    tail: &[capri_core::chat::Message],
    text: &str,
) -> Option<String> {
    // Контекст собираем в один промпт: и локальная модель, и Claude
    // принимают простой текст, а отдельных «ролей» нам и не нужно.
    let mut prompt = String::new();
    for message in tail {
        let who = match message.who {
            capri_core::chat::Who::User => "Пользователь",
            capri_core::chat::Who::Capri => "Капри",
        };
        prompt.push_str(who);
        prompt.push_str(": ");
        prompt.push_str(&message.text);
        prompt.push('\n');
    }
    prompt.push_str("Пользователь: ");
    prompt.push_str(text);
    prompt.push_str("\nКапри:");

    let local = capri_core::ai::LocalAi::new(cfg);
    if local.available() {
        return local.ask(&prompt);
    }
    let chat = capri_core::ai::AiChat::new(cfg);
    if chat.available() {
        return chat.ask(&prompt);
    }
    None
}

/// Разложить действия панелей: что отдать странице, что передать ядру.
fn panel_dispatch(
    events: Receiver<capri_panels::PanelEvent>,
    cfg: &Arc<RwLock<Config>>,
    base_dir: &PathBuf,
    monitors: &[MonitorInfo],
    panels: &Sender<capri_panels::Command>,
    requests: &Sender<UiRequest>,
    ui: &Sender<UiEvent>,
) {
    // Партия живёт, пока открыто окно: ходы приходят по одному, а бот
    // отвечает в том же потоке — гонок за позицию нет.
    let chess = std::sync::Mutex::new(chess_session::Session::default());
    // Шашки живут рядом: у каждой игры своя партия, поэтому переключение
    // вкладок ничего не сбрасывает.
    let checkers = std::sync::Mutex::new(checkers_session::Session::default());

    for event in events {
        match event.action.as_str() {
            // Страница загрузилась — отдаём ей состояние.
            "ready" => {
                // Медиа-поток опрашивает плеер только пока окно открыто.
                if event.panel == capri_panels::PanelId::Media {
                    panels_snapshot::set_media_open(true);
                }
                // Шахматам и шашкам нужен не общий снимок ядра, а свои партии.
                if event.panel == capri_panels::PanelId::Chess {
                    chess_action("chess-ready", &serde_json::Value::Null, &chess, base_dir, panels);
                    checkers_action(
                        "checkers-ready",
                        &serde_json::Value::Null,
                        &checkers,
                        base_dir,
                        panels,
                    );
                    continue;
                }
                // Чату — своя история, а не общий снимок настроек.
                if event.panel == capri_panels::PanelId::Chat {
                    chat_action("chat-ready", &serde_json::Value::Null, cfg, base_dir, panels);
                    continue;
                }
                // Отладка перевода: `CAPRI_TRANSLATE_RECT=...` заодно просит
                // сразу перевести эту область, без клика по кнопке. Иначе
                // цепочку снимок → OCR → перевод не проверить автоматически:
                // шторку выделения не провести синтетическими кликами.
                let state = panel_state(cfg, base_dir, monitors);
                let js = format!("window.capri.setState({state});");
                let _ = panels.send(capri_panels::Command::Eval(event.panel, js));

                if event.panel == capri_panels::PanelId::Translate && debug_rect().is_some() {
                    // Только один раз за запуск: страница может прислать
                    // `ready` несколько раз (перезагрузка WebView2), а прогон
                    // нужен ровно один — иначе лог забивается одинаковыми
                    // строками и непонятно, что вообще произошло.
                    static RAN: std::sync::atomic::AtomicBool =
                        std::sync::atomic::AtomicBool::new(false);
                    if !RAN.swap(true, std::sync::atomic::Ordering::SeqCst) {
                        log::write_line(
                            "INFO",
                            "Перевод: страница готова, запускаю отладочный прогон",
                        );
                        let cfg_bg = Arc::clone(cfg);
                        let base_bg = base_dir.clone();
                        let panels_bg = panels.clone();
                        std::thread::spawn(move || {
                            translate_pick(&cfg_bg, &base_bg, &panels_bg);
                        });
                    }
                }
            }
            "close" | "hidden" => {
                if event.panel == capri_panels::PanelId::Media {
                    panels_snapshot::set_media_open(false);
                }
                let _ = panels.send(capri_panels::Command::Close(event.panel));
            }
            // Кнопка «Выйти» на макете закрывает всё приложение целиком,
            // а не только окно настроек. `UiRequest::Quit` гасит аватар и
            // вместе с ним весь цикл; `Command::Quit` закрыл бы лишь панели.
            "quit" => {
                log::write_line("INFO", "Панель настроек: выход по кнопке");
                let _ = requests.send(UiRequest::Quit);
            }
            // Окно показали повторно: страница уже загружена, `ready` не будет,
            // а медиа-потоку нужно снова начать опрос плеера.
            "shown" => {
                if event.panel == capri_panels::PanelId::Media {
                    panels_snapshot::set_media_open(true);
                }
                log::write_line(
                    "DEBUG",
                    &format!("Панель «{}» снова на экране", event.panel.as_str()),
                );
            }
            "open-config" => {
                let _ = requests.send(UiRequest::OpenConfig);
            }
            // «Сохранить» на макете. Размер, живость и монитор пишутся в
            // config.json сразу по клику, а сохранение прогресса — в момент
            // события; поэтому кнопка лишь подтверждает, что всё уже на диске.
            "save" => {
                let save = capri_core::save::SaveManager::new(base_dir);
                match save.save(&save.load()) {
                    Ok(()) => {
                        log::write_line("INFO", "Настройки: сохранено по кнопке «Сохранить»");
                        let js = "window.capri.status('сохранено');".to_string();
                        let _ = panels
                            .send(capri_panels::Command::Eval(capri_panels::PanelId::Settings, js));
                    }
                    Err(why) => {
                        log::write_line("WARN", &format!("Настройки: не сохранилось — {why}"))
                    }
                }
            }
            "reload-config" => {
                let _ = requests.send(UiRequest::Reload);
            }
            // Ползунок «Размер»: значение приходит числом.
            "set-scale" => {
                if let Some(percent) = number_of(&event.data) {
                    let percent = capri_core::config::clamp_scale(percent.min(255) as u8);
                    let _ = requests.send(UiRequest::Character(CharacterSetting::Scale(percent)));
                }
            }
            // «Живость»: строка `calm` / `normal` / `lively`.
            "set-liveliness" => {
                if let Some(value) = text_of(&event.data) {
                    let value = capri_core::config::normalize_liveliness(&value).to_string();
                    let _ = requests.send(UiRequest::Character(CharacterSetting::Liveliness(value)));
                }
            }
            // «Экран»: номер монитора.
            "set-monitor" => {
                if let Some(index) = number_of(&event.data) {
                    let _ = requests
                        .send(UiRequest::Character(CharacterSetting::Monitor(index.max(0) as u32)));
                }
            }
            // «Поведение»: `{"key": "chatter", "value": true}` — или
            // `{"key": "chatter-minutes", "value": 15, "second": 40}`.
            "set-behavior" => {
                let key = event
                    .data
                    .get("key")
                    .and_then(|k| k.as_str())
                    .unwrap_or_default()
                    .to_string();
                let setting = behavior_setting(&key, &event.data);
                match setting {
                    Some(setting) => {
                        let _ = requests.send(UiRequest::Behavior(setting));
                    }
                    None => log::write_line(
                        "WARN",
                        &format!("Панель «настройки»: поведение «{key}» не знаю"),
                    ),
                }
            }
            // Магазин: `{"skin": "lada"}` — купить и сразу надеть.
            "shop-buy" | "shop-equip" => {
                let buying = event.action == "shop-buy";
                let id = event
                    .data
                    .get("skin")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if id.is_empty() {
                    log::write_line("WARN", "Магазин: не понял, о каком скине речь");
                } else {
                    let manager = capri_core::save::SaveManager::new(base_dir);
                    let mut data = manager.load();
                    let outcome = if buying {
                        data.buy_and_equip(&id).map(|skin| skin.name)
                    } else {
                        data.equip(&id).map(|skin| skin.name)
                    };
                    match outcome {
                        Ok(name) => {
                            if let Err(e) = manager.save(&data) {
                                log::write_line("ERROR", &format!("Скин не сохранился: {e}"));
                            }
                            log::write_line(
                                "INFO",
                                &format!(
                                    "Магазин: «{name}» {}, осталось {} ECS",
                                    if buying { "куплен" } else { "надет" },
                                    data.money
                                ),
                            );
                            // Перекраской фигуры занимается ядро; прогресс уже
                            // записан здесь, поэтому в файл ядро не лезет.
                            let color = capri_core::save::skin_color(&id);
                            let _ = requests.send(UiRequest::SkinColor(color));
                        }
                        Err(why) => {
                            log::write_line("WARN", &format!("Магазин: {why}"));
                            let _ = ui.send(UiEvent::Subtitle(why));
                        }
                    }
                }
                refresh_panel(&event.panel, panels, cfg, base_dir, monitors);
            }
            // Уровни: съесть «вкусняшку» из инвентаря.
            "levels-eat" => {
                let manager = capri_core::save::SaveManager::new(base_dir);
                let mut data = manager.load();
                match data.eat_snack() {
                    Ok(()) => {
                        if let Err(e) = manager.save(&data) {
                            log::write_line("ERROR", &format!("Прогресс не сохранился: {e}"));
                        }
                        log::write_line(
                            "INFO",
                            &format!(
                                "Съела вкусняшку — радость {}, усталость {}",
                                data.mood_joy, data.mood_tired
                            ),
                        );
                    }
                    Err(why) => log::write_line("WARN", &format!("Уровни: {why}")),
                }
                refresh_panel(&event.panel, panels, cfg, base_dir, monitors);
            }
            // Чат: своя история и свой ответ — общего снимка ему мало.
            "chat-send" | "chat-ready" | "chat-clear" => {
                chat_action(&event.action, &event.data, cfg, base_dir, panels);
            }
            // Напоминания: книга живёт в ядре, поэтому панель просит ядро,
            // а снимок для показа читается из файла. Страница после нажатия
            // сама переспросит состояние.
            "reminder-add" => {
                let text = event
                    .data
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !text.is_empty() {
                    let _ = requests.send(UiRequest::ReminderAdd(text));
                }
            }
            "reminder-cancel" => {
                if let Some(id) = event.data.get("id").and_then(|v| v.as_u64()) {
                    let _ = requests.send(UiRequest::ReminderCancel(id));
                }
            }
            "reminder-clear" => {
                let _ = requests.send(UiRequest::ReminderClear);
            }
            // Терминал: вывод идёт построчно, поэтому тоже мимо общего снимка.
            "terminal-run" | "terminal-cancel" => {
                terminal_action(&event.action, &event.data, panels);
            }
            // Перевод: выделение области и распознавание занимают секунды,
            // поэтому тоже мимо общего снимка.
            "translate-pick" | "translate-again" | "translate-copy" => {
                translate_action(&event.action, &event.data, cfg, base_dir, panels);
            }
            // «Отправить в main»: копируем команду в буфер обмена. Буфер
            // живёт за трейтом `Platform`, поэтому идём через запрос к ядру.
            "terminal-copy" => {
                let command = event
                    .data
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if command.is_empty() {
                    log::write_line("WARN", "Терминал: копировать нечего — пустая команда");
                } else if requests.send(UiRequest::CopyToClipboard(command)).is_err() {
                    log::write_line("WARN", "Терминал: ядро недоступно, команда не скопирована");
                }
            }
            // Страница подтверждает, что отрисовалась: по этой строке в
            // capri.log видно, что панель живая, а не белое окно.
            "painted" => {
                let count = event
                    .data
                    .get("skins")
                    .or_else(|| event.data.get("items"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                log::write_line(
                    "INFO",
                    &format!(
                        "Панель «{}» отрисована, элементов на экране: {count}",
                        event.panel.as_str()
                    ),
                );
            }
            // Медиа-виджет: кнопки плеера. Работают и через SMTC, и через
            // медиа-клавиши — как это уже сделано для голосовых команд.
            "media-playpause" | "media-play" | "media-pause" | "media-next" | "media-prev"
            | "media-seek" => {
                let command = match event.action.as_str() {
                    "media-next" => Some(capri_core::platform::MediaCommand::Next),
                    "media-prev" => Some(capri_core::platform::MediaCommand::Prev),
                    "media-play" => Some(capri_core::platform::MediaCommand::Play),
                    "media-pause" => Some(capri_core::platform::MediaCommand::Pause),
                    "media-playpause" => Some(capri_core::platform::MediaCommand::PlayPause),
                    // Перемотка — не команда, а отдельный вызов с секундами.
                    _ => None,
                };

                let result = match (command, number_of(&event.data.get("seconds").cloned()
                    .unwrap_or(serde_json::Value::Null)))
                {
                    (Some(cmd), _) => capri_win::media::media_command(cmd),
                    (None, Some(seconds)) => capri_win::media::seek(seconds as f64),
                    (None, None) => Err("не поняла, куда перематывать".to_string()),
                };

                match result {
                    Ok(()) => log::write_line(
                        "INFO",
                        &format!("Медиа: «{}» выполнено", event.action),
                    ),
                    Err(why) => log::write_line(
                        "WARN",
                        &format!("Медиа: «{}» не вышло — {why}", event.action),
                    ),
                }
            }
            // Страница просит свежие полосы эквалайзера (свой таймер в JS).
            // Поток медиа шлёт их и сам, но ручной запрос делает виджет
            // отзывчивым сразу после переоткрытия окна.
            "media-levels" => {
                let bands = capri_win::audio_loop::bands();
                let js = format!(
                    "window.capri.setState({{ levels: {} }});",
                    serde_json::to_string(&bands).unwrap_or_else(|_| "[]".into())
                );
                let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Media, js));
            }
            // Шахматы: ходы и настройки партии.
            "chess-new" | "chess-move" | "chess-undo" | "chess-level" | "chess-side" => {
                chess_action(event.action.as_str(), &event.data, &chess, base_dir, panels);            }
            "open-failed" => {
                let why = event
                    .data
                    .get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("причина неизвестна");
                log::write_line(
                    "ERROR",
                    &format!("Панель «{}» не открылась: {why}", event.panel.as_str()),
                );
                let _ = ui.send(UiEvent::Subtitle(format!("панель не открылась: {why}")));
            }
            other => {
                log::write_line(
                    "WARN",
                    &format!(
                        "Панель «{}»: действие «{other}» пока не подключено",
                        event.panel.as_str()
                    ),
                );
            }
        }
    }
}

/// Кормить медиа-виджет: трек, обложка и полосы эквалайзера.
///
/// Опрос плеера — блокирующий вызов SMTC, поэтому живёт в своём потоке:
/// панель не должна ждать, пока система ответит про трек. Наружу уходят
/// только готовые строки JSON — страница ничего не считает.
fn media_feeder(
    platform: Arc<capri_win::WindowsPlatform>,
    panels: Sender<capri_panels::Command>,
    cfg: Arc<RwLock<Config>>,
) {
    use std::time::{Duration, Instant};

    /// Опрос плеера: позиция трека меняется нечасто, чаще незачем.
    const TRACK_EVERY: Duration = Duration::from_millis(500);
    /// Эквалайзер: успеть за глазом, но не заваливать веб-вью скриптами.
    const LEVELS_EVERY: Duration = Duration::from_millis(80);

    let mut last_track = Instant::now() - TRACK_EVERY;
    let mut last_levels = Instant::now() - LEVELS_EVERY;

    // Кэш обложки: перекодируем только когда сменился трек.
    let mut cover_title = String::new();
    let mut cover_url = String::new();

    loop {
        std::thread::sleep(Duration::from_millis(40));

        // Виджет не открыт — не тратим ничего: ни SMTC, ни кадры.
        if !panels_snapshot::media_open(&panels, &cfg) {
            // Сбрасываем кэш обложки, чтобы после переоткрытия она обновилась.
            cover_title.clear();
            continue;
        }

        if last_levels.elapsed() >= LEVELS_EVERY {
            last_levels = Instant::now();
            let bands = capri_win::audio_loop::bands();
            // Пустой вектор — визуализатор недоступен; странице это тоже
            // полезно знать, она покажет ровный ряд нулей.
            let js = format!(
                "window.capri.setState({{ levels: {} }});",
                serde_json::to_string(&bands).unwrap_or_else(|_| "[]".into())
            );
            let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Media, js));
        }

        if last_track.elapsed() < TRACK_EVERY {
            continue;
        }
        last_track = Instant::now();

        let media = match platform.now_playing() {
            Some(track) => {
                // Новая обложка — только при смене трека.
                let key = format!("{}\u{1}{}", track.title, track.artist);
                if key != cover_title {
                    cover_title = key;
                    cover_url = capri_win::media::cover_data_url().unwrap_or_default();
                }
                serde_json::json!({
                    "title": track.title,
                    "artist": track.artist,
                    "app": track.app,
                    "playing": track.playing,
                    "position": track.position,
                    "duration": track.duration,
                    "cover": cover_url,
                })
            }
            // Плеер молчит: страница покажет «Ничего не играет».
            None => {
                serde_json::json!({
                    "title": "",
                    "artist": "",
                    "app": "",
                    "playing": false,
                    "position": 0.0,
                    "duration": 0.0,
                    "cover": "",
                })
            }
        };

        let js = format!(
            "window.capri.setState({{ media: {} }});",
            serde_json::to_string(&media).unwrap_or_else(|_| "{}".into())
        );
        let _ = panels.send(capri_panels::Command::Eval(capri_panels::PanelId::Media, js));
    }
}

/// Крошечное состояние «какие панели открыты» для фоновых потоков.
///
/// Панели живут в своём потоке и не делятся списком окон. Вместо общей
/// таблицы с блокировкой держим один атомарный флаг: медиа-виджет либо
/// открыт, либо нет. Этого хватает — опрашивать плеер ради закрытого окна
/// бессмысленно.
mod panels_snapshot {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    static MEDIA_OPEN: AtomicBool = AtomicBool::new(false);

    pub fn set_media_open(open: bool) {
        MEDIA_OPEN.store(open, Ordering::Relaxed);
    }

    pub fn media_open(_panels: &Sender<capri_panels::Command>, _cfg: &Arc<RwLock<Config>>) -> bool {
        MEDIA_OPEN.load(Ordering::Relaxed)
    }
}

/// Состояние для панелей — JSON-литерал для `window.capri.setState(...)`.
///
/// Собираем вручную, а не отдаём весь `Config`: в настройках лежит
/// `anthropic_api_key`, а странице он ни к чему.
fn panel_state(cfg: &Arc<RwLock<Config>>, base_dir: &PathBuf, monitors: &[MonitorInfo]) -> String {
    let config = cfg.read();
    let monitor_list: Vec<serde_json::Value> = monitors
        .iter()
        .map(|m| {
            serde_json::json!({
                "index": m.index,
                "name": m.name,
                "width": m.width,
                "height": m.height,
                "primary": m.primary,
                "label": m.label(),
            })
        })
        .collect();
    let state = serde_json::json!({
        "app": "capri",
        "version": env!("CARGO_PKG_VERSION"),
        "base_dir": base_dir.to_string_lossy(),
        "config_path": base_dir.join("config.json").to_string_lossy(),
        "save_path": base_dir.join("save_data.json").to_string_lossy(),
        "monitors": monitor_list,
        "save": save_snapshot(base_dir),
        "reminders": reminders_snapshot(base_dir),
        "config": {
            "enable_ai": config.enable_ai,
            "local_ai": config.local_ai,
            "local_model": config.local_model,
            "model": config.model,
            "tts_engine": config.tts_engine,
            "voice": config.voice,
            "voice_volume": config.voice_volume,
            // Голос: скорость и тон ползункам нужны числами, а синтезатору —
            // строками вида "+10%". Наружу отдаём проценты.
            "voice_rate": capri_core::config::speech_percent(&config.voice_rate),
            "voice_pitch": capri_core::config::speech_percent(&config.voice_pitch),
            "voice_lively": config.voice_lively,
            "voice_fx": config.voice_fx,
            // Список голосов для раздела «Голос»: имя и как звучит.
            "voices": capri_voice::edge::russian_voices()
                .into_iter()
                .map(|(id, note)| serde_json::json!({ "id": id, "note": note }))
                .collect::<Vec<_>>(),
            "mic_device": config.mic_device,
            "wake_words": config.wake_words,
            // Вид: размер, живость и монитор.
            "character_scale": capri_core::config::clamp_scale(config.character_scale),
            "liveliness": capri_core::config::normalize_liveliness(&config.liveliness),
            "monitor_index": config.monitor_index,
            // Характер: раздел «Поведение».
            "barge_in": config.barge_in,
            "greet_on_start": config.greet_on_start,
            "chatter": config.chatter,
            "chatter_min_minutes": config.chatter_min_minutes,
            "chatter_max_minutes": config.chatter_max_minutes,
            // В конфиге доля 0…1, наружу — проценты.
            "roam_percent": (config.roam_chance.clamp(0.0, 1.0) * 100.0).round(),
            "search_when_unsure": config.search_when_unsure,
            // Discord: раздел «Discord». ID не секрет, но и не напоказ —
            // показываем только факт настройки, сам номер не отдаём.
            "discord_enabled": config.discord_enabled,
            "discord_ready": !config.discord_app_id.trim().is_empty(),
            // Перевод: странице нужен провайдер для подписи в шапке.
            "translate_provider": translate_provider_label(&config, base_dir),
            "translate_to": capri_core::translate::language(&config.translate_to).name,
            // Код языка (для кнопок настроек) и адрес сервера — отдельно от
            // человекочитаемого названия выше.
            "translate_code": capri_core::translate::language(&config.translate_to).code,
            "translate_url": config.translate_libretranslate_url.clone(),
        },
    });
    serde_json::to_string(&state).unwrap_or_else(|_| "null".into())
}

/// Перерисовать панель свежим состоянием (после покупки, еды и т. п.).
fn refresh_panel(
    panel: &capri_panels::PanelId,
    panels: &Sender<capri_panels::Command>,
    cfg: &Arc<RwLock<Config>>,
    base_dir: &PathBuf,
    monitors: &[MonitorInfo],
) {
    let state = panel_state(cfg, base_dir, monitors);
    let js = format!("window.capri.setState({state});");
    let _ = panels.send(capri_panels::Command::Eval(*panel, js));
}

/// Напоминания для панели настроек.
///
/// Читаем файл, а не книгу ядра: панель — вещь интерфейса, и ей не нужно
/// ждать ядро. Файл пишет только ядро, поэтому он всегда честный.
fn reminders_snapshot(base_dir: &PathBuf) -> serde_json::Value {
    let book = capri_core::reminders::Store::new(base_dir).load();
    let now = capri_core::reminders::now_secs();
    serde_json::json!({
        "items": book.to_json(now),
        "count": book.pending().len(),
    })
}

/// Прогресс для панелей «Магазин» и «Уровни».
///
/// Читаем прямо из `save_data.json`: панели — вещь интерфейса, а не ядра,
/// и им не нужно ждать, пока ядро закончит свою работу.
fn save_snapshot(base_dir: &PathBuf) -> serde_json::Value {
    let save = capri_core::save::SaveManager::new(base_dir).load();
    let skins: Vec<serde_json::Value> = capri_core::save::SKINS
        .iter()
        .map(|skin| {
            serde_json::json!({
                "id": skin.id,
                "name": skin.name,
                "price": skin.price,
                "color": skin
                    .color
                    .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])),
                "note": skin.note,
            })
        })
        .collect();
    serde_json::json!({
        "money": save.money,
        "level": save.level,
        "xp": save.xp,
        "xp_to_next": save.xp_to_next,
        "level_reward": capri_core::save::LEVEL_REWARD_MONEY,
        "mood_joy": save.mood_joy,
        "mood_tired": save.mood_tired,
        "joy_label": save.joy_label(),
        "tired_label": save.tired_label(),
        "equipped_skin": save.equipped_skin,
        "owned_skins": save.owned_skins,
        "inventory": save.inventory,
        "snacks_available": save.snack_count(),
        "commands": save.commands,
        "pettings": save.pettings,
        "games": save.games,
        "wins": save.wins,
        "snacks": save.snacks,
        "levels_taken": save.levels_taken,
        "skins": skins,
        "phrase": capri_core::persona::line("shop", &[]),
    })
}

/// Число из данных события: `5`, `"5"` или `{"value": 5}`.
fn number_of(data: &serde_json::Value) -> Option<i64> {
    match data {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        serde_json::Value::Object(map) => map.get("value").and_then(number_of),
        _ => None,
    }
}

/// Флаг из данных события: `true`, `"true"`, `1` или `{"value": …}`.
fn flag_of(data: &serde_json::Value) -> Option<bool> {
    match data {
        serde_json::Value::Bool(on) => Some(*on),
        serde_json::Value::Number(n) => n.as_i64().map(|v| v != 0),
        serde_json::Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "on" | "да" => Some(true),
            "false" | "0" | "off" | "нет" => Some(false),
            _ => None,
        },
        serde_json::Value::Object(map) => map.get("value").and_then(flag_of),
        _ => None,
    }
}

/// Разобрать ключ поведения из панели в понятное ядру изменение.
///
/// Незнакомый ключ — `None`: лучше промолчать в лог, чем записать в
/// `config.json` что попало.
fn behavior_setting(key: &str, data: &serde_json::Value) -> Option<capri_ui::BehaviorSetting> {
    use capri_ui::BehaviorSetting;
    let value = data.get("value").cloned().unwrap_or(serde_json::Value::Null);
    match key {
        "barge-in" => flag_of(&value).map(BehaviorSetting::BargeIn),
        "greet-on-start" => flag_of(&value).map(BehaviorSetting::GreetOnStart),
        "chatter" => flag_of(&value).map(BehaviorSetting::Chatter),
        "search-when-unsure" => flag_of(&value).map(BehaviorSetting::SearchWhenUnsure),
        "roam-chance" => number_of(&value).map(|v| BehaviorSetting::RoamChance(v as f32)),
        "chatter-minutes" => {
            let min = number_of(&value)? as f32;
            let max = data.get("second").and_then(number_of).unwrap_or(min as i64) as f32;
            Some(BehaviorSetting::ChatterMinutes(min, max))
        }
        "translate-to" => text_of(&value).map(BehaviorSetting::TranslateTo),
        "translate-url" => text_of(&value).map(BehaviorSetting::TranslateUrl),
        // Голос: чем и каким голосом говорить.
        "tts-engine" => text_of(&value).map(BehaviorSetting::TtsEngine),
        "voice-name" => text_of(&value).map(BehaviorSetting::VoiceName),
        "voice-lively" => flag_of(&value).map(BehaviorSetting::VoiceLively),
        // Звук: громкость, скорость, тон и пробная фраза.
        "voice-volume" => number_of(&value)
            .map(|v| BehaviorSetting::VoiceVolume(v.clamp(0, 100) as u8)),
        "voice-rate" => {
            number_of(&value).map(|v| BehaviorSetting::VoiceRate(v as i32))
        }
        "voice-pitch" => {
            number_of(&value).map(|v| BehaviorSetting::VoicePitch(v as i32))
        }
        "voice-test" => Some(BehaviorSetting::VoiceTest),
        _ => None,
    }
}

/// Строка из данных события: `"calm"` или `{"value": "calm"}`.
fn text_of(data: &serde_json::Value) -> Option<String> {
    match data {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Object(map) => map.get("value").and_then(text_of),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_values_are_read_in_any_shape() {
        // Страница может прислать число, строку или объект — понимаем все три.
        assert_eq!(number_of(&serde_json::json!(35)), Some(35));
        assert_eq!(number_of(&serde_json::json!("35")), Some(35));
        assert_eq!(number_of(&serde_json::json!({ "value": 35 })), Some(35));
        assert_eq!(number_of(&serde_json::json!("ерунда")), None);
        assert_eq!(number_of(&serde_json::Value::Null), None);

        assert_eq!(text_of(&serde_json::json!("calm")), Some("calm".to_string()));
        assert_eq!(text_of(&serde_json::json!({ "value": "calm" })), Some("calm".to_string()));
        assert_eq!(text_of(&serde_json::json!(7)), None);
    }

    #[test]
    fn behavior_flags_are_understood_from_the_page() {
        // Флаги: страница шлёт `true`/`false`, но понимаем и «да», и 1.
        assert_eq!(flag_of(&serde_json::json!(true)), Some(true));
        assert_eq!(flag_of(&serde_json::json!("off")), Some(false));
        assert_eq!(flag_of(&serde_json::json!(1)), Some(true));
        assert_eq!(flag_of(&serde_json::json!({ "value": false })), Some(false));
        assert_eq!(flag_of(&serde_json::json!("может быть")), None);

        use capri_ui::BehaviorSetting;
        assert_eq!(
            behavior_setting("chatter", &serde_json::json!({ "value": true })),
            Some(BehaviorSetting::Chatter(true))
        );
        assert_eq!(
            behavior_setting("roam-chance", &serde_json::json!({ "value": 35 })),
            Some(BehaviorSetting::RoamChance(35.0))
        );
        assert_eq!(
            behavior_setting(
                "chatter-minutes",
                &serde_json::json!({ "value": 10, "second": 25 })
            ),
            Some(BehaviorSetting::ChatterMinutes(10.0, 25.0))
        );
        // Незнакомый ключ в config.json не пишем.
        assert_eq!(behavior_setting("телепорт", &serde_json::json!(1)), None);
    }

    #[test]
    fn translate_settings_come_from_the_page() {
        use capri_ui::BehaviorSetting;
        // Язык перевода — строка, страница шлёт её как `value`.
        assert_eq!(
            behavior_setting("translate-to", &serde_json::json!({ "value": "ja" })),
            Some(BehaviorSetting::TranslateTo("ja".into()))
        );
        // Адрес сервера — тоже строка; пустой значит «не использовать».
        assert_eq!(
            behavior_setting("translate-url", &serde_json::json!({ "value": "http://127.0.0.1:5000" })),
            Some(BehaviorSetting::TranslateUrl("http://127.0.0.1:5000".into()))
        );
        assert_eq!(
            behavior_setting("translate-url", &serde_json::json!({ "value": "" })),
            Some(BehaviorSetting::TranslateUrl(String::new()))
        );
        // Язык не может прийти числом или объектом без строки внутри.
        assert_eq!(behavior_setting("translate-to", &serde_json::json!(7)), None);
    }

    /// Каталог для теста, который пишет на диск.
    ///
    /// В `target/`, а не в системном `Temp`: в урезанных окружениях (песочницы,
    /// профиль только для чтения) запись в `%TEMP%` запрещена, и тест падал бы
    /// на создании каталога вместо проверки логики.
    fn scratch(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-tmp")
            .join(format!("{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("каталог {} не создался: {e}", dir.display()));
        dir
    }

    #[test]
    fn state_for_panel_keeps_secrets_out() {
        let mut config = Config::default();
        config.character_scale = 55;
        config.liveliness = "Calm".into();
        config.monitor_index = 1;
        config.anthropic_api_key = "sk-очень-секретный".into();
        let cfg = Arc::new(RwLock::new(config));

        let mut primary = MonitorInfo::default();
        primary.width = 2560;
        primary.height = 1440;
        primary.primary = true;
        let monitors = vec![primary];

        let dir = scratch("panel-state");
        let text = panel_state(&cfg, &dir, &monitors);
        // Ключ наружу не отдаём — странице он ни к чему.
        assert!(!text.contains("sk-очень-секретный"), "ключ утёк в панель");
        assert!(!text.contains("anthropic_api_key"));
        // А настройки вида — отдаём, в нормализованном виде.
        assert!(text.contains("\"character_scale\":55"));
        assert!(text.contains("\"liveliness\":\"calm\""));
        assert!(text.contains("\"monitor_index\":1"));
        assert!(text.contains("2560×1440"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_snapshot_has_catalog_and_progress() {
        let dir = scratch("save-snapshot");
        let snapshot = save_snapshot(&dir);

        // Прогресс на месте.
        for key in ["money", "level", "xp", "xp_to_next", "mood_joy", "mood_tired"] {
            assert!(snapshot.get(key).is_some(), "нет поля {key}");
        }
        assert!(snapshot["joy_label"].as_str().unwrap_or("").len() > 3);
        assert!(snapshot["tired_label"].as_str().unwrap_or("").len() > 3);

        // Витрина: каталог скинов с ценами и исходным видом.
        let skins = snapshot["skins"].as_array().expect("каталог скинов");
        assert!(skins.len() >= 5, "на витрине мало скинов");
        assert!(skins.iter().any(|skin| skin["id"] == "purple"));
        assert!(skins.iter().all(|skin| skin["price"].is_number()));
        assert!(skins.iter().any(|skin| skin["color"].is_string()));

        // Фраза сверху витрины — не пустая.
        assert!(!snapshot["phrase"].as_str().unwrap_or("").is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shop_buy_action_writes_progress() {
        let dir = scratch("shop-buy");
        // Своё состояние: 1000 ECS и только исходный скин.
        let manager = capri_core::save::SaveManager::new(&dir);
        let mut data = capri_core::save::SaveData::default();
        data.money = 1000;
        manager.save(&data).expect("файл прогресса");

        let cfg = Arc::new(RwLock::new(Config::default()));
        let monitors = vec![MonitorInfo::default()];
        let (panels_tx, panels_rx) = std::sync::mpsc::channel();
        let (requests_tx, requests_rx) = std::sync::mpsc::channel();
        let (ui_tx, _ui_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        events_tx
            .send(capri_panels::PanelEvent {
                panel: capri_panels::PanelId::Shop,
                action: "shop-buy".into(),
                data: serde_json::json!({ "skin": "toxic" }),
            })
            .expect("событие панели");
        drop(events_tx);

        panel_dispatch(events_rx, &cfg, &dir, &monitors, &panels_tx, &requests_tx, &ui_tx);

        // Покупка записана в save_data.json.
        let after = manager.load();
        assert!(after.owns("toxic"), "скин не попал в гардероб");
        assert_eq!(after.equipped_skin, "toxic");
        assert_eq!(after.money, 1000 - 240);

        // Ядру сказали перекрасить фигуру…
        match requests_rx.try_recv() {
            Ok(UiRequest::SkinColor(color)) => assert_eq!(color, Some([200, 255, 40])),
            other => panic!("не тот запрос ядру: {other:?}"),
        }
        // …а странице отдали свежее состояние.
        assert!(panels_rx.try_recv().is_ok(), "панель не перерисовали");

        let _ = std::fs::remove_file(dir.join("save_data.json"));
    }

    /// Разобрать JS, который ушёл странице, и достать из него состояние.
    ///
    /// Ядро общается с панелью строкой `window.capri.setState({...})`, поэтому
    /// проверять приходится именно её: так тест видит ровно то, что увидит
    /// страница, а не внутреннее состояние сессии.
    fn chess_state_from(cmd: Option<capri_panels::Command>) -> serde_json::Value {
        let Some(capri_panels::Command::Eval(panel, js)) = cmd else {
            panic!("панели ушла не команда Eval: {cmd:?}");
        };
        assert_eq!(panel, capri_panels::PanelId::Chess, "состояние ушло не в шахматы");
        // Отрезаем обёртку `window.capri.setState(` и завершающую `);`.
        let start = js.find('{').unwrap_or_else(|| panic!("в JS нет объекта: {js}"));
        let end = js.rfind('}').unwrap_or_else(|| panic!("в JS нет конца объекта: {js}"));
        serde_json::from_str(&js[start..=end])
            .unwrap_or_else(|e| panic!("состояние не разобралось ({e}): {js}"))
    }

    #[test]
    fn chess_move_gets_a_bot_reply() {
        let dir = scratch("chess-move");
        let cfg = Arc::new(RwLock::new(Config::default()));
        let monitors = vec![MonitorInfo::default()];
        let (panels_tx, panels_rx) = std::sync::mpsc::channel();
        let (requests_tx, _requests_rx) = std::sync::mpsc::channel();
        let (ui_tx, _ui_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        // Человек играет белыми и делает первый ход.
        events_tx
            .send(capri_panels::PanelEvent {
                panel: capri_panels::PanelId::Chess,
                action: "chess-move".into(),
                data: serde_json::json!({ "from": "e2", "to": "e4" }),
            })
            .expect("событие хода");
        drop(events_tx);

        panel_dispatch(events_rx, &cfg, &dir, &monitors, &panels_tx, &requests_tx, &ui_tx);

        let state = chess_state_from(panels_rx.try_recv().ok());
        let chess = &state["chess"];

        // После хода человека и ответа бота — снова ход белых, два полухода.
        assert_eq!(chess["ply"], 2, "бот обязан ответить сразу: {chess}");
        assert_eq!(chess["turn"], "white", "после ответа бота ходят белые");
        assert_eq!(chess["outcome"], serde_json::Value::Null, "партия ещё идёт");
        assert_eq!(chess["thinking"], false, "бот уже не думает");

        // Доска пришла целиком и в ней 64 поля.
        let squares = chess["squares"].as_array().expect("клетки");
        assert_eq!(squares.len(), 64);
        assert_eq!(squares[0]["square"], "a8", "доска сверху вниз");

        // История содержит оба хода.
        let history = chess["history"].as_array().expect("история");
        assert_eq!(history.len(), 2, "два полухода в истории: {history:?}");
        assert_eq!(history[0], "e2-e4", "первым записан ход человека");

        // Материал на месте с обеих сторон: 8 пешек, 2 коня, 2 слона,
        // 2 ладьи и ферзь = 4000 сантипешек у каждой стороны.
        assert_eq!(chess["material"]["white"], 4000);
        assert_eq!(chess["material"]["black"], 4000);
        assert_eq!(chess["material"]["white"], chess["material"]["black"]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chess_rejects_a_move_out_of_turn() {
        let dir = scratch("chess-turn");
        let cfg = Arc::new(RwLock::new(Config::default()));
        let monitors = vec![MonitorInfo::default()];
        let (panels_tx, panels_rx) = std::sync::mpsc::channel();
        let (requests_tx, _requests_rx) = std::sync::mpsc::channel();
        let (ui_tx, _ui_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        // Человек играет белыми. Пробуем тронуть чёрную пешку — это ход не
        // его стороны, и он обязан быть отклонён.
        events_tx
            .send(capri_panels::PanelEvent {
                panel: capri_panels::PanelId::Chess,
                action: "chess-move".into(),
                data: serde_json::json!({ "from": "e7", "to": "e5" }),
            })
            .expect("чужой ход");
        drop(events_tx);

        panel_dispatch(events_rx, &cfg, &dir, &monitors, &panels_tx, &requests_tx, &ui_tx);

        let state = chess_state_from(panels_rx.try_recv().ok());
        let chess = &state["chess"];

        // Партия даже не началась: ход отклонён, бот не отвечал.
        assert_eq!(chess["ply"], 0, "чужой ход не прошёл: {chess}");
        assert_eq!(chess["turn"], "white", "по-прежнему ход белых");
        assert_eq!(
            chess["history"].as_array().map(|h| h.len()),
            Some(0),
            "история пуста"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chess_rejects_two_human_moves_without_the_bot() {
        // Бот отвечает прямо внутри обработки `chess-move`, поэтому два
        // человеческих хода подряд всегда разделены его ответом. Проверяем,
        // что именно так и происходит: после каждого хода ход снова наш.
        let dir = scratch("chess-pairs");
        let cfg = Arc::new(RwLock::new(Config::default()));
        let monitors = vec![MonitorInfo::default()];
        let (panels_tx, panels_rx) = std::sync::mpsc::channel();
        let (requests_tx, _requests_rx) = std::sync::mpsc::channel();
        let (ui_tx, _ui_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        for (from, to) in [("e2", "e4"), ("d2", "d4")] {
            events_tx
                .send(capri_panels::PanelEvent {
                    panel: capri_panels::PanelId::Chess,
                    action: "chess-move".into(),
                    data: serde_json::json!({ "from": from, "to": to }),
                })
                .expect("ход");
        }
        drop(events_tx);

        panel_dispatch(events_rx, &cfg, &dir, &monitors, &panels_tx, &requests_tx, &ui_tx);

        let _first = chess_state_from(panels_rx.try_recv().ok());
        let state = chess_state_from(panels_rx.try_recv().ok());
        let chess = &state["chess"];

        // Оба хода человека прошли, и на каждый бот ответил: 4 полухода.
        let history = chess["history"].as_array().expect("история");
        assert_eq!(history.len(), 4, "два хода и два ответа: {history:?}");
        assert_eq!(history[0], "e2-e4");
        assert_eq!(history[2], "d2-d4", "второй ход человека — третий полуход");
        assert_eq!(chess["turn"], "white", "после ответа бота снова наш ход");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chess_new_game_starts_from_scratch() {
        let dir = scratch("chess-new");
        let cfg = Arc::new(RwLock::new(Config::default()));
        let monitors = vec![MonitorInfo::default()];
        let (panels_tx, panels_rx) = std::sync::mpsc::channel();
        let (requests_tx, _requests_rx) = std::sync::mpsc::channel();
        let (ui_tx, _ui_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        events_tx
            .send(capri_panels::PanelEvent {
                panel: capri_panels::PanelId::Chess,
                action: "chess-move".into(),
                data: serde_json::json!({ "from": "e2", "to": "e4" }),
            })
            .expect("ход");
        // Новая партия чёрными: бот обязан сыграть первым.
        events_tx
            .send(capri_panels::PanelEvent {
                panel: capri_panels::PanelId::Chess,
                action: "chess-new".into(),
                data: serde_json::json!({ "level": "easy", "side": "black" }),
            })
            .expect("новая партия");
        drop(events_tx);

        panel_dispatch(events_rx, &cfg, &dir, &monitors, &panels_tx, &requests_tx, &ui_tx);

        let _first = chess_state_from(panels_rx.try_recv().ok());
        let state = chess_state_from(panels_rx.try_recv().ok());
        let chess = &state["chess"];

        assert_eq!(chess["level"], "easy", "уровень применился");
        assert_eq!(chess["my_side"], "black", "цвет применился");
        // Играя чёрными, человек ждёт хода бота — значит, один полуход уже есть,
        // и теперь очередь человека.
        assert_eq!(chess["ply"], 1, "бот открыл партию за белых: {chess}");
        assert_eq!(chess["turn"], "black", "теперь ход человека");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chess_undo_takes_back_a_full_move() {
        let dir = scratch("chess-undo");
        let cfg = Arc::new(RwLock::new(Config::default()));
        let monitors = vec![MonitorInfo::default()];
        let (panels_tx, panels_rx) = std::sync::mpsc::channel();
        let (requests_tx, _requests_rx) = std::sync::mpsc::channel();
        let (ui_tx, _ui_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        events_tx
            .send(capri_panels::PanelEvent {
                panel: capri_panels::PanelId::Chess,
                action: "chess-move".into(),
                data: serde_json::json!({ "from": "e2", "to": "e4" }),
            })
            .expect("ход");
        events_tx
            .send(capri_panels::PanelEvent {
                panel: capri_panels::PanelId::Chess,
                action: "chess-undo".into(),
                data: serde_json::Value::Null,
            })
            .expect("отмена");
        drop(events_tx);

        panel_dispatch(events_rx, &cfg, &dir, &monitors, &panels_tx, &requests_tx, &ui_tx);

        let _first = chess_state_from(panels_rx.try_recv().ok());
        let state = chess_state_from(panels_rx.try_recv().ok());
        let chess = &state["chess"];

        // Отмена убирает и ответ бота, и наш ход: снова начальная позиция.
        assert_eq!(chess["ply"], 0, "отмена вернула к началу: {chess}");
        assert_eq!(chess["turn"], "white", "снова ход белых");
        assert_eq!(chess["history"].as_array().map(|h| h.len()), Some(0));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chess_ready_does_not_start_a_game_for_white() {
        let dir = scratch("chess-ready");
        let cfg = Arc::new(RwLock::new(Config::default()));
        let monitors = vec![MonitorInfo::default()];
        let (panels_tx, panels_rx) = std::sync::mpsc::channel();
        let (requests_tx, _requests_rx) = std::sync::mpsc::channel();
        let (ui_tx, _ui_rx) = std::sync::mpsc::channel();
        let (events_tx, events_rx) = std::sync::mpsc::channel();

        events_tx
            .send(capri_panels::PanelEvent {
                panel: capri_panels::PanelId::Chess,
                action: "ready".into(),
                data: serde_json::Value::Null,
            })
            .expect("страница загрузилась");
        drop(events_tx);

        panel_dispatch(events_rx, &cfg, &dir, &monitors, &panels_tx, &requests_tx, &ui_tx);

        let state = chess_state_from(panels_rx.try_recv().ok());
        let chess = &state["chess"];

        // Играя белыми, страница получает начальную позицию и ждёт человека.
        assert_eq!(chess["ply"], 0, "партия не началась сама: {chess}");
        assert_eq!(chess["turn"], "white", "первый ход за человеком");
        assert_eq!(chess["my_side"], "white");
        assert_eq!(chess["moves"].as_array().map(|m| m.len()), Some(20), "все 20 ходов");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn chess_win_is_rewarded_only_once() {
        let dir = scratch("chess-reward");
        let manager = capri_core::save::SaveManager::new(&dir);
        let start = manager.load();

        // Мат в один ход за белых: ферзь бьёт h7, король под матом.
        // Ставим сессию в почти законченную позицию напрямую — так тест
        // проверяет начисление, а не игру бота.
        let mut session = chess_session::Session::default();
        session.set_game_for_test(capri_core::chess::Game::from_fen(
            "rnbqkbnr/ppppp2p/8/5pp1/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
        ).expect("позиция"));
        // Ход ферзём h5 — мат? Нет, но партия идёт. Берём настоящий мат:
        session.set_game_for_test(
            capri_core::chess::Game::from_fen(
                "r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4",
            )
            .expect("мат"),
        );
        assert!(session.game().outcome().is_some(), "позиция должна быть матом");

        let cfg = Arc::new(RwLock::new(Config::default()));
        let _ = &cfg;
        let (panels_tx, panels_rx) = std::sync::mpsc::channel();
        let chess = std::sync::Mutex::new(session);

        // Три действия подряд: страница перерисовывается, награда — одна.
        // Диспетчер создаёт свою сессию, поэтому гоняем `chess_action`
        // напрямую с подставленной законченной партией.
        for _ in 0..3 {
            chess_action(
                "chess-level",
                &serde_json::json!({ "level": "easy" }),
                &chess,
                &dir,
                &panels_tx,
            );
        }

        // На каждое действие странице ушло состояние.
        let mut redraws = 0;
        while panels_rx.try_recv().is_ok() {
            redraws += 1;
        }
        assert_eq!(redraws, 3, "страница перерисована на каждое действие");

        let after = manager.load();
        assert_eq!(after.games, 1, "мат посчитан ровно один раз");
        assert_eq!(after.wins, 1, "победа записана один раз");
        // Стартовый баланс не трогаем: проверяем именно прибавку за победу.
        assert_eq!(
            after.money - start.money,
            20,
            "ECS за победу начислены один раз, а не на каждый перерисов"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
