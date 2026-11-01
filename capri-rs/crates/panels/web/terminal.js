'use strict';

// Терминал: команда уходит в PowerShell, вывод появляется построчно.
//
// Ядро зовёт `window.capri.line(payload)` на каждую строку — поэтому здесь
// есть `onLine`, а не только `paint`. Перерисовывать всю ленту на каждой
// строке нельзя: длинный вывод мигал бы и тормозил.

/** Сколько строк держим в ленте — столько же, сколько помнит ядро. */
const MAX_ROWS = 2000;

/** История введённых команд (стрелка вверх). */
const history = [];
let historyAt = -1;
/** Идёт ли команда прямо сейчас. */
let running = false;

function feed() {
  return document.getElementById('feed');
}

/** Есть ли в ленте хоть одна строка. */
function hasRows() {
  const node = feed();
  return Boolean(node && node.querySelector('.term-line'));
}

/** Прокрутить вниз, если человек не отлистал вверх. */
function keepPinned(node) {
  const near = node.scrollHeight - node.scrollTop - node.clientHeight < 60;
  if (near) node.scrollTop = node.scrollHeight;
}

/** Добавить строку вывода. */
function addLine(text, kind, err) {
  const node = feed();
  if (!node) return;

  const empty = document.getElementById('empty');
  if (empty) empty.remove();

  const row = document.createElement('div');
  const mark = kind === 'ok' ? 'ok' : kind === 'fail' ? 'fail' : kind === 'info' ? 'info' : 'plain';
  row.className = 'term-line is-' + mark + (err ? ' is-stderr' : '');
  row.textContent = text;
  node.appendChild(row);

  while (node.querySelectorAll('.term-line').length > MAX_ROWS) {
    const first = node.querySelector('.term-line');
    if (!first) break;
    first.remove();
  }
  keepPinned(node);
}

/** Служебная строка от самой страницы (не от процесса). */
function note(text) {
  addLine(text, 'info', false);
}

/** Показать или спрятать плашку «ЗАПУСКАЕТ». */
function setRunning(on, command) {
  running = on;
  const plate = document.getElementById('running');
  if (plate) plate.hidden = !on;
  if (document.getElementById('running-cmd')) {
    document.getElementById('running-cmd').textContent = command || '';
  }
  if (document.getElementById('run')) document.getElementById('run').disabled = on;
  if (document.getElementById('input')) document.getElementById('input').disabled = on;

  const chip = document.getElementById('chip-state');
  if (chip) {
    chip.textContent = on ? 'выполняется' : 'свободна';
    chip.className = 'chip ' + (on ? 'chip-gold' : 'chip-green');
  }
}

/** Бейджи итога: «6 passing ✓ · 212 мс». */
function badges(summary) {
  const node = document.getElementById('term-badges');
  if (!node) return;
  if (!summary) {
    node.textContent = '—';
    return;
  }
  const parts = [];
  if (summary.passed !== null && summary.passed !== undefined) {
    parts.push(summary.passed + ' прошло ✓');
  }
  if (summary.failed) parts.push(summary.failed + ' упало ✗');
  if (summary.duration_ms !== null && summary.duration_ms !== undefined) {
    parts.push(summary.duration_ms + ' мс');
  }
  node.textContent = parts.length ? parts.join(' · ') : '—';
}

/** Состояние от ядра. */
function paint() {
  const state = window.capri.state || {};
  const term = state.terminal || {};
  if (term.command && document.getElementById('term-command')) {
    document.getElementById('term-command').textContent = term.command;
  }
  if (term.running !== undefined) setRunning(Boolean(term.running), term.command);
  // Ядро ответило — значит связь живая, и «ожидаю ядро…» больше не правда.
  if (!running) window.capri.status('готова к работе');
}

/** Отправка строки от ядра. */
function onLine(payload) {
  if (payload.done) {
    setRunning(false, payload.command || '');
    const code = payload.code;
    if (payload.killed) {
      note('— остановлено —');
      window.capri.status('остановлено человеком');
    } else if (code !== null && code !== undefined) {
      note('— завершено, код ' + code + ' —');
      window.capri.status(code === 0 ? 'готово, код 0' : 'готово, код ' + code);
    } else {
      note('— завершено —');
      window.capri.status('готово');
    }
    if (payload.summary) badges(payload.summary);
    if (document.getElementById('input')) document.getElementById('input').focus();
    return;
  }
  addLine(payload.text || '', payload.kind || 'plain', Boolean(payload.err));
  if (payload.summary) badges(payload.summary);
}

/** Запустить то, что в поле ввода. */
function submit() {
  const input = document.getElementById('input');
  if (!input || running) return;
  const command = String(input.value || '').trim();
  if (!command) return;

  history.push(command);
  historyAt = history.length;

  addLine('PS> ' + command, 'info', false);
  setRunning(true, command);
  window.capri.status('выполняю…');
  window.capri.post('terminal-run', { command });
  input.value = '';
}

document.addEventListener('DOMContentLoaded', () => {
  const input = document.getElementById('input');

  if (input) {
    input.addEventListener('keydown', (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        submit();
        return;
      }
      // Стрелки листают историю команд, как в обычной консоли.
      if (event.key === 'ArrowUp') {
        event.preventDefault();
        if (!history.length) return;
        historyAt = Math.max(0, historyAt - 1);
        input.value = history[historyAt] || '';
      }
      if (event.key === 'ArrowDown') {
        event.preventDefault();
        if (!history.length) return;
        historyAt = Math.min(history.length, historyAt + 1);
        input.value = history[historyAt] || '';
      }
    });
    input.focus();
  }

  window.capri.onAction('run', () => submit());
  window.capri.onAction('cancel', () => {
    window.capri.post('terminal-cancel');
    window.capri.status('прошу остановить…');
  });
  window.capri.onAction('clear', () => {
    const node = feed();
    if (node) {
      node.innerHTML =
        '<div class="term-empty"><div class="term-empty-title">Терминал пуст</div>' +
        '<div class="term-empty-hint">Напиши команду ниже и нажми Enter.</div></div>';
    }
    badges(null);
    window.capri.status('лента очищена');
  });
  // «В main» по макету отправляет команду в главный терминал. У нас это
  // буфер обмена: своего «главного» терминала приложение не имеет.
  window.capri.onAction('to-main', () => {
    const input = document.getElementById('input');
    const command = String((input && input.value) || '').trim();
    if (!command) {
      window.capri.status('сначала напиши команду', true);
      return;
    }
    window.capri.post('terminal-copy', { command });
  });

  window.capri.status('ожидаю ядро…');
  window.capri.post('ready');
});
