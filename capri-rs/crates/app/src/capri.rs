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
use capri_core::save::SaveManager;
use capri_core::{action, Config, Platform, Reply};
use capri_ui::{UiEvent, UiRequest};
use capri_voice::listener::{
    self, ErrorCallback, FinalCallback, ListenerOptions, TextCallback,
};
use capri_voice::{ListenerHandle, Speaker};
use parking_lot::{Mutex, RwLock};

use crate::wake::Wake;

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
    /// Надеть скин.
    SelectSkin(String),
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
            UiRequest::Quit => {
                self.interrupt.store(true, Ordering::Relaxed);
                Job::Say(String::new())
            }
            UiRequest::SetMic(name) => Job::SetMic(name),
            UiRequest::MicNext => Job::MicNext,
            UiRequest::SetVolume(value) => Job::SetVolume(value),
            UiRequest::SetVoiceFx(value) => Job::SetVoiceFx(value),
            UiRequest::SetChatter(on) => Job::SetChatter(on),
            UiRequest::SetSearch(on) => Job::SetSearch(on),
            UiRequest::SetProfile(mode, name, gender) => Job::SetProfile(mode, name, gender),
            UiRequest::Reload => Job::Reload,
            UiRequest::OpenConfig => Job::OpenConfig,
            UiRequest::SelectSkin(name) => Job::SelectSkin(name),
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
        self.greet();

        // «Болталка» живёт отдельным потоком и просто подкидывает работу.
        let chatter = self.start_chatter_thread();
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

            // Спрятаться, если давно тихо и говорить нечего.
            if !self.hidden
                && !self.speaking
                && self.last_activity.elapsed() > self.hide_after
                && Instant::now() > self.awake_until
            {
                self.hidden = true;
                let _ = self.bridge.events.send(UiEvent::Vanish);
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
            Job::SelectSkin(name) => {
                let mut data = self.save.load();
                data.equipped_skin = name.clone();
                if !data.owned_skins.iter().any(|s| s == &name) {
                    data.owned_skins.push(name.clone());
                }
                if let Err(e) = self.save.save(&data) {
                    log::warn!("Не сохранила скин: {e}");
                }
                let _ = self.bridge.events.send(UiEvent::SetSkin(capri_core::save::skin_color(&name)));
                false
            }
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
        self.speaking = true;
        self.bridge.set_speaking(true);

        // Без «перебивания» микрофон на время речи глушим.
        if !barge_in {
            if let Some(listener) = &self.listener {
                listener.set_muted(true);
            }
        }

        let interrupted = self.speaker.say(text);

        self.speaking = false;
        self.bridge.set_speaking(false);
        self.speech_end = Instant::now();
        if barge_in {
            if let Some(listener) = &self.listener {
                listener.request_reset();
            }
        } else if let Some(listener) = &self.listener {
            std::thread::sleep(Duration::from_millis(250));
            listener.set_muted(false);
        }

        self.check_voice_problem();
        interrupted
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

        let interrupted = self.say(&reply.text);

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
        if !interrupted {
            // Даём договорить продолжение: «а ещё…».
            let follow = Duration::from_secs_f32(self.cfg.read().followup_seconds.max(1.0));
            self.awake_until = Instant::now() + follow;
            let _ = self
                .bridge
                .events
                .send(UiEvent::Mode("listening".into(), follow.as_secs_f32()));
        } else {
            let _ = self.bridge.events.send(UiEvent::Mode("idle".into(), 0.0));
        }
    }

    fn engine_stop(&mut self) {
        self.engine.stop_background();
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
}
