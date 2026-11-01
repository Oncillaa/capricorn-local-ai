'use strict';

// Доска шахмат. Договор с оболочкой тот же, что у остальных панелей:
// `paint()` читает window.capri.state, ядро зовёт setState().
//
// Состояние приходит так:
//   state.chess = {
//     squares: [{square:"e4", piece:"♟", kind:"pawn", color:"white"} | {square, piece:null}],
//     moves:   [{from:"e2", to:"e4", uci:"e2e4", text:"e2-e4"}],
//     turn:    "white" | "black",
//     in_check: true|false,
//     outcome: null | "мат" | "ничья",
//     winner:  null | "white" | "black",
//     level:   "normal",
//     my_side: "white",
//     material: { white: 3900, black: 3900 },
//     ply: 0,
//     last_move: "e2-e4" | null,
//     thinking: true|false,
//     comment: "фраза Капри",
//   }
//
// Правила целиком живут в Rust: страница только показывает и отправляет
// выбранный ход. Поэтому здесь нет ни генерации ходов, ни проверки шаха —
// и разойтись с движком она не может.

/** Выбранная клетка: откуда игрок собирается ходить. */
let selected = null;
/** Последнее состояние от ядра. */
let view = {};
/** Отрисовка уже подтверждена ядру — чтобы не спамить лог. */
let painted = false;

/** Построить 64 клетки один раз при загрузке. */
function buildBoard() {
  const board = document.getElementById('board');
  if (!board || board.children.length) return;

  // Восьмая горизонталь сверху — как на бумажной доске.
  for (let rank = 8; rank >= 1; rank -= 1) {
    for (let file = 0; file < 8; file += 1) {
      const cell = document.createElement('button');
      cell.className = 'square';
      cell.dataset.square = String.fromCharCode(97 + file) + rank;
      // Светлые и тёмные поля в шахматном порядке.
      if ((file + rank) % 2 === 0) cell.classList.add('is-dark');
      cell.innerHTML = '<span class="piece"></span>';
      cell.addEventListener('click', () => onCellClick(cell.dataset.square));
      board.appendChild(cell);
    }
  }
}

/**
 * Координаты по краям доски: цифры 8…1 слева, буквы a…h снизу.
 *
 * Раньше их не было вовсе — на макете они есть, и без них клетку
 * «e4» приходится считать в уме.
 */
function buildCoordinates() {
  const ranks = document.getElementById('board-ranks');
  const files = document.getElementById('board-files');
  if (ranks && !ranks.children.length) {
    for (let rank = 8; rank >= 1; rank -= 1) {
      const node = document.createElement('span');
      node.textContent = String(rank);
      ranks.appendChild(node);
    }
  }
  if (files && !files.children.length) {
    for (let file = 0; file < 8; file += 1) {
      const node = document.createElement('span');
      node.textContent = String.fromCharCode(97 + file);
      files.appendChild(node);
    }
  }
}

/** Легальные ходы из выбранной клетки — для подсветки. */
function targetsFrom(square) {
  const moves = Array.isArray(view.moves) ? view.moves : [];
  return moves.filter((m) => m.from === square);
}

/** Чей сейчас ход и можно ли ходить игроку. */
function myTurn() {
  if (view.outcome) return false;
  if (view.thinking) return false;
  return view.turn === view.my_side;
}

/** Клик по клетке: выбрать фигуру или отправить ход. */
function onCellClick(square) {
  if (!myTurn()) {
    window.capri.status(view.outcome ? 'партия закончена' : 'сейчас ходит Капри');
    return;
  }

  // Ход — это клик по клетке, которая входит в список целей выбранной
  // фигуры. Проверять «есть ли ходы у кликнутой клетки» нельзя: у своей
  // же фигуры они тоже есть, и тогда вместо выбора отправился бы ход
  // из прежней клетки в новую.
  if (selected && targetsFrom(selected).some((m) => m.to === square)) {
    sendMove(selected, square);
    return;
  }

  // Иначе: либо выбираем свою фигуру, либо снимаем выбор.
  if (selected === square) {
    selected = null;
    paintSelection();
    return;
  }

  const cell = document.querySelector(`.square[data-square="${square}"]`);
  const hasPiece = cell && cell.querySelector('.piece').textContent.trim();
  const mine = cell && cell.dataset.color === view.my_side;

  if (hasPiece && mine) {
    selected = square;
    paintSelection();
    const count = targetsFrom(square).length;
    window.capri.status(count ? `у фигуры ${count} ход(ов)` : 'этой фигурой ходить нечем');
  } else {
    selected = null;
    paintSelection();
  }
}

/** Отправить ход ядру. Правила проверит Rust. */
function sendMove(from, to) {
  window.capri.post('chess-move', { from, to });
  selected = null;
  paintSelection();
  window.capri.status(`ход ${from}-${to}…`);
}

/** Подсветка выбора и доступных полей. */
function paintSelection() {
  const cells = document.querySelectorAll('.square');
  const targets = selected ? targetsFrom(selected).map((m) => m.to) : [];

  cells.forEach((cell) => {
    const square = cell.dataset.square;
    cell.classList.toggle('is-selected', square === selected);
    cell.classList.toggle('is-target', targets.indexOf(square) >= 0);
  });
}

/** Нарисовать позицию. */
function paint() {
  const state = window.capri.state || {};
  view = state.chess || view;

  buildBoard();

  // Клетки.
  const squares = Array.isArray(view.squares) ? view.squares : [];
  for (const item of squares) {
    const cell = document.querySelector(`.square[data-square="${item.square}"]`);
    if (!cell) continue;
    const piece = cell.querySelector('.piece');
    piece.textContent = item.piece || '';
    // Цвет фигуры нужен для стилей (белые/чёрные) и для «моя ли это фигура».
    if (item.color) {
      cell.dataset.color = item.color;
      cell.classList.toggle('is-white', item.color === 'white');
      cell.classList.toggle('is-black', item.color === 'black');
    } else {
      delete cell.dataset.color;
      cell.classList.remove('is-white', 'is-black');
    }
    // Король под шахом — красная рамка.
    cell.classList.toggle(
      'is-check',
      Boolean(view.in_check) && item.kind === 'king' && item.color === view.turn,
    );
  }

  // Последний ход: подсветить откуда и куда.
  const last = String(view.last_move || '');
  const [lastFrom, lastTo] = last.split('-');
  document.querySelectorAll('.square').forEach((cell) => {
    const square = cell.dataset.square;
    cell.classList.toggle('is-last', square === lastFrom || square === lastTo);
  });

  // Верхние плашки: баланс ECS.
  if (document.getElementById('chip-balance')) {
    document.getElementById('chip-balance').textContent = (Number(view.money) || 0) + ' ECS';
  }

  // Материал: показываем перевес, а не два числа.
  const material = view.material || {};
  const white = Number(material.white) || 0;
  const black = Number(material.black) || 0;
  const diff = Math.round((white - black) / 100);
  const materialNode = document.getElementById('material');
  if (materialNode) {
    if (diff === 0) {
      materialNode.textContent = 'материал: =';
    } else {
      materialNode.textContent =
        'материал: ' + (diff > 0 ? '+' : '') + diff + (diff > 0 ? ' у белых' : ' у чёрных');
    }
  }

  const plyNode = document.getElementById('ply-count');
  if (plyNode) plyNode.textContent = 'ходов: ' + (Number(view.ply) || 0);

  // Строка «чей ход» — крупная плашка под карточкой Капри.
  const turnBar = document.getElementById('turn-bar');
  if (turnBar) {
    const mine = view.turn === view.my_side;
    turnBar.textContent = view.outcome
      ? 'партия закончена'
      : view.thinking
        ? 'Капри думает…'
        : mine
          ? 'твой ход (' + (view.my_side === 'white' ? 'белые' : 'чёрные') + ')'
          : 'ход Капри';
    turnBar.classList.toggle('is-mine', Boolean(mine) && !view.outcome && !view.thinking);
  }

  // Список ходов.
  const moves = Array.isArray(view.history) ? view.history : [];
  document.getElementById('move-count').textContent = String(moves.length);
  document.getElementById('moves').innerHTML = moves
    .map((text, index) => `<li>${window.capri.esc(String(index + 1))}. ${window.capri.esc(text)}</li>`)
    .join('');

  // Комментарий Капри.
  document.getElementById('comment').textContent = view.comment || '…';

  // Счёт по партиям.
  const score = document.getElementById('score');
  if (score) {
    const wins = Number(view.wins) || 0;
    const games = Number(view.games) || 0;
    const draws = Number(view.draws) || 0;
    score.textContent =
      'Счёт против Капри: ' + wins + ' побед · ' + (games - wins - draws) + ' поражений · ' + draws + ' ничьих';
  }

  // Кнопка сложности показывает текущий уровень и переключает его по кругу.
  const levelBtn = document.getElementById('level-btn');
  if (levelBtn) {
    const names = { easy: 'ЛЁГКАЯ', normal: 'ОБЫЧНАЯ', hard: 'СЛОЖНАЯ' };
    levelBtn.textContent = 'СЛОЖНОСТЬ: ' + (names[view.level] || 'ОБЫЧНАЯ');
  }

  // Итог партии — баннером поверх доски.
  const banner = document.getElementById('banner');
  if (view.outcome) {
    banner.hidden = false;
    banner.textContent = view.outcome === 'мат'
      ? 'МАТ · ' + (view.winner === 'white' ? 'белые победили' : view.winner === 'black' ? 'чёрные победили' : '')
      : 'НИЧЬЯ';
  } else {
    banner.hidden = true;
    banner.textContent = '';
  }

  paintSelection();

  // Подтверждение отрисовки: по этой строке в capri.log видно, что панель
  // живая, а не белое окно. Шлём один раз, иначе лог забьётся.
  if (!painted) {
    painted = true;
    window.capri.post('painted', { items: squares.filter((s) => s.piece).length });
  }

  window.capri.status(
    view.outcome
      ? 'партия закончена — нажми «Новая партия»'
      : view.thinking
        ? 'Капри думает…'
        : 'твой ход',
  );
}

// --- кнопки ---------------------------------------------------------------

window.capri.onAction('new-game', () => {
  selected = null;
  window.capri.post('chess-new', { level: view.level || 'normal', side: view.my_side || 'white' });
});

window.capri.onAction('undo', () => {
  selected = null;
  // Отменяем пару: свой ход и ответ бота, иначе отменять придётся дважды.
  window.capri.post('chess-undo');
});

// Сложность переключается по кругу одной кнопкой — как на макете, где нет
// выпадающего списка.
window.capri.onAction('cycle-level', () => {
  const order = ['easy', 'normal', 'hard'];
  const next = order[(order.indexOf(view.level) + 1) % order.length];
  window.capri.post('chess-level', { level: next });
});

// Вкладки игр: готовы только шахматы, остальные честно говорят об этом.
document.querySelectorAll('.game-tab').forEach((tab) => {
  tab.addEventListener('click', () => {
    const game = tab.dataset.game;
    if (game !== 'chess') {
      window.capri.status('эта игра ещё в работе — загляни позже');
      return;
    }
    document.querySelectorAll('.game-tab').forEach((t) => t.classList.remove('is-active'));
    tab.classList.add('is-active');
  });
});

// --- старт ----------------------------------------------------------------

document.addEventListener('DOMContentLoaded', () => {
  buildBoard();
  buildCoordinates();
  window.capri.status('ожидаю ядро…');
  window.capri.post('ready');
});
