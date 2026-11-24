import os

from vanta.infra.task_manager import TaskManager


def test_snapshot_contains_current_process_and_real_system_memory(tmp_path):
    manager = TaskManager(tmp_path / "history.sqlite")
    try:
        snapshot = manager.sample()
        current = next(p for p in snapshot.processes if p.pid == os.getpid())
        assert current.created > 0
        assert current.memory > 0
        assert snapshot.memory_total > 0
        assert 0 <= snapshot.cpu_percent <= 100
    finally:
        manager.close()

import subprocess
import sys

import psutil
import pytest
from vanta.infra.task_manager import ProcessIdentity


def test_reused_pid_is_rejected_before_termination(tmp_path):
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
    manager = TaskManager(tmp_path / "history.sqlite")
    try:
        identity = ProcessIdentity(child.pid, psutil.Process(child.pid).create_time() - 20)
        with pytest.raises(ValueError, match="reused"):
            manager.end_process(identity)
        assert child.poll() is None
        manager.end_process(ProcessIdentity(child.pid, psutil.Process(child.pid).create_time()))
        child.wait(timeout=10)
    finally:
        if child.poll() is None:
            child.terminate()
            child.wait(timeout=10)
        manager.close()


def test_vanta_cannot_terminate_itself(tmp_path):
    manager = TaskManager(tmp_path / "history.sqlite")
    try:
        identity = ProcessIdentity(os.getpid(), psutil.Process().create_time())
        with pytest.raises(PermissionError, match="itself"):
            manager.end_process(identity)
    finally:
        manager.close()


def test_history_counts_only_observed_deltas_and_can_be_reset(tmp_path):
    manager = TaskManager(tmp_path / "history.sqlite")
    try:
        manager.sample()
        assert manager.history()[1] == []
        manager.sample()
        assert manager.history()[1]
        manager.reset_history()
        assert manager.history()[1] == []
        manager.sample()
        assert manager.history()[1] == []
    finally:
        manager.close()


def test_startup_file_disable_and_enable_preserves_contents(tmp_path):
    from vanta.infra.startup import StartupManager
    startup_file = tmp_path / "demo.lnk"
    startup_file.write_bytes(b"original startup shortcut")
    manager = StartupManager(tmp_path / "vanta")
    entry = {"id": "demo", "Name": "demo", "kind": "file", "path": str(startup_file)}
    manager.toggle(entry, False)
    assert not startup_file.exists()
    manager.toggle(entry, True)
    assert startup_file.read_bytes() == b"original startup shortcut"


def test_startup_restore_does_not_overwrite_new_entry(tmp_path):
    from vanta.infra.startup import StartupManager
    startup_file = tmp_path / "demo.lnk"
    startup_file.write_bytes(b"old shortcut")
    manager = StartupManager(tmp_path / "vanta")
    entry = {"id": "demo", "Name": "demo", "kind": "file", "path": str(startup_file)}
    manager.toggle(entry, False)
    startup_file.write_bytes(b"new shortcut")
    with pytest.raises(ValueError, match="overwritten"):
        manager.toggle(entry, True)
    assert startup_file.read_bytes() == b"new shortcut"

@pytest.mark.skipif(os.name != "nt", reason="Windows system integration")
def test_windows_diagnostics_operate_only_on_owned_child(tmp_path):
    from vanta.infra import windows_tasks
    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
    manager = TaskManager(tmp_path / "history.sqlite")
    try:
        process = psutil.Process(child.pid)
        identity = ProcessIdentity(child.pid, process.create_time())
        original_affinity = process.cpu_affinity()
        manager.set_priority(identity, psutil.BELOW_NORMAL_PRIORITY_CLASS)
        assert process.nice() == psutil.BELOW_NORMAL_PRIORITY_CLASS
        manager.set_affinity(identity, original_affinity[:1])
        assert process.cpu_affinity() == original_affinity[:1]
        manager.set_affinity(identity, original_affinity)
        windows_tasks.set_efficiency(identity, True)
        windows_tasks.set_efficiency(identity, False)
        chain = windows_tasks.wait_chain(identity)
        assert "Thread" in chain
        dump = tmp_path / "child.dmp"
        windows_tasks.create_process_dump(identity, str(dump))
        assert dump.read_bytes()[:4] == b"MDMP"
        with pytest.raises(FileExistsError):
            windows_tasks.create_process_dump(identity, str(dump))
        assert windows_tasks.memory_counters()["Commit limit"] > 0
        assert windows_tasks.user_sessions()
    finally:
        child.terminate()
        child.wait(timeout=10)
        manager.close()
