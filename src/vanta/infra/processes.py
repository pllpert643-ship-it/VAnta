from __future__ import annotations

import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

import psutil

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

    def recover(self, job: Job) -> bool:
        """Stop a child left behind by a crashed VAnta instance.

        A new ``ProcessRunner`` cannot reconstruct a ``Popen`` handle, so it
        uses the persisted PID after checking the command line belongs to the
        submitted job. A mismatched live PID is treated as unsafe to touch.
        """
        if job.pid is None:
            return True
        try:
            process = psutil.Process(job.pid)
        except psutil.NoSuchProcess:
            return True
        except psutil.Error:
            return False

        try:
            command_line = process.cmdline()
        except psutil.Error:
            return False
        if not self._matches_job(command_line, job.command):
            return False

        try:
            process.terminate()
            process.wait(timeout=3)
        except psutil.TimeoutExpired:
            try:
                process.kill()
                process.wait(timeout=2)
            except psutil.Error:
                return False
        except psutil.Error:
            return False
        return not process.is_running()

    @staticmethod
    def _matches_job(command_line: list[str], command: list[str]) -> bool:
        if not command_line or not command:
            return False
        expected_executable = os.path.basename(command[0]).casefold()
        actual_executable = os.path.basename(command_line[0]).casefold()
        if expected_executable != actual_executable:
            return False
        actual_arguments = {str(argument).strip('"').casefold() for argument in command_line[1:]}
        expected_arguments = {str(argument).strip('"').casefold() for argument in command[1:]}
        return expected_arguments.issubset(actual_arguments)

    def close(self) -> None:
        for job_id in list(self._handles):
            self.cancel(job_id)
