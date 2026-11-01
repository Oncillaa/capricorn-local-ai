'use strict';

// Чат с Капри: лента сообщений и поле ввода.
//
// Всё, что видно, приходит из `state.chat`: история реплик, имя модели и
// признак «печатает». Страница сама ничего не решает — она только рисует и
// отправляет текст, а ответ приходит отдельным `setState`.

/** Прокрутить ленту вниз — к последнему сообщению. */
function scrollToBottom() {
  const feed = document.getElementById('feed');
  if (feed) feed.scrollTop = feed.scrollHeight;
}

/** Одно сообщение: своё справа, Капри слева. */
function bubble(message) {
  const mine = message.who === 'user';
  const who = mine ? 'ты' : 'Капри';
  const time = String(message.time || '');
  const text = window.capri.esc(message.text || '');
  return (
    `<div class="msg ${mine ? 'is-mine' : 'is-capri'}">` +
    `<div class="msg-head"><span class="msg-who">${window.capri.esc(who)}</span>` +
    (time ? `<span class="msg-time">${window.capri.esc(time)}</span>` : '') +
    `</div>` +
    `<div class="msg-text">${text}</div>` +
    (message.note ? `<div class="msg-note">${window.capri.esc(message.note)}</div>` : '') +
    `</div>`
  );
}

/** Отрисовать состояние, присланное ядром. */
function paint() {
  const state = window.capri.state || {};
  const chat = state.chat || {};
  const messages = Array.isArray(chat.messages) ? chat.messages : [];

  // Шапка: какая модель отвечает и есть ли связь.
  if (document.getElementById('chip-model')) {
    const node = document.getElementById('chip-model');
    node.textContent = 'модель: ' + (chat.model || '—');
    // Имя модели обрезано вёрсткой — целиком его видно в подсказке.
    node.title = chat.model || '';
  }
  if (document.getElementById('brain-line')) {
    const node = document.getElementById('brain-line');
    const provider = chat.provider && chat.provider !== 'нет' ? chat.provider : '';
    node.textContent = provider
      ? 'мозги: ' + provider + (chat.model ? ' · ' + chat.model : '')
      : 'мозги: не настроены';
    node.title = node.textContent;
  }
  if (document.getElementById('chip-state')) {
    const node = document.getElementById('chip-state');
    const on = Boolean(chat.available);
    node.textContent = on ? 'на связи' : 'молчит';
    node.className = 'chip ' + (on ? 'chip-green' : 'chip-alert');
  }

  // Статусбар: сколько реплик помним — иначе «ожидаю ядро…» висело бы вечно.
  if (document.getElementById('status-text')) {
    window.capri.status(messages.length ? 'в истории ' + messages.length + ' реплик' : 'готово');
  }

  // Лента. Пустую историю держим текстом-подсказкой.
  const feed = document.getElementById('feed');
  if (feed) {
    if (!messages.length) {
      feed.innerHTML =
        '<div class="chat-empty">' +
        '<div class="chat-empty-title">Ещё не разговаривали</div>' +
        '<div class="chat-empty-hint">Напиши что-нибудь — или спроси, на какой ' +
        'модели я работаю. Всё останется здесь после перезапуска.</div>' +
        '</div>';
    } else {
      feed.innerHTML = messages.map(bubble).join('');
      scrollToBottom();
    }
  }

  // «Печатает…» — отдельной строкой, чтобы лента не прыгала.
  const typing = document.getElementById('typing');
  if (typing) typing.hidden = !chat.thinking;

  // Пока Капри думает, кнопку не жмём: иначе два запроса подряд.
  const send = document.getElementById('send');
  if (send) send.disabled = Boolean(chat.thinking);
}

// --- ввод ----------------------------------------------------------------

/** Отправить то, что в поле, и очистить его. */
function submit() {
  const input = document.getElementById('input');
  if (!input) return;
  const text = String(input.value || '').trim();
  if (!text) return;

  const send = document.getElementById('send');
  if (send && send.disabled) return;

  window.capri.post('chat-send', { text });
  input.value = '';
  input.style.height = 'auto';
  window.capri.status('отправлено — жду ответ');
  if (send) send.disabled = true;
}

/** Поле ввода растёт под текст, но не выше пяти строк. */
function grow() {
  const input = document.getElementById('input');
  if (!input) return;
  input.style.height = 'auto';
  const line = 20;
  input.style.height = Math.min(input.scrollHeight, line * 5) + 'px';
}

document.addEventListener('DOMContentLoaded', () => {
  const input = document.getElementById('input');

  if (input) {
    input.addEventListener('input', grow);
    input.addEventListener('keydown', (event) => {
      // Enter отправляет, Shift+Enter — перенос строки (привычка из мессенджеров).
      if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        submit();
      }
    });
    input.focus();
  }

  window.capri.onAction('send', () => submit());
  window.capri.onAction('clear', () => {
    window.capri.post('chat-clear');
    window.capri.status('история очищена');
  });

  window.capri.status('ожидаю ядро…');
  window.capri.post('ready');
});
