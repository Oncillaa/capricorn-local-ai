'use strict';

// Мост «страница ↔ ядро», общий для панелей магазина и уровней.
//
// Наружу — ровно два вызова:
//   window.ipc.postMessage(json)  — встроен в wry, отсюда уходит наверх;
//   window.capri.setState(obj)    — ядро зовёт это через evaluate_script.
// Никакого fetch к диску и никаких сетевых запросов: страница видит только
// вшитые в exe файлы через протокол capri://.

window.capri = window.capri || {
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
    if (typeof paint === 'function') paint();
  },

  /**
   * Строка потока: ядро зовёт это по мере появления вывода.
   * В отличие от `setState` не перерисовывает всё заново, а отдаёт одну
   * строку — иначе длинный вывод мигал бы на каждой строке.
   */
  line(payload) {
    if (typeof onLine === 'function') onLine(payload || {});
  },

  /** Строка в статусбаре. */
  status(text, alert = false) {    const node = document.getElementById('status-text');
    if (!node) return;
    node.textContent = text;
    node.className = alert ? 'chip-alert' : 'ok';
  },

  /** Экранирование для innerHTML. */
  esc(value) {
    return String(value === null || value === undefined ? '' : value).replace(
      /[&<>"]/g,
      (s) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[s]),
    );
  },

  /** Обработчик клика по элементам с data-action="name". */
  onAction(name, handler) {
    document.addEventListener('click', (event) => {
      const node = event.target.closest('[data-action="' + name + '"]');
      if (node) handler(node, event);
    });
  },
};
