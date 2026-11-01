//! Распознавание текста с картинки через `Windows.Media.Ocr`.
//!
//! Это встроенный в Windows движок: работает офлайн, без DLL рядом с exe и без
//! скачивания моделей. Плата за это — языки берутся из установленных языковых
//! пакетов: если русского пакета в системе нет, русского текста он не увидит.
//! Поэтому первым делом спрашиваем у системы, что у неё есть, и говорим об
//! этом честно, а не отдаём пустой результат.

use std::sync::OnceLock;

/// Язык распознавания в виде, который понимает WinRT: `ru-RU`, `en-US`.
pub type LanguageTag = String;

/// Что умеет распознавание на этой машине.
#[derive(Debug, Clone, Default)]
pub struct Capability {
    /// Языки, для которых установлен распознаватель (`ru-RU`, `en-US`, …).
    pub languages: Vec<LanguageTag>,
    /// Максимальный размер картинки по стороне, если система его назвала.
    pub max_dimension: u32,
}

impl Capability {
    /// Есть ли хоть один язык. Без этого OCR бесполезен.
    pub fn is_usable(&self) -> bool {
        !self.languages.is_empty()
    }

    /// Есть ли русский распознаватель.
    ///
    /// Проверяем по префиксу: у системы может быть и `ru-RU`, и `ru`.
    pub fn has_russian(&self) -> bool {
        self.languages.iter().any(|tag| tag.to_ascii_lowercase().starts_with("ru"))
    }

    /// Выбрать язык для распознавания: сначала желаемый, потом русский, потом
    /// первый доступный.
    ///
    /// Порядок именно такой: если пользователь просил английский, а пакета нет,
    /// лучше распознать русским (текст на экране часто смешанный), чем никак.
    pub fn pick(&self, wanted: Option<&str>) -> Option<LanguageTag> {
        if let Some(wanted) = wanted {
            let wanted = wanted.trim().to_ascii_lowercase();
            if !wanted.is_empty() {
                if let Some(tag) = self
                    .languages
                    .iter()
                    .find(|tag| tag.to_ascii_lowercase() == wanted)
                {
                    return Some(tag.clone());
                }
                // «en» вместо «en-US»: сравниваем и по префиксу.
                if let Some(tag) = self
                    .languages
                    .iter()
                    .find(|tag| tag.to_ascii_lowercase().starts_with(&wanted))
                {
                    return Some(tag.clone());
                }
            }
        }
        if let Some(tag) = self.languages.iter().find(|tag| tag.to_ascii_lowercase().starts_with("ru")) {
            return Some(tag.clone());
        }
        self.languages.first().cloned()
    }
}

/// Одна распознанная строка и её место на картинке.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    /// Левый верхний угол и размеры в пикселях картинки.
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Результат распознавания.
#[derive(Debug, Clone, Default)]
pub struct Recognized {
    /// Строки сверху вниз.
    pub lines: Vec<Line>,
    /// Язык, которым распознали.
    pub language: LanguageTag,
}

impl Recognized {
    /// Весь текст абзацем: строки через перевод строки.
    ///
    /// Склеивать в один пробел нельзя: OCR на экране видит и заголовки, и
    /// подписи кнопок, и абзацы — без переводов строк перевод получится кашей.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.text.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Есть ли что переводить.
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|line| line.text.trim().is_empty())
    }

    /// Сколько всего символов распознали — для журнала и подсказок.
    pub fn char_count(&self) -> usize {
        self.text().chars().count()
    }
}

/// Что умеет OCR на этой машине.
///
/// Список языков спрашиваем у системы, но кэшируем: панель перевода открывается
/// часто, а перечисление языковых пакетов — не бесплатное.
pub fn capability() -> Result<Capability, String> {
    static CACHE: OnceLock<Result<Capability, String>> = OnceLock::new();
    CACHE.get_or_init(query_capability).clone()
}

#[cfg(windows)]
fn query_capability() -> Result<Capability, String> {
    use windows::Media::Ocr::OcrEngine;

    // OCR — WinRT, а WinRT требует инициализированного COM в этом потоке.
    // Обёртка сама вызовет CoUninitialize на выходе.
    let _com = crate::util::Com::init();

    let mut languages = Vec::new();
    let list = OcrEngine::AvailableRecognizerLanguages()
        .map_err(|e| format!("не смогла спросить языки OCR: {e}"))?;
    for language in list {
        // LanguageTag — BCP-47 (`ru-RU`). Пустые пропускаем: язык без тега
        // бесполезен, его не передать в TryCreateFromLanguage.
        if let Ok(tag) = language.LanguageTag() {
            let tag = tag.to_string();
            if !tag.trim().is_empty() {
                languages.push(tag);
            }
        }
    }

    let max_dimension = OcrEngine::MaxImageDimension().unwrap_or(0);
    Ok(Capability { languages, max_dimension })
}

#[cfg(not(windows))]
fn query_capability() -> Result<Capability, String> {
    Err("распознавание текста есть только в Windows".into())
}

/// Распознать текст на картинке.
///
/// `wanted` — желаемый язык (`ru-RU`, `en`), `None` — взять лучший из доступных.
pub fn recognize(image: &image::RgbaImage, wanted: Option<&str>) -> Result<Recognized, String> {
    #[cfg(windows)]
    {
        return imp::recognize(image, wanted);
    }
    #[cfg(not(windows))]
    {
        let _ = (image, wanted);
        Err("распознавание текста есть только в Windows".into())
    }
}

#[cfg(windows)]
mod imp {
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::core::HSTRING;

    use super::{Capability, Line, Recognized};

    /// Потолок стороны картинки, если система не сказала свой.
    ///
    /// `OcrEngine::MaxImageDimension` в Windows 10/11 обычно 10000, но
    /// полагаться на это нельзя: слишком большая картинка — это отказ
    /// `RecognizeAsync`, а не понятная ошибка.
    const FALLBACK_MAX_DIMENSION: u32 = 8000;

    pub fn recognize(image: &image::RgbaImage, wanted: Option<&str>) -> Result<Recognized, String> {
        let _com = crate::util::Com::init();

        let capability = super::capability()?;
        if !capability.is_usable() {
            return Err(
                "в системе нет ни одного языка распознавания. Добавь языковой пакет: \
                 Параметры → Время и язык → Язык и регион → Добавить язык (нужен компонент «Оптическое \
                 распознавание символов»)."
                    .into(),
            );
        }

        let bitmap = to_software_bitmap(image, &capability)?;

        // Если язык задан явно — распознаём только им: вызывающий знает, чего
        // хочет, и тратить время на перебор незачем.
        if let Some(tag) = wanted.filter(|tag| !tag.trim().is_empty()) {
            let tag = capability.pick(Some(tag)).ok_or_else(|| {
                "не смогла выбрать язык распознавания — в системе нет ни одного пакета OCR"
                    .to_string()
            })?;
            return recognize_with(&bitmap, &tag);
        }

        // Язык не задан — пробуем все доступные движки. Одного «лучшего» тут
        // мало: на экране часто смешаны языки (русский интерфейс с английскими
        // терминами), и русский движок видит латиницу с ошибками, а английский
        // кириллицу — вообще никак. Поэтому берём главный результат и
        // добавляем из остальных те строки, которых в нём нет.
        let mut results = Vec::new();
        let mut last_error: Option<String> = None;
        for tag in &capability.languages {
            match recognize_with(&bitmap, tag) {
                Ok(recognized) => results.push(recognized),
                Err(error) => last_error = Some(error),
            }
        }

        match merge_results(results) {
            Some(recognized) => Ok(recognized),
            // Ни один язык не сработал — показываем последнюю причину: она
            // содержит подсказку про языковой пакет.
            None => Err(last_error.unwrap_or_else(|| {
                "распознавание не сработало ни одним из доступных языков".to_string()
            })),
        }
    }

    /// Свести несколько распознаваний в одно.
    ///
    /// Разные движки читают одно и то же место по-разному, и «кто лучше
    /// вообще» — неправильный вопрос: английский движок правильно читает
    /// латиницу, но на кириллице выдаёт «npneT,», а русский — наоборот. Поэтому
    /// выбираем не движок, а строку: складываем все прочитанные строки в один
    /// список, сортируем по правдоподобности ([`line_score`]) и берём по одной
    /// на каждое место на картинке.
    pub(super) fn merge_results(results: Vec<Recognized>) -> Option<Recognized> {
        if results.is_empty() {
            return None;
        }

        // Язык результата, который прочитал больше всего «своего», — для
        // подписи и как аргумент при равных оценках строк.
        let main = results
            .iter()
            .max_by_key(|recognized| score(recognized))
            .map(|recognized| recognized.language.clone())
            .unwrap_or_default();

        // Все строки со своим языком, лучшие — первыми.
        let mut candidates: Vec<(String, Line)> = Vec::new();
        for recognized in &results {
            for line in &recognized.lines {
                if line.text.trim().is_empty() {
                    continue;
                }
                candidates.push((recognized.language.clone(), line.clone()));
            }
        }
        // `sort_by` стабилен: при равной оценке выше остаётся тот, кто пришёл
        // раньше, — а порядок результатов задаёт система.
        candidates.sort_by(|left, right| {
            line_score(&right.1, &right.0).cmp(&line_score(&left.1, &left.0))
        });

        let mut lines: Vec<Line> = Vec::new();
        for (_, line) in candidates {
            if lines.iter().any(|have| same_place(have, &line)) {
                continue;
            }
            lines.push(line);
        }

        // Порядок чтения — сверху вниз, при равной высоте слева направо.
        lines.sort_by(|left, right| left.y.cmp(&right.y).then(left.x.cmp(&right.x)));
        Some(Recognized { lines, language: main })
    }

    /// Насколько правдоподобно конкретная строка прочитана конкретным языком.
    ///
    /// Считаем буквы, попавшие в «свой» для языка алфавит: для русского —
    /// кириллица, для английского — латиница. Идея простая: если движок читал
    /// свой текст, почти все буквы окажутся в своём алфавите. Если он читал
    /// чужой — почти ни одна.
    fn line_score(line: &Line, tag: &str) -> usize {
        let cyrillic_engine = tag.to_ascii_lowercase().starts_with("ru");
        let mut score = 0usize;
        for ch in line.text.chars() {
            let is_cyrillic = ('\u{0400}'..='\u{04FF}').contains(&ch);
            let is_latin = ch.is_ascii_alphabetic();
            if (cyrillic_engine && is_cyrillic) || (!cyrillic_engine && is_latin) {
                score += 4;
            }
            // Цифры и знаки распознаются любым движком, но текст из одних цифр
            // тоже должен быть выбран — учитываем их с малым весом.
            if ch.is_ascii_digit() {
                score += 1;
            }
        }
        score
    }

    /// Оценка всего результата — по нему выбираем язык для подписи.
    fn score(recognized: &Recognized) -> usize {
        recognized
            .lines
            .iter()
            .map(|line| line_score(line, &recognized.language))
            .sum()
    }

    /// Считаются ли две строки одной и той же строкой на картинке.
    ///
    /// Сравниваем вертикальные полосы: если они перекрываются больше чем
    /// наполовину, это одна строка, прочитанная двумя движками. Координат может
    /// не быть вовсе (тогда высота нулевая) — в этом случае сравниваем текст,
    /// иначе любые две строки без координат считались бы разными и дублировались.
    fn same_place(left: &Line, right: &Line) -> bool {
        if left.height <= 0 || right.height <= 0 {
            return left.text.trim() == right.text.trim();
        }
        let top = left.y.max(right.y);
        let bottom = (left.y + left.height).min(right.y + right.height);
        let overlap = bottom - top;
        if overlap <= 0 {
            return false;
        }
        let smaller = left.height.min(right.height);
        overlap * 2 > smaller
    }

    /// Распознать картинку конкретным языком.
    fn recognize_with(
        bitmap: &SoftwareBitmap,
        tag: &str,
    ) -> Result<Recognized, String> {
        let language = Language::CreateLanguage(&HSTRING::from(tag))
            .map_err(|e| format!("не смогла создать язык «{tag}»: {e}"))?;
        // Движок не создаётся, если для языка нет пакета OCR. Сообщение должно
        // говорить, что делать, а не только «не получилось».
        let engine = OcrEngine::TryCreateFromLanguage(&language).map_err(|e| {
            format!(
                "распознаватель для «{tag}» не создался ({e}). Похоже, для этого языка не \
                 установлен компонент «Оптическое распознавание символов»: Параметры → Время и \
                 язык → Язык и регион → нужный язык → Дополнительные компоненты."
            )
        })?;

        // `join()` ждёт завершения, не заводя async-рантайм: OCR занимает
        // десятки миллисекунд, и городить ради этого executor незачем.
        let result = engine
            .RecognizeAsync(bitmap)
            .map_err(|e| format!("распознавание не запустилось: {e}"))?
            .join()
            .map_err(|e| format!("распознавание не завершилось: {e}"))?;

        let mut lines = Vec::new();
        if let Ok(collection) = result.Lines() {
            for line in collection {
                let text = line
                    .Text()
                    .map(|text| text.to_string())
                    .unwrap_or_default();
                // Рамки у строки нет — только у слов, поэтому берём общий
                // прямоугольник по словам. Он нужен, чтобы панель могла
                // подсветить, откуда взялся текст.
                let rect = line_bounds(&line);
                lines.push(Line { text, x: rect.0, y: rect.1, width: rect.2, height: rect.3 });
            }
        }

        Ok(Recognized { lines, language: tag.to_string() })
    }

    /// Общий прямоугольник строки: (`x`, `y`, ширина, высота).
    ///
    /// Если слов нет или система не дала координат — нули: текст важнее рамки.
    fn line_bounds(line: &windows::Media::Ocr::OcrLine) -> (i32, i32, i32, i32) {
        let words = match line.Words() {
            Ok(words) => words,
            Err(_) => return (0, 0, 0, 0),
        };
        let mut left = f32::MAX;
        let mut top = f32::MAX;
        let mut right = f32::MIN;
        let mut bottom = f32::MIN;
        let mut seen = false;
        for word in words {
            let Ok(rect) = word.BoundingRect() else {
                continue;
            };
            seen = true;
            left = left.min(rect.X);
            top = top.min(rect.Y);
            right = right.max(rect.X + rect.Width);
            bottom = bottom.max(rect.Y + rect.Height);
        }
        if !seen {
            return (0, 0, 0, 0);
        }
        (
            left.round() as i32,
            top.round() as i32,
            (right - left).round() as i32,
            (bottom - top).round() as i32,
        )
    }

    /// RGBA-картинка → `SoftwareBitmap` в формате, который ждёт OCR.
    ///
    /// `Windows.Media.Ocr` принимает только `Bgra8`; альфу ставим непрозрачной,
    /// потому что на прозрачном фоне распознавание видит пустоту.
    fn to_software_bitmap(
        image: &image::RgbaImage,
        capability: &Capability,
    ) -> Result<SoftwareBitmap, String> {
        let limit = if capability.max_dimension > 0 {
            capability.max_dimension
        } else {
            FALLBACK_MAX_DIMENSION
        };
        let (width, height) = (image.width(), image.height());
        let scaled = fit_into(image, width, height, limit);

        let (w, h) = (scaled.width(), scaled.height());
        if w == 0 || h == 0 {
            return Err("картинка пустая — нечего распознавать".into());
        }

        // RGBA → BGRA, альфа в 255.
        let mut pixels = scaled.into_raw();
        for px in pixels.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }

        let buffer = to_buffer(&pixels)?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            w as i32,
            h as i32,
        )
        .map_err(|e| format!("не смогла собрать SoftwareBitmap: {e}"))?;
        let _ = BitmapAlphaMode::Premultiplied;
        Ok(bitmap)
    }

    /// Пиксели → `IBuffer`.
    ///
    /// `Buffer` из WinRT не даёт доступа к своей памяти, поэтому идём штатным
    /// путём: `DataWriter` пишет байты в свой буфер, а `DetachBuffer` отдаёт
    /// его нам уже готовым — без копирования лишний раз.
    fn to_buffer(pixels: &[u8]) -> Result<windows::Storage::Streams::IBuffer, String> {
        use windows::Storage::Streams::DataWriter;

        let writer = DataWriter::new().map_err(|e| format!("не смогла создать DataWriter: {e}"))?;
        writer
            .WriteBytes(pixels)
            .map_err(|e| format!("не смогла записать пиксели: {e}"))?;
        let buffer = writer
            .DetachBuffer()
            .map_err(|e| format!("не смогла забрать буфер: {e}"))?;
        // `Close` освобождает сам DataWriter; буфер уже наш, он не пропадёт.
        let _ = writer.Close();
        Ok(buffer)
    }

    /// Вписать картинку в квадрат `limit` с сохранением пропорций.
    ///
    /// Мелкие картинки не растягиваем: увеличение не добавляет информации, зато
    /// тратит время и размывает штрихи.
    fn fit_into(
        image: &image::RgbaImage,
        width: u32,
        height: u32,
        limit: u32,
    ) -> image::RgbaImage {
        let longest = width.max(height);
        if longest == 0 || longest <= limit {
            return image.clone();
        }
        let scale = limit as f32 / longest as f32;
        let new_w = ((width as f32 * scale).round() as u32).max(1);
        let new_h = ((height as f32 * scale).round() as u32).max(1);
        image::imageops::resize(image, new_w, new_h, image::imageops::FilterType::Triangle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability_with(languages: &[&str]) -> Capability {
        Capability {
            languages: languages.iter().map(|tag| tag.to_string()).collect(),
            max_dimension: 10_000,
        }
    }

    #[test]
    fn empty_capability_is_not_usable() {
        let capability = Capability::default();
        assert!(!capability.is_usable());
        assert!(!capability.has_russian());
        assert_eq!(capability.pick(None), None);
    }

    #[test]
    fn russian_is_detected_by_tag_prefix() {
        // В разных сборках Windows тег бывает и `ru`, и `ru-RU`.
        assert!(capability_with(&["en-US", "ru"]).has_russian());
        assert!(capability_with(&["en-US", "ru-RU"]).has_russian());
        assert!(!capability_with(&["en-US"]).has_russian());
    }

    #[test]
    fn picked_language_prefers_the_requested_one() {
        let capability = capability_with(&["en-US", "ru-RU"]);
        assert_eq!(capability.pick(Some("ru-RU")).as_deref(), Some("ru-RU"));
        assert_eq!(capability.pick(Some("en-US")).as_deref(), Some("en-US"));
        // Неполный тег тоже годится: «en» → «en-US».
        assert_eq!(capability.pick(Some("en")).as_deref(), Some("en-US"));
        // Незнакомый язык — не ошибка, берём русский.
        assert_eq!(capability.pick(Some("de-DE")).as_deref(), Some("ru-RU"));
    }

    #[test]
    fn picked_language_falls_back_to_russian_then_anything() {
        let only_english = capability_with(&["en-US", "de-DE"]);
        assert_eq!(only_english.pick(None).as_deref(), Some("en-US"));

        let with_russian = capability_with(&["en-US", "ru-RU"]);
        assert_eq!(with_russian.pick(None).as_deref(), Some("ru-RU"));

        // Без запроса и без русского — первый доступный, лишь бы не пусто.
        let german = capability_with(&["de-DE"]);
        assert_eq!(german.pick(None).as_deref(), Some("de-DE"));
    }

    #[test]
    fn text_joins_lines_with_newlines_and_skips_empty() {
        let recognized = Recognized {
            language: "ru-RU".into(),
            lines: vec![
                Line { text: "  Первая строка ".into(), x: 0, y: 0, width: 10, height: 10 },
                Line { text: "   ".into(), x: 0, y: 12, width: 1, height: 10 },
                Line { text: "Вторая".into(), x: 0, y: 24, width: 10, height: 10 },
            ],
        };
        assert_eq!(recognized.text(), "Первая строка\nВторая");
        assert!(!recognized.is_empty());
        assert_eq!(recognized.char_count(), "Первая строка\nВторая".chars().count());
    }

    #[test]
    fn recognized_with_only_spaces_counts_as_empty() {
        let recognized = Recognized {
            language: "ru-RU".into(),
            lines: vec![Line { text: " ".into(), x: 0, y: 0, width: 0, height: 0 }],
        };
        assert!(recognized.is_empty());
        assert_eq!(recognized.text(), "");
        assert_eq!(recognized.char_count(), 0);
    }

    #[test]
    fn region_math_is_shared_with_the_screenshot_module() {
        // Язык-теги и регион живут в разных модулях, но панель перевода
        // передаёт регион сюда же — проверяем, что тип на месте.
        let region = crate::screenshot::Region::from_corners(10, 20, 110, 220);
        assert_eq!((region.x, region.y, region.width, region.height), (10, 20, 100, 200));
    }

    /// Строка с координатами — чтобы тесты слияния были короткими.
    fn line(text: &str, y: i32) -> Line {
        Line { text: text.into(), x: 0, y, width: 200, height: 20 }
    }

    #[test]
    fn merge_without_results_is_none() {
        assert!(imp::merge_results(Vec::new()).is_none());
    }

    #[test]
    fn merge_keeps_the_engine_that_read_its_own_alphabet() {
        // Русский движок прочитал латиницу с ошибками («ап»), английский —
        // правильно. Главным должен стать английский, хотя символов у
        // русского больше.
        let russian = Recognized { language: "ru".into(), lines: vec![line("Just ап ordinary developer", 0)] };
        let english = Recognized { language: "en-US".into(), lines: vec![line("Just an ordinary developer", 0)] };
        let merged = imp::merge_results(vec![russian, english]).expect("слияние");
        assert_eq!(merged.language, "en-US");
        assert_eq!(merged.text(), "Just an ordinary developer");
    }

    #[test]
    fn merge_adds_lines_the_main_engine_missed() {
        // Английский движок кириллицу не видит вовсе, русский её читает.
        // После слияния должны остаться обе строки.
        let english = Recognized {
            language: "en-US".into(),
            lines: vec![line("Just an ordinary developer", 0)],
        };
        let russian = Recognized {
            language: "ru".into(),
            lines: vec![
                line("Just an ordinary developer", 0),
                line("Привет, мир", 40),
            ],
        };
        let merged = imp::merge_results(vec![english, russian]).expect("слияние");
        let text = merged.text();
        assert!(text.contains("Just an ordinary developer"), "потерялась латиница: {text}");
        assert!(text.contains("Привет, мир"), "потерялась кириллица: {text}");
        // Дубликата быть не должно: одна и та же строка пришла от двух движков.
        assert_eq!(
            text.matches("Just an ordinary developer").count(),
            1,
            "строка задвоилась: {text}"
        );
    }

    #[test]
    fn merge_orders_lines_top_to_bottom() {
        let english = Recognized {
            language: "en-US".into(),
            lines: vec![line("Top line", 0)],
        };
        let russian = Recognized {
            language: "ru".into(),
            lines: vec![line("Средняя строка", 40)],
        };
        let merged = imp::merge_results(vec![english, russian]).expect("слияние");
        // Порядок задаётся координатой y, а не тем, какой движок ответил первым.
        assert_eq!(merged.lines[0].text, "Top line");
        assert_eq!(merged.lines[1].text, "Средняя строка");
    }

    #[test]
    fn merge_uses_the_engine_that_actually_saw_something() {
        // Главный по оценке ничего не увидел (пустой текст), другой увидел —
        // берём того, кто увидел.
        let empty = Recognized { language: "ru".into(), lines: Vec::new() };
        let english = Recognized { language: "en-US".into(), lines: vec![line("Hello", 0)] };
        let merged = imp::merge_results(vec![empty, english]).expect("слияние");
        assert_eq!(merged.text(), "Hello");
    }

    #[test]
    fn lines_without_coordinates_are_not_duplicated() {
        // Если система не дала координат, сравнивать приходится по тексту:
        // иначе одинаковые строки от двух движков удвоились бы.
        let first = Recognized {
            language: "en-US".into(),
            lines: vec![Line { text: "Same".into(), x: 0, y: 0, width: 0, height: 0 }],
        };
        let second = Recognized {
            language: "ru".into(),
            lines: vec![Line { text: "Same".into(), x: 0, y: 0, width: 0, height: 0 }],
        };
        let merged = imp::merge_results(vec![first, second]).expect("слияние");
        assert_eq!(merged.lines.len(), 1);
    }
}
