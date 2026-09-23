from __future__ import annotations

import json
import sqlite3
from pathlib import Path
from threading import Lock

from vanta.core.models import ActivityEvent, Job


class VantaRepository:
    def __init__(self, database_path: Path) -> None:
        database_path.parent.mkdir(parents=True, exist_ok=True)
        self._connection = sqlite3.connect(database_path, check_same_thread=False)
        self._connection.row_factory = sqlite3.Row
        self._lock = Lock()
        self._create_schema()

    def _create_schema(self) -> None:
        with self._lock, self._connection:
            self._connection.executescript(
                """
                CREATE TABLE IF NOT EXISTS jobs (
                    id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    command_json TEXT NOT NULL,
                    priority INTEGER NOT NULL,
                    required_vram_mb INTEGER NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    started_at TEXT,
                    finished_at TEXT,
                    pid INTEGER,
                    exit_code INTEGER,
                    status_reason TEXT NOT NULL,
                    verified INTEGER NOT NULL DEFAULT 0,
                    before_snapshot_json TEXT
                );
                CREATE TABLE IF NOT EXISTS activity (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    timestamp TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    message TEXT NOT NULL,
                    job_id TEXT,
                    details_json TEXT NOT NULL
                );
                """
            )

    def save_job(self, job: Job) -> None:
        values = job.to_dict()
        with self._lock, self._connection:
            self._connection.execute(
                """
                INSERT INTO jobs (
                    id, name, command_json, priority, required_vram_mb, status,
                    created_at, started_at, finished_at, pid, exit_code,
                    status_reason, verified, before_snapshot_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(id) DO UPDATE SET
                    status=excluded.status,
                    started_at=excluded.started_at,
                    finished_at=excluded.finished_at,
                    pid=excluded.pid,
                    exit_code=excluded.exit_code,
                    status_reason=excluded.status_reason,
                    verified=excluded.verified,
                    before_snapshot_json=excluded.before_snapshot_json
                """,
                (
                    values["id"], values["name"], json.dumps(values["command"]),
                    values["priority"], values["required_vram_mb"], values["status"],
                    values["created_at"], values["started_at"], values["finished_at"],
                    values["pid"], values["exit_code"], values["status_reason"],
                    int(values["verified"]), json.dumps(values["before_snapshot"])
                    if values["before_snapshot"] else None,
                ),
            )

    def add_event(self, event: ActivityEvent) -> None:
        with self._lock, self._connection:
            self._connection.execute(
                """INSERT INTO activity
                   (timestamp, kind, message, job_id, details_json)
                   VALUES (?, ?, ?, ?, ?)""",
                (
                    event.timestamp,
                    event.kind,
                    event.message,
                    event.job_id,
                    json.dumps(event.details),
                ),
            )

    def recent_events(self, limit: int = 30) -> list[dict[str, object]]:
        with self._lock:
            rows = self._connection.execute(
                "SELECT * FROM activity ORDER BY sequence DESC LIMIT ?", (limit,)
            ).fetchall()
        return [dict(row) for row in rows]

    def close(self) -> None:
        with self._lock:
            self._connection.close()
