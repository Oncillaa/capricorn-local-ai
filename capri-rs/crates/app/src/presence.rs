//! Discord Rich Presence: «что слушает Капри» в профиле Discord.
//!
//! Связь — локальный IPC Discord через именованные каналы Windows
//! (`\\.\pipe\discord-ipc-0` … `discord-ipc-9`), без сторонних крейтов:
//! протокол простой — 8 байт заголовка и JSON.
//!
//! Кадр: `op: u32` (little-endian), `длина: u32` (little-endian), тело — UTF-8
//! JSON. `op` 0 — HANDSHAKE, `op` 1 — кадр с командой (`SET_ACTIVITY`), `op` 2 —
//! CLOSE (Discord закрывает соединение). На HANDSHAKE Discord отвечает кадром
//! `evt: "READY"` — только после него связь можно считать рабочей.
//!
//! Чего здесь сознательно нет:
//! * `unwrap`/`expect`/`panic` — любая беда возвращается как `Err(String)`;
//! * фонового потока на всё время жизни — ответы на `SET_ACTIVITY` не читаем
//!   (подробности и обоснование — в [`handshake`] и [`Presence::send_activity`]);
//! * зависимости от самого Discord: если его нет, все каналы просто не
//!   откроются и [`Presence::connect`] вернёт `Err` — это нормальный сценарий,
//!   а не авария.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Map, Value};

use capri_core::platform::Track;

/// Рукопожатие: `{"v": 1, "client_id": "…"}`.
const OP_HANDSHAKE: u32 = 0;
/// Обычный кадр с командой (`SET_ACTIVITY` и ответы Discord на неё).
const OP_FRAME: u32 = 1;
/// Discord закрывает соединение (в теле обычно `{"message": "…"}`).
const OP_CLOSE: u32 = 2;

/// Discord занимает каналы `discord-ipc-0` … `discord-ipc-9`.
const PIPE_COUNT: usize = 10;

/// Сколько ждём ответ `READY` на рукопожатие, прежде чем считать канал мёртвым.
///
/// Discord отвечает за миллисекунды; запас нужен только на очень занятую
/// машину. Без ограничения зависший Discord заблокировал бы запуск Капри.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// Предел на размер ответа Discord. Кадры протокола крошечные; всё, что
/// больше, — мусор в канале, и разбирать его незачем.
const MAX_FRAME_BYTES: usize = 1 << 20;

/// Ограничение Discord на `details`/`state`/`large_text` — 128 символов.
const MAX_TEXT_CHARS: usize = 128;

/// Ключ большой иконки в dev-портале Discord.
const LARGE_IMAGE: &str = "capri";

/// Тип активности: от него зависит предлог в профиле Discord
/// («Играет в …», «Слушает …», «Смотрит …»).
///
/// Список — весь протокол Discord, а не только то, что мы шлём сегодня.
/// Сейчас используются `Listening` (музыка) и `Playing` (по умолчанию);
/// `Watching` и `Competing` пригодятся, когда появятся видео и шахматы.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ActivityType {
    /// `0` — играет.
    #[default]
    Playing,
    /// `2` — слушает музыку.
    Listening,
    /// `3` — смотрит видео.
    Watching,
    /// `5` — соревнуется.
    Competing,
}

impl ActivityType {
    /// Число, которое ждёт Discord в поле `type`.
    #[allow(dead_code)]
    pub fn code(self) -> u8 {
        match self {
            ActivityType::Playing => 0,
            ActivityType::Listening => 2,
            ActivityType::Watching => 3,
            ActivityType::Competing => 5,
        }
    }
}

/// Что показывать в профиле Discord.
#[derive(Clone, Debug)]
pub struct Activity {
    /// Верхняя строка, например «Falling In Reverse».
    pub details: String,
    /// Нижняя строка, например «Joseph · Spotify».
    pub state: String,
    /// Подсказка большой иконки.
    pub large_text: String,
    /// Unix-время начала прослушивания.
    pub timestamp_start: Option<i64>,
    /// «Слушает» / «Играет» / «Смотрит».
    pub kind: ActivityType,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            details: String::new(),
            state: String::new(),
            large_text: String::new(),
            timestamp_start: None,
            kind: ActivityType::Playing,
        }
    }
}

impl Activity {
    /// Музыка: `details` — трек, `state` — исполнитель и плеер,
    /// `timestamp_start` — начало трека, чтобы у Discord шла полоса прогресса.
    ///
    /// `position` и `duration` — секунды; если плеер их не отдаёт, полосы
    /// не будет, но строки всё равно появятся.
    pub fn music(
        title: &str,
        artist: &str,
        app: &str,
        position: Option<f64>,
        now_unix: i64,
    ) -> Self {
        let details = title.trim().to_string();
        let state = match (artist.trim(), app.trim()) {
            ("", "") => String::new(),
            ("", app) => app.to_string(),
            (artist, "") => artist.to_string(),
            (artist, app) => format!("{artist} · {app}"),
        };

        // Discord рисует полосу от `start` до «сейчас». Значит начало трека —
        // это «сейчас минус уже проигранное».
        let timestamp_start = position
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| now_unix - value.round() as i64);

        Self {
            details,
            state,
            large_text: "Капри".to_string(),
            timestamp_start,
            kind: ActivityType::Listening,
        }
    }

    /// Игра: `details` — название, `state` — уровень Капри.
    ///
    /// Пока не используется: игр в проекте ещё нет (шахматы — фаза 5).
    #[allow(dead_code)]
    pub fn game(name: &str, level: u32, now_unix: i64) -> Self {
        Self {
            details: name.trim().to_string(),
            state: format!("Уровень {level}"),
            large_text: "Капри".to_string(),
            timestamp_start: Some(now_unix),
            kind: ActivityType::Playing,
        }
    }

    /// Насколько две активности отличаются по смыслу.
    ///
    /// Сравниваем только то, что видит человек: тип, строки и начало трека.
    /// Секунды прогресса намеренно не учитываем — иначе состояние
    /// «изменилось» на каждой проверке и мы завалили бы Discord апдейтами.
    pub fn same_as(&self, other: &Activity) -> bool {
        self.kind == other.kind
            && self.details == other.details
            && self.state == other.state
            && self.large_text == other.large_text
            && self.timestamp_start == other.timestamp_start
    }
}

/// Живое соединение с Discord IPC.
///
/// Объект переподключается сам: если канал порвался, следующая
/// [`Presence::set_activity`] сначала откроет его заново по сохранённому
/// `client_id`.
pub struct Presence {
    /// Application ID — нужен, чтобы поднять соединение после обрыва.
    client_id: String,
    /// Открытый канал; `None` — связи нет, следующий вызов попробует открыть.
    pipe: Option<File>,
    /// Номер рабочего канала (для логов и сообщений об ошибках).
    pipe_index: usize,
    /// Счётчик запросов — основа для `nonce`.
    sequence: u64,
}

impl Presence {
    /// Подключиться. `client_id` — Application ID из dev-портала Discord.
    pub fn connect(client_id: &str) -> Result<Self, String> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err("пустой client_id: нужен Application ID из dev-портала Discord".to_string());
        }

        let (pipe, pipe_index) = open_pipe(client_id)?;
        log::info!("Discord Rich Presence подключён через discord-ipc-{pipe_index}");
        Ok(Self {
            client_id: client_id.to_string(),
            pipe: Some(pipe),
            pipe_index,
            sequence: 0,
        })
    }

    /// Отправить активность (SET_ACTIVITY). Пустые details/state — очистить статус.
    pub fn set_activity(&mut self, activity: &Activity) -> Result<(), String> {
        let value = activity_json(activity);
        let cleared = value.is_null();
        self.send_activity(value)?;

        if cleared {
            log::debug!("Rich Presence очищен: details и state пустые");
        } else {
            log::debug!(
                "Rich Presence обновлён: «{}» / «{}»",
                activity.details,
                activity.state
            );
        }
        Ok(())
    }

    /// Снять статус (`"activity": null`).
    pub fn clear(&mut self) -> Result<(), String> {
        if self.pipe.is_none() {
            // Связи нет — и показывать уже нечего: Discord сбросил статус,
            // когда канал закрылся. Переподключаться ради пустоты незачем.
            log::debug!("Rich Presence не подключён — очищать нечего");
            return Ok(());
        }
        self.send_activity(Value::Null)?;
        log::debug!("Rich Presence очищен");
        Ok(())
    }

    /// Отправить `SET_ACTIVITY` с готовым `activity` (или `null`).
    ///
    /// Ответ Discord на команду не читаем — обоснование в [`handshake`] и в
    /// [`Self::reconnect_if_needed`]: чтобы не читать канал вечно, ошибки
    /// канала ловятся на следующей записи, после чего канал переоткрывается.
    fn send_activity(&mut self, activity: Value) -> Result<(), String> {
        self.reconnect_if_needed()?;

        let payload = set_activity_payload(activity, &self.make_nonce());
        let frame = encode_frame(OP_FRAME, &payload)?;

        let written = match self.pipe.as_mut() {
            Some(pipe) => pipe.write_all(&frame),
            None => return Err("нет соединения с Discord".to_string()),
        };

        match written {
            Ok(()) => Ok(()),
            Err(err) => {
                // Канал сломался (Discord закрылся или перезапустился).
                // Рвём его здесь, а следующий вызов откроет заново.
                self.pipe = None;
                Err(format!("не удалось отправить активность в Discord: {err}"))
            }
        }
    }

    /// Открыть канал заново, если предыдущий закрылся или ещё не открыт.
    fn reconnect_if_needed(&mut self) -> Result<(), String> {
        if self.pipe.is_some() {
            return Ok(());
        }

        let (pipe, pipe_index) = open_pipe(&self.client_id)?;
        self.pipe = Some(pipe);
        self.pipe_index = pipe_index;
        log::debug!("Rich Presence: переподключились через discord-ipc-{pipe_index}");
        Ok(())
    }

    /// Следующий `nonce` — Discord требует уникальный идентификатор запроса.
    fn make_nonce(&mut self) -> String {
        self.sequence = self.sequence.wrapping_add(1);
        build_nonce(std::process::id(), self.sequence)
    }
}

/// Открыть первый рабочий канал Discord и пройти рукопожатие.
fn open_pipe(client_id: &str) -> Result<(File, usize), String> {
    let mut last_error = "каналы discord-ipc не найдены".to_string();

    for index in 0..PIPE_COUNT {
        let path = format!(r"\\.\pipe\discord-ipc-{index}");
        match OpenOptions::new().read(true).write(true).open(&path) {
            Ok(mut pipe) => {
                // Канал открылся — это Discord. Если рукопожатие не сложилось,
                // соседние индексы уже не помогут: отвечает тот же Discord.
                return match handshake(&mut pipe, client_id) {
                    Ok(()) => Ok((pipe, index)),
                    Err(err) => Err(format!("{path}: {err}")),
                };
            }
            Err(err) => last_error = format!("{path}: {err}"),
        }
    }

    Err(format!(
        "Discord не запущен или IPC недоступен ({last_error})"
    ))
}

/// Рукопожатие: отправить HANDSHAKE и дождаться `READY`.
///
/// Про таймауты. `std::fs::File` умеет только блокирующее чтение: у него нет
/// `set_read_timeout` (тот есть лишь у сокетов), а для именованных каналов
/// Windows std не даёт ни таймаутов, ни небезопасного «неблокирующего» чтения.
/// Поэтому ответ на рукопожатие читаем в отдельном потоке и ждём его через
/// `recv_timeout`: так `connect` гарантированно не зависнет навсегда. Поток
/// живёт ровно до первого кадра, после чего завершается сам.
///
/// Ответы на `SET_ACTIVITY` не читаем вовсе. Discord присылает их на каждую
/// команду, но полезного в них нет (ошибку команды всё равно не исправить
/// повтором), а блокирующее чтение вернуло бы ровно ту проблему, от которой мы
/// уходим. Оборванный канал обнаружится на следующей записи: `write_all`
/// вернёт ошибку, и [`Presence::send_activity`] сбросит канал — следующий вызов
/// переподключится. Это и есть обещанное «ошибки ловим при следующей отправке».
fn handshake(pipe: &mut File, client_id: &str) -> Result<(), String> {
    let payload = json!({ "v": 1, "client_id": client_id });
    let frame = encode_frame(OP_HANDSHAKE, &payload)?;
    pipe.write_all(&frame)
        .map_err(|err| format!("не удалось отправить рукопожатие: {err}"))?;

    // Читаем копией дескриптора, чтобы после рукопожатия канал остался у нас.
    let mut reader = pipe
        .try_clone()
        .map_err(|err| format!("не удалось подготовить чтение ответа Discord: {err}"))?;

    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("capri-discord-ipc".to_string())
        .spawn(move || {
            // Если ожидающий уже ушёл по таймауту, отправлять некому — это нормально.
            let _ = sender.send(read_frame(&mut reader));
        })
        .map_err(|err| format!("не удалось запустить поток чтения ответа Discord: {err}"))?;

    match receiver.recv_timeout(HANDSHAKE_TIMEOUT) {
        Ok(Ok((op, payload))) => {
            if is_ready(op, &payload) {
                Ok(())
            } else if op == OP_CLOSE {
                let reason = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("причина не указана");
                Err(format!("Discord закрыл соединение: {reason}"))
            } else {
                Err(format!("Discord ответил не READY: {payload}"))
            }
        }
        Ok(Err(err)) => Err(err),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
            "Discord не ответил на рукопожатие за {} с",
            HANDSHAKE_TIMEOUT.as_secs()
        )),
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("поток чтения ответа Discord завершился без ответа".to_string())
        }
    }
}

/// Собрать кадр: `op`, длина тела, JSON.
fn encode_frame(op: u32, payload: &Value) -> Result<Vec<u8>, String> {
    let body = serde_json::to_vec(payload)
        .map_err(|err| format!("не удалось сериализовать кадр Discord: {err}"))?;
    let length = u32::try_from(body.len())
        .map_err(|_| format!("кадр Discord слишком велик: {} байт", body.len()))?;

    let mut frame = Vec::with_capacity(8 + body.len());
    frame.extend_from_slice(&op.to_le_bytes());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

/// Прочитать один кадр: ровно 8 байт заголовка и тело по длине.
fn read_frame(reader: &mut impl Read) -> Result<(u32, Value), String> {
    let mut header = [0u8; 8];
    reader
        .read_exact(&mut header)
        .map_err(|err| format!("Discord не прислал заголовок кадра: {err}"))?;

    let op = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let length = usize::try_from(length)
        .map_err(|_| "некорректная длина кадра Discord".to_string())?;

    if length > MAX_FRAME_BYTES {
        return Err(format!("Discord прислал слишком длинный кадр: {length} байт"));
    }

    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .map_err(|err| format!("Discord не прислал тело кадра: {err}"))?;

    let payload = serde_json::from_slice(&body)
        .map_err(|err| format!("кадр Discord не разобрался как JSON: {err}"))?;
    Ok((op, payload))
}

/// Ответ на рукопожатие: `op` 1 и `evt: "READY"`.
fn is_ready(op: u32, payload: &Value) -> bool {
    op == OP_FRAME && payload.get("evt").and_then(Value::as_str) == Some("READY")
}

/// Тело `SET_ACTIVITY`.
fn set_activity_payload(activity: Value, nonce: &str) -> Value {
    json!({
        "cmd": "SET_ACTIVITY",
        "args": {
            "pid": std::process::id(),
            "activity": activity,
        },
        "nonce": nonce,
    })
}

/// Активность в том виде, в каком её ждёт Discord.
///
/// Пустые `details` и `state` — это «статуса нет», поэтому `null`. Если пуста
/// только одна строка, шлём лишь заполненную: Discord не любит пустых строк.
fn activity_json(activity: &Activity) -> Value {
    let details = clip(activity.details.trim());
    let state = clip(activity.state.trim());

    if details.is_empty() && state.is_empty() {
        return Value::Null;
    }

    let mut map = Map::new();
    // Тип активности: без него Discord покажет «Играет», даже если это музыка.
    map.insert(
        "type".to_string(),
        Value::from(u64::from(activity.kind.code())),
    );
    if !details.is_empty() {
        map.insert("details".to_string(), Value::String(details));
    }
    if !state.is_empty() {
        map.insert("state".to_string(), Value::String(state));
    }

    let mut assets = Map::new();
    assets.insert(
        "large_image".to_string(),
        Value::String(LARGE_IMAGE.to_string()),
    );
    let large_text = clip(activity.large_text.trim());
    if !large_text.is_empty() {
        // Пустую подсказку не шлём: Discord показал бы пустую плашку.
        assets.insert("large_text".to_string(), Value::String(large_text));
    }
    map.insert("assets".to_string(), Value::Object(assets));

    if let Some(start) = activity.timestamp_start {
        let mut timestamps = Map::new();
        timestamps.insert("start".to_string(), Value::from(start));
        map.insert("timestamps".to_string(), Value::Object(timestamps));
    }

    Value::Object(map)
}

/// Обрезать строку до лимита Discord: длинное название трека иначе отклонит всю
/// активность. Считаем именно символы, а не байты, — иначе кириллица резалась
/// бы посреди буквы.
fn clip(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

/// Похожий на UUID `nonce`: 8-4-4-4-12 шестнадцатеричных разрядов.
///
/// Отдельный крейт `uuid` ради этого тянуть не стали: Discord'у достаточно,
/// чтобы строка была уникальной и непустой. Уникальность дают время, pid и
/// счётчик запросов.
fn build_nonce(pid: u32, sequence: u64) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let mixed = nanos ^ (u128::from(pid) << 64) ^ (u128::from(sequence) << 32);

    format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        (mixed >> 96) as u32,
        (mixed >> 80) as u16,
        (mixed >> 64) as u16,
        (mixed >> 48) as u16,
        (mixed & 0xffff_ffff_ffff) as u64,
    )
}

/// Сколько секунд ждать между проверками, не изменилось ли что-нибудь.
///
/// Полтора десятка секунд — компромисс: реакция на смену трека заметна глазом,
/// а Discord не получает лишних пакетов.
pub const TICK: Duration = Duration::from_secs(3);

/// Минимальный интервал между двумя отправками в Discord.
///
/// Жёсткий лимит Discord — 5 обновлений за 20 секунд, то есть в среднем раз
/// в 4 секунды. Берём 5: запас на то, что Discord считает окно по-своему.
pub const MIN_GAP: Duration = Duration::from_secs(5);

/// Запустить трансляцию в Discord в отдельном потоке.
///
/// `source` зовут перед каждой отправкой: она возвращает, что показывать
/// (`None` — статус снять). Так модуль не знает ни про медиаплеер, ни про
/// ядро Капри, и его можно проверить без Discord.
///
/// Поток живёт до конца программы; ошибки не пробрасываются наружу — они
/// пишутся в лог, потому что «Discord закрыт» это не авария.
pub fn spawn<F>(client_id: String, source: F)
where
    F: Fn() -> Option<Activity> + Send + 'static,
{
    let spawned = std::thread::Builder::new()
        .name("capri-discord-presence".to_string())
        .spawn(move || run_loop(client_id, source));

    if let Err(err) = spawned {
        log::warn!("Discord Rich Presence: поток не запустился: {err}");
    }
}

/// Цикл трансляции: опрос источника, склейка изменений, троттлинг.
fn run_loop<F>(client_id: String, source: F)
where
    F: Fn() -> Option<Activity>,
{
    let mut presence: Option<Presence> = None;
    let mut shown: Option<Activity> = None;
    let mut last_sent: Option<std::time::Instant> = None;
    // Первое подключение не ждём: Discord обычно уже запущен.
    let mut retry_at = std::time::Instant::now();

    loop {
        std::thread::sleep(TICK);

        let wanted = source();

        // Ничего не поменялось — не трогаем Discord вообще.
        let changed = match (&wanted, &shown) {
            (None, None) => false,
            (Some(new), Some(old)) => !new.same_as(old),
            _ => true,
        };
        if !changed {
            continue;
        }

        // Троттлинг: не чаще, чем раз в MIN_GAP. Изменение не теряется —
        // оно просто уедет следующим кругом, `wanted` читается заново.
        if let Some(sent) = last_sent {
            if sent.elapsed() < MIN_GAP {
                continue;
            }
        }

        // Подключаемся лениво и не чаще, чем раз в полминуты: если Discord
        // закрыт, незачем долбить именованные каналы каждые три секунды.
        if presence.is_none() {
            if std::time::Instant::now() < retry_at {
                continue;
            }
            match Presence::connect(&client_id) {
                Ok(connected) => presence = Some(connected),
                Err(err) => {
                    log::debug!("Discord Rich Presence недоступен: {err}");
                    retry_at = std::time::Instant::now() + Duration::from_secs(30);
                    continue;
                }
            }
        }

        let Some(connection) = presence.as_mut() else {
            continue;
        };

        let result = match &wanted {
            Some(activity) => connection.set_activity(activity),
            None => connection.clear(),
        };

        match result {
            Ok(()) => {
                last_sent = Some(std::time::Instant::now());
                shown = wanted;
            }
            Err(err) => {
                log::debug!("Rich Presence: отправка не удалась: {err}");
                // Канал порвался — переподключимся на следующем круге.
                presence = None;
                retry_at = std::time::Instant::now() + Duration::from_secs(5);
            }
        }
    }
}

/// Активность из того, что сейчас играет.
///
/// На паузе оставляем трек в статусе (человек и правда его «слушает», просто
/// остановил), но без `timestamp_start` — иначе Discord рисовал бы полосу,
/// которая едет без звука.
pub fn activity_for(track: &Track, level: u32, now_unix: i64) -> Activity {
    let _ = level;
    if track.title.trim().is_empty() {
        return Activity::default();
    }
    let position = if track.playing { track.position } else { None };
    Activity::music(&track.title, &track.artist, &track.app, position, now_unix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    fn sample_activity() -> Activity {
        Activity {
            details: "Слушает: Falling In Reverse".to_string(),
            state: "Joseph · Spotify".to_string(),
            large_text: "Капри".to_string(),
            timestamp_start: Some(1_700_000_000),
            kind: ActivityType::Listening,
        }
    }

    #[test]
    fn frame_starts_with_op_and_length() {
        let frame = encode_frame(OP_FRAME, &json!({ "cmd": "SET_ACTIVITY" })).expect("кадр");

        // op little-endian.
        assert_eq!(&frame[0..4], &1u32.to_le_bytes());
        // Длина — ровно размер тела.
        let length = u32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
        assert_eq!(length as usize, frame.len() - 8);

        // Тело — тот же JSON, что положили.
        let payload: Value = serde_json::from_slice(&frame[8..]).expect("JSON в кадре");
        assert_eq!(payload["cmd"], "SET_ACTIVITY");
    }

    #[test]
    fn handshake_frame_is_op_zero() {
        let payload = json!({ "v": 1, "client_id": "1234567890" });
        let frame = encode_frame(OP_HANDSHAKE, &payload).expect("кадр");
        assert_eq!(&frame[0..4], &0u32.to_le_bytes());
        assert_eq!(payload["v"], 1);
        assert_eq!(payload["client_id"], "1234567890");
    }

    #[test]
    fn ready_reply_is_recognised() {
        // Собираем ответ Discord руками: op 1 + READY.
        let body = serde_json::to_vec(&json!({
            "cmd": "DISPATCH",
            "evt": "READY",
            "data": { "v": 1, "config": { "cdn_host": "cdn.discordapp.com" } },
            "nonce": null,
        }))
        .expect("тело READY");

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&OP_FRAME.to_le_bytes());
        bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&body);

        let (op, payload) = read_frame(&mut Cursor::new(bytes)).expect("кадр READY");
        assert!(is_ready(op, &payload));
        assert_eq!(payload["evt"], "READY");
    }

    #[test]
    fn not_ready_is_not_ready() {
        let error = json!({ "cmd": "DISPATCH", "evt": "ERROR", "data": { "message": "нет" } });
        assert!(!is_ready(OP_FRAME, &error));
        assert!(!is_ready(OP_HANDSHAKE, &json!({ "evt": "READY" })));
        assert!(!is_ready(OP_CLOSE, &json!({ "message": "закрыто" })));
    }

    #[test]
    fn refuses_oversized_frame() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&OP_FRAME.to_le_bytes());
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        // Тело не нужно: отказ должен случиться ещё на заголовке.
        assert!(read_frame(&mut Cursor::new(bytes)).is_err());
    }

    #[test]
    fn activity_has_lines_assets_and_time() {
        let value = activity_json(&sample_activity());

        assert_eq!(value["details"], "Слушает: Falling In Reverse");
        assert_eq!(value["state"], "Joseph · Spotify");
        assert_eq!(value["assets"]["large_image"], "capri");
        assert_eq!(value["assets"]["large_text"], "Капри");
        assert_eq!(value["timestamps"]["start"], 1_700_000_000_i64);
    }

    #[test]
    fn empty_activity_becomes_null() {
        // Совсем пустая активность — «снять статус».
        assert!(activity_json(&Activity::default()).is_null());

        // Пустые строки из одних пробелов — тоже пусто.
        let spaces = Activity {
            details: "   ".to_string(),
            state: "\t".to_string(),
            ..Activity::default()
        };
        assert!(activity_json(&spaces).is_null());
    }

    #[test]
    fn keeps_single_filled_line() {
        let only_state = Activity {
            state: "Joseph · Spotify".to_string(),
            ..Activity::default()
        };
        let value = activity_json(&only_state);

        assert!(value.is_object());
        assert!(value.get("details").is_none());
        assert_eq!(value["state"], "Joseph · Spotify");
    }

    #[test]
    fn timestamps_only_when_start_known() {
        let no_time = Activity {
            details: "Слушает: что-то".to_string(),
            ..Activity::default()
        };
        assert!(activity_json(&no_time).get("timestamps").is_none());
    }

    #[test]
    fn long_text_is_clipped() {
        let long = Activity {
            details: "ж".repeat(MAX_TEXT_CHARS * 3),
            ..Activity::default()
        };
        let value = activity_json(&long);
        let details = value["details"].as_str().unwrap_or_default();
        assert_eq!(details.chars().count(), MAX_TEXT_CHARS);
        // Кириллица остаётся целой — режем по символам.
        assert!(details.chars().all(|c| c == 'ж'));
    }

    #[test]
    fn set_activity_payload_carries_pid_and_nonce() {
        let payload = set_activity_payload(Value::Null, "00000000-0000-0000-0000-000000000000");
        assert_eq!(payload["cmd"], "SET_ACTIVITY");
        assert_eq!(payload["nonce"], "00000000-0000-0000-0000-000000000000");
        assert_eq!(payload["args"]["pid"], std::process::id());
        assert!(payload["args"]["activity"].is_null());
    }

    #[test]
    fn nonce_looks_like_uuid() {
        let nonce = build_nonce(4242, 7);
        assert_eq!(nonce.len(), 36);

        let groups: Vec<&str> = nonce.split('-').collect();
        let sizes: Vec<usize> = groups.iter().map(|group| group.len()).collect();
        assert_eq!(sizes, vec![8, 4, 4, 4, 12]);
        assert!(nonce
            .chars()
            .all(|c| c == '-' || c.is_ascii_hexdigit()));
    }

    #[test]
    fn activity_type_matches_discord_codes() {
        // Числа из официального IPC-протокола Discord.
        assert_eq!(ActivityType::Playing.code(), 0);
        assert_eq!(ActivityType::Listening.code(), 2);
        assert_eq!(ActivityType::Watching.code(), 3);
        assert_eq!(ActivityType::Competing.code(), 5);

        // Музыка должна уходить именно как «слушает», а не «играет».
        let music = Activity::music("Joseph", "Falling In Reverse", "Spotify", None, 1);
        assert_eq!(activity_json(&music)["type"], 2);
    }

    #[test]
    fn music_builds_lines_and_start_time() {
        // Играет 30 секунд назад начатый трек.
        let activity = Activity::music(
            "  Joseph  ",
            "Falling In Reverse",
            "Spotify",
            Some(30.0),
            1_700_000_000,
        );
        let value = activity_json(&activity);

        assert_eq!(value["details"], "Joseph", "пробелы обрезаны");
        assert_eq!(value["state"], "Falling In Reverse · Spotify");
        assert_eq!(value["timestamps"]["start"], 1_700_000_000 - 30);
        assert_eq!(value["assets"]["large_image"], "capri");
    }

    #[test]
    fn music_without_position_has_no_bar() {
        let activity = Activity::music("Joseph", "FIR", "Spotify", None, 1_700_000_000);
        assert!(activity_json(&activity).get("timestamps").is_none());
        // Ноль секунд — законная позиция, а не «нет данных».
        let at_start = Activity::music("Joseph", "FIR", "Spotify", Some(0.0), 1_700_000_000);
        assert_eq!(activity_json(&at_start)["timestamps"]["start"], 1_700_000_000);
    }

    #[test]
    fn music_state_survives_missing_parts() {
        let only_title = Activity::music("Joseph", "", "", None, 0);
        let value = activity_json(&only_title);
        assert_eq!(value["details"], "Joseph");
        assert!(value.get("state").is_none(), "пустую строку не шлём");

        let only_artist = Activity::music("", "FIR", "Spotify", None, 0);
        let value = activity_json(&only_artist);
        assert_eq!(value["state"], "FIR · Spotify");
        assert!(value.get("details").is_none());
    }

    #[test]
    fn same_as_ignores_progress_seconds() {
        // Один и тот же трек в двух проверках: позиция разная, начало совпало —
        // значит для Discord ничего не изменилось и слать нечего.
        let first = Activity::music("Joseph", "FIR", "Spotify", Some(10.0), 1_000);
        let second = Activity::music("Joseph", "FIR", "Spotify", Some(13.0), 1_003);
        assert!(first.same_as(&second));

        // Другой трек — изменилось.
        let other = Activity::music("Believer", "FIR", "Spotify", Some(10.0), 1_000);
        assert!(!first.same_as(&other));

        // Тип активности тоже важен.
        let as_game = Activity::game("Chess", 4, 1_000);
        assert!(!first.same_as(&as_game));
    }

    #[test]
    fn activity_for_reads_the_track() {
        let track = Track {
            title: "Wakacchaimai ne".into(),
            artist: "renaseri".into(),
            app: "Яндекс Браузер".into(),
            duration: Some(200.0),
            position: Some(12.0),
            playing: true,
        };
        let activity = activity_for(&track, 4, 1_700_000_000);
        assert_eq!(activity.details, "Wakacchaimai ne");
        assert_eq!(activity.state, "renaseri · Яндекс Браузер");
        assert_eq!(activity.kind, ActivityType::Listening);
        assert_eq!(activity.timestamp_start, Some(1_700_000_000 - 12));

        // Пустое название — статуса нет вовсе.
        let empty = Track { title: "  ".into(), ..Track::default() };
        assert!(activity_json(&activity_for(&empty, 4, 0)).is_null());
    }

    #[test]
    fn paused_track_keeps_title_but_drops_progress() {
        let track = Track {
            title: "Wakacchaimai ne".into(),
            artist: "renaseri".into(),
            app: "Яндекс Браузер".into(),
            duration: Some(200.0),
            position: Some(12.0),
            playing: false,
        };
        let activity = activity_for(&track, 4, 1_700_000_000);
        // Строки остались — человек видит, что слушает.
        assert_eq!(activity.details, "Wakacchaimai ne");
        // А полосы прогресса нет: трек на паузе.
        assert_eq!(activity.timestamp_start, None);
        assert!(activity_json(&activity).get("timestamps").is_none());
    }
}
