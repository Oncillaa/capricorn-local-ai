from PySide6.QtWidgets import (
    QDialog,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QWidget,
    QProgressBar,
    QGridLayout,
)
from PySide6.QtCore import Qt
from PySide6.QtGui import QFont
from save_manager import SaveManager


class LevelsDialog(QDialog):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("CAPRI // УРОВНИ")
        self.resize(500, 400)

        self.setStyleSheet("""
            QDialog { background-color: #1a1124; color: white; }
            QLabel { color: white; font-family: 'Segoe UI'; }
            QProgressBar {
                border: 1px solid #3a2a50;
                border-radius: 5px;
                text-align: center;
                background-color: #110a18;
            }
            QProgressBar::chunk { background-color: #28ff96; border-radius: 4px; }
        """)

        self.data = SaveManager.load()
        self.setup_ui()

    def setup_ui(self):
        layout = QVBoxLayout(self)

        # Заголовок
        title = QLabel("СТАТУС И ПРОГРЕСС")
        title.setFont(QFont("Segoe UI", 16, QFont.Weight.Bold))
        layout.addWidget(title)

        # Блок уровня
        lvl_box = QHBoxLayout()
        lvl_num = QLabel(str(self.data["level"]))
        lvl_num.setFont(QFont("Segoe UI", 48, QFont.Weight.Bold))
        lvl_num.setStyleSheet("color: #28ff96;")

        xp_box = QVBoxLayout()
        xp_label = QLabel(f"Опыт: {self.data['xp']} / {self.data['xp_to_next']}")

        self.xp_bar = QProgressBar()
        self.xp_bar.setMaximum(self.data["xp_to_next"])
        self.xp_bar.setValue(self.data["xp"])
        self.xp_bar.setTextVisible(False)
        self.xp_bar.setFixedHeight(15)

        xp_box.addWidget(xp_label)
        xp_box.addWidget(self.xp_bar)

        lvl_box.addWidget(lvl_num)
        lvl_box.addLayout(xp_box)
        layout.addLayout(lvl_box)

        layout.addSpacing(20)

        # Настроение и Усталость
        stats_grid = QGridLayout()

        stats_grid.addWidget(QLabel("Радость:"), 0, 0)
        joy_bar = QProgressBar()
        joy_bar.setValue(self.data.get("mood_joy", 92))
        joy_bar.setStyleSheet("QProgressBar::chunk { background-color: #ff2260; }")
        stats_grid.addWidget(joy_bar, 0, 1)

        stats_grid.addWidget(QLabel("Усталость:"), 1, 0)
        tired_bar = QProgressBar()
        tired_bar.setValue(self.data.get("mood_tired", 46))
        tired_bar.setStyleSheet("QProgressBar::chunk { background-color: #6432c4; }")
        stats_grid.addWidget(tired_bar, 1, 1)

        layout.addLayout(stats_grid)
        layout.addStretch()

        # Кошелек
        money = QLabel(f"В кошельке: $ {self.data['money']}")
        money.setFont(QFont("Segoe UI", 12, QFont.Weight.Bold))
        money.setStyleSheet("color: #28ff96;")
        layout.addWidget(money, alignment=Qt.AlignmentFlag.AlignRight)
