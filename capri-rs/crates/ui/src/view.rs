//! Настройки вида: размер, живость, монитор.
//!
//! Живут отдельно от [`capri_core::Config`], потому что окну нужны не строки
//! из `config.json`, а готовые числа: сколько пикселей рисовать и во сколько
//! раз медленнее листать кадры.

use capri_core::config::{clamp_scale, normalize_liveliness};

/// Насколько Капри живая.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveliness {
    /// Спокойная: реже машет, медленнее ходит.
    Calm,
    /// Обычная: как раньше.
    Normal,
    /// Живая: всё быстрее.
    Lively,
}

impl Liveliness {
    pub fn parse(value: &str) -> Self {
        match normalize_liveliness(value) {
            "calm" => Self::Calm,
            "normal" => Self::Normal,
            _ => Self::Lively,
        }
    }

    /// Идентификатор для `config.json`.
    pub fn id(self) -> &'static str {
        match self {
            Self::Calm => "calm",
            Self::Normal => "normal",
            Self::Lively => "lively",
        }
    }

    /// Во сколько раз дольше держится кадр анимации.
    ///
    /// «Живая» — исходный темп (штрихи анимации шли с `hold` 1–2). Спокойная
    /// растягивает их втрое: движения становятся плавными, а не дёргаными.
    pub fn frame_slowdown(self) -> u32 {
        match self {
            Self::Calm => 3,
            Self::Normal => 2,
            Self::Lively => 1,
        }
    }

    /// Скорость прогулки по экрану, пикселей за тик.
    pub fn walk_speed(self) -> i32 {
        match self {
            Self::Calm => 4,
            Self::Normal => 6,
            Self::Lively => 9,
        }
    }
}

/// Что окну нужно знать о внешнем виде.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    /// Размер в процентах, 1…100.
    pub scale: u8,
    pub liveliness: Liveliness,
    /// Номер монитора, на котором живёт Капри.
    pub monitor: u32,
}

impl Default for ViewSettings {
    fn default() -> Self {
        // Как в стандартных настройках: во весь рост и самая живая.
        Self { scale: 100, liveliness: Liveliness::Lively, monitor: 0 }
    }
}

impl ViewSettings {
    pub fn from_config(config: &capri_core::Config) -> Self {
        Self {
            scale: clamp_scale(config.character_scale),
            liveliness: Liveliness::parse(&config.liveliness),
            monitor: config.monitor_index,
        }
    }

    pub fn with_scale(self, percent: u8) -> Self {
        Self { scale: clamp_scale(percent), ..self }
    }

    pub fn with_liveliness(self, value: &str) -> Self {
        Self { liveliness: Liveliness::parse(value), ..self }
    }

    pub fn with_monitor(self, index: u32) -> Self {
        Self { monitor: index, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liveliness_is_read_from_any_spelling() {
        assert_eq!(Liveliness::parse("calm"), Liveliness::Calm);
        assert_eq!(Liveliness::parse(" Normal "), Liveliness::Normal);
        assert_eq!(Liveliness::parse("LIVELY"), Liveliness::Lively);
        // Незнакомое значение — не падаем, а берём самое живое.
        assert_eq!(Liveliness::parse("что-то своё"), Liveliness::Lively);
    }

    #[test]
    fn liveliness_ids_survive_a_roundtrip() {
        for value in [Liveliness::Calm, Liveliness::Normal, Liveliness::Lively] {
            assert_eq!(Liveliness::parse(value.id()), value);
        }
    }

    #[test]
    fn calm_is_slower_than_lively() {
        assert!(Liveliness::Calm.frame_slowdown() > Liveliness::Lively.frame_slowdown());
        assert!(Liveliness::Calm.walk_speed() < Liveliness::Lively.walk_speed());
    }

    #[test]
    fn settings_come_from_config() {
        let mut config = capri_core::Config::default();
        config.character_scale = 55;
        config.liveliness = "calm".into();
        config.monitor_index = 1;
        let view = ViewSettings::from_config(&config);
        assert_eq!(view.scale, 55);
        assert_eq!(view.liveliness, Liveliness::Calm);
        assert_eq!(view.monitor, 1);
    }

    #[test]
    fn scale_is_always_inside_the_slider() {
        let view = ViewSettings::default();
        assert_eq!(view.with_scale(0).scale, 1);
        assert_eq!(view.with_scale(250).scale, 100);
        assert_eq!(view.with_scale(55).scale, 55);
    }
}
