'use strict';

// ---------------------------------------------------------------------------
// Мост «страница ↔ ядро».
//
// Наружу — ровно два вызова:
//   window.ipc.postMessage(json)  — встроен в wry, отсюда уходит наверх;
//   window.capri.setState(obj)    — ядро зовёт это через evaluate_script.
//
// Никакого fetch к диску и никаких сетевых запросов: страница видит только
// вшитые в exe файлы через протокол capri://.
// ---------------------------------------------------------------------------

const capri = {
  state: null,

  /** Отправить действие ядру. */
  post(action, data = null) {
    try {
      window.ipc.postMessage(JSON.stringify({ action, data }));
    } catch (e) {
      this.status('мост недоступен: ' + e, true);
    }
  },

  /** Ядро присылает состояние. */
  setState(state) {
    this.state = state || {};
    paint();
  },

  /** Строка в статусбаре. */
  status(text, alert = false) {
    const node = document.getElementById('status-text');
    if (!node) return;
    node.textContent = text;
    node.className = alert ? 'chip-alert' : 'ok';
  },
};

window.capri = capri;

// ---------------------------------------------------------------------------
// Разделы левого меню. Порядок — как на скриншоте настроек.
// `phase` — фаза плана (ПЛАН.md), после которой раздел становится живым.
// ---------------------------------------------------------------------------

const SECTIONS = {
  settings: { title: 'Настройки' },
  behavior: { title: 'Поведение', note: 'болталка, перебивание, прогулки по экрану' },
  shop: { title: 'Магазин', phase: 3, note: 'витрина скинов, цены в ECS, баланс и покупка' },
  agents: { title: 'Агенты', phase: 7, note: 'инструменты ИИ и разрешения на запуск команд' },
  sound: { title: 'Звук', phase: 5, note: 'громкость голоса и сила цифровых эффектов' },
  voice: { title: 'Голос', phase: 5, note: 'движок озвучки (Edge TTS / SAPI), тембр, глитч' },
  translate: { title: 'Перевод', phase: 6, note: 'распознавание текста с экрана и провайдер перевода' },
  discord: { title: 'Discord', phase: 4, note: 'Application ID, иконки активности, что транслировать' },
  system: { title: 'Система', phase: 8, note: 'автозапуск, логи, обновления' },
  authors: { title: 'Авторы', phase: null, note: 'Капри 2.0 — MIT, фан-проект по Cyberpunk: Edgerunners' },
};

function stubMarkup(key) {
  const info = SECTIONS[key] || { title: key };
  const phase = info.phase ? `<span class="phase">Фаза ${info.phase}</span>` : '';
  const note = info.note ? `<p>${info.note}</p>` : '';
  const tail = info.phase
    ? '<p>Раздел подключается на фазе ' + info.phase + ' по плану <b>ПЛАН.md</b>.</p>'
    : '<p>Персонаж Капри и вселенная Cyberpunk: Edgerunners принадлежат правообладателям.</p>';
  return (
    `<h2 class="pane-title">${info.title}${phase}</h2>` +
    `<div class="stub">${note}${tail}</div>`
  );
}

// ---------------------------------------------------------------------------
// Отрисовка того, что пришло из ядра.
// ---------------------------------------------------------------------------

function fact(name, value) {
  return `<dt>${name}</dt><dd>${value == null || value === '' ? '—' : escapeHtml(String(value))}</dd>`;
}

function escapeHtml(text) {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

const LIVELINESS_LABELS = { calm: 'спокойная', normal: 'обычная', lively: 'живая' };

/** Ползунок и подпись — в одно значение. */
function showSize(percent) {
  const size = document.getElementById('size-range');
  const label = document.getElementById('size-value');
  const value = Math.min(100, Math.max(1, Math.round(Number(percent) || 100)));
  if (size) {
    size.value = String(value);
    size.style.setProperty('--fill', value + '%');
  }
  if (label) label.textContent = value + ' %';
  document.querySelectorAll('#size-presets .btn').forEach((btn) => {
    btn.classList.toggle('is-on', Number(btn.dataset.scale) === value);
  });
}

/** Показать, какая живость выбрана. */
function showLiveliness(id) {
  document.querySelectorAll('#liveliness .btn').forEach((btn) => {
    btn.classList.toggle('is-on', btn.dataset.liveliness === id);
  });
}

/** Переключатель «вкл/выкл»: подсветить выбранную кнопку. */
function showFlag(id, on) {
  document.querySelectorAll(`#${id} .btn`).forEach((btn) => {
    btn.classList.toggle('is-on', btn.dataset.on === (on ? '1' : '0'));
  });
}

/** Ползунок с подписью в минутах. */
function showMinutes(id, minutes, suffix) {
  const range = document.getElementById(id);
  const label = document.getElementById(id + '-value');
  const value = Math.max(1, Math.min(120, Math.round(Number(minutes) || 1)));
  if (range) {
    range.value = String(value);
    range.style.setProperty('--fill', Math.round((value / 120) * 100) + '%');
  }
  if (label) label.textContent = value + ' ' + suffix;
}

/** Ползунок с подписью в процентах. */
function showPercent(id, percent) {
  const range = document.getElementById(id);
  const label = document.getElementById(id + '-value');
  const value = Math.max(0, Math.min(100, Math.round(Number(percent) || 0)));
  if (range) {
    range.value = String(value);
    range.style.setProperty('--fill', value + '%');
  }
  if (label) label.textContent = value + ' %';
}

// ---------------------------------------------------------------------------
// Перевод: язык и провайдер.
// ---------------------------------------------------------------------------

/**
 * Языки, на которые имеет смысл переводить.
 *
 * Список повторяет `capri_core::translate::LANGUAGES`, но здесь он нужен
 * целиком: кнопки рисуются на странице, а не приходят из ядра — иначе при
 * каждом открытии настроек пришлось бы ждать ответа, чтобы показать кнопки.
 */
const TRANSLATE_LANGUAGES = [
  ['ru', 'Русский'],
  ['en', 'Английский'],
  ['de', 'Немецкий'],
  ['fr', 'Французский'],
  ['es', 'Испанский'],
  ['it', 'Итальянский'],
  ['uk', 'Украинский'],
  ['tr', 'Турецкий'],
  ['zh', 'Китайский'],
  ['ja', 'Японский'],
];

/** Нарисовать кнопки языков. Вызывается один раз при загрузке страницы. */
function renderTranslateLanguages() {
  const box = document.getElementById('translate-to');
  if (!box || box.dataset.ready) return;
  box.innerHTML = TRANSLATE_LANGUAGES.map(
    ([code, name]) => `<button class="btn" data-code="${code}">${name}</button>`
  ).join('');
  box.dataset.ready = '1';
}

/** Подставить текущие настройки перевода в уже нарисованные элементы. */
function showTranslate(config) {
  const current = config.translate_code || 'ru';
  const box = document.getElementById('translate-to');
  if (box) {
    box.querySelectorAll('.btn').forEach((btn) => {
      btn.classList.toggle('is-on', btn.dataset.code === current);
    });
  }
  const url = document.getElementById('translate-url');
  // Поле нельзя перезаписывать, пока в нём печатают: иначе курсор прыгнет.
  if (url && document.activeElement !== url && !url.dataset.dirty) {
    url.value = config.translate_url || '';
  }
}

/** Список мониторов: сколько их, такой и список. */
function showMonitors(monitors, current) {
  const select = document.getElementById('monitor');
  if (!select) return;
  const list = Array.isArray(monitors) ? monitors : [];
  select.innerHTML = list
    .map((monitor) => {
      const label = monitor.label || `Монитор ${monitor.index + 1}`;
      const selected = Number(monitor.index) === Number(current) ? ' selected' : '';
      return `<option value="${monitor.index}"${selected}>${escapeHtml(label)}</option>`;
    })
    .join('');
  // Монитор мог отключиться — тогда показываем то, что выбрало ядро.
  if (list.length) select.value = String(current);
  select.disabled = list.length < 2;
}

function paint() {
  const state = capri.state || {};
  const config = state.config || {};

  const version = document.getElementById('chip-version');
  if (version) version.textContent = 'v' + (state.version || '—');

  const bridge = document.getElementById('chip-bridge');
  if (bridge) {
    bridge.textContent = 'мост: WebView2 · активен';
    bridge.className = 'chip chip-green';
  }

  const path = document.getElementById('config-path');
  if (path) {
    path.textContent = state.config_path || '';
    path.title = state.config_path || '';
  }

  showSize(config.character_scale);
  showLiveliness(config.liveliness || 'lively');
  showMonitors(state.monitors, config.monitor_index || 0);

  // Раздел «Поведение»: флаги и два интервала.
  showFlag('chatter', config.chatter);
  showFlag('barge-in', config.barge_in);
  showFlag('greet-on-start', config.greet_on_start);
  showFlag('search-when-unsure', config.search_when_unsure);
  showMinutes('chatter-min', config.chatter_min_minutes, 'мин');
  showMinutes('chatter-max', config.chatter_max_minutes, 'мин');
  showPercent('roam', config.roam_percent);
  showTranslate(config);

  const facts = document.getElementById('facts');
  if (facts) {
    const ai = config.local_ai
      ? 'локальная (Ollama/LM Studio)'
      : config.enable_ai
        ? 'внешняя (Claude)'
        : 'выключена';
    facts.innerHTML = [
      fact('Движок ИИ', ai),
      fact('Модель', config.local_model || config.model),
      fact('Озвучка', config.tts_engine),
      fact('Голос', config.voice),
      fact('Громкость голоса', config.voice_volume != null ? config.voice_volume + ' %' : null),
      fact('Микрофон', config.mic_device || 'системный'),
      fact('Слово-будильник', (config.wake_words || []).join(', ')),
      fact('Живость', LIVELINESS_LABELS[config.liveliness] || config.liveliness),
      fact('Каталог', state.base_dir),
    ].join('');
  }

  capri.status('состояние получено от ядра');
}

// ---------------------------------------------------------------------------
// Меню, ползунок, кнопки, клавиши.
// ---------------------------------------------------------------------------

function selectSection(key) {
  document.querySelectorAll('.nav-item').forEach((item) => {
    item.classList.toggle('is-active', item.dataset.section === key);
  });
  document.querySelectorAll('.pane').forEach((pane) => {
    pane.classList.toggle('is-active', pane.dataset.pane === key);
  });
}

/**
 * Отправить размер ядру, не заваливая его сообщениями.
 *
 * Ползунок при перетаскивании сыплет событиями, а каждое — это запись в
 * config.json. Поэтому ждём паузу в 120 мс и отправляем только последнее.
 */
let sizeTimer = 0;
function sendSize(percent) {
  showSize(percent);
  clearTimeout(sizeTimer);
  sizeTimer = setTimeout(() => {
    capri.post('set-scale', Number(percent));
    capri.status('размер: ' + Math.round(percent) + ' %');
  }, 120);
}

function wire() {
  document.querySelectorAll('.nav-item').forEach((item) => {
    item.addEventListener('click', () => selectSection(item.dataset.section));
  });

  // Кнопки вида «Открыть config.json».
  document.querySelectorAll('[data-action]').forEach((node) => {
    node.addEventListener('click', () => {
      const action = node.dataset.action;
      const data = node.dataset.value != null ? node.dataset.value : null;
      capri.post(action, data);
      capri.status('отправлено: ' + action);
    });
  });

  const size = document.getElementById('size-range');
  if (size) size.addEventListener('input', () => sendSize(size.value));

  document.querySelectorAll('#size-presets .btn').forEach((btn) => {
    btn.addEventListener('click', () => sendSize(btn.dataset.scale));
  });

  document.querySelectorAll('#liveliness .btn').forEach((btn) => {
    btn.addEventListener('click', () => {
      const value = btn.dataset.liveliness;
      showLiveliness(value);
      capri.post('set-liveliness', value);
      capri.status('живость: ' + (LIVELINESS_LABELS[value] || value));
    });
  });

  const monitor = document.getElementById('monitor');
  if (monitor) {
    monitor.addEventListener('change', () => {
      capri.post('set-monitor', Number(monitor.value));
      capri.status('экран: ' + monitor.options[monitor.selectedIndex].text);
    });
  }

  wireBehavior();
  wireTranslate();

  // Esc закрывает панель — как у любого нормального окна настроек.
  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') capri.post('close');
  });
}

/**
 * Раздел «Поведение»: флаги и интервалы.
 *
 * Флаги уходят сразу, ползунки — с паузой: тянуть ползунок и писать в
 * config.json на каждый пиксель незачем.
 */
function wireBehavior() {
  const FLAGS = {
    chatter: 'болталка',
    'barge-in': 'перебивание',
    'greet-on-start': 'приветствие',
    'search-when-unsure': 'поиск в сети',
  };
  Object.keys(FLAGS).forEach((id) => {
    document.querySelectorAll(`#${id} .btn`).forEach((btn) => {
      btn.addEventListener('click', () => {
        const on = btn.dataset.on === '1';
        showFlag(id, on);
        capri.post('set-behavior', { key: id, value: on });
        capri.status(FLAGS[id] + ': ' + (on ? 'включено' : 'выключено'));
      });
    });
  });

  const sendMinutes = (() => {
    let timer = 0;
    return () => {
      const min = Number((document.getElementById('chatter-min') || {}).value || 15);
      const max = Number((document.getElementById('chatter-max') || {}).value || 40);
      showMinutes('chatter-min', min, 'мин');
      showMinutes('chatter-max', max, 'мин');
      clearTimeout(timer);
      timer = setTimeout(() => {
        capri.post('set-behavior', {
          key: 'chatter-minutes',
          value: Math.min(min, max),
          second: Math.max(min, max),
        });
        capri.status('болталка: раз в ' + Math.min(min, max) + '–' + Math.max(min, max) + ' мин');
      }, 160);
    };
  })();
  ['chatter-min', 'chatter-max'].forEach((id) => {
    const range = document.getElementById(id);
    if (range) range.addEventListener('input', sendMinutes);
  });

  const roam = document.getElementById('roam');
  if (roam) {
    let timer = 0;
    roam.addEventListener('input', () => {
      showPercent('roam', roam.value);
      clearTimeout(timer);
      timer = setTimeout(() => {
        capri.post('set-behavior', { key: 'roam-chance', value: Number(roam.value) });
        capri.status('шанс прогулки: ' + roam.value + ' %');
      }, 160);
    });
  }
}

/**
 * Раздел «Перевод»: язык и адрес LibreTranslate.
 *
 * Язык уходит сразу — это одна кнопка. Адрес пишется с паузой: человек
 * набирает его посимвольно, и сохранять каждый промежуточный «http://1» в
 * config.json незачем.
 */
function wireTranslate() {
  // Кнопки рисуем сразу, не дожидаясь состояния от ядра: иначе раздел открывался
  // бы пустым, если ядро занято.
  renderTranslateLanguages();

  const box = document.getElementById('translate-to');
  if (box) {
    box.addEventListener('click', (event) => {
      const btn = event.target.closest('.btn');
      if (!btn || !btn.dataset.code) return;
      box.querySelectorAll('.btn').forEach((other) => other.classList.remove('is-on'));
      btn.classList.add('is-on');
      capri.post('set-behavior', { key: 'translate-to', value: btn.dataset.code });
      capri.status('перевод: ' + btn.textContent.trim().toLowerCase());
    });
  }

  const url = document.getElementById('translate-url');
  if (url) {
    let timer = 0;
    url.addEventListener('input', () => {
      url.dataset.dirty = '1';
      clearTimeout(timer);
      timer = setTimeout(() => {
        const value = url.value.trim();
        capri.post('set-behavior', { key: 'translate-url', value });
        capri.status(value ? 'LibreTranslate: ' + value : 'LibreTranslate выключен');
        // После ответа ядра поле снова можно обновлять из состояния.
        delete url.dataset.dirty;
      }, 700);
    });
    url.addEventListener('blur', () => {
      delete url.dataset.dirty;
    });
  }
}

document.addEventListener('DOMContentLoaded', () => {
  // Заглушки для разделов, у которых ещё нет своей разметки.
  document.querySelectorAll('.pane[data-stub]').forEach((pane) => {
    pane.innerHTML = stubMarkup(pane.dataset.pane);
  });
  wire();
  capri.status('ожидаю состояние от ядра…');
  capri.post('ready');
});
