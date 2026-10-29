import math
from PySide6.QtWidgets import QWidget
from PySide6.QtCore import Qt, QRectF, Signal
from PySide6.QtGui import QPainter, QColor, QPen, QFont, QPainterPath


class RadialMenu(QWidget):
    item_selected = Signal(str)

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowFlags(
            Qt.WindowType.FramelessWindowHint
            | Qt.WindowType.WindowStaysOnTopHint
            | Qt.WindowType.Tool
        )
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground)
        self.setMouseTracking(True)
        self.resize(560, 560)  # Чуть больше размер

        self.center_radius = 90
        self.outer_radius = 270
        self.items = [
            "Настройки",
            "Чат",
            "Игры",
            "Магазин",
            "Инструменты",
            "Заметки",
            "Капри",
            "Уровни",
            "Разработчики",
        ]
        self.hovered_index = -1

    def paintEvent(self, event):
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        center = self.rect().center()
        count = len(self.items)
        angle_step = 360 / count

        for i, item in enumerate(self.items):
            start_angle = i * angle_step - 90 - (angle_step / 2)

            # Подсветка при наведении
            if i == self.hovered_index:
                color = QColor(100, 50, 200, 255)
                pen_color = QColor(40, 255, 150)  # Зеленая рамка
            else:
                color = QColor(26, 17, 36, 240)
                pen_color = QColor(100, 50, 200)

            path = QPainterPath()
            path.moveTo(center)
            path.arcTo(
                QRectF(
                    center.x() - self.outer_radius,
                    center.y() - self.outer_radius,
                    self.outer_radius * 2,
                    self.outer_radius * 2,
                ),
                -start_angle,
                -angle_step,
            )
            path.closeSubpath()

            inner_path = QPainterPath()
            inner_path.addEllipse(center, self.center_radius, self.center_radius)
            path = path.subtracted(inner_path)

            painter.setBrush(color)
            painter.setPen(QPen(pen_color, 2))
            painter.drawPath(path)

            # Текст
            painter.setPen(Qt.GlobalColor.white)
            painter.setFont(QFont("Segoe UI", 11, QFont.Weight.Bold))

            text_radius = (self.center_radius + self.outer_radius) / 2
            text_angle = math.radians(start_angle + angle_step / 2)
            tx = center.x() + text_radius * math.cos(text_angle)
            ty = center.y() + text_radius * math.sin(text_angle)

            rect = QRectF(tx - 70, ty - 20, 140, 40)
            painter.drawText(rect, Qt.AlignmentFlag.AlignCenter, item)

        # Центр
        painter.setBrush(QColor(16, 10, 24))
        painter.setPen(QPen(QColor(100, 50, 200), 4))
        painter.drawEllipse(center, self.center_radius, self.center_radius)

        painter.setPen(QColor(40, 255, 150))
        painter.setFont(QFont("Segoe UI", 28, QFont.Weight.Bold))
        painter.drawText(
            QRectF(center.x() - 40, center.y() - 40, 80, 80),
            Qt.AlignmentFlag.AlignCenter,
            "X",
        )

    def mouseMoveEvent(self, event):
        center = self.rect().center()
        dx = event.position().x() - center.x()
        dy = event.position().y() - center.y()
        dist = math.hypot(dx, dy)

        if self.center_radius < dist < self.outer_radius:
            angle = math.degrees(math.atan2(dy, dx)) + 90
            if angle < 0:
                angle += 360

            angle_step = 360 / len(self.items)
            self.hovered_index = int((angle + angle_step / 2) % 360 // angle_step)
            self.update()
        else:
            if self.hovered_index != -1:
                self.hovered_index = -1
                self.update()

    def mousePressEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            center = self.rect().center()
            dx = event.position().x() - center.x()
            dy = event.position().y() - center.y()
            dist = math.hypot(dx, dy)

            if dist < self.center_radius:
                self.close()
            elif self.hovered_index != -1:
                self.item_selected.emit(self.items[self.hovered_index])
                self.close()
