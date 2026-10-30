//! Разговорные нейросети: Claude (Anthropic API) и локальная Ollama.

use std::time::Duration;

use serde_json::{json, Value};

use crate::config::Config;
use crate::persona;

/// Claude через Anthropic Messages API.
pub struct AiChat {
    key: String,
    model: String,
}

impl AiChat {
    pub fn new(cfg: &Config) -> Self {
        Self {
            key: cfg.anthropic_api_key.trim().to_string(),
            model: cfg.model.clone(),
        }
    }

    pub fn available(&self) -> bool {
        !self.key.is_empty()
    }

    /// Спросить нейросеть. `None` — сеть/ключ/лимит подвели.
    pub fn ask(&self, text: &str) -> Option<String> {
        if !self.available() {
            return None;
        }
        let body = json!({
            "model": self.model,
            "max_tokens": 400,
            "system": persona::system_prompt(),
            "messages": [{ "role": "user", "content": text }],
        });
        let result = ureq::post("https://api.anthropic.com/v1/messages")
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

    /// Короткий пересказ отчёта о компьютере.
    pub fn summarize_report(&self, report: &str) -> Option<String> {
        self.ask(&format!(
            "Вот отчёт о компьютере. Перескажи самое важное в двух предложениях, \
             без списков и markdown:\n\n{report}"
        ))
    }
}

/// Локальная нейросеть через Ollama.
pub struct LocalAi {
    url: String,
    model: String,
}

impl LocalAi {
    pub fn new(cfg: &Config) -> Self {
        Self {
            url: cfg.local_ai_url.trim_end_matches('/').to_string(),
            model: cfg.local_model.clone(),
        }
    }

    pub fn available(&self) -> bool {
        self.models().is_some()
    }

    /// Список скачанных моделей (`None` — Ollama не запущена).
    pub fn models(&self) -> Option<Vec<String>> {
        let mut resp = ureq::get(&format!("{}/api/tags", self.url))
            .config()
            .timeout_global(Some(Duration::from_millis(1200)))
            .build()
            .call()
            .ok()?;
        let value: Value = resp.body_mut().read_json().ok()?;
        Some(
            value
                .get("models")?
                .as_array()?
                .iter()
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(|s| s.to_string()))
                .collect(),
        )
    }

    /// Модель, которая реально есть на диске: сначала из настроек, иначе первая скачанная.
    pub fn effective_model(&self) -> Option<String> {
        let models = self.models()?;
        if models.iter().any(|m| m == &self.model) {
            return Some(self.model.clone());
        }
        if let Some(m) = models.iter().find(|m| m.starts_with(&self.model)) {
            return Some(m.clone());
        }
        // Предпочитаем «-instruct»: обычные qwen3 проговаривают рассуждения вслух.
        let preferred = models
            .iter()
            .find(|m| m.contains("instruct"))
            .or_else(|| models.first());
        preferred.cloned()
    }

    pub fn ask(&self, text: &str) -> Option<String> {
        let answer = self.chat(&[
            json!({ "role": "system", "content": persona::system_prompt() }),
            json!({ "role": "user", "content": text }),
        ])?;
        Some(clean_reply(&answer))
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

    fn chat(&self, messages: &[Value]) -> Option<String> {
        let model = self.effective_model()?;
        let body = json!({
            "model": model,
            "messages": messages,
            "stream": false,
            "think": false,
            "options": { "temperature": 0.8, "num_predict": 200 },
        });
        let mut resp = ureq::post(&format!("{}/api/chat", self.url))
            .config()
            .timeout_global(Some(Duration::from_secs(90)))
            .build()
            .send_json(&body)
            .ok()?;
        let value: Value = resp.body_mut().read_json().ok()?;
        let text = value
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())?
            .to_string();
        if text.trim().is_empty() {
            None
        } else {
            Some(text)
        }
    }
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
}
