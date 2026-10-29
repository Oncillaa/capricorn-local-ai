"""Распознавание речи (Vosk офлайн + Google для точности) и синтез голоса."""

import asyncio
import ctypes
import json
import logging
import math
import os
import queue
import random
import re
import shutil
import struct
import tempfile
import threading
import time
import wave

log = logging.getLogger("capri")
RATE = 16000


def _model_complete(folder):
    return os.path.isfile(os.path.join(folder, "am", "final.mdl"))


def ascii_model_path(path):
    if path.isascii():
        return path
    bases = [
        os.environ.get("LOCALAPPDATA", ""),
        os.environ.get("PROGRAMDATA", r"C:\ProgramData"),
        os.environ.get("PUBLIC", r"C:\Users\Public"),
        tempfile.gettempdir(),
    ]
    for base in bases:
        target = os.path.join(base, "Capri", os.path.basename(path.rstrip("\\/")))
        if not base or not target.isascii():
            continue
        try:
            if not _model_complete(target):
                shutil.rmtree(target, ignore_errors=True)
                shutil.copytree(path, target)
            return target
        except OSError:
            pass
    return path


def input_devices():
    import sounddevice as sd

    apis = sd.query_hostapis()
    mme = next((i for i, a in enumerate(apis) if "MME" in a["name"]), None)
    out, seen = [], set()
    for i, d in enumerate(sd.query_devices()):
        if d["max_input_channels"] <= 0 or (mme is not None and d["hostapi"] != mme):
            continue
        name = d["name"].strip()
        low = name.lower()
        if (
            "sound mapper" in low
            or "переназначение звуковых" in low
            or "первичный драйвер" in low
        ):
            continue
        if name not in seen:
            seen.add(name)
            out.append((name, i))
    return out


def default_input_name():
    try:
        import sounddevice as sd

        return sd.query_devices(kind="input")["name"].strip()
    except Exception:
        return "неизвестно"


def resolve_device(name):
    if not name:
        return None
    devices = input_devices()
    for n, i in devices:
        if n == name:
            return i
    for n, i in devices:
        if name.lower()[:20] in n.lower() or n.lower()[:20] in name.lower():
            return i
    return None


def _refresh_devices(sd):
    try:
        sd._terminate()
        sd._initialize()
    except Exception:
        pass


LEVEL_SCALE = 6.0
GATE_MAX = 0.35


class Listener(threading.Thread):
    def __init__(
        self,
        model_path,
        on_final,
        on_partial,
        on_error,
        device_name=None,
        gain=1.0,
        sensitivity=100,
    ):
        super().__init__(daemon=True)
        self.model_path = model_path
        self.on_final = on_final
        self.on_partial = on_partial
        self.on_error = on_error
        self.device_name = device_name
        self.active_name = None
        self.muted = threading.Event()
        self.reset_req = threading.Event()
        self.switch = threading.Event()
        self.gain = gain
        self.sensitivity = sensitivity
        self.level = 0.0

    def set_device(self, name):
        self.device_name = name or None
        self.switch.set()

    def gate_level(self):
        return (100 - self.sensitivity) / 100 * GATE_MAX

    def process(self, data):
        import numpy as np

        x = np.frombuffer(data, np.int16).astype(np.float32)
        if self.gain != 1.0:
            x = np.clip(x * self.gain, -32768, 32767)
        self.level = (
            min(1.0, float(np.sqrt(np.mean(x * x))) / 32768 * LEVEL_SCALE)
            if len(x)
            else 0.0
        )
        if self.level < self.gate_level():
            x[:] = 0
        return x.astype(np.int16).tobytes()

    def run(self):
        try:
            import sounddevice as sd
            import vosk

            vosk.SetLogLevel(-1)
            model = vosk.Model(ascii_model_path(self.model_path))
            rec = vosk.KaldiRecognizer(model, RATE)
        except Exception as e:
            self.on_error(str(e), "model")
            return
        worked = False
        while True:
            self.switch.clear()
            q = queue.Queue(maxsize=300)

            def callback(indata, frames, t, status, q=q):
                try:
                    q.put_nowait(bytes(indata))
                except queue.Full:
                    pass

            device = resolve_device(self.device_name)
            try:
                stream = sd.RawInputStream(
                    samplerate=RATE,
                    blocksize=4000,
                    dtype="int16",
                    channels=1,
                    callback=callback,
                    device=device,
                )
            except Exception as e:
                if device is not None:
                    self.device_name = None
                    continue
                if worked:
                    time.sleep(3)
                    _refresh_devices(sd)
                    continue
                self.on_error(str(e))
                return
            try:
                self.active_name = sd.query_devices(
                    device if device is not None else sd.default.device[0]
                )["name"]
            except Exception:
                self.active_name = default_input_name()
            rec.Reset()
            worked = True
            try:
                if self._listen(stream, q, rec) == "stale":
                    time.sleep(1)
                    _refresh_devices(sd)
            except Exception as e:
                self.on_error(str(e))
                return

    def _listen(self, stream, q, rec):
        with stream:
            buf = bytearray()
            was_muted = False
            last_audio = time.time()
            while not self.switch.is_set():
                try:
                    data = self.process(q.get(timeout=0.5))
                    last_audio = time.time()
                except queue.Empty:
                    if time.time() - last_audio > 3:
                        return "stale"
                    continue
                if self.muted.is_set():
                    was_muted = True
                    continue
                if was_muted or self.reset_req.is_set():
                    was_muted = False
                    self.reset_req.clear()
                    rec.Reset()
                    buf.clear()
                buf += data
                if rec.AcceptWaveform(data):
                    text = json.loads(rec.Result()).get("text", "").strip()
                    audio = bytes(buf)
                    buf.clear()
                    if text:
                        self.on_final(text, audio)
                else:
                    part = json.loads(rec.PartialResult()).get("partial", "")
                    if part:
                        self.on_partial(part)
                    if len(buf) > RATE * 2 * 25:
                        del buf[: RATE * 2 * 5]


def google_stt(audio, language="ru-RU"):
    try:
        import speech_recognition as sr

        r = sr.Recognizer()
        r.operation_timeout = 6
        return r.recognize_google(sr.AudioData(audio, RATE, 2), language=language)
    except Exception:
        return None


# ---------------- синтез речи ----------------


def _mci(cmd):
    buf = ctypes.create_unicode_buffer(256)
    err = ctypes.windll.winmm.mciSendStringW(cmd, buf, 255, 0)
    return err, buf.value


def play_mp3(path, stop_event, volume=100):
    _mci("close capri_tts")
    wav = path.lower().endswith(".wav")
    if (
        _mci(
            f'open "{path}" type {"waveaudio" if wav else "mpegvideo"} alias capri_tts'
        )[0]
        != 0
    ):
        raise RuntimeError("MCI не открыл файл")
    if not wav:
        _mci(f"setaudio capri_tts volume to {int(max(0, min(100, volume)) * 10)}")
    _mci("play capri_tts")
    interrupted = False
    try:
        while True:
            if stop_event.wait(0.05):
                interrupted = True
                _mci("stop capri_tts")
                break
            err, mode = _mci("status capri_tts mode")
            if err or mode != "playing":
                break
    finally:
        _mci("close capri_tts")
    return interrupted


def clean_for_speech(text):
    text = re.sub(r"https?://\S+", "", text)
    text = re.sub(r"[*#_`>|\[\]]", "", text)
    return re.sub(r"\s+", " ", text).strip()


def _ffmpeg():
    import imageio_ffmpeg

    return imageio_ffmpeg.get_ffmpeg_exe()


FX_SR = 24000


def decode(path, sr=FX_SR):
    import subprocess
    import numpy as np

    raw = subprocess.run(
        [
            _ffmpeg(),
            "-v",
            "error",
            "-i",
            path,
            "-f",
            "s16le",
            "-ac",
            "1",
            "-ar",
            str(sr),
            "-",
        ],
        capture_output=True,
        check=True,
        creationflags=0x08000000,
    ).stdout
    return np.frombuffer(raw, np.int16).astype(np.float32) / 32768


def trim_silence(x, sr=FX_SR, thr=0.012, pad=0.04):
    import numpy as np

    loud = np.nonzero(np.abs(x) > thr)[0]
    if not len(loud):
        return x[:0]
    p = int(sr * pad)
    return x[max(0, loud[0] - p) : loud[-1] + p]


def write_wav(dst, x, sr=FX_SR, volume=100):
    import numpy as np

    if not len(x):
        return
    x = x / max(1e-6, float(np.max(np.abs(x)))) * 0.92 * max(0, min(100, volume)) / 100
    with wave.open(dst, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes((x * 32767).astype(np.int16).tobytes())


def fx_process(x, strength=1.0, seed=None):
    import numpy as np

    sr = FX_SR
    if not len(x):
        raise RuntimeError("пустой звук")
    rnd = np.random.default_rng(seed)
    n = len(x)
    t = np.arange(n, dtype=np.float32)

    def shifted(factor, delay_ms, wobble=0.0):
        pos = t * factor + wobble * sr * 0.002 * np.sin(2 * np.pi * 0.7 * t / sr)
        pos -= delay_ms * sr / 1000
        return np.interp(pos, t, x, left=0.0, right=0.0)

    out = x.copy()
    s = strength
    for factor, delay, gain in (
        (1.012, 11, 0.32),
        (0.989, 19, 0.30),
        (1.024, 29, 0.18),
    ):
        out += shifted(factor, delay, wobble=1.0) * gain * s
    slice_len = int(sr * 0.065)
    for _ in range(int(rnd.integers(0, 3) * s) if n > sr * 1.5 and s >= 0.7 else 0):
        at = int(rnd.integers(sr // 2, n - slice_len * 4))
        piece = out[at : at + slice_len].copy()
        reps = int(rnd.integers(2, 4))
        stutter = np.concatenate([piece * (0.9**i) for i in range(reps)])
        out = np.concatenate([out[:at], stutter, out[at:]])
    crushed = np.round(out * 96) / 96
    c = 0.22 * s * s
    return out * (1 - c) + crushed * c


def _pct(value):
    m = re.match(r"([+-]?\d+)\s*%", value or "")
    return int(m.group(1)) if m else 0


def lively_semitones(sentences, base, rnd, lively=True):
    out = []
    for s in sentences:
        k = base
        if lively:
            k += rnd.uniform(-2.5, 3.0)
            end = s.rstrip()
            if end.endswith("!"):
                k += 4.0
            elif end.endswith("?"):
                k += 3.0
            elif end.endswith("…") or end.endswith("..."):
                k -= 2.5
        out.append(k)
    return out


def pitch_shift(x, semitones):
    import numpy as np

    factor = 2 ** (semitones / 12)
    pos = np.arange(0, len(x) - 1, factor)
    return np.interp(pos, np.arange(len(x)), x).astype(np.float32)


TTS_CACHE = os.path.join(
    os.environ.get("LOCALAPPDATA", tempfile.gettempdir()), "Capri", "tts_cache"
)
CACHE_MAX_TEXT = 160
CACHE_MAX_FILES = 600


def _trim_cache():
    try:
        files = sorted(
            (os.path.join(TTS_CACHE, f) for f in os.listdir(TTS_CACHE)),
            key=os.path.getmtime,
        )
        for f in files[:-CACHE_MAX_FILES]:
            os.remove(f)
    except OSError:
        pass


class Speaker:
    def __init__(self, cfg):
        self.voice = cfg.get("voice", "ru-RU-SvetlanaNeural")
        self.rate = cfg.get("voice_rate", "+5%")
        self.pitch = cfg.get("voice_pitch", "+8Hz")
        self.fx = float(cfg.get("voice_fx", 1.0))
        self.lively = bool(cfg.get("voice_lively", True))
        self.shift = float(cfg.get("voice_shift", 2.0))
        self.engine = cfg.get("tts_engine", "edge")
        self.path = os.path.join(tempfile.gettempdir(), f"capri_tts_{os.getpid()}.mp3")
        self.fx_path = os.path.join(
            tempfile.gettempdir(), f"capri_tts_{os.getpid()}.wav"
        )
        self.stop_event = threading.Event()
        self.volume = int(cfg.get("voice_volume", 100))
        self.edge_error = None

    def stop(self):
        self.stop_event.set()

    def say(self, text):
        self.stop_event.clear()
        text = clean_for_speech(text)
        if not text:
            return False
        try:
            path = self.cached_render(text)
            return play_mp3(path, self.stop_event, self.volume)
        except Exception as e:
            self.edge_error = str(e)
            log.info("edge-tts недоступен (%s), использую голос Windows", e)
        try:
            return self._sapi(text)
        except Exception:
            return False

    def render(self, text, dst):
        import edge_tts

        sentences = [s for s in re.split(r"(?<=[.!?…])\s+", text) if s.strip()] or [
            text
        ]
        semis = lively_semitones(sentences, self.shift, random.Random(), self.lively)
        rates = [
            f"{round(_pct(self.rate) - (2 ** (k / 12) - 1) * 100):+d}%" for k in semis
        ]
        parts = [f"{self.path[:-4]}_{i}.mp3" for i in range(len(sentences))]

        async def go():
            await asyncio.gather(
                *(
                    edge_tts.Communicate(s, self.voice, rate=r, pitch=self.pitch).save(
                        f
                    )
                    for s, r, f in zip(sentences, rates, parts)
                )
            )

        asyncio.run(go())

        try:
            import numpy as np

            gap = np.zeros(int(FX_SR * 0.13), np.float32)
            chunks = []
            for f, k in zip(parts, semis):
                chunks += [pitch_shift(trim_silence(decode(f)), k), gap]
            x = np.concatenate(chunks)
            if self.fx > 0:
                x = fx_process(x, self.fx)
            write_wav(dst, x, volume=self.volume)
            return dst
        except Exception:
            if len(parts) == 1:
                return parts[0]
            asyncio.run(
                edge_tts.Communicate(
                    text, self.voice, rate=self.rate, pitch=self.pitch
                ).save(self.path)
            )
            return self.path

    def _cache_path(self, text):
        import hashlib

        key = "|".join(
            map(
                str,
                (
                    text,
                    self.engine,
                    self.voice,
                    self.rate,
                    self.pitch,
                    self.shift,
                    self.lively,
                    self.fx,
                    self.volume,
                ),
            )
        )
        return os.path.join(
            TTS_CACHE, hashlib.sha1(key.encode("utf-8")).hexdigest() + ".wav"
        )

    def cached_render(self, text):
        path = self._cache_path(text)
        if os.path.isfile(path):
            os.utime(path)
            return path
        out = self.render(text, self.fx_path)
        if out.endswith(".wav") and len(text) <= CACHE_MAX_TEXT:
            try:
                os.makedirs(TTS_CACHE, exist_ok=True)
                shutil.copyfile(out, path)
                _trim_cache()
            except OSError:
                pass
        return out

    def prewarm(self, phrases):
        for text in phrases:
            text = clean_for_speech(text)
            if not text or os.path.isfile(self._cache_path(text)):
                continue
            try:
                self.cached_render(text)
            except Exception:
                return
            time.sleep(0.5)

    def _sapi(self, text):
        import comtypes.client

        sp = comtypes.client.CreateObject("SAPI.SpVoice")
        sp.Volume = max(0, min(100, self.volume))
        sp.Speak(text, 1)
        while not sp.WaitUntilDone(50):
            if self.stop_event.is_set():
                sp.Speak("", 2)
                return True
        return False


def make_glitch_sound(path, volume=100):
    rnd = random.Random(42)
    frames = bytearray()
    sr = 22050
    n = int(sr * 0.22)
    freq = 900
    for i in range(n):
        if i % 600 == 0:
            freq = rnd.choice([300, 700, 1200, 1800, 2400])
        env = (1 - i / n) ** 1.5
        sq = 1 if math.sin(2 * math.pi * freq * i / sr) > 0 else -1
        noise = rnd.uniform(-1, 1) if (i // 400) % 3 == 0 else 0
        v = int(5500 * volume / 100 * env * (0.6 * sq + 0.5 * noise))
        frames += struct.pack("<h", max(-32767, min(32767, v)))
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(bytes(frames))


def play_sound_async(path):
    try:
        import winsound

        winsound.PlaySound(path, winsound.SND_FILENAME | winsound.SND_ASYNC)
    except Exception:
        pass
