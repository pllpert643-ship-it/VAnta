from __future__ import annotations

from collections import deque

from PySide6.QtCore import QPointF, QRectF, Qt
from PySide6.QtGui import QColor, QPainter, QPainterPath, QPen
from PySide6.QtWidgets import QFrame, QHBoxLayout, QLabel, QVBoxLayout, QWidget


class MetricCard(QFrame):
    def __init__(self, title: str, accent: bool = False, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setObjectName("metricCard")
        layout = QVBoxLayout(self)
        layout.setContentsMargins(15, 12, 15, 12)
        layout.setSpacing(3)

        self.title = QLabel(title.upper())
        self.title.setObjectName("overline")
        self.value = QLabel("—")
        self.value.setObjectName("metricAccent" if accent else "metricValue")
        self.detail = QLabel("Waiting for telemetry")
        self.detail.setObjectName("muted")

        layout.addWidget(self.title)
        layout.addWidget(self.value)
        layout.addWidget(self.detail)

    def update_value(self, value: str, detail: str) -> None:
        self.value.setText(value)
        self.detail.setText(detail)


class TrendChart(QWidget):
    def __init__(self, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        self.setMinimumHeight(150)
        self.gpu_values: deque[float] = deque(maxlen=60)
        self.vram_values: deque[float] = deque(maxlen=60)

    def add_sample(self, gpu: float, vram: float) -> None:
        self.gpu_values.append(max(0.0, min(100.0, gpu)))
        self.vram_values.append(max(0.0, min(100.0, vram)))
        self.update()

    def paintEvent(self, event: object) -> None:  # noqa: N802 - Qt API
        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        bounds = QRectF(self.rect()).adjusted(6, 7, -6, -8)

        grid_pen = QPen(QColor("#222633"), 1)
        painter.setPen(grid_pen)
        for index in range(5):
            y = bounds.top() + (bounds.height() * index / 4)
            painter.drawLine(QPointF(bounds.left(), y), QPointF(bounds.right(), y))

        self._draw_series(painter, bounds, list(self.vram_values), QColor("#4DD8C7"), 2.0)
        self._draw_series(painter, bounds, list(self.gpu_values), QColor("#A98DFF"), 2.3)

    @staticmethod
    def _draw_series(
        painter: QPainter,
        bounds: QRectF,
        values: list[float],
        color: QColor,
        width: float,
    ) -> None:
        if len(values) < 2:
            return
        path = QPainterPath()
        for index, value in enumerate(values):
            x = bounds.left() + bounds.width() * index / max(59, len(values) - 1)
            y = bounds.bottom() - bounds.height() * value / 100
            if index == 0:
                path.moveTo(x, y)
            else:
                path.lineTo(x, y)
        painter.setPen(QPen(color, width, Qt.PenStyle.SolidLine, Qt.PenCapStyle.RoundCap))
        painter.drawPath(path)


class FlowStep(QFrame):
    def __init__(self, number: str, title: str, detail: str, parent: QWidget | None = None) -> None:
        super().__init__(parent)
        row = QHBoxLayout(self)
        row.setContentsMargins(0, 4, 0, 4)
        row.setSpacing(10)
        badge = QLabel(number)
        badge.setAlignment(Qt.AlignmentFlag.AlignCenter)
        badge.setFixedSize(26, 26)
        badge.setStyleSheet(
            "color:#BBAAFF; background:#211C38; border:1px solid #493D78; "
            "border-radius:13px; font-weight:700;"
        )
        labels = QVBoxLayout()
        labels.setSpacing(0)
        title_label = QLabel(title)
        title_label.setStyleSheet("font-weight:650; color:#F2F2F8;")
        detail_label = QLabel(detail)
        detail_label.setObjectName("muted")
        labels.addWidget(title_label)
        labels.addWidget(detail_label)
        row.addWidget(badge)
        row.addLayout(labels, 1)
