'use strict';

// Экран «Уровни»: опыт, состояние, статистика, инвентарь и гардероб.
// Всё приходит из `state.save` — страница ничего не считает сама, кроме
// ширины полос.

/** Ширина полосы в процентах. */
function width(percent) {
  return Math.max(0, Math.min(100, Math.round(percent))) + '%';
}

/** Отрисовать состояние, присланное ядром. */
function paint() {
  const state = window.capri.state || {};
  const save = state.save || {};
  const esc = window.capri.esc;

  document.getElementById('chip-balance').textContent = (save.money || 0) + ' ECS';
  if (state.save_path) document.getElementById('save-path').textContent = state.save_path;

  // Опыт и уровень.
  const level = save.level || 1;
  const xp = save.xp || 0;
  const next = save.xp_to_next || 1;
  document.getElementById('level-number').textContent = level;
  document.getElementById('xp-bar').style.width = width((xp / next) * 100);
  document.getElementById('xp-text').textContent = `${xp} / ${next} опыта`;
  document.getElementById('xp-next').textContent = `до ${level + 1} уровня`;

  // Награды за следующий уровень — на макете они стоят строкой под опытом.
  const reward = save.level_reward || 70;
  document.getElementById('reward-money').textContent = `Уровень ${level + 1}: +${reward} ECS`;
  document.getElementById('reward-snack').textContent = `Уровень ${level + 1}: вкусняшка`;

  // Состояние.
  const joy = save.mood_joy || 0;
  const tired = save.mood_tired || 0;
  const joyLabel = save.joy_label || '';
  const tiredLabel = save.tired_label || '';
  document.getElementById('joy-bar').style.width = width(joy);
  document.getElementById('tired-bar').style.width = width(tired);
  document.getElementById('joy-label').textContent = joy + (joyLabel ? ' · ' + joyLabel : '');
  document.getElementById('tired-label').textContent =
    tired + (tiredLabel ? ' · ' + tiredLabel : '');

  // Подпись под портретом — короткая сводка состояния.
  const moodParts = [joyLabel, tiredLabel].filter(Boolean);
  document.getElementById('portrait-caption').textContent = moodParts.length
    ? 'Сейчас ' + moodParts.join(', ')
    : 'Сейчас спокойна';

  // Статистика — четыре плитки, как на макете.
  const stats = [
    ['Съедено', save.snacks || 0],
    ['Игр сыграно', save.games || 0],
    ['Твоих побед', save.wins || 0],
    ['Поглаживаний', save.pettings || 0],
  ];
  document.getElementById('stats').innerHTML = stats
    .map(
      ([label, value]) =>
        `<div class="stat"><b>${esc(value)}</b><span>${esc(label)}</span></div>`,
    )
    .join('');

  // Инвентарь: вкусняшки и прочее.
  const items = save.inventory || [];
  document.getElementById('inventory').innerHTML = items.length
    ? items.map((item) => `<span class="item">${esc(item)}</span>`).join('')
    : '<span class="empty">пусто — заработай уровень</span>';

  const snacks = save.snacks_available || 0;
  const eat = document.querySelector('[data-action="eat"]');
  if (eat) {
    eat.disabled = snacks <= 0;
    eat.textContent = snacks > 0 ? `Съесть вкусняшку (${snacks})` : 'Съесть вкусняшку';
  }

  // Гардероб: плашки-скины, надетый подсвечен.
  const skins = save.skins || [];
  const owned = save.owned_skins || [];
  document.getElementById('skins-owned').innerHTML = owned.length
    ? owned
        .map((id) => {
          const skin = skins.find((s) => s.id === id);
          const name = skin ? skin.name : id;
          const on = save.equipped_skin === id;
          return `<span class="skin ${on ? 'is-on' : ''}">${esc(name)}</span>`;
        })
        .join('')
    : '<span class="empty">только исходный вид</span>';

  window.capri.status(
    `уровень ${level} · ${joyLabel || '—'} · ${tiredLabel || '—'}`,
  );

  // Подтверждение ядру: страница отрисовалась (видно в capri.log).
  window.capri.post('painted', { level: level, items: items.length });
}

window.capri.onAction('eat', (node) => {
  window.capri.post('levels-eat');
  node.disabled = true;
  node.textContent = 'Жую…';
});

document.addEventListener('DOMContentLoaded', () => {
  window.capri.status('ожидаю ядро…');
  window.capri.post('ready');
});
