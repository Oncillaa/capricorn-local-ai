//! Ответ Капри на команду.

/// Действие, которое должна выполнить оболочка (UI/диалоги) после реплики.
pub mod action {
    pub const SAY: &str = "say";
    pub const ANSWER: &str = "answer";
    pub const QUIT: &str = "quit";
    pub const HIDE: &str = "hide";
    pub const WAVE: &str = "wave";
    pub const ROAM: &str = "roam";
    /// `move:<угол>` — перейти в угол экрана.
    pub const MOVE: &str = "move:";
    /// Сменить микрофон на следующий (делает оболочка — там живёт слушатель).
    pub const MIC_NEXT: &str = "mic_next";
    pub const MIC_INFO: &str = "mic_info";
    pub const ANIM: &str = "anim:";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,
    pub action: Option<String>,
}

impl Reply {
    pub fn say(text: impl Into<String>) -> Self {
        Self { text: text.into(), action: None }
    }

    pub fn act(text: impl Into<String>, action: impl Into<String>) -> Self {
        Self { text: text.into(), action: Some(action.into()) }
    }

    /// То же, что `Reply::say`, но с пометкой «это ответ нейросети»
    /// (включает телепорт, если он включён в настройках).
    pub fn answer(text: impl Into<String>) -> Self {
        Self::act(text, action::ANSWER)
    }

    pub fn action_is(&self, a: &str) -> bool {
        self.action.as_deref() == Some(a)
    }

    /// `move:` / `anim:` — вернуть хвост после двоеточия.
    pub fn action_arg(&self, prefix: &str) -> Option<&str> {
        self.action
            .as_deref()
            .and_then(|a| a.strip_prefix(prefix))
    }
}
