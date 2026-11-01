'use strict';

// Панель перевода.
//
// Страница ничего не знает про OCR и провайдеров: она просит ядро «выдели и
// переведи», а ядро присылает результат через `window.capri.translate(...)`.
// Так вся работа с экраном и сетью остаётся в Rust, а здесь — только показ.
//
// Состояния: `idle` (пусто) → `busy` (выделение/распознавание/перевод) →
// `done` или `error`.

/** Текст перевода: нужен кнопке «Копировать». */
let translated = '';
/** Исходный текст: по нему кнопка «Ещё раз» просит перевод заново. */
let source = '';
/** Языки последнего перевода — для подписи. */
let last = { from: '', to: '' };

function el(id) {
  return document.getElementById(id);
}

/** Показать одно состояние из набора, спрятав остальные. */
function show(name) {
  const busy = el('tr-busy');
  const error = el('tr-error');
  const columns = el('tr-columns');
  const empty = el('tr-empty');
  if (busy) busy.hidden = name !== 'busy';
  if (error) error.hidden = name !== 'error';
  if (columns) columns.hidden = name !== 'done';
  if (empty) empty.hidden = name === 'done' || name === 'error' || name === 'busy';
  // Кнопка выделения не нужна, пока идёт работа: иначе легко запустить две.
  const pick = el('tr-pick');
  if (pick) pick.disabled = name === 'busy';
}

/** Строка в статусбаре. */
function state(text, alert) {
  const node = el('tr-state');
  if (!node) return;
  node.textContent = text;
  node.className = alert ? 'chip chip-alert' : 'chip chip-green';
}

/** Подпись «English → Русский» в шапке. */
function direction(from, to) {
  const node = el('tr-direction');
  if (!node) return;
  if (!from || !to) {
    node.textContent = 'ещё не переводила';
    return;
  }
  node.textContent = from + ' → ' + to;
}

/** Показать ошибку по-человечески. */
function fail(message) {
  show('error');
  const node = el('tr-error');
  if (node) node.textContent = message || 'не получилось перевести';
  state('ошибка', true);
  capri.status('перевод не удался', true);
}

/** Ядро присылает снимок настроек: берём из него язык и провайдера. */
function paint() {
  const state_ = (capri.state || {});
  const provider = el('tr-provider');
  if (provider && state_.translate_provider) {
    provider.textContent = state_.translate_provider;
  }
  const hint = el('tr-language-hint');
  if (hint && state_.translate_to) {
    hint.textContent = 'перевод на ' + state_.translate_to;
  }
}

/** Начать выделение области. */
function pick() {
  show('busy');
  if (el('tr-busy-text')) el('tr-busy-text').textContent = 'выделяю область…';
  state('выделение');
  capri.post('translate-pick');
}

/** Перевести уже распознанный текст заново. */
function again() {
  if (!source) {
    pick();
    return;
  }
  show('busy');
  if (el('tr-busy-text')) el('tr-busy-text').textContent = 'перевожу заново…';
  state('перевод');
  capri.post('translate-again', { text: source });
}

/** Результат распознавания и перевода — зовёт ядро. */
window.capri.translate = function (payload) {
  const data = payload || {};

  if (data.stage) {
    // Промежуточный этап: показываем, на чём именно задержка.
    show('busy');
    if (el('tr-busy-text')) el('tr-busy-text').textContent = data.stage;
    return;
  }

  if (data.error) {
    fail(data.error);
    return;
  }

  // Отмена выделения — не ошибка. Возвращаемся к тому, что было: если перевод
  // уже показывался, он и остаётся, а не пропадает из-за нажатого Esc.
  if (data.cancelled) {
    if (translated) {
      show('done');
      state('готова');
      capri.status('отменено');
    } else {
      show('idle');
      state('готова');
      capri.status('отменено');
    }
    return;
  }

  source = data.source || '';
  translated = data.text || '';
  last = { from: data.from_name || '', to: data.to_name || '' };

  if (el('tr-source')) el('tr-source').textContent = source;
  if (el('tr-target')) el('tr-target').textContent = translated;
  if (el('tr-from-label')) {
    el('tr-from-label').textContent = last.from
      ? 'Распознано (' + last.from + ')'
      : 'Распознанный текст';
  }
  if (el('tr-to-label')) {
    el('tr-to-label').textContent = last.to ? 'Перевод (' + last.to + ')' : 'Перевод';
  }
  if (el('tr-provider') && data.provider) el('tr-provider').textContent = data.provider;

  direction(last.from, last.to);
  show('done');
  state('готово');
  capri.status(
    data.cached ? 'из кэша' : 'переведено',
    false,
  );
};

// Кнопки.
capri.onAction('pick', pick);
capri.onAction('again', again);
capri.onAction('copy', () => {
  if (!translated) {
    capri.status('переводить пока нечего', true);
    return;
  }
  capri.post('translate-copy', { text: translated });
  capri.status('скопировано');
});

// Esc закрывает панель, как и в остальных окнах.
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') capri.post('close');
});

show('idle');
state('готова');

// Сообщаем ядру, что страница живая: без этого оно не пришлёт настройки
// (провайдера и язык) — и в шапке остались бы прочерки.
window.capri.post('ready');
