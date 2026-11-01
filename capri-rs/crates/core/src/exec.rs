//! Запуск команд и разбор их вывода — чистая часть.
//!
//! Здесь нет ни процессов, ни Windows: только то, что можно проверить
//! тестами. Сам запуск живёт в `capri-win` (`run_stream`), потому что он
//! умеет разговаривать с системой.
//!
//! Зачем разделять: странице нужно красить строки и считать «6 passing» —
//! это логика, а не системный вызов. Так её видно в тестах.

/// Как выглядит строка вывода.
///
/// Страница красит по этому признаку, а не угадывает по словам: правила
/// разбора должны быть в одном месте.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Обычная строка.
    Plain,
    /// Успех: галочка, «passed», «ok».
    Ok,
    /// Провал: «fail», «error», красный крестик.
    Fail,
    /// Служебное: приглашение `PS`, эхо команды.
    Info,
}

impl LineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LineKind::Plain => "plain",
            LineKind::Ok => "ok",
            LineKind::Fail => "fail",
            LineKind::Info => "info",
        }
    }
}

/// Определить вид строки.
///
/// Смотрим на начало строки, а не на вхождение подстроки: иначе обычное
/// предложение со словом «ошибка» внутри красилось бы как провал.
pub fn classify(line: &str) -> LineKind {
    let t = line.trim();
    if t.is_empty() {
        return LineKind::Plain;
    }

    // Галочки и крестики, которые печатают тест-раннеры.
    if t.starts_with('✓') || t.starts_with('✔') || t.starts_with("[ok]") {
        return LineKind::Ok;
    }
    if t.starts_with('✗') || t.starts_with('✘') || t.starts_with('×') || t.starts_with("FAIL") {
        return LineKind::Fail;
    }

    let lower = t.to_lowercase();

    // Приглашение PowerShell и строка с самой командой — служебные.
    if lower.starts_with("ps ") || lower.starts_with("ps>") || lower.starts_with("> ") {
        return LineKind::Info;
    }

    // «6 passing», «2 failed», «0 failures» — итоги прогонов.
    if let Some(rest) = lower.strip_prefix("test result:") {
        return if rest.contains("failed.") && !rest.contains("0 failed") {
            LineKind::Fail
        } else {
            LineKind::Ok
        };
    }

    // Провал ищем раньше успеха: в строке «3 failed, 6 passed» важнее провал.
    // Проверяем НАЧАЛО строки: иначе обычное предложение со словом «ошибка»
    // внутри красилось бы как провал.
    for marker in [
        "error",
        "failed",
        "failure",
        "panic",
        "fatal",
        "npm err",
        "ошибка",
        "не удалось",
        "исключение",
    ] {
        if lower.starts_with(marker) {
            return LineKind::Fail;
        }
    }
    // «3 failed, 6 passed» начинается с числа, поэтому первое правило его не
    // ловит. Смотрим на число перед словом: ноль — это не провал.
    for word in ["failed", "failing", "errors"] {
        if number_before(&lower, word).is_some_and(|n| n > 0) {
            return LineKind::Fail;
        }
    }
    // Паника Rust печатает «thread '…' panicked at …».
    if lower.starts_with("thread ") && lower.contains("panicked") {
        return LineKind::Fail;
    }

    // Успех, наоборот, узнаём по вхождению: «6 passing», «Done in 3s» —
    // эти слова не встречаются в обычном тексте так же часто, как «ошибка».
    for marker in ["passed", "passing", "success", "готово", "done"] {
        if lower.contains(marker) {
            return LineKind::Ok;
        }
    }

    LineKind::Plain
}

/// Итог прогона, вытащенный из вывода.
///
/// Показывается бейджами над лентой: «6 passing ✓» вместо того, чтобы человек
/// вычитывал простыню.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub passed: Option<u32>,
    pub failed: Option<u32>,
    pub duration_ms: Option<u64>,
}

impl Summary {
    /// Есть ли что показать.
    pub fn is_empty(&self) -> bool {
        self.passed.is_none() && self.failed.is_none() && self.duration_ms.is_none()
    }

    /// Для страницы.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "passed": self.passed,
            "failed": self.failed,
            "duration_ms": self.duration_ms,
        })
    }
}

/// Собрать итог из всего вывода.
///
/// Берём последнее совпадение: раннеры печатают промежуточные итоги по
/// файлам, и интересен финальный.
pub fn summarize<I, S>(lines: I) -> Summary
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut summary = Summary::default();
    for line in lines {
        let line = line.as_ref();
        if let Some(n) = number_before(line, "passing") {
            summary.passed = Some(n);
        }
        if let Some(n) = number_before(line, "passed") {
            summary.passed = Some(n);
        }
        if let Some(n) = number_before(line, "failing") {
            summary.failed = Some(n);
        }
        if let Some(n) = number_before(line, "failed") {
            summary.failed = Some(n);
        }
        if let Some(ms) = milliseconds(line) {
            summary.duration_ms = Some(ms);
        }
    }
    summary
}

/// Число перед словом: «6 passing» → 6. Регистр не важен.
fn number_before(line: &str, word: &str) -> Option<u32> {
    let lower = line.to_lowercase();
    let at = lower.find(word)?;
    let before = lower[..at].trim_end();
    // Отступаем назад по цифрам — это и есть число.
    let digits: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// «(212ms)» или «212 мс» → 212.
fn milliseconds(line: &str) -> Option<u64> {
    let lower = line.to_lowercase();
    let bytes = lower.as_bytes();
    let mut i = 0;
    while i + 2 <= bytes.len() {
        if &lower[i..i + 2] == "ms" {
            let digits: String = lower[..i]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !digits.is_empty() {
                return digits.parse().ok();
            }
        }
        i += 1;
    }
    None
}

/// Закончился ли прогон: печатать итог есть смысл только тогда.
pub fn looks_finished(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("passing") || lower.contains("test result") || lower.contains("failing")
}

/// Человекочитаемая подпись для плашки «ЗАПУСКАЕТ».
///
/// В интерфейсе это одна строка, поэтому длинные команды подрезаем по
/// середине, а не по концу: конец обычно и есть самое важное (аргументы).
pub fn describe(command: &str) -> String {
    let text = command.trim();
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= 64 {
        return text.to_string();
    }
    let head: String = chars[..28].iter().collect();
    let tail: String = chars[chars.len() - 28..].iter().collect();
    format!("{head}…{tail}")
}

/// Ограничение вывода по объёму.
///
/// Лента в веб-вью не переживёт мегабайты: длинный `npm install` печатает
/// их запросто. Держим только первые строки и говорим об этом честно.
pub const MAX_LINES: usize = 2000;

/// Что делать со строкой сверх лимита.
pub fn over_limit(shown: usize) -> Option<String> {
    if shown == MAX_LINES {
        return Some(format!("…вывод обрезан после {MAX_LINES} строк"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_and_crosses_are_recognised() {
        assert_eq!(classify("✓ returns 20 items per page"), LineKind::Ok);
        assert_eq!(classify("✔ paginates with a cursor"), LineKind::Ok);
        assert_eq!(classify("✗ rejects a bad cursor"), LineKind::Fail);
        assert_eq!(classify("FAIL src/main.rs"), LineKind::Fail);
    }

    #[test]
    fn a_prompt_is_service_output() {
        assert_eq!(classify("PS C:\\night-city-api> npm test"), LineKind::Info);
        assert_eq!(classify("PS>"), LineKind::Info);
    }

    #[test]
    fn a_test_result_line_is_read_by_its_numbers() {
        assert_eq!(classify("test result: ok. 175 passed; 0 failed"), LineKind::Ok);
        assert_eq!(
            classify("test result: FAILED. 66 passed; 1 failed"),
            LineKind::Fail,
            "провал виден по «FAILED», а не по «passed»"
        );
    }

    #[test]
    fn failure_wins_over_success_in_the_same_line() {
        // «3 failed, 6 passed» — это провал, хоть слово «passed» и есть.
        assert_eq!(classify("3 failed, 6 passed"), LineKind::Fail);
    }

    #[test]
    fn an_error_at_the_start_is_a_failure() {
        assert_eq!(classify("error: не найден файл"), LineKind::Fail);
        assert_eq!(classify("Error: spawn failed"), LineKind::Fail);
    }

    #[test]
    fn a_plain_sentence_stays_plain() {
        // Слово «ошибка» в середине предложения — не повод красить строку.
        assert_eq!(classify("Я расскажу про ошибка позже"), LineKind::Plain);
        assert_eq!(classify("Compiling capri-core v0.2.0"), LineKind::Plain);
        assert_eq!(classify(""), LineKind::Plain);
    }

    #[test]
    fn a_summary_is_taken_from_the_last_line() {
        let lines = vec![
            "GET /gigs".to_string(),
            "2 passing (10ms)".to_string(),
            "6 passing (212ms)".to_string(),
        ];
        let summary = summarize(&lines);
        assert_eq!(summary.passed, Some(6), "берём последний итог, а не первый");
        assert_eq!(summary.duration_ms, Some(212));
    }

    #[test]
    fn a_summary_counts_failures() {
        let summary = summarize(["1 failing", "test result: FAILED. 66 passed; 1 failed"]);
        assert_eq!(summary.failed, Some(1));
        assert_eq!(summary.passed, Some(66));
    }

    #[test]
    fn an_empty_summary_knows_it_is_empty() {
        let summary = summarize(Vec::<String>::new());
        assert!(summary.is_empty());
        assert!(summary.to_json()["passed"].is_null());
    }

    #[test]
    fn a_summary_is_json_for_the_page() {
        let summary = summarize(["6 passing (212ms)"]);
        let json = summary.to_json();
        assert_eq!(json["passed"], 6);
        assert_eq!(json["duration_ms"], 212);
    }

    #[test]
    fn done_is_detected_where_there_is_something_to_report() {
        assert!(looks_finished("6 passing (212ms)"));
        assert!(looks_finished("test result: ok."));
        assert!(!looks_finished("Compiling capri-core"));
    }

    #[test]
    fn a_long_command_is_cut_in_the_middle() {
        let long = "npm run build --workspace=capri-core --features everything --verbose --yes";
        let text = describe(long);
        assert!(text.contains('…'), "длинная команда должна подрезаться: {text}");
        assert!(text.len() < long.len());
        // Короткая команда не портится.
        assert_eq!(describe("npm test"), "npm test");
        assert_eq!(describe("  calc  "), "calc");
    }

    #[test]
    fn the_limit_reports_itself_once() {
        assert!(over_limit(10).is_none());
        assert!(over_limit(MAX_LINES).is_some());
        assert!(over_limit(MAX_LINES + 5).is_none(), "сообщение печатаем один раз");
    }
}
