//! Громкость и яркость.

use crate::util::ps;

#[cfg(windows)]
mod com_audio {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    fn endpoint() -> windows::core::Result<IAudioEndpointVolume> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
        }
    }

    pub fn volume() -> Option<u8> {
        let ep = endpoint().ok()?;
        let v = unsafe { ep.GetMasterVolumeLevelScalar().ok()? };
        Some((v * 100.0).round().clamp(0.0, 100.0) as u8)
    }

    pub fn muted() -> Option<bool> {
        let ep = endpoint().ok()?;
        unsafe { ep.GetMute().ok().map(|b| b.as_bool()) }
    }

    pub fn set_volume(percent: u8) -> Result<(), String> {
        let ep = endpoint().map_err(|e| format!("звук недоступен: {e}"))?;
        let value = (percent.min(100) as f32) / 100.0;
        unsafe {
            ep.SetMasterVolumeLevelScalar(value, std::ptr::null())
                .map_err(|e| format!("не смогла поставить громкость: {e}"))?;
            ep.SetMute(false, std::ptr::null())
                .map_err(|e| format!("не смогла включить звук: {e}"))?;
        }
        Ok(())
    }

    pub fn set_mute(on: bool) -> Result<(), String> {
        let ep = endpoint().map_err(|e| format!("звук недоступен: {e}"))?;
        unsafe {
            ep.SetMute(on, std::ptr::null())
                .map_err(|e| format!("не смогла переключить звук: {e}"))?;
        }
        Ok(())
    }
}

pub fn get_volume() -> Option<u8> {
    #[cfg(windows)]
    {
        if let Some(v) = com_audio::volume() {
            return Some(v);
        }
    }
    None
}

pub fn set_volume(percent: u8) -> Result<(), String> {
    #[cfg(windows)]
    {
        return com_audio::set_volume(percent);
    }
    #[cfg(not(windows))]
    {
        let _ = percent;
        Err("только для Windows".into())
    }
}

pub fn set_mute(on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        return com_audio::set_mute(on);
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err("только для Windows".into())
    }
}

pub fn is_muted() -> Option<bool> {
    #[cfg(windows)]
    {
        return com_audio::muted();
    }
    #[cfg(not(windows))]
    None
}

// ---------------------------------------------------------------- яркость

/// Яркость основного монитора в процентах (ноутбуки и мониторы с DDC/CI).
pub fn get_brightness() -> Option<u8> {
    let script = "(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction Stop).CurrentBrightness";
    ps(script, 8).ok().and_then(|s| s.trim().parse::<u8>().ok())
}

pub fn set_brightness(percent: u8) -> Result<(), String> {
    let value = percent.min(100);
    let script = format!(
        "$m = Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods -ErrorAction Stop; \
         if (-not $m) {{ throw 'no monitor' }}; \
         Invoke-CimMethod -InputObject $m -MethodName WmiSetBrightness -Arguments @{{ Timeout = 1; Brightness = {value} }} | Out-Null; \
         'ok'"
    );
    match ps(&script, 10) {
        Ok(out) if out.contains("ok") => Ok(()),
        Ok(_) => Err("монитор отказался подчиняться".into()),
        Err(_) => Err("этот монитор не поддерживает управление яркостью".into()),
    }
}
