'use strict';

// Медиа-виджет: что играет, обложка, эквалайзер и управление плеером.
//
// Договор с оболочкой тот же, что у магазина и уровней:
//   paint() читает window.capri.state и рисует;
//   оболочка зовёт window.capri.setState(state) — мост сам вызывает paint().
// Оболочка шлёт два вида состояния:
//   { media: {...} }  — трек целиком (раз в полсекунды);
//   { levels: [...] } — полосы эквалайзера (двадцать раз в секунду).
// Поэтому paint() дорисовывает поверх уже известного, а не переписывает всё.

/** Как часто просим у ядра свежие полосы. */
const LEVELS_EVERY_MS = 80;

/** Последний известный трек: обновляется, только когда пришёл `media`. */
let track = {};

function formatTime(seconds) {
  const value = Number(seconds);
  if (!Number.isFinite(value) || value < 0) return '0:00';
  const total = Math.floor(value);
  return Math.floor(total / 60) + ':' + String(total % 60).padStart(2, '0');
}

/** Отрисовать полосу перемотки и подписи времени. */
function paintTimeline() {
  const duration = Number(track.duration) || 0;
  const position = Math.max(0, Number(track.position) || 0);

  document.getElementById('time-current').textContent = formatTime(position);
  document.getElementById('time-total').textContent = formatTime(duration);

  const share = duration > 0 ? Math.min(1, position / duration) : 0;
  const percent = (share * 100).toFixed(2) + '%';
  document.getElementById('seek-fill').style.width = percent;
  document.getElementById('seek-knob').style.left = percent;

  const seek = document.getElementById('seek');
  seek.setAttribute('aria-valuenow', String(Math.round(share * 100)));
  seek.setAttribute('aria-valuetext', formatTime(position) + ' из ' + formatTime(duration));
}

/** Эквалайзер: значения 0.0…1.0, пустой массив — звука нет. */
function paintBands(levels) {
  const bars = document.querySelectorAll('.equalizer i');
  if (!bars.length) return;

  const values = Array.isArray(levels) ? levels : [];
  for (let i = 0; i < bars.length; i += 1) {
    const raw = Number(values[i]);
    const value = Number.isFinite(raw) ? Math.min(1, Math.max(0, raw)) : 0;
    // Минимум 3 % — чтобы дорожки были видны даже в полной тишине.
    bars[i].style.height = (3 + value * 97).toFixed(1) + '%';
    bars[i].classList.toggle('is-hot', value > 0.62);
  }
}

function paintCover(cover) {
  const img = document.getElementById('cover-img');
  const fallback = document.getElementById('cover-fallback');
  if (cover) {
    img.src = cover;
    img.hidden = false;
    fallback.hidden = true;
  } else {
    img.removeAttribute('src');
    img.hidden = true;
    fallback.hidden = false;
  }
}

/** Отрисовать состояние, присланное ядром. */
function paint() {
  const state = window.capri.state || {};

  // Пришёл трек — запоминаем. Пришли только полосы — рисуем их и уходим:
  // трек не перерисовываем, иначе эквалайзер дёргал бы обложку и текст.
  if (state.media) {
    track = state.media;
  }
  if (state.levels !== undefined) {
    paintBands(state.levels);
  }
  if (!state.media) {
    // Состояния трека в этом вызове нет — либо оно уже нарисовано раньше,
    // либо ещё ни разу не приходило. В обоих случаях перерисовывать нечего.
    return;
  }

  const title = String(track.title || '').trim();
  document.getElementById('track-title').textContent = title || 'Ничего не играет';
  document.getElementById('track-artist').textContent =
    String(track.artist || '').trim() || 'включи что-нибудь — и я подпою';

  const app = String(track.app || '').trim();
  // На макете справа стоит просто имя плеера («ЯНДЕКС МУЗЫКА»), без «плеер:».
  document.getElementById('chip-source').textContent = app || '—';

  document.getElementById('track-state').textContent = title
    ? track.playing
      ? 'играет'
      : 'на паузе'
    : '';

  const playpause = document.getElementById('playpause');
  playpause.textContent = track.playing ? '⏸' : '▶';
  playpause.setAttribute('aria-label', track.playing ? 'Пауза' : 'Воспроизвести');
  playpause.classList.toggle('is-paused', !track.playing);

  paintTimeline();
  paintCover(String(track.cover || ''));

  window.capri.status(
    title ? (track.playing ? '▶ ' : '❚❚ ') + title + (app ? ' · ' + app : '') : 'плеер молчит',
  );
}

// ---------------------------------------------------------------------------
// Перемотка: тянем ползунок, отпускаем — отправляем секунды ядру.
// ---------------------------------------------------------------------------

let dragging = false;

function seekTo(clientX) {
  const box = document.getElementById('seek').getBoundingClientRect();
  if (box.width <= 0) return 0;
  const share = Math.min(1, Math.max(0, (clientX - box.left) / box.width));
  const duration = Number(track.duration) || 0;

  document.getElementById('seek-fill').style.width = share * 100 + '%';
  document.getElementById('seek-knob').style.left = share * 100 + '%';
  document.getElementById('time-current').textContent = formatTime(share * duration);
  return share * duration;
}

document.getElementById('seek').addEventListener('pointerdown', (event) => {
  dragging = true;
  document.getElementById('seek').setPointerCapture(event.pointerId);
  seekTo(event.clientX);
});

document.getElementById('seek').addEventListener('pointermove', (event) => {
  if (dragging) seekTo(event.clientX);
});

document.getElementById('seek').addEventListener('pointerup', (event) => {
  if (!dragging) return;
  dragging = false;
  const seconds = seekTo(event.clientX);
  window.capri.post('media-seek', { seconds: Math.max(0, Math.round(seconds)) });
  window.capri.status('перематываю на ' + formatTime(seconds));
});

// Клавиатурой по полосе: стрелки — по пять секунд.
document.getElementById('seek').addEventListener('keydown', (event) => {
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
  event.preventDefault();
  const duration = Number(track.duration) || 0;
  const step = event.key === 'ArrowRight' ? 5 : -5;
  const target = Math.min(duration, Math.max(0, (Number(track.position) || 0) + step));
  window.capri.post('media-seek', { seconds: Math.round(target) });
});

// ---------------------------------------------------------------------------
// Кнопки управления.
// ---------------------------------------------------------------------------

window.capri.onAction('playpause', () => window.capri.post('media-playpause'));
window.capri.onAction('next', () => window.capri.post('media-next'));
window.capri.onAction('prev', () => window.capri.post('media-prev'));

// ---------------------------------------------------------------------------
// Старт.
// ---------------------------------------------------------------------------

document.addEventListener('DOMContentLoaded', () => {
  window.capri.status('ожидаю ядро…');
  window.capri.post('ready');

  // Эквалайзер живёт своим таймером: оболочка тоже шлёт полосы, но ручной
  // запрос делает виджет отзывчивым сразу после переоткрытия окна.
  setInterval(() => window.capri.post('media-levels'), LEVELS_EVERY_MS);
});
