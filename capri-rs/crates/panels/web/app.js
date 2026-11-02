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

/**
 * Перейти в раздел по имени — снаружи, из ядра.
 *
 * Кольцевому меню нужно открыть не просто «Настройки», а сразу нужный
 * раздел: сектор «Напоминания» обязан приводить в напоминания. Если страница
 * ещё не собрана (ядро успело раньше загрузки), просьба ждёт в
 * `pendingSection` и применяется на `DOMContentLoaded`.
 */
capri.selectSection = function selectSectionFromOutside(key) {
  if (!key) return;
  if (document.readyState === 'loading') {
    capri.pendingSection = key;
    return;
  }
  if (typeof selectSection === 'function') selectSection(key);
};

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

// ---------------------------------------------------------------------------
// Голос, звук и магазин.
// ---------------------------------------------------------------------------

const ENGINE_NOTES = {
  edge: 'живые голоса из сети',
  sapi: 'встроенный в Windows, офлайн',
  silero: 'офлайн-модель',
};

/** Нарисовать список голосов. Без состояния рисуем заглушку, а не пустоту. */
function renderVoiceList(config) {
  const box = document.getElementById('voice-name');
  if (!box) return;
  const voices = Array.isArray(config && config.voices) ? config.voices : [];
  if (!voices.length) {
    box.innerHTML = '<span class="dim">список придёт от ядра</span>';
    return;
  }
  const current = (config && config.voice) || '';
  box.innerHTML = voices
    .map((voice) => {
      const on = voice.id === current ? ' is-on' : '';
      return `<button class="btn${on}" data-voice="${escapeHtml(voice.id)}" title="${escapeHtml(voice.id)}">${escapeHtml(voice.note || voice.id)}</button>`;
    })
    .join('');
}

/** Раздел «Голос»: движок, голос, живая интонация. */
function showVoice(config) {
  const engine = config.tts_engine || 'edge';
  document.querySelectorAll('#tts-engine .btn').forEach((btn) => {
    btn.classList.toggle('is-on', btn.dataset.engine === engine);
  });
  renderVoiceList(config);
  showFlag('voice-lively', config.voice_lively);
}

/** Подпись ползунка речи: «обычно» вместо «+0%» — понятнее без объяснений. */
function speechLabel(percent) {
  const value = Number(percent) || 0;
  if (value === 0) return 'обычно';
  return (value > 0 ? '+' : '') + value + ' %';
}

/** Раздел «Звук»: громкость, скорость, тон. */
function showSound(config) {
  const volume = document.getElementById('voice-volume');
  if (volume && document.activeElement !== volume) {
    volume.value = config.voice_volume != null ? config.voice_volume : 100;
  }
  const volumeValue = document.getElementById('voice-volume-value');
  if (volumeValue) {
    volumeValue.textContent =
      (config.voice_volume != null ? config.voice_volume : 100) + ' %';
  }

  const rate = document.getElementById('voice-rate');
  if (rate && document.activeElement !== rate) {
    rate.value = config.voice_rate || 0;
  }
  const rateValue = document.getElementById('voice-rate-value');
  if (rateValue) rateValue.textContent = speechLabel(config.voice_rate || 0);

  const pitch = document.getElementById('voice-pitch');
  if (pitch && document.activeElement !== pitch) {
    pitch.value = config.voice_pitch || 0;
  }
  const pitchValue = document.getElementById('voice-pitch-value');
  if (pitchValue) pitchValue.textContent = speechLabel(config.voice_pitch || 0);
}

/** Раздел «Магазин»: кошелёк и скины с кнопками. */
function showShop(state) {
  const save = (state && state.save) || {};
  const money = document.getElementById('shop-money');
  if (money) money.textContent = (save.money != null ? save.money : 0) + ' ECS';

  const level = document.getElementById('shop-level');
  if (level) {
    const xp = save.xp != null ? save.xp : 0;
    const next = save.xp_to_next != null ? save.xp_to_next : 0;
    level.textContent = `${save.level != null ? save.level : 1} · ${xp}/${next} XP`;
  }

  const box = document.getElementById('shop-skins');
  if (!box) return;
  const skins = Array.isArray(save.skins) ? save.skins : [];
  const owned = Array.isArray(save.owned_skins) ? save.owned_skins : [];
  const equipped = save.equipped_skin || '';
  box.innerHTML = skins
    .map((skin) => {
      const isOwned = skin.id === 'purple' || owned.includes(skin.id);
      const isOn = skin.id === equipped;
      const swatch = skin.color || '#8b5cf6';
      let button;
      if (isOn) {
        button = '<button class="btn" disabled>Надет</button>';
      } else if (isOwned) {
        button = `<button class="btn btn-accent" data-shop="equip" data-skin="${escapeHtml(skin.id)}">Надеть</button>`;
      } else {
        const poor = (save.money || 0) < skin.price;
        button = `<button class="btn${poor ? '' : ' btn-accent'}" data-shop="buy" data-skin="${escapeHtml(skin.id)}"${poor ? ' disabled' : ''}>${poor ? 'не хватает' : 'Купить'}</button>`;
      }
      return `<div class="skin-row${isOn ? ' is-on' : ''}">
        <span class="skin-dot" style="background:${swatch}"></span>
        <span class="skin-text"><b>${escapeHtml(skin.name)}</b><small>${escapeHtml(skin.note || '')}</small></span>
        <span class="skin-price">${skin.price > 0 ? skin.price + ' ECS' : 'бесплатно'}</span>
        ${button}
      </div>`;
    })
    .join('');
}

/** Напоминания: что ждёт своего часа и когда. */
function showReminders(state) {
  const box = document.getElementById('reminder-list');
  if (!box) return;
  const all = (state && state.reminders && state.reminders.items) || [];
  const waiting = all.filter((item) => !item.fired);

  const count = document.getElementById('reminder-count');
  if (count) {
    count.textContent = waiting.length
      ? `${waiting.length} ${plural(waiting.length, 'штука', 'штуки', 'штук')}`
      : 'пусто';
  }

  if (!waiting.length) {
    box.innerHTML = '<p class="hint">Пока ничего. Напиши выше — или просто попроси голосом.</p>';
    return;
  }

  box.innerHTML = waiting
    .map(
      (item) => `<div class="reminder-row">
        <span class="reminder-when">${escapeHtml(item.left || '')}</span>
        <span class="reminder-what">${escapeHtml(item.text || '')}</span>
        <button class="btn" data-reminder="${item.id}">Убрать</button>
      </div>`,
    )
    .join('');
}

/**
 * Список заметок: что записано и когда.
 *
 * Заметки, в отличие от напоминаний, не «ждут» и не срабатывают — они просто
 * лежат, поэтому и порядок в списке обратный: свежая мысль сверху.
 */
function showNotes(state) {
  const box = document.getElementById('note-list');
  if (!box) return;
  const all = ((state && state.notes && state.notes.items) || []).slice().reverse();

  const count = document.getElementById('note-count');
  if (count) {
    count.textContent = all.length
      ? `${all.length} ${plural(all.length, 'запись', 'записи', 'записей')}`
      : 'пусто';
  }

  if (!all.length) {
    box.innerHTML = '<p class="hint">Пока пусто. Напиши выше — или скажи «Капри, запиши …».</p>';
    return;
  }

  box.innerHTML = all
    .map(
      (item) => `<div class="reminder-row">
        <span class="reminder-when">${escapeHtml(item.when || '')}</span>
        <span class="reminder-what">${escapeHtml(item.text || '')}</span>
        <button class="btn" data-note="${item.id}">Убрать</button>
      </div>`,
    )
    .join('');
}

/** Форма слова по числу — чтобы «1 штука», «2 штуки», «5 штук». */
function plural(count, one, few, many) {
  const tail = count % 100;
  if (tail >= 11 && tail <= 14) return many;
  const last = count % 10;
  if (last === 1) return one;
  if (last >= 2 && last <= 4) return few;
  return many;
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
  showVoice(config);
  showSound(config);
  showShop(state);
  showReminders(state);
  showNotes(state);

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
  wireVoice();
  wireSound();
  wireShop();
  wireReminders();
  wireNotes();

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
    'voice-lively': 'живая интонация',
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

/**
 * Раздел «Голос»: движок, голос из списка, живая интонация.
 *
 * Список голосов рисуется из состояния — он приходит от ядра вместе с
 * названиями и описаниями, дублировать его на странице нельзя: разъедется.
 */
function wireVoice() {
  const engine = document.getElementById('tts-engine');
  if (engine) {
    engine.addEventListener('click', (event) => {
      const btn = event.target.closest('.btn');
      if (!btn || !btn.dataset.engine) return;
      engine.querySelectorAll('.btn').forEach((other) => other.classList.remove('is-on'));
      btn.classList.add('is-on');
      capri.post('set-behavior', { key: 'tts-engine', value: btn.dataset.engine });
      capri.status('голос: ' + (ENGINE_NOTES[btn.dataset.engine] || btn.dataset.engine));
    });
  }

  // Кнопки голосов перерисовываются вместе с состоянием, поэтому слушаем
  // контейнер, а не сами кнопки: иначе после первой же перерисовки клики
  // перестали бы работать.
  const voices = document.getElementById('voice-name');
  if (voices) {
    voices.addEventListener('click', (event) => {
      const btn = event.target.closest('.btn');
      if (!btn || !btn.dataset.voice) return;
      voices.querySelectorAll('.btn').forEach((other) => other.classList.remove('is-on'));
      btn.classList.add('is-on');
      // Запоминаем выбор и сразу даём послушать — иначе непонятно, что вышло.
      capri.state = capri.state || {};
      capri.state.config = capri.state.config || {};
      capri.state.config.voice = btn.dataset.voice;
      capri.post('set-behavior', { key: 'voice-name', value: btn.dataset.voice });
      capri.status('голос: ' + btn.textContent.trim());
    });
  }
}

/**
 * Раздел «Звук»: три ползунка и пробная фраза.
 *
 * Ползунки шлют события при каждом сдвиге, а каждое — запись в config.json,
 * поэтому ждём паузу. Подпись обновляем сразу: иначе ползунок «немой».
 */
function wireSound() {
  const sliders = [
    { id: 'voice-volume', key: 'voice-volume', label: 'voice-volume-value', suffix: ' %' },
    { id: 'voice-rate', key: 'voice-rate', label: 'voice-rate-value', speech: true },
    { id: 'voice-pitch', key: 'voice-pitch', label: 'voice-pitch-value', speech: true },
  ];

  sliders.forEach((slider) => {
    const input = document.getElementById(slider.id);
    if (!input) return;
    let timer = 0;
    input.addEventListener('input', () => {
      const label = document.getElementById(slider.label);
      if (label) {
        label.textContent = slider.speech ? speechLabel(input.value) : input.value + slider.suffix;
      }
      clearTimeout(timer);
      timer = setTimeout(() => {
        capri.post('set-behavior', { key: slider.key, value: Number(input.value) });
        capri.status('звук: ' + (label ? label.textContent : input.value));
      }, 160);
    });
  });

  const test = document.getElementById('voice-test');
  if (test) {
    test.addEventListener('click', () => {
      capri.post('set-behavior', { key: 'voice-test', value: true });
      capri.status('говорю пробную фразу…');
    });
  }
}

/** Раздел «Магазин»: покупка и надевание скина прямо из настроек. */
function wireShop() {
  const box = document.getElementById('shop-skins');
  if (!box) return;
  box.addEventListener('click', (event) => {
    const btn = event.target.closest('[data-shop]');
    if (!btn) return;
    const action = btn.dataset.shop === 'buy' ? 'shop-buy' : 'shop-equip';
    capri.post(action, { skin: btn.dataset.skin });
    btn.disabled = true;
    btn.textContent = btn.dataset.shop === 'buy' ? 'Покупаю…' : 'Надеваю…';
    // Состояние придёт от ядра на 'ready': кошелёк и галочка «надет»
    // обновятся сами, без перезагрузки страницы.
    setTimeout(() => capri.post('ready'), 400);
  });
}

/**
 * Раздел «Напоминания»: поставить, убрать одно, убрать все.
 *
 * Книга напоминаний живёт в ядре, поэтому страница не правит файл сама —
 * она просит ядро и через мгновение переспрашивает состояние. Так список
 * показывает то же, что Капри реально скажет.
 */
function wireReminders() {
  const add = document.getElementById('reminder-add');
  const input = document.getElementById('reminder-text');
  if (add && input) {
    const send = () => {
      const text = input.value.trim();
      if (!text) {
        capri.status('напиши, о чём напомнить');
        input.focus();
        return;
      }
      capri.post('reminder-add', { text });
      capri.status('напоминание поставлено');
      input.value = '';
      setTimeout(() => capri.post('ready'), 500);
    };
    add.addEventListener('click', send);
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        send();
      }
    });
  }

  const list = document.getElementById('reminder-list');
  if (list) {
    list.addEventListener('click', (event) => {
      const btn = event.target.closest('[data-reminder]');
      if (!btn) return;
      capri.post('reminder-cancel', { id: Number(btn.dataset.reminder) });
      btn.disabled = true;
      btn.textContent = 'Убираю…';
      setTimeout(() => capri.post('ready'), 500);
    });
  }

  const clear = document.getElementById('reminder-clear');
  if (clear) {
    clear.addEventListener('click', () => {
      capri.post('reminder-clear');
      capri.status('убираю все напоминания');
      setTimeout(() => capri.post('ready'), 500);
    });
  }
}

/** Заметки: записать строку, убрать одну, убрать все. */
function wireNotes() {
  const add = document.getElementById('note-add');
  const input = document.getElementById('note-text');
  if (add && input) {
    const send = () => {
      const text = input.value.trim();
      if (!text) {
        capri.status('напиши, что записать');
        input.focus();
        return;
      }
      capri.post('note-add', { text });
      capri.status('записала');
      input.value = '';
      setTimeout(() => capri.post('ready'), 500);
    };
    add.addEventListener('click', send);
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        send();
      }
    });
  }

  const list = document.getElementById('note-list');
  if (list) {
    list.addEventListener('click', (event) => {
      const btn = event.target.closest('[data-note]');
      if (!btn) return;
      capri.post('note-remove', { id: Number(btn.dataset.note) });
      btn.disabled = true;
      btn.textContent = 'Убираю…';
      setTimeout(() => capri.post('ready'), 500);
    });
  }

  const clear = document.getElementById('note-clear');
  if (clear) {
    clear.addEventListener('click', () => {
      capri.post('note-clear');
      capri.status('убираю все заметки');
      setTimeout(() => capri.post('ready'), 500);
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

  // Ядро могло попросить раздел до того, как страница собралась.
  if (capri.pendingSection) {
    const wanted = capri.pendingSection;
    capri.pendingSection = null;
    if (typeof selectSection === 'function') selectSection(wanted);
  }
});
