//! Связка всего: слух, разбор команд, голос и окно.
//!
//! Это перенос класса `Capri` из `main.py`. Логика решений (какую команду
//! выполнить, что ответить) осталась в [`capri_core::Engine`]; здесь —
//! оболочка: когда проснуться, что сказать, что показать на экране.
//!
//! Потоки такие:
//!
//! * поток микрофона зовёт [`Bridge::heard`] и [`Bridge::partial`];
//! * окно зовёт [`Bridge::request`] — из меню и кнопок;
//! * главный поток крутит [`Capri::run`] и разбирает очередь [`Job`].
//!
//! Все решения принимаются в одном потоке, поэтому состояние Капри
//! (просыпание, речь, фон) не нужно защищать блокировками.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use capri_core::engine::Engine;
use capri_core::persona::{self, line};
use capri_core::reminders::{self, Book, Store};
use capri_core::save::SaveManager;
use capri_core::{action, Config, Platform, Reply};
use capri_ui::{UiEvent, UiRequest};
use capri_voice::listener::{
    self, ErrorCallback, FinalCallback, ListenerOptions, TextCallback,
};
use capri_voice::{ListenerHandle, Speaker};
use parking_lot::{Mutex, RwLock};

use crate::wake::Wake;

/// Панели, которые уже нарисованы и открываются по-настоящему.
///
/// Список берётся прямо из `PanelId::ALL`, а не пишется руками: раньше здесь
/// лежал ручной перечень, панель перевода в него забыли вписать, и кольцо на
/// живой кнопке отвечало «ещё в разработке — появится на фазе 6», хотя страница
/// уже была в сборке. Теперь забыть нечего: появилась панель — она открывается.
fn panel_is_ready(name: &str) -> bool {
    capri_panels::PanelId::ALL
        .iter()
        .any(|id| id.as_str() == name)
}

/// Привести название движка речи к тому, что знает синтезатор.
///
/// Страница присылает что угодно, а неизвестное имя означает «не говорит
/// вообще» — молчащая Капри это худший из возможных исходов настройки.
fn normalise_engine(name: &str) -> String {
    match name.trim().to_lowercase().as_str() {
        "sapi" => "sapi".to_string(),
        "silero" => "silero".to_string(),
        _ => "edge".to_string(),
    }
}

/// Что нужно Капри, чтобы работать.
pub struct Deps {
    pub base_dir: PathBuf,
    pub cfg: Arc<RwLock<Config>>,
    pub platform: Arc<dyn Platform>,
    pub speaker: Arc<Speaker>,
    pub events: Sender<UiEvent>,
    /// Уровень микрофона (биты `f32`) — для полоски в настройках.
    pub mic_level: Arc<AtomicU32>,
    /// Текущий микрофон (для меню и подсказок).
    pub current_mic: Arc<RwLock<Option<String>>>,
    /// Канал к панелям интерфейса (WebView2): попросить открыть окно.
    pub panels: Sender<capri_panels::Command>,
}

/// Работа для главного потока: всё, что меняет состояние Капри.
pub enum Job {
    /// Готовая фраза — из речи (со звуком) или из текстового поля.
    Command {
        text: String,
        audio: Vec<i16>,
        from_voice: bool,
    },
    /// Сказать что-то самой (служебное сообщение).
    Say(String),
    /// Поболтать по своей инициативе.
    Chatter,
    /// Позвать Капри.
    Call,
    /// Перечитать `config.json`.
    Reload,
    /// Сменить микрофон (`None` — системный по умолчанию).
    SetMic(Option<String>),
    /// Следующий микрофон по кругу.
    MicNext,
    /// Громкость голоса.
    SetVolume(u8),
    /// Сила эффектов голоса.
    SetVoiceFx(f32),
    /// Самостоятельные появления.
    SetChatter(bool),
    /// Поиск в браузере, если не знаю.
    SetSearch(bool),
    /// Режим обращения, имя, пол.
    SetProfile(String, String, String),
    /// Открыть `config.json`.
    OpenConfig,
    /// Открыть панель интерфейса по имени (`"settings"`, `"shop"`, …).
    OpenPanel(String),
    /// Надеть скин.
    SelectSkin(String),
    /// Перекрасить фигуру без записи в прогресс.
    SkinColor(Option<[u8; 3]>),
    /// Капри погладили (клик по фигуре) — растёт радость и статистика.
    Petted,
    /// Сменить размер, живость или монитор.
    Character(capri_ui::CharacterSetting),
    /// Изменить характер: болталка, перебивание, прогулки.
    Behavior(capri_ui::BehaviorSetting),
    /// Выбран пункт радиального меню (идентификатор сектора).
    RadialAction(String),
    /// Горячая клавиша не встала.
    HotkeyError(String),
    /// Положить текст в буфер обмена.
    CopyToClipboard(String),
    /// Поставить напоминание из панели (тело просьбы без «напомни»).
    ReminderAdd(String),
    /// Убрать одно напоминание по номеру.
    ReminderCancel(u64),
    /// Убрать все напоминания.
    ReminderClear,
    /// Закрыть Капри совсем (меню в трее, кнопка в панели, выход из окна).
    Quit,
    /// Озвучка закончилась (приходит из потока речи).
    SpeechDone {
        interrupted: bool,
    },
}

/// Мост между потоками: сюда можно звать откуда угодно.
pub struct Bridge {
    jobs: Sender<Job>,
    pub events: Sender<UiEvent>,
    pub current_mic: Arc<RwLock<Option<String>>>,
    /// Говорит ли Капри прямо сейчас (нужно для «перебей меня»).
    speaking: Arc<AtomicBool>,
    /// Попросили замолчать.
    interrupt: Arc<AtomicBool>,
    /// Слово-будильник — чтобы проснуться по неполной фразе.
    wake: Arc<Wake>,
    /// До какого момента Капри проснулась (по неполным фразам).
    awake_until: Arc<Mutex<Instant>>,
    /// Длина окна прослушивания в миллисекундах.
    window_ms: Arc<AtomicI32>,
}

impl Bridge {
    /// Пришла готовая фраза.
    pub fn heard(&self, text: &str, audio: &[i16]) {
        // Шум микрофона Vosk иногда превращает в «бай бай бай бай …».
        // Такое не команда: молча выкидываем, иначе Капри отвечает на шум.
        if capri_voice::listener::looks_like_noise(text) {
            log::debug!("Похоже на шум, пропускаю: {text}");
            return;
        }
        let _ = self.jobs.send(Job::Command {
            text: text.to_string(),
            audio: audio.to_vec(),
            from_voice: true,
        });
    }

    /// Пришла неполная фраза: если в ней обращение — просыпаемся сразу,
    /// не дожидаясь конца предложения.
    pub fn partial(&self, text: &str) {
        if self.speaking.load(Ordering::Relaxed) {
            return;
        }
        let window = self.window();
        let mut awake = self.awake_until.lock();
        if Instant::now() > *awake && self.wake.has(text) {
            *awake = Instant::now() + window;
            let _ = self.events.send(UiEvent::Summon);
            let _ = self.events.send(UiEvent::Mode("listening".into(), window.as_secs_f32()));
        }
    }

    fn window(&self) -> Duration {
        let ms = self.window_ms.load(Ordering::Relaxed).max(200);
        Duration::from_millis(ms as u64)
    }

    /// Слух не поднялся: объясняем по-русски, без паники.
    pub fn listener_error(&self, kind: &str) {
        let text = match kind {
            "model_broken" => line("model_broken", &[]),
            _ => line("no_mic", &[]),
        };
        log::warn!("Слух недоступен ({kind}): {text}");
        let _ = self.jobs.send(Job::Say(text));
    }

    /// Запрос из окна: меню, кнопки, текстовое поле.
    pub fn request(&self, request: UiRequest) {
        let job = match request {
            UiRequest::Command(text) => Job::Command { text, audio: Vec::new(), from_voice: false },
            UiRequest::Interrupt => {
                self.interrupt.store(true, Ordering::Relaxed);
                return;
            }
            UiRequest::Call => Job::Call,
            UiRequest::Quit => Job::Quit,
            UiRequest::SetMic(name) => Job::SetMic(name),
            UiRequest::MicNext => Job::MicNext,
            UiRequest::SetVolume(value) => Job::SetVolume(value),
            UiRequest::SetVoiceFx(value) => Job::SetVoiceFx(value),
            UiRequest::SetChatter(on) => Job::SetChatter(on),
            UiRequest::SetSearch(on) => Job::SetSearch(on),
            UiRequest::SetProfile(mode, name, gender) => Job::SetProfile(mode, name, gender),
            UiRequest::Reload => Job::Reload,
            UiRequest::OpenConfig => Job::OpenConfig,
            UiRequest::OpenPanel(name) => Job::OpenPanel(name),
            UiRequest::SelectSkin(name) => Job::SelectSkin(name),
            UiRequest::SkinColor(color) => Job::SkinColor(color),
            UiRequest::Petted => Job::Petted,
            UiRequest::Character(setting) => Job::Character(setting),
            UiRequest::Behavior(setting) => Job::Behavior(setting),
            UiRequest::RadialAction(id) => Job::RadialAction(id),
            UiRequest::HotkeyError(why) => Job::HotkeyError(why),
            UiRequest::CopyToClipboard(text) => Job::CopyToClipboard(text),
            UiRequest::ReminderAdd(body) => Job::ReminderAdd(body),
            UiRequest::ReminderCancel(id) => Job::ReminderCancel(id),
            UiRequest::ReminderClear => Job::ReminderClear,
        };
        let _ = self.jobs.send(job);
    }

    /// Пометка «говорю» — её ставят и слух, и главный поток.
    pub fn set_speaking(&self, value: bool) {
        self.speaking.store(value, Ordering::Relaxed);
    }

    /// Забрали ли просьбу замолчать.
    pub fn take_interrupt(&self) -> bool {
        self.interrupt.swap(false, Ordering::Relaxed)
    }
}

pub struct Capri {
    base_dir: PathBuf,
    cfg: Arc<RwLock<Config>>,
    platform: Arc<dyn Platform>,
    speaker: Arc<Speaker>,
    engine: Engine,
    save: SaveManager,
    wake: Wake,
    bridge: Arc<Bridge>,
    listener: Option<ListenerHandle>,

    /// До какого момента Капри считает, что её позвали.
    awake_until: Instant,
    speaking: bool,
    /// Слова последней фразы — чтобы не слышать саму себя.
    speech_words: Vec<String>,
    speech_end: Instant,
    /// Недавние «болталки», чтобы не повторяться.
    recent_chatter: Vec<String>,
    /// Уже предупредили про недоступный сервер голоса?
    voice_warned: bool,
    /// Через сколько прятаться, если тихо.
    hide_after: Duration,
    /// Когда последний раз что-то происходило (для «болталки»).
    last_activity: Instant,
    /// Спрятались уже?
    hidden: bool,
    /// Время последнего «просыпания» по неполной фразе.
    last_partial_wake: Instant,
    /// Живём ли ещё (гасит поток «болталки»).
    alive: Arc<AtomicBool>,
    /// Канал к панелям интерфейса.
    panels: Sender<capri_panels::Command>,
    /// Канал в поток озвучки: главный цикл не ждёт, пока голос прозвучит.
    speech: Sender<String>,
    /// Слушать продолжение после этой фразы?
    follow_up_after_speech: bool,
    /// Напоминания: книга в памяти и её файл рядом с `config.json`.
    reminders: Book,
    reminder_store: Store,
}

impl Capri {
    /// Собрать всё и сразу поднять слух.
    pub fn start(deps: Deps, requests: Receiver<UiRequest>) -> (Self, Receiver<Job>, Arc<Bridge>) {
        let snapshot = deps.cfg.read().clone();
        persona::set_user(snapshot.user_name.clone(), &snapshot.user_gender);

        let engine = Engine::new(Arc::clone(&deps.cfg), Arc::clone(&deps.platform), deps.base_dir.clone());
        let wake = Arc::new(Wake::new(&snapshot.wake_words));
        let (jobs_tx, jobs_rx) = std::sync::mpsc::channel();

        let bridge = Arc::new(Bridge {
            jobs: jobs_tx,
            events: deps.events.clone(),
            current_mic: Arc::clone(&deps.current_mic),
            speaking: Arc::new(AtomicBool::new(false)),
            interrupt: Arc::new(AtomicBool::new(false)),
            wake: Arc::clone(&wake),
            awake_until: Arc::new(Mutex::new(Instant::now())),
            window_ms: Arc::new(AtomicI32::new((snapshot.listen_window_seconds * 1000.0) as i32)),
        });

        // Окно может присылать запросы в любое время — перекладываем их в работу.
        {
            let bridge = Arc::clone(&bridge);
            std::thread::spawn(move || {
                while let Ok(request) = requests.recv() {
                    bridge.request(request);
                }
            });
        }

        let listener = Self::spawn_listener(&deps, &snapshot, &bridge);

        // Озвучка — отдельный поток. Раньше главный цикл стоял, пока голос
        // синтезировался и звучал: на медленной сети это секунды на каждую
        // фразу, и всё это время окно, меню и трей не отвечали.
        let (speech_tx, speech_rx) = std::sync::mpsc::channel::<String>();
        {
            let speaker = Arc::clone(&deps.speaker);
            let bridge = Arc::clone(&bridge);
            std::thread::spawn(move || {
                for text in speech_rx {
                    let interrupted = speaker.say(&text);
                    if bridge.jobs.send(Job::SpeechDone { interrupted }).is_err() {
                        break;
                    }
                }
            });
        }

        // Напоминания читаю сразу: если человек закрыл Капри с невыполненными,
        // они должны ждать его и в следующий раз.
        let reminder_store = Store::new(&deps.base_dir);
        let reminders = reminder_store.load();

        let capri = Self {
            base_dir: deps.base_dir.clone(),
            cfg: Arc::clone(&deps.cfg),
            platform: deps.platform,
            speaker: deps.speaker,
            engine,
            save: SaveManager::new(&deps.base_dir),
            wake: (*wake).clone(),
            bridge: Arc::clone(&bridge),
            listener,
            awake_until: Instant::now(),
            speaking: false,
            speech_words: Vec::new(),
            speech_end: Instant::now(),
            recent_chatter: Vec::new(),
            voice_warned: false,
            hide_after: Duration::from_secs_f32(snapshot.hide_after_seconds.max(1.0)),
            last_activity: Instant::now(),
            hidden: false,
            last_partial_wake: Instant::now() - Duration::from_secs(10),
            alive: Arc::new(AtomicBool::new(true)),
            panels: deps.panels.clone(),
            speech: speech_tx,
            follow_up_after_speech: false,
            reminders,
            reminder_store,
        };
        (capri, jobs_rx, bridge)
    }

    fn spawn_listener(
        deps: &Deps,
        snapshot: &Config,
        bridge: &Arc<Bridge>,
    ) -> Option<ListenerHandle> {
        let on_partial: TextCallback = {
            let bridge = Arc::clone(bridge);
            Arc::new(move |text: &str| bridge.partial(text))
        };
        let on_final: FinalCallback = {
            let bridge = Arc::clone(bridge);
            Arc::new(move |text: &str, audio: &[i16]| bridge.heard(text, audio))
        };
        let on_error: ErrorCallback = {
            let bridge = Arc::clone(bridge);
            Arc::new(move |kind: &str| bridge.listener_error(kind))
        };

        let handle = ListenerHandle::spawn(
            ListenerOptions {
                model_dir: deps.base_dir.join(&snapshot.vosk_model),
                device: snapshot.mic_device.clone(),
                gain: snapshot.mic_gain,
                sensitivity: snapshot.mic_sensitivity,
            },
            on_partial,
            on_final,
            on_error,
        );

        if let Some(handle) = &handle {
            if let Some(name) = handle.active_name() {
                *deps.current_mic.write() = Some(name);
            }
            // Уровень для индикатора: из слуха в общий атомик.
            let source = Arc::clone(&handle.level);
            let target = Arc::clone(&deps.mic_level);
            std::thread::spawn(move || loop {
                target.store(source.load(Ordering::Relaxed), Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(60));
            });
        }
        handle
    }

    // ------------------------------------------------------------------ цикл

    /// Крутить Капри до выхода. Возвращает код выхода.
    pub fn run(&mut self, jobs: Receiver<Job>) -> i32 {
        // Скин, купленный в прошлый раз, надеваем сразу — до приветствия,
        // чтобы Капри не появилась в чужом цвете.
        self.wear_equipped_skin();
        self.greet();

        // «Болталка» живёт отдельным потоком и просто подкидывает работу.
        let chatter = self.start_chatter_thread();
        let mut last_mood_tick = Instant::now();
        let mut last_fullscreen_check = Instant::now();
        let mut fullscreen = false;

        loop {
            match jobs.recv_timeout(Duration::from_millis(250)) {
                Ok(job) => {
                    if self.handle_job(job) {
                        break;
                    }
                    self.last_activity = Instant::now();
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }

            // Просьба замолчать из меню.
            if self.bridge.take_interrupt() && self.speaking {
                self.speaker.stop();
                log::info!("Капри перебили");
            }

            // Напоминания проверяем каждый тик. Свою речь не перебиваем:
            // сработает сразу, как только Капри договорит.
            if !self.speaking {
                self.fire_reminders();
            }

            // Спрятаться, если давно тихо и говорить нечего.
            if !self.hidden
                && !self.speaking
                && self.last_activity.elapsed() > self.hide_after
                && Instant::now() > self.awake_until
            {
                self.hidden = true;
                let _ = self.bridge.events.send(UiEvent::Vanish);
            }

            // Настроение: раз в минуту усталость подрастает, радость тает.
            if last_mood_tick.elapsed() > Duration::from_secs(60) {
                last_mood_tick = Instant::now();
                self.tick_mood();
            }

            // Раз в полминуты смотрим, не на весь ли экран чужая игра:
            // в полноэкранном приложении Капри не вылезает поболтать.
            if last_fullscreen_check.elapsed() > Duration::from_secs(30) {
                last_fullscreen_check = Instant::now();
                fullscreen = self.platform.fullscreen_app_active();
                if fullscreen {
                    log::debug!("На весь экран открыто другое приложение — молчу");
                }
            }
            let _ = fullscreen;
        }

        self.alive.store(false, Ordering::Relaxed);
        self.bridge.set_speaking(false);
        let _ = chatter.join();
        log::info!("Капри ушла на покой");
        0
    }

    /// Обработать одну работу. `true` — пора выходить.
    fn handle_job(&mut self, job: Job) -> bool {
        match job {
            Job::Command { text, audio, from_voice } => {
                self.handle_command(text, audio, from_voice);
                self.engine.quit_requested()
            }
            Job::Say(text) => {
                self.summon();
                if text.is_empty() {
                    let _ = self.bridge.events.send(UiEvent::Vanish);
                } else {
                    self.say(&text);
                }
                false
            }
            // «Закрыть» из меню и любое другое «выйти совсем». Раньше здесь
            // отправлялся `Say("")`, то есть Капри просто пряталась: окно
            // исчезало, а процесс оставался висеть в диспетчере задач, держа
            // горячую клавишу и имя единственного экземпляра. Теперь выходим
            // по-настоящему — цикл `run` завершится на `true`.
            // Напоминания из панели настроек: тело просьбы приходит без
            // «напомни», разбор времени и фразы — те же, что у голоса.
            Job::ReminderAdd(body) => {
                self.summon();
                let now = reminders::now_secs();
                match reminders::parse_when(&body, now) {
                    Some((what, due)) if !what.trim().is_empty() => {
                        let id = self.reminders.add(what.clone(), due);
                        let _ = self.reminder_store.save(&self.reminders);
                        log::info!(
                            "Напоминание #{id} из панели на {}: {}",
                            reminders::human_left(due, now),
                            what
                        );
                        self.say(&reminders::confirm(&what, due, now));
                    }
                    _ => {
                        log::info!("Панель прислала напоминание без времени: {body}");
                        self.say(reminders::ask_when());
                    }
                }
                false
            }
            Job::ReminderCancel(id) => {
                let text = self
                    .reminders
                    .pending()
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| r.text.clone());
                match text {
                    Some(text) => {
                        self.reminders.cancel(id);
                        let _ = self.reminder_store.save(&self.reminders);
                        log::info!("Напоминание #{id} убрано из панели: {text}");
                        self.summon();
                        self.say(&reminders::cancelled(Some(&text)));
                    }
                    None => log::info!("Напоминание #{id} уже не ждёт своего часа"),
                }
                false
            }
            Job::ReminderClear => {
                let count = self.reminders.clear();
                let _ = self.reminder_store.save(&self.reminders);
                log::info!("Все напоминания убраны из панели: {count}");
                self.summon();
                self.say(&reminders::cancelled_all(count));
                false
            }
            Job::Quit => {
                log::info!("Просьба закрыть — выхожу");
                true
            }
            Job::SpeechDone { interrupted } => {
                self.finish_speech(interrupted);
                false
            }
            Job::Chatter => {
                if !self.speaking {
                    let text = self.chatter_line();
                    let roam = rand_percent() < (self.cfg.read().roam_chance.clamp(0.0, 1.0) * 100.0);
                    if roam {
                        let _ = self.bridge.events.send(UiEvent::Roam);
                        std::thread::sleep(Duration::from_millis(1500));
                    } else {
                        self.summon();
                    }
                    self.say(&text);
                }
                false
            }
            Job::Call => {
                if self.speaking {
                    self.speaker.stop();
                    self.bridge.set_speaking(false);
                    self.speaking = false;
                    self.awake_until = Instant::now() + self.listen_window();
                    let _ = self
                        .bridge
                        .events
                        .send(UiEvent::Mode("listening".into(), self.listen_window().as_secs_f32()));
                    let _ = self.bridge.events.send(UiEvent::Subtitle(line("listen", &[])));
                    return false;
                }
                self.summon();
                self.awake_until = Instant::now() + self.listen_window();
                let _ = self
                    .bridge
                    .events
                    .send(UiEvent::Mode("listening".into(), self.listen_window().as_secs_f32()));
                let _ = self.bridge.events.send(UiEvent::Subtitle(line("listen", &[])));
                false
            }
            Job::Reload => {
                self.reload_settings(true);
                false
            }
            Job::SetMic(name) => {
                self.set_mic(name);
                false
            }
            Job::MicNext => {
                let reply = self.next_mic();
                self.summon();
                self.say(&reply);
                false
            }
            Job::SetVolume(value) => {
                let value = value.min(100);
                self.speaker.set_volume(value);
                if let Err(e) = self.cfg.read().set_value("voice_volume", serde_json::json!(value)) {
                    log::warn!("Не сохранила громкость: {e}");
                }
                false
            }
            Job::SetVoiceFx(value) => {
                if let Err(e) = self.cfg.read().set_value("voice_fx", serde_json::json!(value)) {
                    log::warn!("Не сохранила силу эффектов: {e}");
                }
                false
            }
            Job::SetChatter(on) => {
                if let Err(e) = self.cfg.read().set_value("chatter", serde_json::json!(on)) {
                    log::warn!("Не сохранила болталку: {e}");
                }
                log::info!(
                    "Самостоятельные появления: {}",
                    if on { "включены" } else { "выключены" }
                );
                false
            }
            Job::SetSearch(on) => {
                if let Err(e) = self.cfg.read().set_value("search_when_unsure", serde_json::json!(on)) {
                    log::warn!("Не сохранила поиск: {e}");
                }
                false
            }
            Job::SetProfile(mode, name, gender) => {
                let cfg = self.cfg.read();
                let _ = cfg.set_value("user_name_mode", serde_json::json!(mode));
                let _ = cfg.set_value("user_name", serde_json::json!(name));
                let _ = cfg.set_value("user_gender", serde_json::json!(gender));
                drop(cfg);
                persona::set_user(name.clone(), &gender);
                log::info!("Обращение: {}", if name.is_empty() { "без имени".to_string() } else { name });
                false
            }
            Job::OpenConfig => {
                let path = self.base_dir.join("config.json");
                if let Err(e) = self.platform.launch(&path.to_string_lossy()) {
                    log::warn!("Не открыла config.json: {e}");
                }
                false
            }
            // «Отправить в main» из терминала: своего второго терминала у
            // приложения нет, поэтому команда уходит в буфер обмена — её
            // можно вставить в любой настоящий терминал.
            Job::CopyToClipboard(text) => {
                match self.platform.clipboard_set(&text) {
                    Ok(()) => log::info!("Команда скопирована в буфер: {text}"),
                    Err(why) => log::warn!("Не скопировала команду: {why}"),
                }
                false
            }
            Job::OpenPanel(name) => {
                match capri_panels::PanelId::parse(&name) {
                    Some(panel) => {
                        if self.panels.send(capri_panels::Command::Open(panel)).is_err() {
                            log::warn!("Панели недоступны: канал закрыт");
                        } else {
                            log::info!("Открываю панель «{}»", panel.as_str());
                        }
                    }
                    None => log::warn!("Неизвестная панель: {name}"),
                }
                false
            }
            // Скин по голосу: надеть можно только купленное. Покупка живёт
            // в магазине, где видно цену и баланс.
            Job::SelectSkin(name) => {
                let mut data = self.save.load();
                match data.equip(&name) {
                    Ok(skin) => {
                        if let Err(e) = self.save.save(&data) {
                            log::warn!("Не сохранила скин: {e}");
                        }
                        log::info!("Надела скин «{}»", skin.name);
                        let _ = self.bridge.events.send(UiEvent::SetSkin(skin.color));
                    }
                    Err(why) => {
                        log::info!("Скин «{name}» не надела: {why}");
                        self.say(&why);
                    }
                }
                false
            }
            // Перекраска без записи: прогресс уже сохранила оболочка.
            Job::SkinColor(color) => {
                let _ = self.bridge.events.send(UiEvent::SetSkin(color));
                false
            }
            // Погладили: радость растёт, статистика помнит.
            Job::Petted => {
                let data = self.save.update(|data| data.on_pet());
                log::debug!("Поглаживания: {} (радость {})", data.pettings, data.mood_joy);
                false
            }
            Job::Character(setting) => {
                self.apply_character(setting);
                false
            }
            Job::Behavior(setting) => {
                self.apply_behavior(setting);
                false
            }
            // Радиальное меню: у каждого сектора своя судьба — часть уже
            // открывает панель, остальные честно говорят, что их ещё нет.
            Job::RadialAction(id) => match capri_core::menu::find(&id) {
                Some(sector) => match sector.target {
                    // Панель может быть ещё не нарисована: тогда честно
                    // говорим, на какой фазе она появится, а не открываем
                    // пустое окно.
                    capri_core::menu::Target::Panel(panel) if panel_is_ready(panel) => {
                        self.handle_job(Job::OpenPanel(panel.to_string()))
                    }
                    capri_core::menu::Target::Panel(_) | capri_core::menu::Target::Soon => {
                        let text = format!(
                            "«{}» ещё в разработке — она появится на фазе {}.",
                            sector.label, sector.phase
                        );
                        self.handle_job(Job::Say(text))
                    }
                    // Переключение страниц кольца разбирает само окно меню:
                    // до ядра такой сектор доходить не должен.
                    capri_core::menu::Target::Submenu | capri_core::menu::Target::Back => {
                        log::debug!("Меню: «{}» — это навигация, ядру делать нечего", sector.label);
                        false
                    }
                },
                None => {
                    log::warn!("Меню прислало неизвестный пункт: {id}");
                    false
                }
            },
            Job::HotkeyError(why) => {
                log::warn!("{why}");
                // Молчать нельзя: иначе «клавиша не работает» выглядит как поломка.
                let _ = self.bridge.events.send(UiEvent::Subtitle(why));
                false
            }
        }
    }

    /// Размер, живость и монитор: пишем в `config.json` и сразу показываем.
    ///
    /// Настройки меняются ползунком, то есть часто. Поэтому сохраняем только
    /// само значение (`set_value` перечитывает файл и меняет один ключ), а окно
    /// получает событие и перерисовывается без перезапуска.
    fn apply_character(&mut self, setting: capri_ui::CharacterSetting) {
        {
            let mut cfg = self.cfg.write();
            match &setting {
                capri_ui::CharacterSetting::Scale(percent) => {
                    let percent = capri_core::config::clamp_scale(*percent);
                    cfg.character_scale = percent;
                    if let Err(e) = cfg.set_value("character_scale", serde_json::json!(percent)) {
                        log::warn!("Не сохранила размер: {e}");
                    }
                    log::info!("Размер Капри: {percent} %");
                }
                capri_ui::CharacterSetting::Liveliness(value) => {
                    let value = capri_core::config::normalize_liveliness(value);
                    cfg.liveliness = value.to_string();
                    if let Err(e) = cfg.set_value("liveliness", serde_json::json!(value)) {
                        log::warn!("Не сохранила живость: {e}");
                    }
                    log::info!("Живость: {value}");
                }
                capri_ui::CharacterSetting::Monitor(index) => {
                    cfg.monitor_index = *index;
                    if let Err(e) = cfg.set_value("monitor_index", serde_json::json!(index)) {
                        log::warn!("Не сохранила монитор: {e}");
                    }
                    log::info!("Монитор: {index}");
                }
            }
        }
        let _ = self.bridge.events.send(UiEvent::Character(setting));
    }

    /// Характер: болталка, перебивание, прогулки, поиск.
    ///
    /// Как и вид, это меняется ползунком и сразу пишется в `config.json` —
    /// перезапуск не нужен, следующий шаг ядра уже видит новые значения.
    fn apply_behavior(&mut self, setting: capri_ui::BehaviorSetting) {
        use capri_ui::BehaviorSetting;
        {
            let mut cfg = self.cfg.write();
            match &setting {
                BehaviorSetting::BargeIn(on) => {
                    cfg.barge_in = *on;
                    if let Err(e) = cfg.set_value("barge_in", serde_json::json!(on)) {
                        log::warn!("Не сохранила перебивание: {e}");
                    }
                    log::info!(
                        "Перебивать меня: {}",
                        if *on { "можно" } else { "нельзя" }
                    );
                }
                BehaviorSetting::GreetOnStart(on) => {
                    cfg.greet_on_start = *on;
                    if let Err(e) = cfg.set_value("greet_on_start", serde_json::json!(on)) {
                        log::warn!("Не сохранила приветствие: {e}");
                    }
                    log::info!(
                        "Здороваться при запуске: {}",
                        if *on { "да" } else { "нет" }
                    );
                }
                BehaviorSetting::Chatter(on) => {
                    cfg.chatter = *on;
                    if let Err(e) = cfg.set_value("chatter", serde_json::json!(on)) {
                        log::warn!("Не сохранила болталку: {e}");
                    }
                    log::info!(
                        "Болталка: {}",
                        if *on { "включена" } else { "выключена" }
                    );
                }
                BehaviorSetting::ChatterMinutes(min, max) => {
                    let (min, max) = capri_core::config::clamp_chatter_minutes(*min, *max);
                    cfg.chatter_min_minutes = min;
                    cfg.chatter_max_minutes = max;
                    if let Err(e) = cfg.set_value("chatter_min_minutes", serde_json::json!(min)) {
                        log::warn!("Не сохранила начало болталки: {e}");
                    }
                    if let Err(e) = cfg.set_value("chatter_max_minutes", serde_json::json!(max)) {
                        log::warn!("Не сохранила конец болталки: {e}");
                    }
                    log::info!("Болталка: раз в {min:.0}–{max:.0} минут");
                }
                BehaviorSetting::RoamChance(percent) => {
                    // В конфиге доля 0…1, в настройках — проценты.
                    let share = (percent.clamp(0.0, 100.0) / 100.0 * 1000.0).round() / 1000.0;
                    cfg.roam_chance = share;
                    if let Err(e) = cfg.set_value("roam_chance", serde_json::json!(share)) {
                        log::warn!("Не сохранила шанс прогулки: {e}");
                    }
                    log::info!("Шанс прогулки: {:.0} %", share * 100.0);
                }
                BehaviorSetting::SearchWhenUnsure(on) => {
                    cfg.search_when_unsure = *on;
                    if let Err(e) = cfg.set_value("search_when_unsure", serde_json::json!(on)) {
                        log::warn!("Не сохранила поиск: {e}");
                    }
                    log::info!(
                        "Искать в сети, когда не уверена: {}",
                        if *on { "да" } else { "нет" }
                    );
                }
                BehaviorSetting::TranslateTo(code) => {
                    // Незнакомый код не беда: `translate::language` вернёт
                    // русский, но записывать в конфиг мусор всё равно незачем.
                    let code = capri_core::translate::language(code).code.to_string();
                    cfg.translate_to = code.clone();
                    if let Err(e) = cfg.set_value("translate_to", serde_json::json!(code)) {
                        log::warn!("Не сохранила язык перевода: {e}");
                    }
                    log::info!(
                        "Перевод: перевожу на {}",
                        capri_core::translate::language(&code).russian_name
                    );
                }
                BehaviorSetting::TranslateUrl(url) => {
                    let url = url.trim().trim_end_matches('/').to_string();
                    cfg.translate_libretranslate_url = url.clone();
                    if let Err(e) =
                        cfg.set_value("translate_libretranslate_url", serde_json::json!(url))
                    {
                        log::warn!("Не сохранила адрес LibreTranslate: {e}");
                    }
                    log::info!(
                        "Перевод: LibreTranslate {}",
                        if url.is_empty() { "выключен" } else { "включён" }
                    );
                }
                BehaviorSetting::TtsEngine(engine) => {
                    let engine = normalise_engine(engine);
                    cfg.tts_engine = engine.clone();
                    if let Err(e) = cfg.set_value("tts_engine", serde_json::json!(engine)) {
                        log::warn!("Не сохранила движок речи: {e}");
                    }
                    log::info!("Голос: движок {engine}");
                }
                BehaviorSetting::VoiceName(name) => {
                    let name = name.trim().to_string();
                    cfg.voice = name.clone();
                    if let Err(e) = cfg.set_value("voice", serde_json::json!(name)) {
                        log::warn!("Не сохранила голос: {e}");
                    }
                    log::info!("Голос: {name}");
                }
                BehaviorSetting::VoiceLively(on) => {
                    cfg.voice_lively = *on;
                    if let Err(e) = cfg.set_value("voice_lively", serde_json::json!(on)) {
                        log::warn!("Не сохранила живость голоса: {e}");
                    }
                    log::info!(
                        "Живая интонация: {}",
                        if *on { "включена" } else { "выключена" }
                    );
                }
                BehaviorSetting::VoiceVolume(volume) => {
                    let volume = (*volume).min(100);
                    cfg.voice_volume = volume;
                    if let Err(e) = cfg.set_value("voice_volume", serde_json::json!(volume)) {
                        log::warn!("Не сохранила громкость голоса: {e}");
                    }
                    log::info!("Громкость голоса: {volume}");
                }
                BehaviorSetting::VoiceRate(percent) => {
                    let rate = capri_core::config::speech_string(*percent);
                    cfg.voice_rate = rate.clone();
                    if let Err(e) = cfg.set_value("voice_rate", serde_json::json!(rate)) {
                        log::warn!("Не сохранила скорость речи: {e}");
                    }
                    log::info!("Скорость речи: {rate}");
                }
                BehaviorSetting::VoicePitch(percent) => {
                    let pitch = capri_core::config::speech_string(*percent);
                    cfg.voice_pitch = pitch.clone();
                    if let Err(e) = cfg.set_value("voice_pitch", serde_json::json!(pitch)) {
                        log::warn!("Не сохранила тон речи: {e}");
                    }
                    log::info!("Тон речи: {pitch}");
                }
                // Пробная фраза: ничего не запоминаем, только говорим.
                BehaviorSetting::VoiceTest => {}
            }
        }

        // Голос проверяем уже после того, как отпустили конфиг: синтез идёт
        // секундами, и держать на это замок настроек нельзя.
        if matches!(setting, BehaviorSetting::VoiceTest) {
            let phrase = "Привет! Это мой голос. Так я буду с тобой разговаривать.";
            self.handle_job(Job::Say(phrase.to_string()));
        }
    }

    // ------------------------------------------------------- речь и слух

    fn listen_window(&self) -> Duration {
        Duration::from_secs_f32(self.cfg.read().listen_window_seconds.max(1.0))
    }

    fn summon(&mut self) {
        self.hidden = false;
        let _ = self.bridge.events.send(UiEvent::Summon);
    }

    /// Надеть скин, который был выбран в прошлый раз.
    fn wear_equipped_skin(&mut self) {
        let data = self.save.load();
        let color = capri_core::save::skin_color(&data.equipped_skin);
        let _ = self.bridge.events.send(UiEvent::SetSkin(color));
        if let Some(skin) = capri_core::save::skin_by_id(&data.equipped_skin) {
            log::info!("Скин при запуске: {}", skin.name);
        }
    }

    /// Минутный тик настроения: усталость растёт (ночью быстрее),
    /// радость тает, если Капри долго не трогают.
    fn tick_mood(&mut self) {
        let night = {
            use chrono::Timelike;
            let hour = chrono::Local::now().hour();
            hour >= 23 || hour < 6
        };
        let before = self.save.load();
        let mut changed = false;
        let data = self.save.update(|data| {
            changed = data.tick_mood(night);
        });
        if changed {
            log::debug!(
                "Настроение: радость {} ({}), усталость {} ({})",
                data.mood_joy,
                data.joy_label(),
                data.mood_tired,
                data.tired_label()
            );
        }
        // Совсем вымоталась — честно об этом говорит, но не чаще раза в час:
        // порог проверяем по переходу, а не по состоянию.
        if data.mood_tired >= 90 && before.mood_tired < 90 {
            let text = persona::line_first("tired", &[("name", &capri_core::persona::user_name())]);
            if !text.is_empty() {
                self.say(&text);
            }
        }
    }

    /// Поздороваться при запуске.
    fn greet(&mut self) {
        if !self.cfg.read().greet_on_start {
            return;
        }
        self.summon();
        let text = line("greet", &[]);
        self.say(&text);
    }

    /// Сказать фразу. `true` — речь прервали.
    fn say(&mut self, text: &str) -> bool {
        if text.trim().is_empty() {
            let _ = self.bridge.events.send(UiEvent::Mode("idle".into(), 0.0));
            return false;
        }
        log::info!("Капри: {text}");
        let _ = self.bridge.events.send(UiEvent::Subtitle(text.to_string()));
        let _ = self.bridge.events.send(UiEvent::Mode("speaking".into(), 0.0));

        let barge_in = self.cfg.read().barge_in && self.listener.is_some();
        self.speech_words = self
            .wake_normalized(text)
            .split_whitespace()
            .map(|w| w.to_string())
            .collect();

        // Новая фраза важнее прошлой: просим замолчать и ставим новую.
        if self.speaking {
            self.speaker.stop();
        }
        self.speaking = true;
        self.bridge.set_speaking(true);

        // Без «перебивания» микрофон на время речи глушим.
        if !barge_in {
            if let Some(listener) = &self.listener {
                listener.set_muted(true);
            }
        }

        // Сама озвучка — в стороне; сюда вернётся `Job::SpeechDone`.
        if self.speech.send(text.to_string()).is_err() {
            log::warn!("Поток озвучки не отвечает — говорю прямо здесь");
            let interrupted = self.speaker.say(text);
            self.finish_speech(interrupted);
        }
        false
    }

    /// Озвучка закончилась: снять пометки, вернуть микрофон, дать время на продолжение.
    fn finish_speech(&mut self, interrupted: bool) {
        if !self.speaking {
            // Фразу уже перебили или учли (например, «позвать Капри»).
            return;
        }
        self.speaking = false;
        self.bridge.set_speaking(false);
        self.speech_end = Instant::now();
        if let Some(listener) = &self.listener {
            if self.cfg.read().barge_in {
                listener.request_reset();
            } else {
                listener.set_muted(false);
            }
        }

        if interrupted {
            let _ = self.bridge.events.send(UiEvent::Mode("idle".into(), 0.0));
        } else if self.follow_up_after_speech {
            // Даём договорить продолжение: «а ещё…».
            let follow = Duration::from_secs_f32(self.cfg.read().followup_seconds.max(1.0));
            self.awake_until = Instant::now() + follow;
            let _ = self
                .bridge
                .events
                .send(UiEvent::Mode("listening".into(), follow.as_secs_f32()));
        }
        self.follow_up_after_speech = false;
        self.check_voice_problem();
    }

    /// Слова фразы в том же виде, в каком приходят из распознавания.
    fn wake_normalized(&self, text: &str) -> String {
        text.to_lowercase().replace('ё', "е")
    }

    /// Слышит ли Капри собственную речь.
    fn is_echo(&self, text: &str) -> bool {
        let words: Vec<String> = self
            .wake_normalized(text)
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
            .filter(|w| !w.is_empty())
            .collect();
        !words.is_empty() && words.iter().all(|w| self.speech_words.contains(w))
    }

    /// Готовая фраза из микрофона.
    fn handle_command(&mut self, text: String, audio: Vec<i16>, from_voice: bool) {
        let now = Instant::now();
        let low = self.wake_normalized(&text);

        // Любая новая «стоп» гасит автопрокрутку.
        if self.engine.autoscroll_active() && self.wake.is_stop(&low) && !self.is_echo(&text) {
            self.engine.stop_background();
            let _ = self.bridge.events.send(UiEvent::Subtitle(line("scroll_stop", &[])));
            return;
        }

        // Пока говорю — либо перебивают, либо не слушаю.
        if self.speaking {
            let barge_in = self.cfg.read().barge_in;
            if !(barge_in && self.wants_interrupt(&text)) {
                return;
            }
            self.speaker.stop();
            self.speaking = false;
            self.bridge.set_speaking(false);
            self.awake_until = now + self.listen_window();
            let _ = self
                .bridge
                .events
                .send(UiEvent::Mode("listening".into(), self.listen_window().as_secs_f32()));
            let _ = self.bridge.events.send(UiEvent::Subtitle(line("listen", &[])));
            log::info!("Капри перебили");
        }

        // Эхо собственной речи сразу после ответа.
        if self.speech_end.elapsed() < Duration::from_secs(1) && self.is_echo(&text) {
            return;
        }

        let woke = self.wake.has(&text);
        if !woke && now > self.awake_until {
            return;
        }
        let mut cmd = if woke { self.wake.strip(&text) } else { text.clone() };

        // Google может расслышать лучше — но только короткие фразы.
        if from_voice && self.cfg.read().use_google_stt && capri_voice::google::worth_trying(&text) {
            if let Some(better) = capri_voice::google::recognize(&audio, "ru-RU") {
                let better = better.to_lowercase();
                if better.chars().count() >= 2 {
                    cmd = better;
                }
            }
        }

        // Короткое «стоп» — просто замолчать.
        if self.wake.is_stop(&cmd) && cmd.split_whitespace().count() <= 3 {
            self.awake_until = now + self.listen_window();
            if self.engine.stop_background() {
                let _ = self.bridge.events.send(UiEvent::Subtitle(line("scroll_stop", &[])));
            }
            return;
        }

        // Слишком коротко — считаем, что просто позвали.
        if cmd.chars().count() < 2 {
            self.awake_until = now + self.listen_window();
            let _ = self.bridge.events.send(UiEvent::Subtitle(String::new()));
            if self.last_partial_wake.elapsed() > Duration::from_secs(3) {
                self.last_partial_wake = now;
                self.summon();
                let _ = self
                    .bridge
                    .events
                    .send(UiEvent::Mode("listening".into(), self.listen_window().as_secs_f32()));
            }
            return;
        }

        self.awake_until = Instant::now();
        self.process_command(&cmd);
    }

    /// Собирается ли пользователь перебить.
    fn wants_interrupt(&self, text: &str) -> bool {
        (self.wake.has(text) || self.wake.is_stop(text)) && !self.is_echo(text)
    }

    /// Главный разбор: команда → действие → реплика.
    fn process_command(&mut self, cmd: &str) {
        self.summon();
        let _ = self.bridge.events.send(UiEvent::Mode("thinking".into(), 0.0));
        let _ = self.bridge.events.send(UiEvent::Subtitle(format!("> {cmd}")));
        log::info!("Команда: {cmd}");
        self.reload_settings(false);

        // Если обращение всё ещё внутри — убираем.
        let mut command = cmd.to_string();
        if self.wake.has(&command) {
            let stripped = self.wake.strip(&command);
            if stripped.chars().count() < 2 {
                self.awake_until = Instant::now() + self.listen_window();
                let _ = self
                    .bridge
                    .events
                    .send(UiEvent::Mode("listening".into(), self.listen_window().as_secs_f32()));
                let _ = self.bridge.events.send(UiEvent::Subtitle(line("listen", &[])));
                return;
            }
            command = stripped;
        }

        // Напоминания — раньше движка и нейросети: это наши собственные часы
        // и файл, и никакая модель не должна выдумывать за них время.
        if let Some(reply) = self.try_reminder(&command) {
            self.save.reward();
            self.follow_up_after_speech = true;
            self.say(&reply);
            return;
        }

        let mut reply = self.engine.handle(&command);

        // Награда за понятую команду.
        if reply.is_some() {
            self.save.reward();
        }

        // Смена микрофона — это умеет только оболочка.
        if let Some(r) = &reply {
            if r.action_is(action::MIC_INFO) {
                reply = Some(Reply::say(line("mic_info", &[("t", &self.current_mic_name())])));
            } else if r.action_is(action::MIC_NEXT) {
                reply = Some(Reply::say(self.next_mic()));
            }
        }

        // Не поняла — спрашиваем нейросеть или ищем в браузере.
        let reply = match reply {
            Some(reply) => reply,
            None => {
                let snapshot = self.cfg.read().clone();
                let ai = capri_core::ai::AiChat::new(&snapshot);
                if snapshot.enable_ai && ai.available() {
                    match ai.ask(&command) {
                        Some(text) => Reply::answer(text),
                        None => self.engine.fallback(&command, false),
                    }
                } else if snapshot.local_ai {
                    let local = capri_core::ai::LocalAi::new(&snapshot);
                    match local.ask(&command) {
                        Some(text) => Reply::answer(text),
                        None => self.engine.fallback(&command, true),
                    }
                } else {
                    self.engine.fallback(&command, false)
                }
            }
        };

        // Действия оболочки по пометке ответа.
        if reply.action_is(action::ANSWER) && self.cfg.read().teleport_on_answer {
            let _ = self.bridge.events.send(UiEvent::Hop);
        }
        if reply.action_is(action::WAVE) {
            let _ = self.bridge.events.send(UiEvent::PlayAnim("capri".into()));
        } else if let Some(where_) = reply.action_arg(action::MOVE) {
            let _ = self.bridge.events.send(UiEvent::MoveTo(where_.to_string()));
        } else if reply.action_is(action::ROAM) {
            let _ = self.bridge.events.send(UiEvent::Vanish);
            std::thread::sleep(Duration::from_millis(800));
            let _ = self.bridge.events.send(UiEvent::Roam);
            std::thread::sleep(Duration::from_millis(1500));
        }

        // Продолжение слушаем, только если фраза не оборвётся: решает поток
        // озвучки, поэтому просьбу ставим заранее.
        self.follow_up_after_speech =
            !reply.action_is(action::QUIT) && !reply.action_is(action::HIDE);
        self.say(&reply.text);

        if reply.action_is(action::QUIT) {
            self.engine_stop();
            let _ = self.bridge.events.send(UiEvent::Vanish);
            std::process::exit(0);
        }
        if reply.action_is(action::HIDE) {
            self.hidden = true;
            let _ = self.bridge.events.send(UiEvent::Vanish);
            return;
        }
    }

    fn engine_stop(&mut self) {
        self.engine.stop_background();
    }

    /// Понять просьбу про напоминания. `Some(фраза)` — ответ уже готов.
    ///
    /// Живёт в оболочке, а не в `Engine`: напоминанию нужны часы, файл и
    /// книга, а движок разбора про диск и время ничего не знает. Сами фразы
    /// собирает [`capri_core::reminders`] — их покрывают тесты.
    fn try_reminder(&mut self, command: &str) -> Option<String> {
        let low = command.to_lowercase();

        // «какие напоминания», «список напоминаний».
        let asks_for_list = low.contains("напоминан")
            && (low.contains("какие")
                || low.contains("список")
                || low.contains("что там")
                || low.contains("мои")
                || low.contains("покажи"));
        if asks_for_list {
            return Some(reminders::list(&self.reminders, reminders::now_secs()));
        }

        // «отмени напоминание», «удали последнее напоминание», «убери все».
        let asks_to_cancel = low.contains("напоминан")
            && (low.contains("отмен") || low.contains("удали") || low.contains("убери"));
        if asks_to_cancel {
            let all = low.contains("все") || low.contains("всё");
            if all {
                let count = self.reminders.clear();
                let _ = self.reminder_store.save(&self.reminders);
                log::info!("Напоминания: убрала все ({count})");
                return Some(reminders::cancelled_all(count));
            }
            let last = self
                .reminders
                .pending()
                .last()
                .map(|r| (r.id, r.text.clone()));
            let reply = match last {
                Some((id, text)) => {
                    self.reminders.cancel(id);
                    let _ = self.reminder_store.save(&self.reminders);
                    log::info!("Напоминание убрано: {text}");
                    reminders::cancelled(Some(&text))
                }
                None => reminders::cancelled(None),
            };
            return Some(reply);
        }

        // «напомни через 10 минут выпить воды».
        let body = reminders::requested(command)?;
        if body.trim().is_empty() {
            return Some(reminders::ask_what().to_string());
        }
        let now = reminders::now_secs();
        let Some((text, due)) = reminders::parse_when(&body, now) else {
            // Время не назвали, но просьба явная — уточняем.
            return Some(reminders::ask_when().to_string());
        };
        if text.trim().is_empty() {
            return Some(reminders::ask_what().to_string());
        }

        let id = self.reminders.add(text.clone(), due);
        let _ = self.reminder_store.save(&self.reminders);
        log::info!(
            "Напоминание #{id} на {}: {}",
            reminders::human_left(due, now),
            text
        );
        Some(reminders::confirm(&text, due, now))
    }

    /// Сказать всё, что уже наступило. Тик главного цикла.
    fn fire_reminders(&mut self) {
        let fired = self.reminders.take_fired(reminders::now_secs());
        if fired.is_empty() {
            return;
        }
        // Отмечаем сработавшими сразу: если озвучка упадёт, напоминание не
        // повторится при следующем запуске до бесконечности.
        let _ = self.reminder_store.save(&self.reminders);
        for reminder in &fired {
            log::info!("Напоминание сработало: {}", reminder.text);
        }
        self.summon();
        let phrase = reminders::announce(&fired);
        self.say(&phrase);
    }

    /// Предупредить, если сервер голоса не отвечает.
    fn check_voice_problem(&mut self) {
        if self.voice_warned {
            return;
        }
        let Some(error) = self.speaker.error() else { return };
        if !error.contains("edge-tts") {
            return;
        }
        self.voice_warned = true;
        let _ = self.bridge.events.send(UiEvent::Subtitle(error));
    }

    // ------------------------------------------------------- настройки

    /// Перечитать `config.json`, если он изменился.
    fn reload_settings(&mut self, speak: bool) {
        let path = self.base_dir.join("config.json");
        let fresh = Config::load(&path);
        let changed = {
            let current = self.cfg.read();
            current.voice != fresh.voice
                || current.voice_fx != fresh.voice_fx
                || current.voice_volume != fresh.voice_volume
                || current.chatter != fresh.chatter
                || current.user_name != fresh.user_name
                || current.sites.len() != fresh.sites.len()
                || current.apps.len() != fresh.apps.len()
                || current.hotkey_menu != fresh.hotkey_menu
        };
        if !changed {
            return;
        }
        {
            let mut guard = self.cfg.write();
            *guard = fresh.clone();
        }
        self.engine.set_lists();
        self.wake = Wake::new(&fresh.wake_words);
        persona::set_user(fresh.user_name.clone(), &fresh.user_gender);
        // Горячая клавиша могла поменяться — окно перевесит её само.
        let _ = self
            .bridge
            .events
            .send(UiEvent::Hotkey(fresh.hotkey_menu.clone()));
        self.bridge
            .window_ms
            .store((fresh.listen_window_seconds * 1000.0) as i32, Ordering::Relaxed);

        if let Some(listener) = &self.listener {
            listener.set_gain(fresh.mic_gain);
            listener.set_sensitivity(fresh.mic_sensitivity);
        }
        if let Some(warning) = &fresh.load_warning {
            log::warn!("config.json: {warning}");
            if speak {
                let _ = self.bridge.events.send(UiEvent::Subtitle(warning.clone()));
            }
        }
        log::info!("Настройки перечитаны");
    }

    fn current_mic_name(&self) -> String {
        if let Some(listener) = &self.listener {
            if let Some(name) = listener.active_name() {
                return name;
            }
        }
        self.cfg
            .read()
            .mic_device
            .clone()
            .unwrap_or_else(listener::default_input_name)
    }

    fn set_mic(&mut self, name: Option<String>) {
        let value = name.filter(|n| !n.trim().is_empty());
        let _ = self.cfg.read().set_value(
            "mic_device",
            match &value {
                Some(name) => serde_json::json!(name),
                None => serde_json::Value::Null,
            },
        );
        if let Some(listener) = &self.listener {
            listener.set_device(value.clone());
        }
        *self.bridge.current_mic.write() = value.clone();
        log::info!(
            "Выбран микрофон: {}",
            value.as_deref().unwrap_or("по умолчанию")
        );
    }

    /// Следующий микрофон по кругу.
    fn next_mic(&mut self) -> String {
        let devices = listener::input_devices();
        let current = self.current_mic_name();
        if devices.len() < 2 {
            return line("mic_only_one", &[("t", &current)]);
        }
        let index = devices
            .iter()
            .position(|name| name == &current || current.starts_with(name.as_str()) || name.starts_with(&current))
            .map(|i| i as i64)
            .unwrap_or(-1);
        let next = devices[((index + 1) as usize) % devices.len()].clone();
        self.set_mic(Some(next.clone()));
        line("mic_set", &[("t", &next)])
    }

    // ------------------------------------------------------- болталка

    fn start_chatter_thread(&self) -> std::thread::JoinHandle<()> {
        let bridge = Arc::clone(&self.bridge);
        let cfg = Arc::clone(&self.cfg);
        let alive = Arc::clone(&self.alive);
        std::thread::spawn(move || {
            let mut due = Instant::now() + next_chatter_delay(&cfg);
            while alive.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(5));
                if !cfg.read().chatter {
                    due = Instant::now() + next_chatter_delay(&cfg);
                    continue;
                }
                if Instant::now() < due {
                    continue;
                }
                let _ = bridge.jobs.send(Job::Chatter);
                due = Instant::now() + next_chatter_delay(&cfg);
            }
        })
    }

    fn chatter_line(&mut self) -> String {
        let hour = chrono::Local::now().format("%H").to_string().parse::<u32>().unwrap_or(12);
        let mut text: Option<String> = None;
        if hour < 5 && rand_percent() < 50.0 {
            text = Some(line("chatter_night", &[]));
        } else if (6..11).contains(&hour) && rand_percent() < 30.0 {
            text = Some(line("chatter_morning", &[]));
        } else if self.cfg.read().local_ai && rand_percent() < 40.0 {
            let snapshot = self.cfg.read().clone();
            text = capri_core::ai::LocalAi::new(&snapshot).chatter();
        }
        for _ in 0..5 {
            let repeated = text
                .as_ref()
                .map(|t| self.recent_chatter.iter().any(|r| r == t))
                .unwrap_or(true);
            if !repeated {
                break;
            }
            text = Some(line("chatter", &[]));
        }
        let text = text.unwrap_or_else(|| line("chatter", &[]));
        self.recent_chatter.push(text.clone());
        if self.recent_chatter.len() > 6 {
            self.recent_chatter.remove(0);
        }
        text
    }
}

fn next_chatter_delay(cfg: &Arc<RwLock<Config>>) -> Duration {
    let snapshot = cfg.read();
    let lo = snapshot.chatter_min_minutes.max(0.5);
    let hi = snapshot.chatter_max_minutes.max(lo);
    let minutes = lo + (hi - lo) * (rand_percent() / 100.0);
    Duration::from_secs_f32(minutes * 60.0)
}

/// Случайное число 0…100.
fn rand_percent() -> f32 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos % 10_000) as f32 / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chatter_delay_stays_in_bounds() {
        let cfg = Arc::new(RwLock::new(Config::default()));
        for _ in 0..20 {
            let delay = next_chatter_delay(&cfg);
            assert!(delay.as_secs_f32() >= 30.0);
            assert!(delay.as_secs_f32() <= 60.0 * 60.0);
        }
    }

    #[test]
    fn percent_is_within_range() {
        for _ in 0..20 {
            let value = rand_percent();
            assert!((0.0..100.0).contains(&value));
        }
    }

    #[test]
    fn every_ring_sector_that_names_a_panel_can_actually_open_it() {
        // Живой случай: страница перевода была в сборке, но её не было в
        // ручном списке «готовых», и кнопка в кольце отвечала «ещё в
        // разработке». Теперь список выводится из `PanelId::ALL`, и этот тест
        // стережёт, чтобы сектор не сослался на панель, которой нет.
        for sector in capri_core::menu::SECTORS.iter().chain(capri_core::menu::TOOLS.iter()) {
            if let Some(panel) = capri_core::menu::panel_of(sector) {
                assert!(
                    panel_is_ready(panel),
                    "сектор «{}» открывает панель «{panel}», а её нет в PanelId",
                    sector.label
                );
            }
        }
    }

    #[test]
    fn translate_panel_is_openable() {
        // Прямая проверка того, что человек видел своими глазами.
        assert!(panel_is_ready("translate"));
        assert!(panel_is_ready("chat"));
    }
}
