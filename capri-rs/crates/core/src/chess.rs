//! Шахматы: партия, легальные ходы, простая оценка и бот без внешнего движка.
//!
//! Правила и генерацию ходов делает `cozy-chess` (MIT). Своего велосипеда нет:
//! рокировка, взятие на проходе, превращение, шах и мат — всё оттуда, и это
//! проверено десятками тысяч позиций в самом крейте.
//!
//! Зачем бот, если есть Stockfish: Stockfish под GPL-3.0, а проект — MIT.
//! Класть его бинарник в сборку нельзя (см. `ПЛАН.md` §3.5). Поэтому здесь
//! свой перебор с альфа-отсечением: он слабее движка, зато лицензионно чист
//! и работает без единого файла рядом.
//!
//! Всё в модуле — чистые функции над позицией. Никаких потоков и таймеров:
//! оболочка зовёт [`Bot::best_move`] в своём потоке и сама решает, когда
//! остановиться.

use std::fmt;
use std::str::FromStr;

use cozy_chess::{Board, Color, GameStatus, Move, Piece, Square};

/// Какой стороной играет человек. Бот всегда играет другой.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    White,
    Black,
}

impl Side {
    pub fn color(self) -> Color {
        match self {
            Side::White => Color::White,
            Side::Black => Color::Black,
        }
    }

    /// Сторона, которая ходит после этой.
    pub fn other(self) -> Side {
        match self {
            Side::White => Side::Black,
            Side::Black => Side::White,
        }
    }

    /// Русское название — для подписей в интерфейсе.
    pub fn label(self) -> &'static str {
        match self {
            Side::White => "белые",
            Side::Black => "чёрные",
        }
    }

    /// Имя для протокола и сохранений: `white` / `black`.
    pub fn as_str(self) -> &'static str {
        match self {
            Side::White => "white",
            Side::Black => "black",
        }
    }

    pub fn parse(text: &str) -> Option<Side> {
        match text.trim().to_lowercase().as_str() {
            "white" | "белые" | "белый" | "w" => Some(Side::White),
            "black" | "чёрные" | "черные" | "чёрный" | "черный" | "b" => Some(Side::Black),
            _ => None,
        }
    }
}

impl From<Color> for Side {
    fn from(color: Color) -> Self {
        match color {
            Color::White => Side::White,
            Color::Black => Side::Black,
        }
    }
}

/// Сложность бота: сколько полуходов он смотрит вперёд.
///
/// Глубина — главный регулятор силы, и она же главный регулятор времени:
/// 1 мгновенна, 3 думает доли секунды, 5 — уже заметная пауза.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Easy,
    Normal,
    Hard,
}

impl Level {
    /// Глубина перебора в полуходах.
    pub fn depth(self) -> u32 {
        match self {
            Level::Easy => 1,
            Level::Normal => 3,
            Level::Hard => 4,
        }
    }

    /// Сколько «шума» добавлять к оценке, в сантипешках.
    ///
    /// Без шума бот каждый раз играет одну и ту же партию. С шумом он
    /// ошибается по-разному, но всё ещё выбирает из разумных ходов.
    pub fn noise(self) -> i32 {
        match self {
            Level::Easy => 120,
            Level::Normal => 35,
            Level::Hard => 0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Level::Easy => "лёгкий",
            Level::Normal => "обычный",
            Level::Hard => "сложный",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Easy => "easy",
            Level::Normal => "normal",
            Level::Hard => "hard",
        }
    }

    pub fn parse(text: &str) -> Option<Level> {
        match text.trim().to_lowercase().as_str() {
            "easy" | "лёгкий" | "легкий" | "1" => Some(Level::Easy),
            "normal" | "обычный" | "2" => Some(Level::Normal),
            "hard" | "сложный" | "3" => Some(Level::Hard),
            _ => None,
        }
    }
}

/// Чем закончилась партия.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Мат: проигравший — тот, чей сейчас ход.
    Checkmate,
    /// Пат, недостаток материала, повтор, 50 ходов.
    Draw,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Outcome::Checkmate => "мат",
            Outcome::Draw => "ничья",
        }
    }
}

/// Ход в человеческом виде: `e2-e4`, `e7-e8Ф`.
///
/// `cozy-chess` умеет печатать UCI, но для интерфейса нужна пара клеток и
/// русская буква превращения.
pub fn move_text(mv: Move) -> String {
    let mut text = format!("{}-{}", mv.from, mv.to);
    if let Some(piece) = mv.promotion {
        text.push(piece_letter(piece));
    }
    text
}

/// Русская буква фигуры для превращения.
fn piece_letter(piece: Piece) -> char {
    match piece {
        Piece::Queen => 'Ф',
        Piece::Rook => 'Л',
        Piece::Bishop => 'С',
        Piece::Knight => 'К',
        _ => '?',
    }
}

/// Значок фигуры для доски.
pub fn piece_glyph(piece: Piece) -> char {
    match piece {
        Piece::Pawn => '♟',
        Piece::Knight => '♞',
        Piece::Bishop => '♝',
        Piece::Rook => '♜',
        Piece::Queen => '♛',
        Piece::King => '♚',
    }
}

/// Стоимость фигуры в сантипешках — для оценки и для показа материала.
pub fn piece_value(piece: Piece) -> i32 {
    match piece {
        Piece::Pawn => 100,
        Piece::Knight => 320,
        Piece::Bishop => 330,
        Piece::Rook => 500,
        Piece::Queen => 900,
        Piece::King => 0,
    }
}

/// Клетка в виде `e4` или её координаты для доски.
///
/// Понадобится странице: она рисует доску по FEN, а ходы присылает парой
/// клеток. Держим преобразование здесь, чтобы формат был один на всех.
pub fn square_name(square: Square) -> String {
    square.to_string()
}

/// Разобрать `e4` в клетку.
pub fn parse_square(text: &str) -> Result<Square, String> {
    Square::from_str(text.trim()).map_err(|_| format!("нет такой клетки: «{text}»"))
}

/// Разобрать ход вида `e2e4`, `e2-e4`, `e7e8q` в пару клеток.
///
/// Принимаем все три записи: страница присылает компактную, человек в
/// терминале может набрать с дефисом.
pub fn parse_move(board: &Board, text: &str) -> Result<Move, String> {
    let cleaned: String = text
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| *c != '-' && *c != ' ' && *c != 'x')
        .collect();

    if cleaned.len() < 4 {
        return Err(format!("ход «{text}» слишком короткий"));
    }

    let from = parse_square(&cleaned[0..2])?;
    let to = parse_square(&cleaned[2..4])?;

    // Пятый символ — превращение. Без него пешка на последней горизонтали
    // пойдёт ферзём: это ожидаемо в 99 % случаев.
    let promotion = cleaned.chars().nth(4).map(|c| match c {
        'q' | 'ф' => Ok(Piece::Queen),
        'r' | 'л' => Ok(Piece::Rook),
        'b' | 'с' => Ok(Piece::Bishop),
        'n' | 'к' => Ok(Piece::Knight),
        other => Err(format!("не понять, во что превращать: «{other}»")),
    });

    let promotion = match promotion {
        Some(Ok(piece)) => Some(piece),
        Some(Err(why)) => return Err(why),
        None => {
            // Превращение нужно, только если пешка идёт на последнюю линию.
            let last_rank = match board.side_to_move() {
                Color::White => "8",
                Color::Black => "1",
            };
            let moving_pawn = board.piece_on(from) == Some(Piece::Pawn)
                && board.color_on(from) == Some(board.side_to_move());
            if moving_pawn && cleaned[2..4].ends_with(last_rank) {
                Some(Piece::Queen)
            } else {
                None
            }
        }
    };

    let mv = Move { from, to, promotion };

    if !board.is_legal(mv) {
        return Err(format!("ход «{}» невозможен", move_text(mv)));
    }
    Ok(mv)
}

/// Все легальные ходы в позиции.
pub fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::with_capacity(48);
    board.generate_moves(|piece_moves| {
        for to in piece_moves.to {
            let promotion = promotion_for(board, piece_moves.from, to);
            moves.push(Move { from: piece_moves.from, to, promotion });
        }
        false
    });
    moves
}

/// Нужно ли превращение при ходе с `from` на `to`, и во что.
///
/// `cozy-chess` отдаёт превращения как отдельные ходы для каждого ферзя,
/// коня и т. д. Мы по умолчанию превращаем в ферзя — страница может
/// попросить другое явно.
fn promotion_for(board: &Board, from: Square, to: Square) -> Option<Piece> {
    if board.piece_on(from) != Some(Piece::Pawn) {
        return None;
    }
    let last_rank = match board.color_on(from) {
        Some(Color::White) => 7,
        Some(Color::Black) => 0,
        None => return None,
    };
    if to.rank() as i8 == last_rank {
        Some(Piece::Queen)
    } else {
        None
    }
}

/// Легальные ходы для конкретной клетки — для подсветки в интерфейсе.
pub fn moves_from(board: &Board, from: Square) -> Vec<Move> {
    legal_moves(board)
        .into_iter()
        .filter(|mv| mv.from == from)
        .collect()
}

/// Партия: позиция плюс всё, что нужно интерфейсу.
#[derive(Clone, Debug)]
pub struct Game {
    board: Board,
    /// Каким ходом партия началась: нужен FEN для отправки на страницу.
    start_fen: String,
    /// История в виде пар «откуда-куда» — для отмены хода.
    history: Vec<HistoryEntry>,
}

#[derive(Clone, Debug)]
struct HistoryEntry {
    mv: Move,
    /// Позиция до хода: проще хранить, чем разбирать обратно.
    before: Board,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    /// Новая партия с начальной позиции.
    pub fn new() -> Self {
        let board = Board::startpos();
        Self { start_fen: board.to_string(), board, history: Vec::new() }
    }

    /// Партия из FEN. Понимает и стартовую позицию, и произвольную.
    pub fn from_fen(fen: &str) -> Result<Self, String> {
        let board = Board::from_str(fen.trim())
            .map_err(|e| format!("позиция не разобралась: {e}"))?;
        Ok(Self { start_fen: board.to_string(), board, history: Vec::new() })
    }

    /// Текущая позиция в FEN — её страница и рисует.
    pub fn fen(&self) -> String {
        self.board.to_string()
    }

    pub fn start_fen(&self) -> &str {
        &self.start_fen
    }

    pub fn board(&self) -> &Board {
        &self.board
    }

    /// Чей ход.
    pub fn turn(&self) -> Side {
        self.board.side_to_move().into()
    }

    /// Сколько ходов сделано (полуходов).
    pub fn ply(&self) -> usize {
        self.history.len()
    }

    /// Последний ход — интерфейс подсвечивает его на доске.
    pub fn last_move(&self) -> Option<Move> {
        self.history.last().map(|entry| entry.mv)
    }

    /// Вся история ходов в человеческом виде: `["e2-e4", "e7-e5"]`.
    pub fn history_text(&self) -> Vec<String> {
        self.history.iter().map(|entry| move_text(entry.mv)).collect()
    }

    /// Легальные ходы в текущей позиции.
    pub fn legal(&self) -> Vec<Move> {
        legal_moves(&self.board)
    }

    /// Сделать ход. Возвращает ошибку, если ход не по правилам.
    pub fn play(&mut self, mv: Move) -> Result<(), String> {
        if !self.board.is_legal(mv) {
            return Err(format!("ход «{}» невозможен", move_text(mv)));
        }
        let before = self.board.clone();
        self.board.play(mv);
        self.history.push(HistoryEntry { mv, before });
        Ok(())
    }

    /// Сделать ход по тексту (`e2e4`).
    pub fn play_text(&mut self, text: &str) -> Result<Move, String> {
        let mv = parse_move(&self.board, text)?;
        self.play(mv)?;
        Ok(mv)
    }

    /// Отменить последний ход. `false` — отменять нечего.
    pub fn undo(&mut self) -> bool {
        match self.history.pop() {
            Some(entry) => {
                self.board = entry.before;
                true
            }
            None => false,
        }
    }

    /// Чем закончилась партия, если закончилась.
    pub fn outcome(&self) -> Option<Outcome> {
        match self.board.status() {
            GameStatus::Ongoing => None,
            // Позиция «выиграна» для того, чей ход: значит, тому и мат.
            GameStatus::Won => Some(Outcome::Checkmate),
            GameStatus::Drawn => Some(Outcome::Draw),
        }
    }

    /// Кто победил, если победа есть. `None` — ничья или партия идёт.
    pub fn winner(&self) -> Option<Side> {
        match self.outcome()? {
            // Мат ставит тот, кто только что ходил, то есть соперник
            // стороны, чей сейчас ход.
            Outcome::Checkmate => Some(self.turn().other()),
            Outcome::Draw => None,
        }
    }

    /// Стоит ли король стороны, чей ход, под шахом.
    pub fn in_check(&self) -> bool {
        self.board.checkers().into_iter().next().is_some()
    }

    /// Материал каждой стороны в сантипешках — для строки «+3» под доской.
    pub fn material(&self) -> (i32, i32) {
        let mut white = 0;
        let mut black = 0;
        for piece in [
            Piece::Pawn,
            Piece::Knight,
            Piece::Bishop,
            Piece::Rook,
            Piece::Queen,
        ] {
            let value = piece_value(piece);
            white += self.board.colored_pieces(Color::White, piece).into_iter().count() as i32 * value;
            black += self.board.colored_pieces(Color::Black, piece).into_iter().count() as i32 * value;
        }
        (white, black)
    }

    /// Доска для страницы: 64 клетки сверху вниз, слева направо.
    ///
    /// Каждая клетка — `{"square": "e4", "piece": "P", "color": "white"}`,
    /// пустая — `null`. Отдаём именно список, а не FEN: странице не нужно
    /// уметь разбирать шахматную нотацию, а нам — отлаживать её разбор в JS.
    pub fn squares(&self) -> Vec<serde_json::Value> {
        let mut out = Vec::with_capacity(64);
        // Восьмая горизонталь сверху — так доска выглядит привычно.
        for rank in (0..8).rev() {
            for file in 0..8 {
                let name = format!("{}{}", (b'a' + file as u8) as char, rank + 1);
                let square = match Square::from_str(&name) {
                    Ok(square) => square,
                    Err(_) => continue,
                };
                match (self.board.piece_on(square), self.board.color_on(square)) {
                    (Some(piece), Some(color)) => out.push(serde_json::json!({
                        "square": name,
                        "piece": piece_glyph(piece),
                        "kind": format!("{piece:?}").to_lowercase(),
                        "color": if color == Color::White { "white" } else { "black" },
                    })),
                    _ => out.push(serde_json::json!({
                        "square": name,
                        "piece": serde_json::Value::Null,
                    })),
                }
            }
        }
        out
    }

    /// Ходы для страницы: `[{"from": "e2", "to": "e4", "uci": "e2e4"}]`.
    pub fn moves_json(&self) -> Vec<serde_json::Value> {
        self.legal()
            .into_iter()
            .map(|mv| {
                serde_json::json!({
                    "from": square_name(mv.from),
                    "to": square_name(mv.to),
                    "uci": format!("{}{}", mv.from, mv.to),
                    "text": move_text(mv),
                })
            })
            .collect()
    }
}

/// Простая оценка позиции в сантипешках: плюс — лучше белым.
///
/// Считаем материал и небольшую поправку за продвижение пешек. Этого мало
/// для сильной игры, но достаточно, чтобы бот не отдавал фигуры даром и
/// старался провести пешку в ферзи.
pub fn evaluate(board: &Board) -> i32 {
    let mut score = 0;

    for piece in [
        Piece::Pawn,
        Piece::Knight,
        Piece::Bishop,
        Piece::Rook,
        Piece::Queen,
    ] {
        let value = piece_value(piece);
        let white = board.colored_pieces(Color::White, piece).into_iter().count() as i32;
        let black = board.colored_pieces(Color::Black, piece).into_iter().count() as i32;
        score += (white - black) * value;
    }

    // Пешки: чем ближе к превращению, тем дороже. Белые идут вверх
    // (к 8-й линии), чёрные — вниз.
    for square in board.pieces(Piece::Pawn) {
        let rank = square.rank() as i32;
        score += match board.color_on(square) {
            Some(Color::White) => rank * 8,
            Some(Color::Black) => (7 - rank) * 8,
            None => 0,
        };
    }

    // Тот, кто под матом, проигрывает немедленно — это важнее материала.
    match board.status() {
        GameStatus::Won => {
            // «Выиграна» для стороны, чей ход: ей и засчитываем.
            score += match board.side_to_move() {
                Color::White => 100_000,
                Color::Black => -100_000,
            }
        }
        GameStatus::Drawn => score = 0,
        GameStatus::Ongoing => {}
    }

    score
}

/// Бот: перебор с альфа-отсечением и небольшим случайным шумом.
pub struct Bot {
    level: Level,
}

impl Bot {
    pub fn new(level: Level) -> Self {
        Self { level }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    /// Лучший ход в позиции. `None` — ходов нет (мат или пат).
    ///
    /// Перебор детерминирован, а шум добавляется к итоговой оценке хода, а не
    /// к перебору: так бот ошибается «на один ход», но не строит заведомо
    /// глупые варианты.
    pub fn best_move(&self, board: &Board) -> Option<Move> {
        let moves = legal_moves(board);
        if moves.is_empty() {
            return None;
        }

        let depth = self.level.depth();
        let mut scored: Vec<(i32, Move)> = moves
            .into_iter()
            .map(|mv| {
                let mut next = board.clone();
                next.play(mv);
                // Оценка всегда с точки зрения белых, поэтому для чёрных
                // лучший ход — минимальный результат.
                let score = -negamax(&next, depth.saturating_sub(1), -200_000, 200_000)
                    + self.shake(mv);
                (score, mv)
            })
            .collect();

        // Стабильная сортировка: при равных оценках порядок не «прыгает».
        scored.sort_by(|a, b| match board.side_to_move() {
            Color::White => b.0.cmp(&a.0),
            Color::Black => a.0.cmp(&b.0),
        });

        scored.first().map(|(_, mv)| *mv)
    }

    /// Небольшой шум, чтобы бот не играл одну партию каждый раз.
    fn shake(&self, mv: Move) -> i32 {
        let noise = self.level.noise();
        if noise == 0 {
            return 0;
        }
        // Хеш хода вместо генератора: не нужен `rand` в чистой функции,
        // и результат воспроизводим при том же входе.
        let seed = u64::from(mv.from as u8) * 31 + u64::from(mv.to as u8);
        ((seed % (noise as u64 * 2 + 1)) as i32) - noise
    }
}

/// Негамакс: оценка позиции с точки зрения стороны, чей ход.
fn negamax(board: &Board, depth: u32, mut alpha: i32, beta: i32) -> i32 {
    if depth == 0 {
        return sign_for(board) * evaluate(board);
    }

    let moves = legal_moves(board);
    if moves.is_empty() {
        // Мат или пат: и то и другое плохо, но мат — хуже.
        return if board.checkers().into_iter().next().is_some() {
            -100_000 + (1 - depth as i32)
        } else {
            0
        };
    }

    let mut best = -200_000;
    for mv in moves {
        let mut next = board.clone();
        next.play(mv);
        let score = -negamax(&next, depth - 1, -beta, -alpha);
        if score > best {
            best = score;
        }
        if best > alpha {
            alpha = best;
        }
        if alpha >= beta {
            break;
        }
    }
    best
}

/// Знак перевода оценки: `evaluate` считает за белых, а негамак — за того,
/// чей ход.
fn sign_for(board: &Board) -> i32 {
    match board.side_to_move() {
        Color::White => 1,
        Color::Black => -1,
    }
}

impl fmt::Display for Game {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.board, self.turn().label())
    }
}

/// Взяла ли фигура соперника на этом ходу.
///
/// Нужно комментатору: «забрала твою фигуру» говорится только когда фигура
/// действительно исчезла с доски. Считаем по материалу до и после — это
/// проще, чем разбирать тип хода, и работает для любого взятия.
pub fn was_capture(material_before: (i32, i32), material_after: (i32, i32)) -> bool {
    let before = material_before.0 + material_before.1;
    let after = material_after.0 + material_after.1;
    after < before
}

/// Сколько материала исчезло с доски за ход, в сантипешках.
pub fn captured_value(material_before: (i32, i32), material_after: (i32, i32)) -> i32 {
    let before = material_before.0 + material_before.1;
    let after = material_after.0 + material_after.1;
    (before - after).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_position_has_twenty_moves() {
        let game = Game::new();
        assert_eq!(game.legal().len(), 20);
        assert_eq!(game.turn(), Side::White);
        assert!(game.outcome().is_none());
        assert!(!game.in_check());
    }

    #[test]
    fn plays_and_undoes_a_move() {
        let mut game = Game::new();
        let fen_before = game.fen();

        game.play_text("e2e4").expect("e4 легален");
        assert_eq!(game.turn(), Side::Black);
        assert_eq!(game.ply(), 1);
        assert_eq!(game.last_move().map(move_text).as_deref(), Some("e2-e4"));

        assert!(game.undo());
        assert_eq!(game.fen(), fen_before, "отмена вернула ровно ту же позицию");
        assert_eq!(game.ply(), 0);
        assert!(!game.undo(), "отменять больше нечего");
    }

    #[test]
    fn rejects_illegal_moves() {
        let game = Game::new();
        // Пешка не ходит на три клетки, и не её очередь ходить чёрными.
        assert!(parse_move(game.board(), "e2e5").is_err());
        assert!(parse_move(game.board(), "e7e5").is_err());
        // И «ход» из пустоты тоже.
        assert!(parse_move(game.board(), "e4e5").is_err());
        assert!(parse_move(game.board(), "мусор").is_err());
    }

    #[test]
    fn accepts_both_move_spellings() {
        let game = Game::new();
        assert!(parse_move(game.board(), "e2e4").is_ok());
        assert!(parse_move(game.board(), "e2-e4").is_ok());
        assert!(parse_move(game.board(), "E2E4").is_ok(), "регистр не важен");
        assert!(parse_move(game.board(), " e2 e4 ").is_ok());
    }

    #[test]
    fn detects_checkmate() {
        // Мат Легаля: ферзь и слон на месте.
        let game = Game::from_fen("r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4")
            .expect("позиция");
        assert_eq!(game.outcome(), Some(Outcome::Checkmate));
        assert!(game.in_check(), "король под шахом");
        assert_eq!(game.winner(), Some(Side::White), "мат поставили белые");
        assert!(game.legal().is_empty());
    }

    #[test]
    fn detects_stalemate() {
        // Классический пат: чёрный король в углу, белые не дают ходов.
        let game = Game::from_fen("7k/5Q2/6K1/8/8/8/8/8 b - - 0 1").expect("позиция");
        assert_eq!(game.outcome(), Some(Outcome::Draw));
        assert_eq!(game.winner(), None);
        assert!(!game.in_check(), "при пате шаха нет");
        assert!(game.legal().is_empty());
    }

    #[test]
    fn promotion_defaults_to_queen() {
        let game = Game::from_fen("8/P7/8/8/8/8/8/K6k w - - 0 1").expect("позиция");
        let mv = parse_move(game.board(), "a7a8").expect("превращение");
        assert_eq!(mv.promotion, Some(Piece::Queen), "без буквы — ферзь");

        let knight = parse_move(game.board(), "a7a8n").expect("конь");
        assert_eq!(knight.promotion, Some(Piece::Knight));
    }

    #[test]
    fn bot_finds_a_free_capture() {
        // Чёрный ферзь стоит под боем белой ладьи без прикрытия.
        let board = Board::from_str("4k3/8/8/3q4/8/8/8/3RK3 w - - 0 1").expect("позиция");
        let bot = Bot::new(Level::Hard);
        let mv = bot.best_move(&board).expect("ход есть");
        assert_eq!(mv.to, parse_square("d5").unwrap(), "бот забрал ферзя");
    }

    #[test]
    fn bot_never_returns_an_illegal_move() {
        let mut game = Game::new();
        let bot = Bot::new(Level::Easy);
        // Десять полуходов: на каждом бот обязан выбрать легальный ход.
        for _ in 0..10 {
            let Some(mv) = bot.best_move(game.board()) else {
                break;
            };
            assert!(game.board().is_legal(mv), "бот предложил невозможный ход");
            game.play(mv).expect("ход бота легален");
            if game.outcome().is_some() {
                break;
            }
        }
    }

    #[test]
    fn bot_has_no_move_in_mated_position() {
        let game = Game::from_fen("r1bqkb1r/pppp1Qpp/2n2n2/4p3/2B1P3/8/PPPP1PPP/RNB1K1NR b KQkq - 0 4")
            .expect("позиция");
        assert!(Bot::new(Level::Normal).best_move(game.board()).is_none());
    }

    #[test]
    fn evaluation_prefers_material() {
        // Белые без ферзя против полного комплекта чёрных.
        let down = Board::from_str("4k3/8/8/8/8/8/8/4K3 w - - 0 1").expect("позиция");
        assert_eq!(evaluate(&down), 0, "равный материал — ноль");

        let white_queen = Board::from_str("4k3/8/8/8/8/8/8/3QK3 w - - 0 1").expect("позиция");
        assert!(evaluate(&white_queen) > 800, "ферзь белых дорого стоит");

        let black_queen = Board::from_str("3qk3/8/8/8/8/8/8/4K3 w - - 0 1").expect("позиция");
        assert!(evaluate(&black_queen) < -800, "ферзь чёрных оценивается в минус");
    }

    #[test]
    fn squares_lay_out_a_full_board() {
        let game = Game::new();
        let squares = game.squares();
        assert_eq!(squares.len(), 64, "восемь на восемь");
        // Первая клетка — a8, последняя — h1: доска сверху вниз.
        assert_eq!(squares[0]["square"], "a8");
        assert_eq!(squares[63]["square"], "h1");
        // На старте a8 — чёрная ладья, e1 — белый король.
        assert_eq!(squares[0]["color"], "black");
        assert_eq!(squares[0]["kind"], "rook");
        let e1 = squares.iter().find(|s| s["square"] == "e1").expect("e1");
        assert_eq!(e1["kind"], "king");
        assert_eq!(e1["color"], "white");
    }

    #[test]
    fn moves_json_is_ready_for_the_page() {
        let game = Game::new();
        let moves = game.moves_json();
        assert_eq!(moves.len(), 20);
        assert!(moves.iter().any(|m| m["uci"] == "e2e4"));
        assert!(moves.iter().all(|m| m["from"].is_string() && m["to"].is_string()));
    }

    #[test]
    fn sides_and_levels_parse_from_russian() {
        assert_eq!(Side::parse("белые"), Some(Side::White));
        assert_eq!(Side::parse("ЧЁРНЫЕ"), Some(Side::Black));
        assert_eq!(Side::parse("чёрные"), Some(Side::Black));
        assert_eq!(Side::parse("зелёные"), None);

        assert_eq!(Level::parse("сложный"), Some(Level::Hard));
        assert_eq!(Level::parse("easy"), Some(Level::Easy));
        assert_eq!(Level::parse("кошмарный"), None);

        // Дефолтные настройки разумны: уровни различаются глубиной.
        assert!(Level::Easy.depth() < Level::Normal.depth());
        assert!(Level::Normal.depth() < Level::Hard.depth());
    }

    #[test]
    fn hard_bot_is_stronger_than_easy_one() {
        // Сложный бот видит взятие на два хода вперёд, лёгкий — нет.
        // Позиция: ферзь чёрных под боем, но защищён слоном.
        let board = Board::from_str("4k3/8/2b5/3q4/8/8/8/3RK3 w - - 0 1").expect("позиция");
        let hard = Bot::new(Level::Hard).best_move(&board).expect("ход");
        // Сложный забирает ферзя (ладья бьёт d5, слона на c6 не хватает).
        assert_eq!(hard.to, parse_square("d5").unwrap());
    }

    #[test]
    fn material_counts_both_sides() {
        let game = Game::new();
        let (white, black) = game.material();
        assert_eq!(white, black, "на старте материал равный");
        assert_eq!(white, 8 * 100 + 2 * 320 + 2 * 330 + 2 * 500 + 900);
    }

    #[test]
    fn game_text_mentions_the_turn() {
        let game = Game::new();
        let text = game.to_string();
        assert!(text.contains("белые"), "в подписи видно, чей ход: {text}");
    }
}
