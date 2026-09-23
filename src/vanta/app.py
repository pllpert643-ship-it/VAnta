from __future__ import annotations

import os
import sys
from pathlib import Path

from PySide6.QtGui import QColor, QPalette
from PySide6.QtWidgets import QApplication

from vanta.core.rules import RuleConfig, RuleEngine
from vanta.core.scheduler import Scheduler
from vanta.core.storage import VantaRepository
from vanta.infra.monitor import SystemMonitor
from vanta.infra.processes import ProcessRunner
from vanta.ui.main_window import MainWindow
from vanta.ui.theme import APP_STYLESHEET


def data_directory() -> Path:
    base = Path(os.environ.get("LOCALAPPDATA", Path.home() / ".vanta"))
    return base / "VAnta"


def run_app() -> int:
    app = QApplication(sys.argv)
    app.setApplicationName("VAnta")
    app.setOrganizationName("VAnta")
    app.setStyle("Fusion")
    app.setStyleSheet(APP_STYLESHEET)
    palette = QPalette()
    palette.setColor(QPalette.ColorRole.Window, QColor("#090A0F"))
    palette.setColor(QPalette.ColorRole.WindowText, QColor("#E7E9F2"))
    palette.setColor(QPalette.ColorRole.Base, QColor("#0D0F16"))
    palette.setColor(QPalette.ColorRole.Text, QColor("#E7E9F2"))
    palette.setColor(QPalette.ColorRole.Highlight, QColor("#6E55C7"))
    app.setPalette(palette)

    data_dir = data_directory()
    repository = VantaRepository(data_dir / "vanta.db")
    monitor = SystemMonitor()
    runner = ProcessRunner(data_dir / "logs")
    scheduler = Scheduler(
        rules=RuleEngine(RuleConfig(max_concurrent_jobs=1, reserved_vram_mb=512)),
        runner=runner,
        repository=repository,
    )
    window = MainWindow(monitor=monitor, scheduler=scheduler)
    window.show()
    return app.exec()
