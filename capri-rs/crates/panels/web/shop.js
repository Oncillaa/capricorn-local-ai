'use strict';

// Витрина магазина: скины, цены в ECS, покупка и надевание.
// Ядро присылает каталог и баланс в `state.save`, страница только рисует.

const VIOLET = '#b026ff';

/** Переключение вкладок левого меню. */
function selectTab(name) {
  document.querySelectorAll('.nav-item[data-tab]').forEach((node) => {
    node.classList.toggle('is-active', node.dataset.tab === name);
  });
  document.querySelectorAll('.pane[data-pane]').forEach((node) => {
    node.classList.toggle('is-active', node.dataset.pane === name);
  });
}

document.querySelectorAll('.nav-item[data-tab]').forEach((node) => {
  node.addEventListener('click', () => selectTab(node.dataset.tab));
});

/** Один скин на витрине. */
function skinCard(skin, save) {
  const esc = window.capri.esc;
  const owned = (save.owned_skins || []).includes(skin.id);
  const equipped = save.equipped_skin === skin.id;
  const affordable = (save.money || 0) >= skin.price;
  const color = skin.color || VIOLET;

  let button;
  if (equipped) {
    button = '<button class="btn is-on" disabled>Надет</button>';
  } else if (owned) {
    button = `<button class="btn btn-accent" data-action="equip" data-skin="${esc(skin.id)}">Надеть</button>`;
  } else if (affordable) {
    button = `<button class="btn btn-accent" data-action="buy" data-skin="${esc(skin.id)}">Купить</button>`;
  } else {
    const need = skin.price - (save.money || 0);
    button = `<button class="btn" disabled>Ещё ${need} ECS</button>`;
  }

  const badge = equipped ? '<span class="skin-badge">НАДЕТ</span>' : '';
  const classes = ['skin'];
  if (owned) classes.push('is-owned');
  if (equipped) classes.push('is-equipped');
  const price = skin.price === 0 ? 'бесплатно' : `${skin.price} ECS`;

  return `
    <div class="${classes.join(' ')}">
      ${badge}
      <div class="skin-preview" style="color: ${esc(color)}">
        <div class="skin-figure"></div>
      </div>
      <div class="skin-title">
        <span class="skin-name">${esc(skin.name)}</span>
        <span class="skin-price">${esc(price)}</span>
      </div>
      <div class="skin-note">${esc(skin.note)}</div>
      ${button}
    </div>`;
}

/** Отрисовать состояние, присланное ядром. */
function paint() {
  const state = window.capri.state || {};
  const save = state.save || {};
  const skins = save.skins || [];

  document.getElementById('chip-version').textContent = 'v' + (state.version || '—');
  document.getElementById('chip-bridge').textContent = 'мост: есть';
  document.getElementById('balance').textContent = (save.money || 0) + ' ECS';
  document.getElementById('phrase').textContent = save.phrase || '—';
  if (state.save_path) document.getElementById('save-path').textContent = state.save_path;

  const grid = document.getElementById('skins');
  grid.innerHTML = skins.length
    ? skins.map((skin) => skinCard(skin, save)).join('')
    : '<div class="empty">каталог пуст — ядро не прислало скины</div>';

  const owned = (save.owned_skins || []).length;
  const current = skins.find((skin) => skin.id === save.equipped_skin);
  window.capri.status(
    `витрина: ${skins.length} скинов · куплено ${owned} · надет «${current ? current.name : save.equipped_skin || '—'}»`,
  );

  // Подтверждение ядру: страница отрисовалась (видно в capri.log).
  window.capri.post('painted', { skins: skins.length, money: save.money || 0 });
}

window.capri.onAction('buy', (node) => {
  const id = node.dataset.skin;
  window.capri.post('shop-buy', { skin: id });
  node.disabled = true;
  node.textContent = 'Покупаю…';
});

window.capri.onAction('equip', (node) => {
  const id = node.dataset.skin;
  window.capri.post('shop-equip', { skin: id });
  node.disabled = true;
  node.textContent = 'Надеваю…';
});

document.addEventListener('DOMContentLoaded', () => {
  window.capri.status('ожидаю ядро…');
  window.capri.post('ready');
});
