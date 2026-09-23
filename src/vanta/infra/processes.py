from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

from vanta.core.models import Job


@dataclass(slots=True)
class ProcessHandle:
    process: subprocess.Popen[bytes]
    log_file: object


class ProcessRunner:
    def __init__(self, log_directory: Path) -> None:
        self.log_directory = log_directory
        self.log_directory.mkdir(parents=True, exist_ok=True)
        self._handles: dict[str, ProcessHandle] = {}

    def start(self, job: Job) -> int:
        log_path = self.log_directory / f"{job.id}.log"
        log_file = log_path.open("ab")
        creation_flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
        try:
            process = subprocess.Popen(
                job.command,
                stdin=subprocess.DEVNULL,
                stdout=log_file,
                stderr=subprocess.STDOUT,
                shell=False,
                creationflags=creation_flags,
            )
        except Exception:
            log_file.close()
            raise
        self._handles[job.id] = ProcessHandle(process=process, log_file=log_file)
        return process.pid

    def poll(self, job_id: str) -> int | None:
        handle = self._handles.get(job_id)
        if handle is None:
            return -1
        return_code = handle.process.poll()
        if return_code is not None:
            handle.log_file.close()
            self._handles.pop(job_id, None)
        return return_code

    def cancel(self, job_id: str) -> bool:
        handle = self._handles.get(job_id)
        if handle is None:
            return False
        handle.process.terminate()
        try:
            handle.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            handle.process.kill()
            handle.process.wait(timeout=2)
        handle.log_file.close()
        self._handles.pop(job_id, None)
        return True

    def close(self) -> None:
        for job_id in list(self._handles):
            self.cancel(job_id)
