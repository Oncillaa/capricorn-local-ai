//! Русские шашки: правила, ходы и оценка позиции.
//!
//! Всё в модуле — чистые функции над позицией: ни потоков, ни файлов, ни
//! времени. Бот отдельным типом, потому что странице он нужен целиком.
//!
//! Доска хранится как `[file][rank]`: `file` 0…7 — это a…h, `rank` 0…7 —
//! это 1…8. Тёмные поля (фигуры стоят только на них) — там, где
//! `(file + rank) % 2 == 1`.

use serde_json::json;

/// Кто играет.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    White,
    Black,
}

impl Side {
    pub fn other(self) -> Side {
        match self {
            Side::White => Side::Black,
            Side::Black => Side::White,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Side::White => "white",
            Side::Black => "black",
        }
    }

    pub fn parse(text: &str) -> Option<Side> {
        match text.trim().to_lowercase().as_str() {
            "white" | "белые" | "белый" => Some(Side::White),
            "black" | "чёрные" | "черные" | "чёрный" | "черный" => Some(Side::Black),
            _ => None,
        }
    }

    /// Горизонталь, на которой простая становится дамкой.
    fn last_rank(self) -> u8 {
        match self {
            Side::White => 7,
            Side::Black => 0,
        }
    }
}

/// Чем закончилась партия.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    WhiteWins,
    BlackWins,
    Draw,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::WhiteWins => "white",
            Outcome::BlackWins => "black",
            Outcome::Draw => "draw",
        }
    }
}

/// Поле на доске.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Square {
    pub file: u8,
    pub rank: u8,
}

impl Square {
    /// Поле по координатам. Вне доски — `None`.
    pub fn new(file: u8, rank: u8) -> Option<Square> {
        if file < 8 && rank < 8 {
            Some(Square { file, rank })
        } else {
            None
        }
    }

    /// Имя в шахматной нотации: `a3`.
    pub fn name(self) -> String {
        let file = (b'a' + self.file) as char;
        format!("{file}{}", self.rank + 1)
    }

    /// Разобрать `a3`.
    pub fn parse(text: &str) -> Option<Square> {
        let text = text.trim().to_lowercase();
        let mut chars = text.chars();
        let file = chars.next()?;
        if !file.is_ascii_lowercase() || !('a'..='h').contains(&file) {
            return None;
        }
        let rank: u8 = chars.as_str().parse().ok()?;
        if !(1..=8).contains(&rank) {
            return None;
        }
        Square::new(file as u8 - b'a', rank - 1)
    }

    /// Тёмное ли поле. Фигуры стоят только на тёмных.
    ///
    /// Тёмные — это `a1` и `h8`, как на настоящей доске: у нас тёмные там,
    /// где сумма координат чётная.
    pub fn is_dark(self) -> bool {
        (self.file + self.rank) % 2 == 0
    }

    /// Шаг по диагонали. `None`, если за краем доски.
    fn step(self, dir: (i8, i8)) -> Option<Square> {
        let file = self.file as i8 + dir.0;
        let rank = self.rank as i8 + dir.1;
        if (0..8).contains(&file) && (0..8).contains(&rank) {
            Some(Square { file: file as u8, rank: rank as u8 })
        } else {
            None
        }
    }
}

/// Фигура: чья и дамка ли.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Piece {
    pub side: Side,
    pub king: bool,
}

impl Piece {
    pub fn simple(side: Side) -> Self {
        Piece { side, king: false }
    }

    pub fn king(side: Side) -> Self {
        Piece { side, king: true }
    }
}

/// Ход. `captured` непустой у боя; при серии в нём все взятые поля.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Move {
    pub from: Square,
    pub to: Square,
    pub captured: Vec<Square>,
}

/// Партия.
#[derive(Debug, Clone)]
pub struct Game {
    board: [[Option<Piece>; 8]; 8],
    side: Side,
    ply: usize,
    /// Полуходы подряд без взятия и без хода простой шашкой — для ничьей.
    quiet_plies: usize,
}

/// Все четыре диагонали.
const DIRS: [(i8, i8); 4] = [(-1, -1), (-1, 1), (1, -1), (1, 1)];

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    /// Начальная расстановка: по 12 простых, ход белых.
    pub fn new() -> Self {
        let mut board = [[None; 8]; 8];
        for rank in 0..8u8 {
            for file in 0..8u8 {
                let square = Square { file, rank };
                if !square.is_dark() {
                    continue;
                }
                if rank < 3 {
                    board[file as usize][rank as usize] = Some(Piece::simple(Side::White));
                } else if rank > 4 {
                    board[file as usize][rank as usize] = Some(Piece::simple(Side::Black));
                }
            }
        }
        Game { board, side: Side::White, ply: 0, quiet_plies: 0 }
    }

    /// Пустая доска — нужна только тестам, чтобы собрать позицию вручную.
    #[cfg(test)]
    fn empty(side: Side) -> Self {
        Game { board: [[None; 8]; 8], side, ply: 0, quiet_plies: 0 }
    }

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn ply(&self) -> usize {
        self.ply
    }

    fn at(&self, square: Square) -> Option<Piece> {
        self.board[square.file as usize][square.rank as usize]
    }

    fn set(&mut self, square: Square, piece: Option<Piece>) {
        self.board[square.file as usize][square.rank as usize] = piece;
    }

    pub fn piece_at(&self, square: Square) -> Option<Piece> {
        self.at(square)
    }

    pub fn piece_count(&self, side: Side) -> usize {
        let mut count = 0;
        for file in 0..8 {
            for rank in 0..8 {
                if let Some(piece) = self.board[file][rank] {
                    if piece.side == side {
                        count += 1;
                    }
                }
            }
        }
        count
    }

    /// Все допустимые ходы. Если есть хоть один бой — только бои.
    pub fn legal_moves(&self) -> Vec<Move> {
        let mut captures = Vec::new();
        for file in 0..8u8 {
            for rank in 0..8u8 {
                let square = Square { file, rank };
                let Some(piece) = self.at(square) else { continue };
                if piece.side != self.side {
                    continue;
                }
                let mut found = Vec::new();
                self.chains(square, piece, &mut Vec::new(), &mut found);
                for (to, captured) in found {
                    if !captured.is_empty() {
                        captures.push(Move { from: square, to, captured });
                    }
                }
            }
        }
        if !captures.is_empty() {
            return captures;
        }

        // Боев нет — обычные ходы.
        let mut moves = Vec::new();
        for file in 0..8u8 {
            for rank in 0..8u8 {
                let from = Square { file, rank };
                let Some(piece) = self.at(from) else { continue };
                if piece.side != self.side {
                    continue;
                }
                for dir in DIRS {
                    if piece.king {
                        // Дамка идёт по диагонали, пока путь свободен.
                        let mut cur = from;
                        while let Some(next) = cur.step(dir) {
                            if self.at(next).is_some() {
                                break;
                            }
                            moves.push(Move { from, to: next, captured: Vec::new() });
                            cur = next;
                        }
                    } else {
                        // Простая — только вперёд, на одну клетку.
                        let forward = match piece.side {
                            Side::White => dir.1 > 0,
                            Side::Black => dir.1 < 0,
                        };
                        if !forward {
                            continue;
                        }
                        if let Some(next) = from.step(dir) {
                            if self.at(next).is_none() {
                                moves.push(Move { from, to: next, captured: Vec::new() });
                            }
                        }
                    }
                }
            }
        }
        moves
    }

    /// Собрать все полные серии взятий, начинающиеся с `square`.
    ///
    /// Рекурсия останавливается только тогда, когда бить больше нечем:
    /// по правилам русских шашек серию обрывать нельзя.
    fn chains(
        &self,
        square: Square,
        piece: Piece,
        captured: &mut Vec<Square>,
        out: &mut Vec<(Square, Vec<Square>)>,
    ) {
        // Шашка, дошедшая до последней горизонтали по пути, бьёт уже как дамка.
        let king = piece.king || square.rank == piece.side.last_rank();
        let mut continued = false;

        for dir in DIRS {
            if king {
                let mut cur = square;
                // Идём по диагонали до первой фигуры.
                loop {
                    let Some(next) = cur.step(dir) else { break };
                    match self.at(next) {
                        None => {
                            cur = next;
                            continue;
                        }
                        Some(target) => {
                            // Своя фигура или уже взятая — эта диагональ закрыта.
                            if target.side == piece.side || captured.contains(&next) {
                                break;
                            }
                            // За взятой фигурой можно встать на любое свободное поле.
                            let mut landing = next;
                            while let Some(free) = landing.step(dir) {
                                if self.at(free).is_some() {
                                    break;
                                }
                                continued = true;
                                captured.push(next);
                                self.chains(free, Piece::king(piece.side), captured, out);
                                captured.pop();
                                landing = free;
                            }
                            break;
                        }
                    }
                }
            } else {
                let Some(mid) = square.step(dir) else { continue };
                let Some(land) = square.step(dir).and_then(|s| s.step(dir)) else { continue };
                // Простая бьёт в любую сторону, но только через соседнюю клетку.
                if let Some(target) = self.at(mid) {
                    if target.side != piece.side && !captured.contains(&mid) && self.at(land).is_none()
                    {
                        continued = true;
                        captured.push(mid);
                        self.chains(land, piece, captured, out);
                        captured.pop();
                    }
                }
            }
        }

        if !continued {
            out.push((square, captured.clone()));
        }
    }

    /// Применить ход. Нелегальный — `Err`.
    pub fn apply(&mut self, mv: &Move) -> Result<(), String> {
        let legal = self.legal_moves();
        let known = legal.iter().any(|candidate| {
            candidate.from == mv.from && candidate.to == mv.to && candidate.captured == mv.captured
        });
        if !known {
            return Err("такого хода нет".into());
        }

        let Some(mut piece) = self.at(mv.from) else {
            return Err("на исходном поле пусто".into());
        };
        let was_king = piece.king;

        self.set(mv.from, None);
        for square in &mv.captured {
            self.set(*square, None);
        }
        // Превращение — после хода: шашка встала на последнюю горизонталь.
        if mv.to.rank == piece.side.last_rank() {
            piece.king = true;
        }
        self.set(mv.to, Some(piece));

        // Тихий ход — это ход дамки без взятия. Всё остальное сбрасывает счётчик.
        if mv.captured.is_empty() && was_king {
            self.quiet_plies += 1;
        } else {
            self.quiet_plies = 0;
        }
        self.side = self.side.other();
        self.ply += 1;
        Ok(())
    }

    /// Чем всё кончилось, если кончилось.
    pub fn outcome(&self) -> Option<Outcome> {
        if self.piece_count(Side::White) == 0 {
            return Some(Outcome::BlackWins);
        }
        if self.piece_count(Side::Black) == 0 {
            return Some(Outcome::WhiteWins);
        }
        // Ходов нет — этой стороне и проигрыш.
        if self.legal_moves().is_empty() {
            return Some(match self.side {
                Side::White => Outcome::BlackWins,
                Side::Black => Outcome::WhiteWins,
            });
        }
        // 15 ходов каждой стороны без боя и без хода простой — ничья.
        if self.quiet_plies >= 30 {
            return Some(Outcome::Draw);
        }
        None
    }

    /// Состояние для страницы.
    pub fn state_json(&self) -> serde_json::Value {
        let mut cells = Vec::new();
        // Строка 0 — восьмая горизонталь, столбец 0 — файл «a».
        for rank in (0..8u8).rev() {
            let mut row = Vec::new();
            for file in 0..8u8 {
                let text = match self.board[file as usize][rank as usize] {
                    None => String::new(),
                    Some(piece) => {
                        let letter = match piece.side {
                            Side::White => "w",
                            Side::Black => "b",
                        };
                        if piece.king {
                            letter.to_uppercase()
                        } else {
                            letter.to_string()
                        }
                    }
                };
                row.push(json!(text));
            }
            cells.push(json!(row));
        }

        let moves: Vec<serde_json::Value> = self
            .legal_moves()
            .iter()
            .map(|mv| json!({ "from": mv.from.name(), "to": mv.to.name() }))
            .collect();

        json!({
            "cells": cells,
            "side": self.side.as_str(),
            "moves": moves,
            "outcome": self.outcome().map(|o| o.as_str()),
            "ply": self.ply,
            "white": self.piece_count(Side::White),
            "black": self.piece_count(Side::Black),
        })
    }
}

/// Ход в человеческом виде: `c3-d4` или `c3xf6` при бое.
///
/// Серия пишется одной парой «откуда-куда»: перечислять все съеденные поля
/// в списке ходов негде, а счёт взятых виден по доске.
pub fn move_text(mv: &Move) -> String {
    let mark = if mv.captured.is_empty() { '-' } else { 'x' };
    format!("{}{mark}{}", mv.from.name(), mv.to.name())
}

/// Насколько сложно играть.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Easy,
    Normal,
    Hard,
}

impl Level {
    pub fn parse(text: &str) -> Option<Level> {
        match text.trim().to_lowercase().as_str() {
            "easy" | "лёгкая" | "легкая" | "лёгкий" | "легкий" => Some(Level::Easy),
            "normal" | "обычная" | "обычный" => Some(Level::Normal),
            "hard" | "сложная" | "сложный" => Some(Level::Hard),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Easy => "easy",
            Level::Normal => "normal",
            Level::Hard => "hard",
        }
    }

    /// На сколько полуходов вперёд считаем.
    fn depth(self) -> u32 {
        match self {
            Level::Easy => 1,
            Level::Normal => 3,
            Level::Hard => 5,
        }
    }
}

/// Вес фигуры: дамка втрое дороже простой.
fn value(piece: Piece) -> i32 {
    if piece.king {
        300
    } else {
        100
    }
}

/// Оценка позиции с точки зрения `side`.
///
/// Кроме материала считаем продвижение простых: шашка у последней
/// горизонтали почти дамка, и перебор без этого «не видит» смысла идти вперёд.
fn evaluate(game: &Game, side: Side) -> i32 {
    let mut score = 0;
    for file in 0..8u8 {
        for rank in 0..8u8 {
            let Some(square) = Square::new(file, rank) else { continue };
            let Some(piece) = game.piece_at(square) else { continue };
            let mut worth = value(piece);
            if !piece.king {
                let advance = match piece.side {
                    Side::White => rank as i32,
                    Side::Black => 7 - rank as i32,
                };
                worth += advance * 3;
            }
            if piece.side == side {
                score += worth;
            } else {
                score -= worth;
            }
        }
    }
    score
}

/// Простой бот: перебор с альфа-отсечением.
pub struct Bot {
    level: Level,
}

impl Bot {
    pub fn new(level: Level) -> Self {
        Bot { level }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    /// Лучший ход. `None`, если ходов нет.
    pub fn best_move(&self, game: &Game) -> Option<Move> {
        let moves = game.legal_moves();
        if moves.is_empty() {
            return None;
        }
        // Лёгкий уровень ошибается нарочно: иногда берёт случайный ход.
        if self.level == Level::Easy && moves.len() > 1 {
            let pick = (game.ply() * 7 + moves.len()) % moves.len();
            // Но очевидный бой всё-таки замечает.
            if moves.iter().all(|mv| mv.captured.is_empty()) {
                return moves.get(pick).cloned();
            }
        }

        let me = game.side();
        let mut best: Option<(Move, i32)> = None;
        for mv in moves {
            let mut probe = game.clone();
            if probe.apply(&mv).is_err() {
                continue;
            }
            let score = -self.search(&probe, self.level.depth().saturating_sub(1), i32::MIN + 1, i32::MAX - 1)
                + evaluate(&probe, me) / 10;
            match &best {
                Some((_, best_score)) if *best_score >= score => {}
                _ => best = Some((mv, score)),
            }
        }
        best.map(|(mv, _)| mv)
    }

    /// Негамакс с альфа-отсечением. Оценка — со стороны того, чей ход.
    fn search(&self, game: &Game, depth: u32, mut alpha: i32, beta: i32) -> i32 {
        if depth == 0 || game.outcome().is_some() {
            return match game.outcome() {
                Some(Outcome::Draw) => 0,
                Some(_) => {
                    // Проигрыш тому, чей ход, — это очень плохо для него.
                    -100_000
                }
                None => evaluate(game, game.side()),
            };
        }
        let moves = game.legal_moves();
        if moves.is_empty() {
            return -100_000;
        }
        let mut best = i32::MIN + 1;
        for mv in moves {
            let mut probe = game.clone();
            if probe.apply(&mv).is_err() {
                continue;
            }
            let score = -self.search(&probe, depth - 1, -beta, -alpha);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Собрать позицию из списка полей — для проверки конкретных правил.
    fn position(pieces: &[(&str, Piece)], side: Side) -> Game {
        let mut game = Game::empty(side);
        for (name, piece) in pieces {
            let square = Square::parse(name).unwrap_or_else(|| panic!("поле {name}"));
            game.set(square, Some(*piece));
        }
        game
    }

    fn w() -> Piece {
        Piece::simple(Side::White)
    }
    fn b() -> Piece {
        Piece::simple(Side::Black)
    }
    fn wk() -> Piece {
        Piece::king(Side::White)
    }
    fn bk() -> Piece {
        Piece::king(Side::Black)
    }

    #[test]
    fn the_start_position_has_twelve_pieces_each() {
        let game = Game::new();
        assert_eq!(game.piece_count(Side::White), 12);
        assert_eq!(game.piece_count(Side::Black), 12);
        assert_eq!(game.side(), Side::White);
        assert!(!game.legal_moves().is_empty());
        assert_eq!(game.outcome(), None);
    }

    #[test]
    fn a_man_moves_forward_only() {
        let game = position(&[("c3", w()), ("f6", b())], Side::White);
        assert!(game.legal_moves().iter().any(|mv| mv.to.name() == "d4"));
        // Назад простой шашке нельзя.
        assert!(game.legal_moves().iter().all(|mv| mv.to.name() != "b2"));
    }

    #[test]
    fn a_man_captures_backwards() {
        // Белая c3, чёрная b2 — бой назад, приземление на a1.
        let game = position(&[("c3", w()), ("b2", b())], Side::White);
        let moves = game.legal_moves();
        assert_eq!(moves.len(), 1, "бой обязан быть единственным ходом");
        assert_eq!(moves[0].to.name(), "a1");
        assert_eq!(moves[0].captured.len(), 1);
    }

    #[test]
    fn capturing_is_mandatory() {
        // У белых есть и тихий ход, и бой — остаётся только бой.
        let game = position(&[("c3", w()), ("d4", b()), ("a1", w())], Side::White);
        let moves = game.legal_moves();
        assert!(moves.iter().all(|mv| !mv.captured.is_empty()), "бои обязательны");
        assert_eq!(moves.len(), 1);
    }

    #[test]
    fn a_double_capture_is_one_move() {
        // c3 бьёт d4, встаёт на e5, бьёт f6 и встаёт на g7.
        let game = position(&[("c3", w()), ("d4", b()), ("f6", b())], Side::White);
        let moves = game.legal_moves();
        assert_eq!(moves.len(), 1);
        assert_eq!(moves[0].captured.len(), 2, "серия — один ход с двумя взятыми");
        assert_eq!(moves[0].to.name(), "g7");
    }

    #[test]
    fn a_king_walks_any_distance() {
        // Чёрная h2 стоит вне диагоналей белой дамки, поэтому боя нет и
        // дамка просто ходит.
        let game = position(&[("c3", wk()), ("h2", bk())], Side::White);
        let targets: Vec<String> = game.legal_moves().iter().map(|mv| mv.to.name()).collect();
        assert!(targets.contains(&"a1".to_string()), "дальний конец диагонали: {targets:?}");
        assert!(targets.contains(&"h8".to_string()));
    }

    #[test]
    fn a_king_captures_from_a_distance() {
        // Дамка c3, чёрная f6: бьёт и встаёт на g7 или h8.
        let game = position(&[("c3", wk()), ("f6", b())], Side::White);
        let moves = game.legal_moves();
        assert_eq!(moves.len(), 2, "за взятой фигурой два свободных поля: {moves:?}");
        let targets: Vec<String> = moves.iter().map(|mv| mv.to.name()).collect();
        assert!(targets.contains(&"g7".to_string()));
        assert!(targets.contains(&"h8".to_string()));
    }

    #[test]
    fn a_man_is_promoted_on_the_last_rank() {
        // c7 — тёмное поле; d8 — тоже тёмное и последнее для белых.
        let mut game = position(&[("c7", w()), ("h4", b())], Side::White);
        let mv = Move {
            from: Square::parse("c7").unwrap_or_else(|| panic!("c7")),
            to: Square::parse("d8").unwrap_or_else(|| panic!("d8")),
            captured: Vec::new(),
        };
        game.apply(&mv).unwrap_or_else(|e| panic!("ход не принят: {e}"));
        let piece = game.piece_at(Square::parse("d8").unwrap_or_else(|| panic!("d8")));
        assert_eq!(piece, Some(Piece::king(Side::White)));
    }

    #[test]
    fn a_man_promoted_mid_chain_keeps_capturing_as_a_king() {
        // b6 бьёт c7 и встаёт на d8 — там простая становится дамкой, но
        // серия не кончена: с d8 дамка бьёт f6 и встаёт на g5 или h4.
        let game = position(&[("b6", w()), ("c7", bk()), ("f6", b())], Side::White);
        let moves = game.legal_moves();
        assert_eq!(moves.len(), 2, "две точки приземления за f6: {moves:?}");
        for mv in &moves {
            assert_eq!(mv.captured.len(), 2, "серия из двух взятий: {mv:?}");
            assert!(
                mv.to.name() == "g5" || mv.to.name() == "h4",
                "дамой бьют на расстояние, а не в упор: {}",
                mv.to.name()
            );
        }
    }

    #[test]
    fn losing_all_pieces_loses_the_game() {
        let game = position(&[("c3", w())], Side::Black);
        assert_eq!(game.outcome(), Some(Outcome::WhiteWins));
    }

    #[test]
    fn having_no_moves_loses_the_game() {
        // Чёрная h2 заперта: вперёд — край доски, а g1 занята белой.
        let game = position(&[("g1", w()), ("h2", b())], Side::Black);
        assert!(game.legal_moves().is_empty());
        assert_eq!(game.outcome(), Some(Outcome::WhiteWins));
    }

    #[test]
    fn quiet_king_moves_are_counted_and_end_in_a_draw() {
        // Дамки стоят вне диагоналей друг друга, поэтому первый ход тихий.
        let mut game = position(&[("c3", wk()), ("h2", bk())], Side::White);
        let quiet = game
            .legal_moves()
            .into_iter()
            .find(|mv| mv.captured.is_empty())
            .unwrap_or_else(|| panic!("тихий ход должен быть"));
        game.apply(&quiet).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(game.quiet_plies, 1, "ход дамки без боя считается тихим");

        // Дальше 30 таких полуходов — ничья. Гонять их по доске нет смысла:
        // важно само правило, а не то, куда разбегутся дамки.
        game.quiet_plies = 30;
        assert_eq!(game.outcome(), Some(Outcome::Draw));
    }

    #[test]
    fn a_capture_resets_the_draw_counter() {
        let mut game = position(&[("c3", w()), ("d4", b()), ("h6", b())], Side::White);
        game.quiet_plies = 20;
        let mv = game
            .legal_moves()
            .into_iter()
            .find(|mv| !mv.captured.is_empty())
            .unwrap_or_else(|| panic!("бой должен быть"));
        game.apply(&mv).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(game.quiet_plies, 0, "взятие сбрасывает счётчик ничьей");
    }

    #[test]
    fn an_illegal_move_is_refused() {
        let mut game = Game::new();
        let bad = Move {
            from: Square::parse("a1").unwrap_or_else(|| panic!("a1")),
            to: Square::parse("h8").unwrap_or_else(|| panic!("h8")),
            captured: Vec::new(),
        };
        assert!(game.apply(&bad).is_err());
        // И партия не сдвинулась.
        assert_eq!(game.ply(), 0);
    }

    #[test]
    fn parsing_accepts_names_and_rejects_junk() {
        assert_eq!(Side::parse("белые"), Some(Side::White));
        assert_eq!(Side::parse("BLACK"), Some(Side::Black));
        assert_eq!(Side::parse("зелёные"), None);
        assert_eq!(Square::parse("a3"), Some(Square { file: 0, rank: 2 }));
        assert_eq!(Square::parse("i9"), None);
        assert_eq!(Square::parse(""), None);
        assert_eq!(Level::parse("сложная"), Some(Level::Hard));
        assert_eq!(Level::parse("???"), None);
    }

    #[test]
    fn the_state_for_the_page_is_well_formed() {
        let state = Game::new().state_json();
        let cells = state["cells"].as_array().unwrap_or_else(|| panic!("нет cells"));
        assert_eq!(cells.len(), 8, "восемь горизонталей");
        for row in cells {
            assert_eq!(row.as_array().map(|r| r.len()), Some(8), "восемь полей в строке");
        }
        // Верхняя строка — восьмая горизонталь, там только чёрные.
        let top = cells[0].as_array().unwrap_or_else(|| panic!("нет строки"));
        assert!(top.iter().any(|c| c == "b"), "на 8-й горизонтали стоят чёрные");
        assert_eq!(state["side"], "white");
        assert!(!state["moves"].as_array().map(|m| m.is_empty()).unwrap_or(true));
        assert!(state["outcome"].is_null());
    }

    #[test]
    fn square_helpers_agree() {
        let sq = Square::parse("a3").unwrap_or_else(|| panic!("a3"));
        assert_eq!(sq.name(), "a3");
        assert!(sq.is_dark(), "a3 — тёмное поле");
        assert!(
            Square::parse("a1").unwrap_or_else(|| panic!("a1")).is_dark(),
            "a1 — тёмное, как на настоящей доске"
        );
        assert!(
            !Square::parse("h1").unwrap_or_else(|| panic!("h1")).is_dark(),
            "h1 — светлое"
        );
        assert_eq!(Square::new(8, 0), None, "за доской поля нет");
    }

    #[test]
    fn a_move_is_written_for_the_list() {
        let quiet = Move {
            from: Square::parse("c3").unwrap_or_else(|| panic!("c3")),
            to: Square::parse("d4").unwrap_or_else(|| panic!("d4")),
            captured: Vec::new(),
        };
        assert_eq!(move_text(&quiet), "c3-d4");

        let game = position(&[("c3", w()), ("d4", b())], Side::White);
        let beat = game.legal_moves().into_iter().next().unwrap_or_else(|| panic!("бой"));
        assert_eq!(move_text(&beat), "c3xe5", "бой помечается крестиком");
    }

    #[test]
    fn the_bot_returns_a_legal_move() {
        let game = Game::new();
        let bot = Bot::new(Level::Normal);
        let mv = bot.best_move(&game).unwrap_or_else(|| panic!("бот обязан найти ход"));
        assert!(
            game.legal_moves().iter().any(|m| m.from == mv.from && m.to == mv.to),
            "ход бота обязан быть легальным"
        );
    }

    #[test]
    fn the_bot_takes_a_free_piece() {
        // Белая c3 может побить d4 — это очевидно выгодно.
        let game = position(&[("c3", w()), ("d4", b()), ("h6", b())], Side::White);
        let mv = Bot::new(Level::Hard)
            .best_move(&game)
            .unwrap_or_else(|| panic!("бот обязан найти ход"));
        assert!(!mv.captured.is_empty(), "бот должен взять фигуру, а выбрал {mv:?}");
    }
}
