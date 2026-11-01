//! РџСЂРѕР±Р° РіРѕС‚РѕРІРѕРіРѕ РіРѕР»РѕСЃР°: РІРµСЃСЊ РїСѓС‚СЊ `Speaker::say` СЃ Р·Р°РїР°СЃРЅС‹Рј РіРѕР»РѕСЃРѕРј Windows.
//!
//! Р—Р°РїСѓСЃРє: `cargo run -p capri-voice --example speaker_probe`

use std::time::Instant;

fn main() {
    let mut cfg = capri_core::Config::default();
    cfg.tts_engine = "edge".to_string();
    cfg.voice_volume = 70;
    let speaker = capri_voice::speaker::Speaker::new(
        &cfg,
        std::path::Path::new("target"),
        Some(std::path::Path::new("C:\\Users\\Oncilla\\Desktop\\capricorn-main")),
    );

    let started = Instant::now();
    let interrupted = speaker.say("РџСЂРёРІРµС‚! Р­С‚Рѕ РљР°РїСЂРё. РџСЂРѕРІРµСЂРєР° РіРѕР»РѕСЃР°.");
    println!("say РІРµСЂРЅСѓР» {interrupted} Р·Р° {:?}", started.elapsed());
    match speaker.error() {
        Some(error) => println!("РѕС€РёР±РєР° РіРѕР»РѕСЃР°: {error}"),
        None => println!("РѕС€РёР±РѕРє РЅРµС‚"),
    }
}


