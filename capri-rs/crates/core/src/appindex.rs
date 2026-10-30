//! Индекс установленных программ и поиск по нему «на слух».
//!
//! Строится в фоне при старте: «открой хром», «открой роблокс», «закрой клод»
//! должны находить программы, даже если название распознано с ошибкой.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use parking_lot::RwLock;

use crate::config::Map;
use crate::platform::Platform;
use crate::util::{norm, translit};

pub struct AppIndex {
    items: RwLock<Vec<(String, String)>>,
    ready: AtomicBool,
}

impl Default for AppIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl AppIndex {
    pub fn new() -> Self {
        Self { items: RwLock::new(Vec::new()), ready: AtomicBool::new(false) }
    }

    /// Построить индекс в фоновом потоке (не тормозим запуск Капри).
    pub fn build_async(self: &Arc<Self>, platform: Arc<dyn Platform>) {
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            let programs = platform.installed_programs();
            let items: Vec<(String, String)> = programs
                .into_iter()
                .map(|p| (norm(&p.name), p.target))
                .filter(|(name, _)| !name.is_empty())
                .collect();
            log::info!("Найдено приложений: {}", items.len());
            *me.items.write() = items;
            me.ready.store(true, Ordering::Relaxed);
        });
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    pub fn len(&self) -> usize {
        self.items.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Подходящие программы, лучшие первыми: (оценка, название, цель).
    pub fn candidates(&self, query: &str, aliases: &Map, limit: usize) -> Vec<(f64, String, String)> {
        let q = norm(query);
        if q.is_empty() {
            return Vec::new();
        }
        let mut variants = vec![q.clone(), translit(&q)];
        if let Some(alias) = aliases.get(&q) {
            variants.push(norm(alias));
        }
        variants.retain(|v| !v.is_empty());
        variants.dedup();

        let items = self.items.read();
        let mut scored: Vec<(f64, String, String)> = Vec::new();
        for (name, target) in items.iter() {
            let mut best = 0.0f64;
            for v in &variants {
                let score = if *v == *name {
                    1.0
                } else if v.chars().count() >= 3
                    && (name.starts_with(v.as_str())
                        || name.contains(&format!(" {v}"))
                        || name.contains(&format!("{v} ")))
                {
                    0.93 - 0.003 * name.chars().count() as f64
                } else {
                    let mut s = crate::util::similarity(v, name);
                    let first = name.split_whitespace().next().unwrap_or(name);
                    if sounds_alike(v, name) {
                        s = s.max(0.97);
                    } else if sounds_alike(v, first) {
                        s = s.max(0.86);
                    }
                    s
                };
                best = best.max(score);
            }
            if best >= 0.72 {
                scored.push((best, name.clone(), target.clone()));
            }
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// Самый подходящий вариант.
    pub fn find(&self, query: &str, aliases: &Map) -> Option<(String, String)> {
        self.candidates(query, aliases, 1)
            .into_iter()
            .next()
            .map(|(_, name, target)| (name, target))
    }

    /// Похожие ярлыки того же приложения (Roblox, Roblox Player, …).
    pub fn similar(&self, name: &str) -> Vec<(String, String)> {
        let prefix = name.split_whitespace().next().unwrap_or(name).to_string();
        if prefix.is_empty() {
            return Vec::new();
        }
        let items = self.items.read();
        let mut out: Vec<(String, String)> = items
            .iter()
            .filter(|(n, _)| {
                n != name
                    && n.starts_with(&prefix)
                    && !n.contains("studio")
                    && !n.contains("uninstall")
                    && !n.contains("update")
            })
            .cloned()
            .collect();
        out.sort_by_key(|(n, _)| n.len());
        out
    }
}

/// Фонетический скелет: «cloud», «claude», «клод» → «kld».
fn sound(s: &str) -> String {
    let mut t: String = translit(&s.to_lowercase()).chars().filter(|c| !c.is_whitespace()).collect();
    for (a, b) in [("ph", "f"), ("ck", "k"), ("c", "k"), ("q", "k"), ("w", "v"), ("x", "ks"), ("z", "s")] {
        t = t.replace(a, b);
    }
    t
}

fn skeleton(s: &str) -> String {
    sound(s).chars().filter(|c| !matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')).collect()
}

/// Созвучны, если совпадают согласные и первый звук
/// («роблокс» = «roblox», но «запрет» ≠ «aseprite»).
fn sounds_alike(a: &str, b: &str) -> bool {
    let (sa, sb) = (skeleton(a), skeleton(b));
    sa.chars().count() >= 3
        && sa == sb
        && sound(a).chars().next() == sound(b).chars().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phonetic_matching() {
        assert!(sounds_alike("роблокс", "roblox"));
        assert!(sounds_alike("клод", "claude"));
        assert!(!sounds_alike("запрет", "aseprite"));
    }

    #[test]
    fn scores_exact_first() {
        let index = AppIndex::new();
        *index.items.write() = vec![
            ("google chrome".to_string(), "appid:chrome".to_string()),
            ("telegram desktop".to_string(), "C:/tg.exe".to_string()),
        ];
        let found = index.candidates("телеграм", &Map::new(), 3);
        assert!(!found.is_empty());
        let exact = index.candidates("google chrome", &Map::new(), 3);
        assert_eq!(exact[0].1, "google chrome");
    }
}
