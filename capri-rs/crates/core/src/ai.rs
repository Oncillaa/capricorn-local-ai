//! Разговорные нейросети: облачные провайдеры и локальная Ollama.
//!
//! Ключ у нас один (`Config::anthropic_api_key` — имя историческое). Чей он,
//! определяем по префиксу: так человеку не нужно выбирать провайдера в
//! настройках и разбираться, «чей ключ» ему выдал сервис.

use std::time::Duration;

use serde_json::{json, Value};

use crate::config::Config;
use crate::persona;

/// Кто отвечает на вопрос.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// Anthropic Messages API (`sk-ant-…`).
    Anthropic,
    /// OpenAI-совместимый (`sk-…`, `gsk_…`, `nvapi-…`).
    OpenAi,
}

impl Provider {
    /// Определить провайдера по виду ключа.
    ///
    /// Префиксы — единственное, что можно проверить без сети, не тратя
    /// запрос впустую. Неизвестный ключ считаем OpenAI-совместимым: этот
    /// формат (`/chat/completions` + Bearer) поддерживают почти все.
    pub fn detect(key: &str) -> Option<Self> {
        let key = key.trim();
        if key.is_empty() {
            return None;
        }
        if key.starts_with("sk-ant-") {
            return Some(Provider::Anthropic);
        }
        // gsk_ — Groq, nvapi- — NVIDIA NIM, sk- — OpenAI и совместимые.
        Some(Provider::OpenAi)
    }

    pub fn label(self) -> &'static str {
        match self {
            Provider::Anthropic => "Claude",
            Provider::OpenAi => "OpenAI-совместимый",
        }
    }

    /// Адрес API.
    pub fn endpoint(self) -> &'static str {
        match self {
            Provider::Anthropic => "https://api.anthropic.com/v1/messages",
            Provider::OpenAi => "https://api.openai.com/v1/chat/completions",
        }
    }
}

/// Облачная модель: Anthropic или что-то OpenAI-совместимое.
pub struct AiChat {
    key: String,
    model: String,
    provider: Provider,
}

impl AiChat {
    pub fn new(cfg: &Config) -> Self {
        let key = cfg.anthropic_api_key.trim().to_string();
        // Провайдера без ключа не определить — берём Anthropic как заглушку,
        // но `available()` вернёт false, и запроса всё равно не будет.
        let provider = Provider::detect(&key).unwrap_or(Provider::Anthropic);
        Self { key, model: cfg.model.clone(), provider }
    }

    pub fn available(&self) -> bool {
        !self.key.is_empty()
    }

    /// Чей ключ и на какой модели работаем — для настроек и чата.
    pub fn label(&self) -> String {
        self.provider.label().to_string()
    }

    /// Спросить нейросеть. `None` — сеть/ключ/лимит подвели.
    pub fn ask(&self, text: &str) -> Option<String> {
        if !self.available() {
            return None;
        }
        match self.provider {
            Provider::Anthropic => self.ask_anthropic(text),
            Provider::OpenAi => self.ask_openai(text),
        }
    }

    fn ask_anthropic(&self, text: &str) -> Option<String> {
        let body = json!({
            "model": self.model,
            "max_tokens": 400,
            "system": persona::system_prompt(),
            "messages": [{ "role": "user", "content": text }],
        });
        let result = ureq::post(self.provider.endpoint())
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
            .config()
            .timeout_global(Some(Duration::from_secs(45)))
            .build()
            .send_json(&body);
        match result {
            Ok(mut resp) => {
                let value: Value = resp.body_mut().read_json().ok()?;
                let text = value
                    .get("content")?
                    .as_array()?
                    .iter()
                    .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join(" ");
                Some(clean_reply(&text))
            }
            Err(e) => {
                log::warn!("Claude не ответил: {e}");
                None
            }
        }
    }

    /// OpenAI-совместимый формат: `/chat/completions` с Bearer-ключом.
    fn ask_openai(&self, text: &str) -> Option<String> {
        let body = json!({
            "model": self.model,
            "max_tokens": 400,
            "messages": [
                { "role": "system", "content": persona::system_prompt() },
                { "role": "user", "content": text },
            ],
        });
        let result = ureq::post(self.provider.endpoint())
            .header("Authorization", &format!("Bearer {}", self.key))
            .config()
            .timeout_global(Some(Duration::from_secs(45)))
            .build()
            .send_json(&body);
        match result {
            Ok(mut resp) => {
                let value: Value = resp.body_mut().read_json().ok()?;
                let text = value
                    .get("choices")?
                    .as_array()?
                    .first()?
                    .get("message")?
                    .get("content")?
                    .as_str()?
                    .to_string();
                Some(clean_reply(&text))
            }
            Err(e) => {
                log::warn!("{} не ответил: {e}", self.provider.label());
                None
            }
        }
    }

    /// Короткий пересказ отчёта о компьютере.
    pub fn summarize_report(&self, report: &str) -> Option<String> {
        self.ask(&format!(
            "Вот отчёт о компьютере. Перескажи самое важное в двух предложениях, \
             без списков и markdown:\n\n{report}"
        ))
    }
}

/// Локальная нейросеть: Ollama или LM Studio (и любой OpenAI-совместимый сервер).
///
/// Раньше здесь был только протокол Ollama (`/api/tags`, `/api/chat`), а в
/// README при этом написано «настрой LM Studio на порту 1234». LM Studio таких
/// адресов не знает — он отдаёт `/v1/models` и `/v1/chat/completions`. Поэтому
/// связка «человек сделал по инструкции, а Капри говорит «мозги не настроены»»
/// была гарантированной. Теперь пробуем оба протокола и несколько адресов.
pub struct LocalAi {
    /// Адреса в порядке предпочтения: сначала из настроек, потом обычные.
    urls: Vec<String>,
    model: String,
}

/// С каким сервером удалось поговорить.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Flavor {
    /// Ollama: `/api/tags`, `/api/chat`.
    Ollama,
    /// OpenAI-совместимый (LM Studio, llama.cpp, vLLM): `/v1/models`,
    /// `/v1/chat/completions`.
    OpenAi,
}

/// Куда стучаться, если в настройках пусто или неверный адрес.
///
/// 1234 — LM Studio (о ней и написано в README), 11434 — Ollama. Порядок важен:
/// LM Studio человек ставит по инструкции, и она должна работать по умолчанию.
const DEFAULT_PORTS: [u16; 2] = [1234, 11434];

impl LocalAi {
    pub fn new(cfg: &Config) -> Self {
        let mut urls = Vec::new();
        let configured = cfg.local_ai_url.trim().trim_end_matches('/').to_string();
        if !configured.is_empty() {
            urls.push(configured);
        }
        for port in DEFAULT_PORTS {
            let url = format!("http://127.0.0.1:{port}");
            if !urls.contains(&url) {
                urls.push(url);
            }
        }
        Self {
            urls,
            model: cfg.local_model.clone(),
        }
    }

    pub fn available(&self) -> bool {
        self.probe().is_some()
    }

    /// Найти живой сервер и понять, на каком он протоколе.
    ///
    /// Возвращает адрес, протокол и список моделей — одним запросом, чтобы не
    /// стучаться дважды: `available()` и `effective_model()` идут подряд.
    fn probe(&self) -> Option<(String, Flavor, Vec<String>)> {
        for url in &self.urls {
            if let Some(models) = self.models_ollama(url) {
                return Some((url.clone(), Flavor::Ollama, models));
            }
            if let Some(models) = self.models_openai(url) {
                return Some((url.clone(), Flavor::OpenAi, models));
            }
        }
        None
    }

    /// Список моделей у Ollama (`GET /api/tags`).
    fn models_ollama(&self, url: &str) -> Option<Vec<String>> {
        let mut resp = ureq::get(&format!("{url}/api/tags"))
            .config()
            .timeout_global(Some(Duration::from_millis(1200)))
            .build()
            .call()
            .ok()?;
        let value: Value = resp.body_mut().read_json().ok()?;
        let models: Vec<String> = value
            .get("models")?
            .as_array()?
            .iter()
            .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
            .collect();
        // Пустой список — сервер ответил, но моделей нет. Это всё равно «живой».
        Some(models)
    }

    /// Список моделей у OpenAI-совместимого сервера (`GET /v1/models`).
    fn models_openai(&self, url: &str) -> Option<Vec<String>> {
        let mut resp = ureq::get(&format!("{url}/v1/models"))
            .config()
            .timeout_global(Some(Duration::from_millis(1200)))
            .build()
            .call()
            .ok()?;
        let value: Value = resp.body_mut().read_json().ok()?;
        let models: Vec<String> = value
            .get("data")?
            .as_array()?
            .iter()
            .filter_map(|m| m.get("id").and_then(|n| n.as_str()).map(|s| s.to_string()))
            .collect();
        Some(models)
    }

    /// Список моделей живого сервера (`None` — ни один не ответил).
    pub fn models(&self) -> Option<Vec<String>> {
        self.probe().map(|(_, _, models)| models)
    }

    /// Модель, которая реально есть на сервере: сначала из настроек, иначе
    /// подходящая по вкусу.
    pub fn effective_model(&self) -> Option<String> {
        let (_, _, models) = self.probe()?;
        Some(pick_model(&self.model, &models))
    }

    /// Адрес и протокол живого сервера — для панелей и логов.
    pub fn describe(&self) -> Option<String> {
        let (url, flavor, _) = self.probe()?;
        Some(match flavor {
            Flavor::Ollama => format!("Ollama · {url}"),
            Flavor::OpenAi => format!("LM Studio · {url}"),
        })
    }

    pub fn ask(&self, text: &str) -> Option<String> {
        let answer = self.chat(&[
            json!({ "role": "system", "content": persona::system_prompt() }),
            json!({ "role": "user", "content": text }),
        ])?;
        Some(clean_reply(&answer))
    }

    /// Спросить без персоны Капри и без чистки ответа.
    ///
    /// Нужно для задач, где личность мешает: перевод должен вернуть перевод, а
    /// не «Ой, сейчас посмотрю, солнце». `clean_reply` тут тоже лишний — он
    /// срезает длину и выкидывает строки, а переводу нужен весь текст.
    pub fn ask_plain(&self, prompt: &str) -> Option<String> {
        let answer = self.chat_with(
            &[
                json!({
                    "role": "system",
                    "content": "Ты точный переводчик. Отвечаешь только переводом, без пояснений."
                }),
                json!({ "role": "user", "content": prompt }),
            ],
            TRANSLATE_TOKENS,
        )?;
        let answer = answer.trim();
        if answer.is_empty() {
            None
        } else {
            Some(answer.to_string())
        }
    }

    /// Реплика «сама по себе» — чтобы Капри не молчала минутами.
    pub fn chatter(&self) -> Option<String> {
        let answer = self.chat(&[
            json!({ "role": "system", "content": persona::system_prompt() }),
            json!({
                "role": "user",
                "content": "Скажи одну короткую живую фразу сама, без вопроса-заглушки. \
                            Не упоминай, что ты ИИ. Одно предложение."
            }),
        ])?;
        Some(clean_reply(&answer))
    }

    /// Пересказ отчёта о компьютере.
    pub fn summarize_report(&self, report: &str) -> Option<String> {
        let answer = self.chat(&[
            json!({ "role": "system", "content": "Ты кратко пересказываешь отчёты о компьютере по-русски, двумя предложениями." }),
            json!({ "role": "user", "content": report }),
        ])?;
        Some(clean_reply(&answer))
    }

    /// Один запрос к живому серверу — каким бы протоколом он ни говорил.
    fn chat(&self, messages: &[Value]) -> Option<String> {
        self.chat_with(messages, REPLY_TOKENS)
    }

    /// То же, но с явным лимитом длины ответа.
    ///
    /// Лимит нужен разный: реплике Капри хватает пары сотен токенов, а перевод
    /// приходит куском до 1800 символов — на нём прежние 200 токенов обрезали
    /// текст на середине, и человек видел перевод, оборванный на полуслове.
    fn chat_with(&self, messages: &[Value], tokens: u32) -> Option<String> {
        let (url, flavor, models) = self.probe()?;
        let model = pick_model(&self.model, &models);
        if model.is_empty() {
            return None;
        }
        let (path, body) = match flavor {
            Flavor::Ollama => (
                format!("{url}/api/chat"),
                json!({
                    "model": model,
                    "messages": messages,
                    "stream": false,
                    "think": false,
                    "options": { "temperature": 0.8, "num_predict": tokens },
                }),
            ),
            Flavor::OpenAi => (
                format!("{url}/v1/chat/completions"),
                json!({
                    "model": model,
                    "messages": messages,
                    "stream": false,
                    "temperature": 0.8,
                    "max_tokens": tokens,
                }),
            ),
        };
        let mut resp = ureq::post(&path)
            .config()
            .timeout_global(Some(Duration::from_secs(90)))
            .build()
            .send_json(&body)
            .ok()?;
        let value: Value = resp.body_mut().read_json().ok()?;
        let text = match flavor {
            // Ollama: {"message": {"content": "…"}}
            Flavor::Ollama => value
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string()),
            // OpenAI: {"choices": [{"message": {"content": "…"}}]}
            Flavor::OpenAi => value
                .get("choices")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("message"))
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .map(|s| s.to_string()),
        }?;
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
}

/// Сколько токенов просить на обычную реплику.
const REPLY_TOKENS: u32 = 400;

/// Сколько — на перевод куска текста (до `translate::CHUNK_CHARS` символов).
const TRANSLATE_TOKENS: u32 = 2000;

/// Выбрать модель из тех, что реально есть на сервере.
///
/// Порядок: точное имя из настроек → имя, начинающееся так же (в LM Studio
/// к имени добавляется квантование) → первая разговорная модель.
fn pick_model(configured: &str, models: &[String]) -> String {
    let configured = configured.trim();
    if !configured.is_empty() {
        if let Some(found) = models.iter().find(|m| m.as_str() == configured) {
            return found.clone();
        }
        if let Some(found) = models.iter().find(|m| m.starts_with(configured)) {
            return found.clone();
        }
    }
    // Эмбеддинги лежат в том же списке, но ответить не могут: выбрать
    // `text-embedding-…` значит получить пустой ответ на любой вопрос.
    let chat: Vec<&String> = models
        .iter()
        .filter(|m| !m.contains("embed") && !m.contains("rerank"))
        .collect();
    chat.iter()
        .find(|m| m.contains("instruct"))
        .or_else(|| chat.first())
        .map(|m| (*m).clone())
        .or_else(|| models.first().cloned())
        .unwrap_or_default()
}

/// Убрать markdown, звёздочки и обрезать слишком длинный ответ:
/// его же читает синтезатор речи.
pub fn clean_reply(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let line = line.trim_start_matches(['#', '>', '-', '*', '•']).trim();
        let line = line.replace("**", "").replace('*', "").replace('`', "");
        if !line.is_empty() {
            out.push_str(&line);
            out.push(' ');
        }
    }
    let out = out.trim().to_string();
    // Слишком длинный ответ слушать невозможно — режем по границе предложения.
    if out.chars().count() > 600 {
        let head: String = out.chars().take(600).collect();
        let cut = head.rfind(['.', '!', '?']).unwrap_or(head.len() - 1);
        return head[..=cut].trim().to_string();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_from_settings_wins_when_it_exists() {
        // Имя точь-в-точь как в настройках — берём его, а не «первое похожее».
        let models = vec!["llama3:8b".to_string(), "qwen3:8b".to_string()];
        assert_eq!(pick_model("qwen3:8b", &models), "qwen3:8b");
    }

    #[test]
    fn model_is_found_by_prefix() {
        // LM Studio дописывает к имени квантование, поэтому точного совпадения
        // может не быть: `qwen2.5-coder-7b-instruct` → `…-instruct-gguf`.
        let models = vec![
            "qwen2.5-coder-7b-instruct-gguf".to_string(),
            "llama3:8b".to_string(),
        ];
        assert_eq!(
            pick_model("qwen2.5-coder-7b-instruct", &models),
            "qwen2.5-coder-7b-instruct-gguf"
        );
    }

    #[test]
    fn embeddings_are_never_chosen() {
        // Эмбеддинги лежат в том же `/v1/models`, что и чат-модели. Выбрать
        // `text-embedding-…` значит получить пустой ответ на любой вопрос —
        // ровно то, на что жаловался человек.
        let models = vec![
            "text-embedding-nomic-embed-text-v1.5".to_string(),
            "dolphin3-cyber-8b".to_string(),
        ];
        assert_eq!(pick_model("нет-такой", &models), "dolphin3-cyber-8b");
        assert_eq!(pick_model("", &models), "dolphin3-cyber-8b");
    }

    #[test]
    fn instruct_is_preferred_over_plain_chat_models() {
        // Обычные qwen3 проговаривают рассуждения вслух — их слышно в колонках.
        let models = vec!["qwen3:8b".to_string(), "qwen2.5:7b-instruct".to_string()];
        assert_eq!(pick_model("", &models), "qwen2.5:7b-instruct");
    }

    #[test]
    fn no_models_is_an_empty_name() {
        assert_eq!(pick_model("qwen3:8b", &[]), "");
    }

    #[test]
    fn cleans_markdown() {
        let text = clean_reply("**Привет!** Вот *список*:\n- первое\n- второе");
        assert!(!text.contains('*'));
        assert!(text.contains("первое"));
    }

    #[test]
    fn trims_long_answers() {
        let long = "Предложение. ".repeat(200);
        assert!(clean_reply(&long).chars().count() <= 601);
    }

    #[test]
    fn an_anthropic_key_is_recognised() {
        // `sk-ant-` — единственный префикс, который нельзя спутать.
        assert_eq!(Provider::detect("sk-ant-api03-xxx"), Some(Provider::Anthropic));
        assert_eq!(Provider::Anthropic.label(), "Claude");
        assert!(Provider::Anthropic.endpoint().contains("anthropic.com"));
    }

    #[test]
    fn other_keys_are_treated_as_openai_compatible() {
        // Groq, NVIDIA, OpenAI — все говорят на `/chat/completions`.
        for key in ["sk-proj-xxx", "gsk_xxx", "nvapi-xxx", "что-то своё"] {
            assert_eq!(
                Provider::detect(key),
                Some(Provider::OpenAi),
                "ключ {key} должен уйти в OpenAI-совместимый формат"
            );
        }
        assert!(Provider::OpenAi.endpoint().ends_with("/chat/completions"));
    }

    #[test]
    fn an_empty_key_has_no_provider() {
        assert_eq!(Provider::detect(""), None);
        assert_eq!(Provider::detect("   "), None);
        // Без ключа запроса не будет, чем бы он ни оказался.
        let cfg = Config { anthropic_api_key: String::new(), ..Config::default() };
        assert!(!AiChat::new(&cfg).available());
    }
}
