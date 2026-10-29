import chess
import random
from PySide6.QtWidgets import (
    QDialog,
    QVBoxLayout,
    QLabel,
    QPushButton,
    QGridLayout,
    QWidget,
)
from PySide6.QtGui import QFont, QIcon, QPixmap
from PySide6.QtCore import Qt


class ChessGame(QDialog):
    def __init__(self, capri_instance, parent=None):
        super().__init__(parent)
        self.capri = capri_instance
        self.setWindowTitle("CAPRI // ШАХМАТЫ")
        self.resize(600, 650)

        self.setStyleSheet("""
            QDialog { background-color: #1a1124; color: white; }
            QLabel { color: white; }
        """)

        self.board = chess.Board()
        self.selected_square = None
        self.buttons = {}

        self.setup_ui()
        self.update_board()

    def setup_ui(self):
        layout = QVBoxLayout(self)

        # Заголовок
        self.status_label = QLabel("Твой ход (белые)")
        self.status_label.setFont(QFont("Segoe UI", 14, QFont.Weight.Bold))
        self.status_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(self.status_label)

        # Доска
        grid = QGridLayout()
        grid.setSpacing(0)

        # Картинки шахмат (используем юникод для простоты, чтобы не искать картинки)
        self.pieces = {
            "P": "♙",
            "N": "♘",
            "B": "♗",
            "R": "♖",
            "Q": "♕",
            "K": "♔",
            "p": "♟",
            "n": "♞",
            "b": "♝",
            "r": "♜",
            "q": "♛",
            "k": "♚",
        }

        for row in range(8):
            for col in range(8):
                btn = QPushButton()
                btn.setFixedSize(70, 70)
                btn.setFont(QFont("Segoe UI", 40))

                # Цвета клеток
                is_light = (row + col) % 2 == 0
                color = "#3a2a50" if is_light else "#241833"
                btn.setStyleSheet(
                    f"background-color: {color}; border: none; color: white;"
                )

                btn.clicked.connect(
                    lambda checked, r=row, c=col: self.on_square_click(r, c)
                )
                grid.addWidget(btn, row, col)

                # Запоминаем кнопку по координатам (a1, b1 и т.д.)
                sq_name = chess.square_name(chess.square(col, 7 - row))
                self.buttons[sq_name] = btn

        layout.addLayout(grid)

        # Кнопка сброса
        reset_btn = QPushButton("НОВАЯ ПАРТИЯ")
        reset_btn.setStyleSheet(
            "background-color: #28ff96; color: black; padding: 10px; font-weight: bold;"
        )
        reset_btn.clicked.connect(self.reset_game)
        layout.addWidget(reset_btn)

    def update_board(self):
        for sq_name, btn in self.buttons.items():
            square = chess.parse_square(sq_name)
            piece = self.board.piece_at(square)

            if piece:
                btn.setText(self.pieces[piece.symbol()])
                # Цвет фигур: белые - белые, черные - красные (как на скрине)
                btn.setStyleSheet(
                    btn.styleSheet()
                    + f" color: {'#ff2260' if piece.color == chess.BLACK else 'white'};"
                )
            else:
                btn.setText("")
                # Возвращаем базовый цвет
                row = 7 - chess.square_rank(square)
                col = chess.square_file(square)
                is_light = (row + col) % 2 == 0
                color = "#3a2a50" if is_light else "#241833"
                btn.setStyleSheet(f"background-color: {color}; border: none;")

    def on_square_click(self, row, col):
        sq_name = chess.square_name(chess.square(col, 7 - row))

        if self.board.turn == chess.BLACK:
            return  # Ждем хода ИИ

        if self.selected_square is None:
            # Выбор фигуры
            piece = self.board.piece_at(chess.parse_square(sq_name))
            if piece and piece.color == chess.WHITE:
                self.selected_square = sq_name
                # Подсветка выбранной клетки
                self.buttons[sq_name].setStyleSheet(
                    "background-color: #28ff96; border: none;"
                )
        else:
            # Попытка хода
            move = chess.Move.from_uci(f"{self.selected_square}{sq_name}")
            if move in self.board.legal_moves:
                self.board.push(move)
                self.selected_square = None
                self.update_board()
                self.status_label.setText("Ход Капри...")
                # Задержка для имитации "раздумий" ИИ
                from PySide6.QtCore import QTimer

                QTimer.singleShot(500, self.ai_move)
            else:
                # Отмена выбора
                self.selected_square = None
                self.update_board()

    def ai_move(self):
        if self.board.is_game_over():
            self.status_label.setText("Игра окончена!")
            return

        legal_moves = list(self.board.legal_moves)
        if legal_moves:
            move = random.choice(legal_moves)  # Простой рандомный ИИ
            self.board.push(move)
            self.update_board()
            self.status_label.setText("Твой ход (белые)")

            if self.board.is_game_over():
                self.status_label.setText("Игра окончена!")

    def reset_game(self):
        self.board.reset()
        self.selected_square = None
        self.update_board()
        self.status_label.setText("Твой ход (белые)")
