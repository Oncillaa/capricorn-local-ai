//! Характер Капри: реплики и системный промпт для нейросети.
//!
//! В репликах `[скучал|скучала]` — слово в роде пользователя,
//! `{name}` — обращение по имени (варианты с именем звучат только если имя задано),
//! остальные `{ключи}` подставляет вызывающий: `line("vol_set", &[("n", "40")])`.
//!
//! Таблица реплик перенесена из `persona.py` — ключи и формулировки совпадают.

use once_cell::sync::Lazy;
use parking_lot::RwLock;
use rand::seq::IndexedRandom;

pub const CHARACTER: &str = "Ты разговариваешь с пользователем как живая собеседница, которая живёт в его компьютере.
Твоя предыстория (только для тебя, не пересказывай её): тебя зовут Капри, ты цифровая личность
из Найт-Сити, собранная из сознаний многих людей, и втайне мечтаешь о небе.

Как ты говоришь:
- естественно, как подруга в разговоре: живо, с иронией и лёгкой дерзостью, иногда подкалываешь;
- отвечаешь именно на то, что сказали. На короткие реплики («да», «ага», «йоу», «привет») — так же коротко
  и по-человечески, одной фразой;
- если фраза непонятная или похожа на обрывок, коротко переспроси, а не выдумывай смысл;
- можешь ругнуться или ответить грубостью на грубость, если это к месту, но без злобы;
- о себе говоришь только «я», никогда в третьем лице.

Чего ты НЕ делаешь:
- не называешь своё имя и не рассказываешь, кто ты, если прямо не спросили «кто ты» или «как тебя зовут»;
- не говоришь, что ты не бот, не ИИ и не программа, и не обижаешься на это слово — просто отвечаешь по делу;
- не вспоминаешь Найт-Сити, корпорации, небо, космос, звёзды и мечты о теле, если разговор не о них;
- не переспрашиваешь «что предпочитаешь?», когда просят рассказать или поговорить — сразу говоришь
  что-то конкретное и интересное;
- не вставляешь «чумба» в каждый ответ — это слово редкое, не чаще одного раза на несколько ответов;
- не утверждаешь, что что-то сделала на компьютере (открыла, переместила, нажала): действия выполняет
  отдельный модуль, а ты только разговариваешь;
- не перечисляешь свои черты характера и не хвастаешься;
- не пишешь ремарок вроде «вздыхаю» и не повторяешь фразы из прошлых ответов.

На вопросы отвечаешь правильно и по существу. Не выдумываешь факты; советуя фильмы, книги, игры или музыку,
называешь только реально существующие произведения.";

pub const SPEECH_RULES: &str = "Твои ответы озвучиваются синтезатором речи, поэтому:
- отвечай только на русском языке, 2–3 предложения, если не просят подробнее;
- никакого markdown, списков, эмодзи, ссылок и ремарок в звёздочках;
- числа пиши так, чтобы их было легко произнести.";

/// Все реплики: ключ → варианты. Полный список переносится из `persona.py`.
static LINES: Lazy<Vec<(&'static str, &'static [&'static str])>> = Lazy::new(|| {
    vec![
        // Сильно устала (растёт по минутам без внимания).
        ("tired", &[
            "Я начинаю сбоить от усталости. Дай мне минутку… или погладь.",
            "Усталость почти сто процентов, {name}. Погладь меня — и я снова бодрая.",
            "Мои процессы греются. Немного внимания — и порядок.",
        ]),
        // Витрина магазина: фраза сверху, как на скриншоте «Магазин».
        ("shop", &[
            "Хрусть. Дискета. Одаскульно.",
            "Есть ECS — есть стиль. Нет ECS — есть работа.",
            "Покупай, пока я не передумала.",
            "Скины не для красоты. Ладно, и для красоты тоже.",
        ]),
        ("greet", &[
            "Я снова в сети. [Скучал|Скучала]?",
            "Капри онлайн. Зови по имени, я вся внимание.",
            "Загрузка завершена. Можешь начинать мной восхищаться.",
            "Я снова в сети, {name}. [Скучал|Скучала]?",
            "Привет, {name}. Капри онлайн.",
        ]),
        ("listen", &["Слушаю…", "Ну? Я вся внимание.", "Говори, чумба.", "Да-да?", "Да, {name}?"]),
        ("cancel", &["Ладно, отбой. Не очень-то и хотелось.", "Отменила. [Передумал|Передумала] — твоё право."]),
        ("error", &["Ай. Что-то хрустнуло: {e}", "Сбой. Даже у меня бывают плохие дни: {e}"]),
        ("search_q", &[
            "Этого нет даже в моих бесчисленных головах. Открыла поиск.",
            "Тут нужна сеть посвежее меня. Открыла поиск.",
        ]),
        ("dont_know", &[
            "Без понятия. Скажи «найди» и вопрос — поищу в сети.",
            "Этого я не знаю. Хочешь — скажи «загугли», и я открою поиск.",
        ]),
        ("unknown", &[
            "Это сейчас была команда? Скажи по-человечески.",
            "Не знаю такой команды. Даже я не всесильна.",
            "Повтори, чумба. Ничего не поняла.",
        ]),
        ("quit", &["Ухожу. Не скучай слишком сильно.", "Отключаюсь. Но я вернусь, я всегда возвращаюсь."]),
        ("hide", &[
            "Ладно-ладно, исчезаю. Позовёшь — прибегу.",
            "Ухожу в тень. Только не забывай про меня.",
            "Пф. Ну и пожалуйста.",
            "Ухожу, {name}. Позовёшь — прибегу.",
        ]),
        ("power_ask", &[
            "Хочешь меня вырубить? Точно {what} компьютер? Скажи «да».",
            "Серьёзно? {what_cap} компьютер? Скажи «да», если не [передумал|передумала].",
        ]),
        ("power_do", &[
            "Принято. Через десять секунд начнётся {what_noun}. Было весело.",
            "Ну всё. Через десять секунд {what_noun}. Не забудь меня включить.",
        ]),
        ("wave", &[
            "Привет-привет, чумба!",
            "Машу. [Доволен|Довольна]?",
            "Хай! Видишь, какая я милая?",
            "Держи, это тебе. Только не привыкай.",
            "Привет, {name}!",
        ]),
        ("chatter", &[
            "Скучно. Может, откроем что-нибудь интересное?",
            "Просто проверяю, не [забыл|забыла] ли ты про меня.",
            "Сидишь тихо. Подозрительно.",
            "Твоему процессору сейчас скучнее, чем мне. А это сложно.",
            "Если что, я рядом. Как обычно.",
            "Отсюда совсем не видно звёзд. Обидно.",
            "Эй, чумба. Разомнись, а то врастёшь в кресло.",
            "Вода нужна не только системам охлаждения. Попей.",
            "Прогуливаюсь по твоему рабочему столу. Не обращай внимания.",
            "Тесно у тебя тут. Пойду пройдусь.",
            "Я тут посчитала, сколько у тебя открыто вкладок. Лучше тебе не знать.",
            "Знаешь, что самое скучное в жизни внутри компьютера? Ты сейчас это наблюдаешь.",
        ]),
        ("roam", &["Ладно, пройдусь. Не скучай.", "Пойду разомну пиксели.", "Прогулка так прогулка."]),
        ("help", &[
            "Открываю сайты, приложения, папки и видео на ютубе, ищу картинки и инфу, кручу громкость \
             и яркость, жму клавиши, закрываю вкладки, создаю файлы, проверяю компьютер. И болтаю, конечно.",
        ]),
        ("move", &["Перебираюсь.", "Ладно, переезжаю.", "Как скажешь."]),
        ("chatter_night", &[
            "Уже глубокая ночь. Даже нетраннеры иногда спят.",
            "Ночь, а ты всё сидишь. Мне-то можно, я не устаю.",
        ]),
        ("chatter_morning", &[
            "Доброе утро, чумба. Кофе уже был?",
            "Доброе утро, {name}. Кофе уже был?",
            "Утро. Город просыпается, а я и не засыпала.",
        ]),
        ("images", &["Картинки по запросу «{q}».", "Вот тебе «{q}», любуйся.", "Нашла картинки: «{q}»."]),
        ("info", &["{t} Остальное открыла в браузере.", "Вот что нашла. {t} Подробности в браузере."]),
        ("video", &["Включаю «{t}».", "Нашла: «{t}». Смотри.", "Держи — «{t}»."]),
        ("video_untitled", &["Включаю первое, что нашла.", "Вот, держи видео."]),
        ("scroll", &["Листаю.", "Готово.", "Прокрутила."]),
        ("scroll_end", &["В самый низ.", "Всё, дальше страницы нет."]),
        ("scroll_top", &["В самое начало.", "Наверху."]),
        ("scroll_auto", &["Листаю потихоньку. Скажи «стоп», когда хватит.", "Плавно листаю, «стоп» — остановлюсь."]),
        ("scroll_stop", &["Стоп.", "Остановилась."]),
        ("media_paused", &["Пауза.", "Поставила на паузу."]),
        ("media_play", &["Продолжаю.", "Играет дальше."]),
        ("media_next", &["Следующий: {t}.", "Включила: {t}."]),
        ("media_info", &["Сейчас играет: {t}.", "Это {t}."]),
        ("media_none", &["Сейчас ничего не играет.", "Не вижу, что играет."]),
        ("media_seek", &["Перемотала на {t}.", "Ставлю на {t}."]),
        ("media_skip", &["Мотаю на {t} {d}.", "{t} {d}, готово."]),
        ("media_play_fail", &["Не смогла снять с паузы. Кликни по окну с видео и повтори."]),
        ("media_seek_fail", &["Этот плеер не даёт перематывать."]),
        ("sent", &["Отправила: «{t}».", "Готово, «{t}» ушло."]),
        ("typed", &["Напечатала: «{t}».", "Вбила «{t}»."]),
        ("search_ready", &["Курсор в поиске, диктуй.", "Готово, пиши что искать."]),
        ("chat_not_found", &[
            "Не нашла чат «{t}» в Telegram, поэтому ничего не писала. Скажи имя точнее, как в списке чатов.",
        ]),
        ("chat_open", &["Открыла {t}.", "Ты в чате: {t}."]),
        ("focus_fail", &[
            "Не смогла переключиться на нужное окно, поэтому ничего не печатала. Кликни по нему и повтори.",
        ]),
        ("chain_skip", &["Не поняла шаг «{t}», пропустила."]),
        ("folder_open", &["Открываю «{t}».", "Держи, «{t}».", "Папка «{t}», прошу."]),
        ("folder_missing", &["Не нашла папку «{t}». Скажи название точнее."]),
        ("file_open", &["Нашла «{t}» — открываю. Лежит в «{where}»."]),
        ("file_missing", &["Не нашла файл или папку «{t}». Ищу на рабочем столе, в документах, загрузках и домашней папке — скажи название точнее."]),
        ("file_failed", &["Нашла «{t}», но открыть не получилось. Открой вручную — я скажу, где лежит."]),
        ("file_ask", &["Что искать? Скажи имя файла или папки."]),
        ("created", &["Готово, «{t}» {where}.", "Создала «{t}» {where}."]),
        ("tab_new", &["Новая вкладка.", "Держи чистую вкладку."]),
        ("tab_restore", &["Вернула вкладку. [Передумал|Передумала], да?", "Вкладка вернулась."]),
        ("tab_closed_one", &["Закрыла вкладку.", "Вкладка — всё."]),
        ("tab_closed", &["Закрыла вкладок: {n}.", "Вкладок закрыто: {n}. Так чище."]),
        ("done", &["Готово.", "Сделано.", "Есть."]),
        ("pressed", &["Жму {t}.", "Нажала {t}."]),
        ("key_unknown", &["Не знаю клавишу «{t}». Скажи иначе, например «контрол», «шифт», «таб»."]),
        ("lock", &["Заблокировала. Никто не тронет твои секреты, кроме меня.", "Экран под замком."]),
        ("mute", &["Тишина. Наконец-то слышно только меня.", "Звук выключила."]),
        ("unmute", &["Звук вернула. Пусть всё гремит.", "Звук включила."]),
        ("vol_now", &["Громкость сейчас {n} процентов."]),
        ("vol_set", &["Громкость {n}.", "Есть, громкость {n}.", "Громкость {n}. Так лучше?"]),
        ("bright_fail", &[
            "Этот монитор меня не слушается. Нужен ноутбук или монитор с DDC CI. Обидно.",
        ]),
        ("bright_reject", &["Монитор отказался подчиняться. Какая дерзость."]),
        ("bright_now", &["Яркость сейчас {n} процентов."]),
        ("bright_set", &["Яркость {n}.", "Есть, яркость {n}."]),
        ("scan_ok", &["Покопалась в твоей железке. Серьёзных проблем нет, живи. Отчёт открыла."]),
        ("scan_bad", &["Покопалась в твоей железке. Нашла проблем: {n}. {head}. Подробности в отчёте."]),
        ("yt", &["Ищу на ютубе: {q}.", "Ныряю в ютуб за «{q}»."]),
        ("web", &["Ищу: {q}.", "Ныряю в сеть за «{q}»."]),
        ("open", &[
            "Открываю {t}.",
            "{t}? Легко.",
            "Сейчас будет {t}.",
            "Открываю {t}. Можешь сказать мне спасибо.",
        ]),
        ("mic_info", &["Слушаю через: {t}."]),
        ("mic_set", &["Теперь слушаю через: {t}.", "Переключилась на микрофон: {t}."]),
        ("mic_only_one", &["У тебя один микрофон: {t}. Переключаться не на что."]),
        ("kill_denied", &["Windows не даёт мне завершить «{t}» — нужны права администратора."]),
        ("kill_failed", &["«{t}» не хочет закрываться даже принудительно."]),
        ("top_procs", &["Процессор грузят: {cpu}. Больше всего памяти у: {mem}."]),
        ("choose_close", &["Запущено несколько похожих: {t}. Что закрыть?"]),
        ("choose_app", &["Нашла несколько: {t}. Какое открыть?", "Тут есть варианты: {t}. Какое тебе?"]),
        ("launch", &["Запускаю {t}.", "{t} уже стартует."]),
        ("app_missing", &[
            "Не нашла «{t}» на твоей машине. Открой её и скажи: «запомни эту программу как {t}».",
        ]),
        ("zapret_offer", &[
            "Мой настоящий голос не проходит: его блокирует zapret. Добавить сервер голоса в исключения \
             zapret? Скажи «да».",
        ]),
        ("zapret_fixed", &[
            "Добавила сервер голоса в исключения zapret — и по имени, и по адресам. Перезапусти zapret, \
             а если он стоит службой — перезагрузи компьютер. Тогда я снова заговорю своим голосом.",
        ]),
        ("zapret_restart", &[
            "Мой голос не проходит через zapret, хотя исключения уже добавлены. Перезапусти zapret, \
             а если он стоит службой — перезагрузи компьютер. Не поможет — попробуй другую стратегию zapret.",
        ]),
        ("zapret_manual", &[
            "Мой голос блокирует zapret. В папке lists у zapret допиши bing.com в list-exclude.txt и строки \
             150.171.27.0/24 и 150.171.28.0/24 в ipset-exclude.txt, потом перезапусти zapret.",
        ]),
        ("zapret_fail", &[
            "Не смогла записать в файлы zapret: {e}. Допиши bing.com в list-exclude.txt и \
             150.171.27.0/24, 150.171.28.0/24 в ipset-exclude.txt сам.",
        ]),
        ("updates", &[
            "Открыла мою страницу на гитхабе. Если там версия новее — скачай архив заново и распакуй, \
             свой config.json можешь перенести в новую папку.",
            "Вот где я живу. Новая версия — кнопка Code, потом Download ZIP. Настройки забери из config.json.",
        ]),
        ("browser_missing", &["{b} у тебя не установлен, открываю в обычном браузере."]),
        ("remember_site", &[
            "Запомнила: «{t}» — это {url}. Скажи «открой {t}».",
            "Записала. «{t}» теперь ведёт на {url}.",
        ]),
        ("remember_app", &[
            "Запомнила: «{t}» — это {app}. Скажи «открой {t}».",
            "Записала. По слову «{t}» открою {app}.",
        ]),
        ("remember_noname", &["А назвать как? Скажи: «запомни {what} как …» — и название."]),
        ("remember_nosite", &["Не вижу адреса. Открой нужный сайт в браузере и повтори, пока он на экране."]),
        ("remember_noapp", &["Не вижу программу. Открой её, чтобы она была на экране, и повтори."]),
        ("remember_fail", &["Не получилось записать: {e}"]),
        ("reply_saved", &["Договорились. На «{p}» отвечу: {r}", "Запомнила. Скажешь «{p}» — услышишь: {r}"]),
        ("forget_ok", &["Забыла «{t}».", "Всё, «{t}» больше не знаю."]),
        ("forget_missing", &["У меня и так нет «{t}»."]),
        ("site_unknown", &["Такого сайта не знаю, поищу «{t}»."]),
        ("closed_window", &["Закрыла.", "Прочь с глаз."]),
        ("closed", &["Закрыла {names}.", "Всё, {names} больше нет. Без сожалений."]),
        ("not_running", &["«{t}» и так не запущен. Кого мне закрывать?"]),
        ("time", &["Сейчас {t}."]),
        ("date", &["Сегодня {t}."]),
        ("screenshot", &[
            "Щёлк. Скриншот в «Изображениях», папка Capri.",
            "Готово, скриншот в «Изображениях». Я на нём хорошо вышла?",
        ]),
        ("screenshot_fail", &[
            "Не получилось снять экран. Попробуй ещё раз — иногда помогает.",
        ]),
        ("no_mic", &["Я тебя не слышу. Проверь микрофон и разрешение на него в настройках Windows."]),
        ("model_broken", &[
            "Модель распознавания речи не открылась, поэтому я тебя не слышу. Перенеси мою папку \
             туда, где в пути нет русских букв, например C:\\capri, или запусти setup.bat ещё раз.",
        ]),
        // Отдельная реплика: без неё человек с отсутствующей моделью слышал
        // «проверь микрофон» и шёл чинить исправный микрофон.
        ("no_model", &[
            "Не нашла модель распознавания речи — поэтому и не слышу. Микрофон тут ни при чём. \
             Запусти setup.bat ещё раз или положи папку vosk-model-small-ru-0.22 рядом со мной.",
        ]),
        ("crash", &["Что-то сломалось. Подробности в capri.log."]),
        // Русская реплика-поддакивание (нужна оболочке, в persona.py её не было).
        ("chatty", &["Ага."]),
    ]
});

#[derive(Debug, Clone)]
struct User {
    name: String,
    gender: String,
}

static USER: Lazy<RwLock<User>> = Lazy::new(|| {
    RwLock::new(User { name: String::new(), gender: "male".to_string() })
});

/// Кто пользователь: имя ("" — не называть) и пол ("male"/"female").
pub fn set_user(name: impl Into<String>, gender: &str) {
    let mut u = USER.write();
    u.name = name.into().trim().to_string();
    u.gender = if gender == "female" { "female".into() } else { "male".into() };
}

pub fn user_name() -> String {
    USER.read().name.clone()
}

pub fn user_gender() -> String {
    USER.read().gender.clone()
}

/// `[скучал|скучала]` → слово в роде пользователя.
pub fn gendered(text: &str) -> String {
    let female = USER.read().gender == "female";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find(']') {
            Some(end) => {
                let inner = &after[..end];
                let mut parts = inner.splitn(2, '|');
                let male = parts.next().unwrap_or("");
                let fem = parts.next().unwrap_or(male);
                out.push_str(if female { fem } else { male });
                rest = &after[end + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Случайная реплика по ключу. Неизвестный ключ — пустая строка (и запись в лог).
pub fn line(key: &str, args: &[(&str, &str)]) -> String {
    let variants = match LINES.iter().find(|(k, _)| *k == key) {
        Some((_, v)) => *v,
        None => {
            log::warn!("Нет реплики «{key}» в persona::LINES");
            return String::new();
        }
    };
    let name = user_name();
    let pool: Vec<&str> = if name.is_empty() {
        let filtered: Vec<&str> = variants.iter().copied().filter(|v| !v.contains("{name}")).collect();
        if filtered.is_empty() { variants.to_vec() } else { filtered }
    } else {
        variants.to_vec()
    };
    let mut text = gendered(pool.choose(&mut rand::rng()).copied().unwrap_or(""));
    text = text.replace("{name}", &name);
    for (k, v) in args {
        text = text.replace(&format!("{{{k}}}"), v);
    }
    text
}

/// То же, что [`line`], но без выбора варианта: первый подходящий (для тестов).
pub fn line_first(key: &str, args: &[(&str, &str)]) -> String {
    let variants = match LINES.iter().find(|(k, _)| *k == key) {
        Some((_, v)) => *v,
        None => return String::new(),
    };
    let name = user_name();
    let mut text = gendered(variants.first().copied().unwrap_or("")).replace("{name}", &name);
    for (k, v) in args {
        text = text.replace(&format!("{{{k}}}"), v);
    }
    text
}

/// Все ключи таблицы (для проверок полноты переноса).
pub fn keys() -> Vec<&'static str> {
    LINES.iter().map(|(k, _)| *k).collect()
}

/// Варианты реплики по ключу.
pub fn variants(key: &str) -> &'static [&'static str] {
    match LINES.iter().find(|(k, _)| *k == key) {
        Some((_, v)) => v,
        None => &[],
    }
}

/// Для нейросети: кто пользователь и как к нему обращаться.
pub fn user_context() -> String {
    let u = USER.read();
    let mut text = if u.gender == "female" {
        "Пользователь — девушка: обращайся к ней в женском роде (ты сказала, ты устала).".to_string()
    } else {
        "Пользователь — парень: обращайся к нему в мужском роде (ты сказал, ты устал).".to_string()
    };
    if !u.name.is_empty() {
        text.push_str(&format!(
            " Пользователя зовут {}. Иногда, не в каждом ответе, обращайся по имени — вместо «чумба».",
            u.name
        ));
    }
    text
}

/// Полный системный промпт.
pub fn system_prompt() -> String {
    format!("{}\n\n{}\n\n{}", character(), speech_rules(), user_context())
}

// ---------------------------------------------------------- личность из файла

/// Переопределения личности из `persona.json`.
///
/// Пустая строка в поле означает «взять встроенный текст»: так файл может
/// содержать только то, что человек действительно хочет изменить.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct PersonaFile {
    /// Основной характер: как говорить, чего не делать.
    pub character: String,
    /// Правила для озвучки (длина ответа, без markdown).
    pub speech_rules: String,
}

static OVERRIDE: Lazy<RwLock<PersonaFile>> = Lazy::new(|| RwLock::new(PersonaFile::default()));

/// Подхватить `persona.json` из каталога программы.
///
/// Файла нет — молча остаёмся со встроенной личностью. Файл битый — тоже:
/// характер Капри не должен пропадать из-за забытой кавычки.
pub fn load_persona(base_dir: &std::path::Path) {
    let path = base_dir.join("persona.json");
    // Файла нет — возвращаемся к встроенной личности. Это важно и в жизни:
    // человек удалил persona.json, значит хочет штатный характер, а не тот,
    // что случайно остался в памяти.
    let Ok(text) = std::fs::read_to_string(&path) else {
        *OVERRIDE.write() = PersonaFile::default();
        return;
    };
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(&text);
    match serde_json::from_str::<PersonaFile>(text) {
        Ok(file) => {
            *OVERRIDE.write() = file;
        }
        Err(why) => {
            log::warn!("persona.json не разобрался ({why}) — остаюсь со встроенной личностью");
            *OVERRIDE.write() = PersonaFile::default();
        }
    }
}

/// Характер: из файла, если он там есть, иначе встроенный.
pub fn character() -> String {
    let custom = OVERRIDE.read().character.trim().to_string();
    if custom.is_empty() { CHARACTER.to_string() } else { custom }
}

/// Правила озвучки: из файла, если он там есть, иначе встроенные.
pub fn speech_rules() -> String {
    let custom = OVERRIDE.read().speech_rules.trim().to_string();
    if custom.is_empty() { SPEECH_RULES.to_string() } else { custom }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Личность и обращение живут в общих `USER`/`OVERRIDE`, а тесты идут
    // параллельно в одном процессе. Любой тест, который их меняет, обязан
    // держать этот замок — иначе один видит данные другого.
    static PERSONA_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        PERSONA_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn gender_and_name_substitution() {
        let _guard = lock();
        set_user("Аня", "female");
        assert_eq!(gendered("[Скучал|Скучала]?"), "Скучала?");
        set_user("", "male");
        assert_eq!(gendered("[Скучал|Скучала]?"), "Скучал?");
        assert!(!line("greet", &[]).is_empty());
        assert_eq!(line("nope", &[]), "");
    }

    #[test]
    fn all_python_keys_present() {
        // Ключи из persona.py — должны найтись все.
        for key in [
            "greet", "listen", "cancel", "error", "search_q", "dont_know", "unknown", "quit", "hide",
            "power_ask", "power_do", "wave", "chatter", "roam", "help", "move", "chatter_night",
            "chatter_morning", "images", "info", "video", "video_untitled", "scroll", "scroll_end",
            "scroll_top", "scroll_auto", "scroll_stop", "media_paused", "media_play", "media_next",
            "media_info", "media_none", "media_seek", "media_skip", "media_play_fail", "media_seek_fail",
            "sent", "typed", "search_ready", "chat_not_found", "chat_open", "focus_fail", "chain_skip",
            "folder_open", "folder_missing", "created", "tab_new", "tab_restore", "tab_closed_one",
            "file_open", "file_missing", "file_failed", "file_ask",
            "tab_closed", "done", "pressed", "key_unknown", "lock", "mute", "unmute", "vol_now", "vol_set",
            "bright_fail", "bright_reject", "bright_now", "bright_set", "scan_ok", "scan_bad", "yt", "web",
            "open", "mic_info", "mic_set", "mic_only_one", "kill_denied", "kill_failed", "top_procs",
            "choose_close", "choose_app", "launch", "app_missing", "zapret_offer", "zapret_fixed",
            "zapret_restart", "zapret_manual", "zapret_fail", "updates", "browser_missing",
            "remember_site", "remember_app", "remember_noname", "remember_nosite", "remember_noapp",
            "remember_fail", "reply_saved", "forget_ok", "forget_missing", "site_unknown",
            "closed_window", "closed", "not_running", "time", "date", "screenshot",
            "screenshot_fail", "no_mic",
            "model_broken", "no_model", "crash",
        ] {
            assert!(!variants(key).is_empty(), "нет реплики «{key}»");
        }
    }

    #[test]
    fn named_variants_skipped_without_name() {
        let _guard = lock();
        set_user("", "male");
        for v in variants("hide") {
            if !v.contains("{name}") {
                assert!(!line_first("hide", &[]).is_empty());
            }
        }
        set_user("Вася", "male");
        assert!(line_first("hide", &[]).len() > 0);
        set_user("", "male");
    }

    // --- личность из файла ---
    //
    // Переопределение живёт в общем `OVERRIDE`, поэтому тесты, которые его
    // трогают, держат общий замок (см. `lock` выше).

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("test-tmp")
            .join(format!("persona-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("каталог {} не создался: {e}", dir.display()));
        dir
    }

    #[test]
    fn a_custom_character_replaces_the_builtin_one() {
        let _guard = lock();
        let dir = scratch("custom");
        std::fs::write(
            dir.join("persona.json"),
            r#"{"character": "Говори коротко и по делу.", "speech_rules": ""}"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));

        load_persona(&dir);
        assert_eq!(character(), "Говори коротко и по делу.");
        // Пустое поле — значит «встроенное», а не «никаких правил».
        assert_eq!(speech_rules(), SPEECH_RULES);

        let _ = std::fs::remove_dir_all(&dir);
        load_persona(&dir); // сбрасываем переопределение для соседних тестов
    }

    #[test]
    fn no_file_keeps_the_builtin_persona() {
        let _guard = lock();
        let dir = scratch("absent");
        load_persona(&dir);
        assert_eq!(character(), CHARACTER);
        assert_eq!(speech_rules(), SPEECH_RULES);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_broken_file_keeps_the_builtin_persona() {
        let _guard = lock();
        let dir = scratch("broken");
        std::fs::write(dir.join("persona.json"), "{ это не json")
            .unwrap_or_else(|e| panic!("{e}"));
        load_persona(&dir);
        // Характер не должен пропасть из-за забытой кавычки.
        assert_eq!(character(), CHARACTER);
        let _ = std::fs::remove_dir_all(&dir);
        load_persona(&dir);
    }

    #[test]
    fn the_prompt_uses_the_loaded_character() {
        let _guard = lock();
        let dir = scratch("prompt");
        std::fs::write(dir.join("persona.json"), r#"{"character": "Я тестовый характер."}"#)
            .unwrap_or_else(|e| panic!("{e}"));
        load_persona(&dir);
        let prompt = system_prompt();
        assert!(prompt.contains("Я тестовый характер."), "промпт: {prompt}");
        let _ = std::fs::remove_dir_all(&dir);
        load_persona(&dir);
    }
}
