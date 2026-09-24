from __future__ import annotations

from PySide6.QtCore import Qt
from PySide6.QtWidgets import (
    QDialog,
    QDialogButtonBox,
    QGridLayout,
    QGroupBox,
    QLabel,
    QTableWidget,
    QTableWidgetItem,
    QVBoxLayout,
)

from vanta.core.models import Priority
from vanta.core.policy_lab import QueuePolicy, SimulationJob, SimulationResult, simulate


DEMO_JOBS = [
    SimulationJob("Warm-up batch", Priority.NORMAL, 512, 6, 0),
    SimulationJob("Critical inference", Priority.CRITICAL, 800, 4, 1),
    SimulationJob("Small preview", Priority.LOW, 256, 2, 1),
]


class PolicyLabDialog(QDialog):
    def __init__(self, parent=None) -> None:
        super().__init__(parent)
        self.setWindowTitle("VAnta Policy Lab")
        self.setMinimumSize(760, 520)
        root = QVBoxLayout(self)
        root.setContentsMargins(22, 20, 22, 18)
        root.setSpacing(14)

        title = QLabel("Compare queue policies")
        title.setObjectName("sectionTitle")
        description = QLabel(
            "A deterministic replay shows what happens when a high-priority job "
            "needs more VRAM than the current free capacity. The replay uses "
            "two hypothetical slots; the live controller runs one job at a time."
        )
        description.setWordWrap(True)
        description.setObjectName("muted")
        root.addWidget(title)
        root.addWidget(description)

        jobs_box = QGroupBox("Scenario workload")
        jobs_layout = QVBoxLayout(jobs_box)
        jobs_table = QTableWidget(len(DEMO_JOBS), 5)
        jobs_table.setHorizontalHeaderLabels(["JOB", "PRIORITY", "VRAM", "DURATION", "ARRIVES"])
        jobs_table.setEditTriggers(QTableWidget.EditTrigger.NoEditTriggers)
        jobs_table.verticalHeader().setVisible(False)
        jobs_table.horizontalHeader().setStretchLastSection(True)
        for row, job in enumerate(DEMO_JOBS):
            values = [
                job.name,
                job.priority.label,
                f"{job.required_vram_mb} MB",
                f"{job.duration_s:.0f} sec",
                f"{job.submitted_at_s:.0f} sec",
            ]
            for column, value in enumerate(values):
                jobs_table.setItem(row, column, QTableWidgetItem(value))
        jobs_layout.addWidget(jobs_table)
        root.addWidget(jobs_box)

        results_grid = QGridLayout()
        self.strict_summary = self._result_box("Strict priority")
        self.fit_summary = self._result_box("Fit what fits")
        results_grid.addWidget(self.strict_summary, 0, 0)
        results_grid.addWidget(self.fit_summary, 0, 1)
        root.addLayout(results_grid)

        explanation = QLabel()
        explanation.setWordWrap(True)
        explanation.setObjectName("muted")
        root.addWidget(explanation)

        strict = simulate(DEMO_JOBS, policy=QueuePolicy.STRICT_PRIORITY)
        fit = simulate(DEMO_JOBS, policy=QueuePolicy.FIT_WHAT_FITS)
        self._set_result(self.strict_summary, strict)
        self._set_result(self.fit_summary, fit)
        explanation.setText(
            "Strict priority protects ordering and makes the small preview wait behind "
            "the blocked critical job. Fit what fits starts the preview earlier, "
            "improving its wait time while preserving the VRAM reserve."
        )

        buttons = QDialogButtonBox(QDialogButtonBox.StandardButton.Close)
        buttons.rejected.connect(self.reject)
        buttons.accepted.connect(self.accept)
        root.addWidget(buttons)

    @staticmethod
    def _result_box(title: str) -> QGroupBox:
        box = QGroupBox(title)
        layout = QVBoxLayout(box)
        label = QLabel()
        label.setWordWrap(True)
        label.setProperty("resultLabel", True)
        layout.addWidget(label)
        return box

    @staticmethod
    def _set_result(box: QGroupBox, result: SimulationResult) -> None:
        label = box.findChild(QLabel)
        if label is None:
            return
        order = " → ".join(result.order) or "No jobs started"
        label.setText(
            f"Start order: {order}\n"
            f"Average wait: {result.average_wait_s:.1f} sec\n"
            f"Makespan: {result.makespan_s:.1f} sec\n"
            f"GPU busy: {result.utilization_pct:.0f}%"
        )
