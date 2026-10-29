from PySide6.QtWidgets import (
    QDialog,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QWidget,
    QGridLayout,
)
from PySide6.QtCore import Qt
from PySide6.QtGui import QFont, QColor
from save_manager import SaveManager


class ShopDialog(QDialog):
    def __init__(self, capri_instance, parent=None):
        super().__init__(parent)
        self.capri = capri_instance
        self.setWindowTitle("CAPRI // МАГАЗИН")
        self.resize(700, 450)

        self.setStyleSheet("""
            QDialog { background-color: #1a1124; color: white; }
            QLabel { color: white; font-family: 'Segoe UI'; }
            QPushButton { 
                background-color: #2a1e3d; 
                border: 2px solid #6432c4; 
                color: white; 
                padding: 10px;
                font-weight: bold;
            }
            QPushButton:hover { background-color: #3d2b59; }
            QPushButton:disabled { border-color: #444; color: #888; }
        """)

        self.skins = {
            "purple": {"name": "Фиолетовый", "price": 0, "color": None},
            "neon": {"name": "Неон", "price": 150, "color": QColor(40, 255, 150)},
            "red": {"name": "Алый", "price": 300, "color": QColor(255, 34, 96)},
            "ice": {"name": "Лёд", "price": 450, "color": QColor(100, 200, 255)},
            "toxic": {"name": "Токсин", "price": 350, "color": QColor(200, 255, 40)},
            "sunset": {"name": "Закат", "price": 400, "color": QColor(255, 140, 0)},
        }

        self.data = SaveManager.load()

        # Создаем базовый лейаут один раз
        self.main_layout = QVBoxLayout(self)
        self.setup_ui()

    def setup_ui(self):
        # Очищаем старый лейаут полностью
        while self.main_layout.count():
            item = self.main_layout.takeAt(0)
            widget = item.widget()
            if widget:
                widget.deleteLater()
            elif item.layout():
                # Удаляем вложенные лейауты
                while item.layout().count():
                    subitem = item.layout().takeAt(0)
                    if subitem.widget():
                        subitem.widget().deleteLater()

        # Шапка с балансом
        header = QHBoxLayout()
        title = QLabel("СТИЛИ")
        title.setFont(QFont("Segoe UI", 16, QFont.Weight.Bold))
        self.balance_label = QLabel(f"$ {self.data['money']}")
        self.balance_label.setFont(QFont("Segoe UI", 16, QFont.Weight.Bold))
        self.balance_label.setStyleSheet("color: #28ff96;")

        header.addWidget(title)
        header.addStretch()
        header.addWidget(self.balance_label)
        self.main_layout.addLayout(header)

        # Сетка скинов
        grid = QGridLayout()
        row, col = 0, 0

        for skin_id, info in self.skins.items():
            card = QWidget()
            # Цвет рамки: зеленый, если скин надет, иначе фиолетовый
            border_color = (
                "#28ff96" if self.data["equipped_skin"] == skin_id else "#3a2a50"
            )
            card.setStyleSheet(
                f"background-color: #241833; border-radius: 8px; border: 2px solid {border_color};"
            )

            card_layout = QVBoxLayout(card)

            name = QLabel(info["name"])
            name.setAlignment(Qt.AlignmentFlag.AlignCenter)
            name.setFont(QFont("Segoe UI", 12, QFont.Weight.Bold))

            btn = QPushButton()
            if self.data["equipped_skin"] == skin_id:
                btn.setText("НАДЕТО")
                btn.setEnabled(False)
            elif skin_id in self.data["owned_skins"]:
                btn.setText("НАДЕТЬ")
                btn.clicked.connect(lambda checked, s=skin_id: self.equip_skin(s))
            else:
                btn.setText(f"КУПИТЬ • {info['price']} $")
                btn.clicked.connect(
                    lambda checked, s=skin_id, p=info["price"]: self.buy_skin(s, p)
                )

            card_layout.addWidget(name)
            card_layout.addWidget(btn)
            grid.addWidget(card, row, col)

            col += 1
            if col > 2:
                col = 0
                row += 1

        self.main_layout.addLayout(grid)

    def buy_skin(self, skin_id, price):
        if self.data["money"] >= price:
            self.data["money"] -= price
            self.data["owned_skins"].append(skin_id)
            SaveManager.save(self.data)

            # Отправляем голос в фоновый поток (без зависаний)
            self.capri.jobs.put(
                ("say", f"Куплен скин {self.skins[skin_id]['name']}. Отличный выбор!")
            )

            self.data = SaveManager.load()
            self.setup_ui()  # Перерисовываем
        else:
            self.capri.jobs.put(("say", "Не хватает денег. Придётся поработать."))

    def equip_skin(self, skin_id):
        self.data["equipped_skin"] = skin_id
        SaveManager.save(self.data)

        # Отправляем сигнал для смены цвета (если None - сбросит на стандартный)
        self.capri.update_avatar_skin.emit(self.skins[skin_id]["color"])

        self.setup_ui()  # Перерисовываем
