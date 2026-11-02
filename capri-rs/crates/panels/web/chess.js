'use strict';

// Доска игр: шахматы и шашки на одной странице. Договор с оболочкой тот же,
// что у остальных панелей: `paint()` читает window.capri.state, ядро зовёт
// setState().
//
// Игра выбирается по тому, какое поле пришло: state.chess или state.checkers.
// Так ядру не нужно сообщать «какая игра сейчас», и разойтись они не могут.
//
// Доска у обеих игр одна: 64 кнопки, клетка хранит и шахматную фигуру, и
// шашку. Что именно рисовать — решает пришедшее состояние.
//
// Состояние шахмат приходит так:
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
// Состояние шашек:
//   state.checkers = {
//     cells:   [["","b",…], …] — 8 строк по 8, строка 0 сверху (8-я горизонталь);
//              "w"/"b" — простая, "W"/"B" — дамка, "" — пусто;
//     moves:   [{from:"c3", to:"d4"}],
//     turn:    "white" | "black",
//     outcome: null | "white" | "black" | "draw",
//     winner:  null | "white" | "black",
//     ply: 0, white: 12, black: 12,
//     my_side: "white", level: "normal",
//     thinking: true|false, comment: "фраза Капри", history: ["c3-d4", …],
//     money: 0, wins: 0, games: 0, draws: 0,
//   }
//
// Правила обеих игр целиком живут в Rust: страница только показывает и
// отправляет выбранный ход. Поэтому здесь нет ни генерации ходов, ни
// проверки шаха или боя — и разойтись с движком она не может.

/** Выбранная клетка: откуда игрок собирается ходить. */
let selected = null;
/** Последнее состояние от ядра. */
let view = {};
/** Отрисовка уже подтверждена ядру — чтобы не спамить лог. */
let painted = false;
/** Какая игра сейчас на доске: `chess` или `checkers`. */
let game = 'chess';

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
      // Тёмные поля — те, где (файл + номер горизонтали) чётные. В шахматах
      // это a1: файл 0, горизонталь 1, сумма 1 — нечётная. Значит a1 тёмное,
      // то есть «чётные» тут наоборот. Шашки ходят только по тёмным, и от
      // этого зависит вся раскраска доски — проверить легко: a1 тёмное.
      if ((file + rank) % 2 !== 0) cell.classList.add('is-dark');
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
  if (game === 'checkers') {
    onCheckerClick(square);
    return;
  }
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

// --- шашки ----------------------------------------------------------------
//
// Общая доска, но своя логика кликов и своя отрисовка. Всё, что касается
// правил, приходит от ядра: список ходов, позиция, итог.

/** Буква в клетке шашечной доски: «w», «b», «W», «B» или пусто. */
function checkerAt(square) {
  const file = square.charCodeAt(0) - 97;
  const rank = Number(square.slice(1));
  if (!(file >= 0 && file < 8 && rank >= 1 && rank <= 8)) return '';
  const rows = Array.isArray(view.cells) ? view.cells : [];
  const row = rows[8 - rank];
  return row && typeof row[file] === 'string' ? row[file] : '';
}

/** Моя ли это шашка. В шашках цвет читается по регистру буквы. */
function isMine(letter) {
  if (!letter) return false;
  return view.my_side === 'white'
    ? letter === 'w' || letter === 'W'
    : letter === 'b' || letter === 'B';
}

/**
 * Клик по полю в шашках.
 *
 * Порядок тот же, что в шахматах, и по той же причине: сначала проверяем,
 * не цель ли это уже выбранной шашки. У своего поля тоже есть ходы, и при
 * обратной проверке вместо выбора отправился бы ход из прежнего поля.
 */
function onCheckerClick(square) {
  if (!myTurn()) {
    window.capri.status(view.outcome ? 'партия закончена' : 'сейчас ходит Капри');
    return;
  }

  if (selected && targetsFrom(selected).some((m) => m.to === square)) {
    sendCheckerMove(selected, square);
    return;
  }

  if (selected === square) {
    selected = null;
    paintSelection();
    return;
  }

  if (isMine(checkerAt(square))) {
    selected = square;
    paintSelection();
    const count = targetsFrom(square).length;
    window.capri.status(count ? `у шашки ${count} ход(ов)` : 'этой шашкой ходить нечем');
  } else {
    selected = null;
    paintSelection();
  }
}

/** Отправить ход ядру. Правила и серию взятий проверит Rust. */
function sendCheckerMove(from, to) {
  window.capri.post('checkers-move', { from, to });
  selected = null;
  paintSelection();
  window.capri.status(`ход ${from}-${to}…`);
}

/** Нарисовать позицию шашек. */
function paintCheckers(state) {
  view = state.checkers || view;

  document.querySelectorAll('.square').forEach((cell) => {
    const letter = checkerAt(cell.dataset.square);
    // Шашка рисуется кругом через CSS, буква нужна только для цвета и дамки.
    cell.querySelector('.piece').textContent = '';
    cell.classList.toggle('is-checker', Boolean(letter));
    cell.classList.toggle('is-crown', letter === 'W' || letter === 'B');
    cell.classList.toggle('is-white', letter === 'w' || letter === 'W');
    cell.classList.toggle('is-black', letter === 'b' || letter === 'B');
    // Шаха в шашках не бывает — снимаем след от шахматной партии.
    cell.classList.remove('is-check');
    if (letter) {
      cell.dataset.color = letter === 'w' || letter === 'W' ? 'white' : 'black';
    } else {
      delete cell.dataset.color;
    }
  });

  // Последний ход: в истории он записан как «c3-d4» или «c3:d4».
  const last = Array.isArray(view.history) && view.history.length
    ? String(view.history[view.history.length - 1]).split(/[-:]/)
    : [];
  document.querySelectorAll('.square').forEach((cell) => {
    cell.classList.toggle('is-last', last.indexOf(cell.dataset.square) >= 0);
  });

  if (document.getElementById('chip-balance')) {
    document.getElementById('chip-balance').textContent = (Number(view.money) || 0) + ' ECS';
  }

  // Вместо шахматного материала — перевес по шашкам.
  const materialNode = document.getElementById('material');
  if (materialNode) {
    const diff = (Number(view.white) || 0) - (Number(view.black) || 0);
    materialNode.textContent =
      diff === 0
        ? 'шашки: поровну'
        : 'шашки: ' + (diff > 0 ? '+' : '') + diff + (diff > 0 ? ' у белых' : ' у чёрных');
  }

  const plyNode = document.getElementById('ply-count');
  if (plyNode) plyNode.textContent = 'ходов: ' + (Number(view.ply) || 0);

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

  const moves = Array.isArray(view.history) ? view.history : [];
  document.getElementById('move-count').textContent = String(moves.length);
  document.getElementById('moves').innerHTML = moves
    .map((text, index) => `<li>${window.capri.esc(String(index + 1))}. ${window.capri.esc(text)}</li>`)
    .join('');

  document.getElementById('comment').textContent = view.comment || '…';

  const score = document.getElementById('score');
  if (score) {
    const wins = Number(view.wins) || 0;
    const games = Number(view.games) || 0;
    const draws = Number(view.draws) || 0;
    score.textContent =
      'Счёт против Капри: ' + wins + ' побед · ' + (games - wins - draws) + ' поражений · ' + draws + ' ничьих';
  }

  const levelBtn = document.getElementById('level-btn');
  if (levelBtn) {
    const names = { easy: 'ЛЁГКАЯ', normal: 'ОБЫЧНАЯ', hard: 'СЛОЖНАЯ' };
    levelBtn.textContent = 'СЛОЖНОСТЬ: ' + (names[view.level] || 'ОБЫЧНАЯ');
  }

  const banner = document.getElementById('banner');
  if (view.outcome) {
    banner.hidden = false;
    banner.textContent =
      view.outcome === 'draw'
        ? 'НИЧЬЯ'
        : view.winner === 'white'
          ? 'БЕЛЫЕ ПОБЕДИЛИ'
          : 'ЧЁРНЫЕ ПОБЕДИЛИ';
  } else {
    banner.hidden = true;
    banner.textContent = '';
  }

  paintSelection();

  if (!painted) {
    painted = true;
    window.capri.post('painted', { items: moves.length });
  }

  window.capri.status(
    view.outcome
      ? 'партия закончена — нажми «Новая партия»'
      : view.thinking
        ? 'Капри думает…'
        : 'твой ход',
  );
}

/**
 * Переключить игру.
 *
 * Доска одна, меняется только её содержимое. Чистим её полностью: у шашек
 * нет ни шаха, ни материала, и остатки шахматных классов только путали бы.
 * Состояние нужной игры просим у ядра — страница его не выдумывает.
 */
function switchGame(next) {
  if (next === game) return;
  game = next;
  selected = null;
  painted = false;

  document.querySelectorAll('.game-tab').forEach((tab) => {
    tab.classList.toggle('is-active', tab.dataset.game === next);
  });

  document.querySelectorAll('.square').forEach((cell) => {
    cell.querySelector('.piece').textContent = '';
    cell.classList.remove(
      'is-checker', 'is-crown', 'is-white', 'is-black', 'is-check',
      'is-selected', 'is-target', 'is-last',
    );
    delete cell.dataset.color;
  });
  document.getElementById('moves').innerHTML = '';
  document.getElementById('move-count').textContent = '0';
  document.getElementById('banner').hidden = true;

  window.capri.status(next === 'checkers' ? 'шашки: прошу позицию…' : 'шахматы: прошу позицию…');
  // `ready` у панели один: игры различаются действием, а не страницей.
  window.capri.post(next === 'checkers' ? 'checkers-ready' : 'chess-ready');
}

/**
 * Привести вкладки и заголовок окна в соответствие с тем, что на доске.
 *
 * Без этого вкладка и доска могли разойтись: ядро решает, какое состояние
 * прислать, а подсветку вкладки никто не обновлял — и на шашечной доске
 * горела «ШАХМАТЫ».
 */
function markTab() {
  document.querySelectorAll('.game-tab').forEach((tab) => {
    tab.classList.toggle('is-active', tab.dataset.game === game);
  });
  document.title =
    'CAPRI // ИГРЫ - ' + (game === 'checkers' ? 'ШАШКИ' : 'ШАХМАТЫ');
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

  // Какая игра пришла, ту и рисуем. Ядро отдаёт ровно одно из двух полей,
  // поэтому «шахматы поверх шашек» получить нельзя.
  if (state.checkers) {
    game = 'checkers';
    paintCheckers(state);
    markTab();
    return;
  }
  if (state.chess) {
    game = 'chess';
  }
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
  markTab();

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
  // Партия начинается в той игре, которая сейчас на доске.
  const action = game === 'checkers' ? 'checkers-new' : 'chess-new';
  window.capri.post(action, { level: view.level || 'normal', side: view.my_side || 'white' });
});

window.capri.onAction('undo', () => {
  selected = null;
  // Отменяем пару: свой ход и ответ бота, иначе отменять придётся дважды.
  // В шашках отмены нет — там правило боя, и откат его ломает.
  if (game === 'checkers') {
    window.capri.status('в шашках отмены нет — начни новую партию');
    return;
  }
  window.capri.post('chess-undo');
});

// Сложность переключается по кругу одной кнопкой — как на макете, где нет
// выпадающего списка.
window.capri.onAction('cycle-level', () => {
  const order = ['easy', 'normal', 'hard'];
  const next = order[(order.indexOf(view.level) + 1) % order.length];
  const action = game === 'checkers' ? 'checkers-level' : 'chess-level';
  window.capri.post(action, { level: next });
});

// Вкладки игр: шахматы и шашки готовы, «Очко» честно говорит об этом.
document.querySelectorAll('.game-tab').forEach((tab) => {
  tab.addEventListener('click', () => {
    const next = tab.dataset.game;
    if (next !== 'chess' && next !== 'checkers') {
      window.capri.status('эта игра ещё в работе — загляни позже');
      return;
    }
    switchGame(next);
  });
});

// --- старт ----------------------------------------------------------------

document.addEventListener('DOMContentLoaded', () => {
  buildBoard();
  buildCoordinates();
  window.capri.status('ожидаю ядро…');
  window.capri.post('ready');
});
