"""Safety tests: never run real memory-maintenance calls on the host."""
import pytest

from vanta.infra.memory_actions import MemoryActions


class UntouchableNative:
    def __getattr__(self, name):
        raise AssertionError(f"native work before validation: {name}")


@pytest.mark.parametrize("operation,confirmed,error", [
    ("unknown", True, ValueError),
    ("trim_system_cache", False, PermissionError),
    ("purge_standby", 1, PermissionError),
])
def test_invalid_or_unconfirmed_requests_never_touch_native(operation, confirmed, error):
    with pytest.raises(error):
        MemoryActions(native=UntouchableNative()).execute(operation, confirmed=confirmed)

from contextlib import contextmanager


class FakeNative:
    def __init__(self):
        self.events = []
        self.samples = iter([100, 140])
        self.created = 1234.5
        self.admin = True

    def available(self):
        return next(self.samples)

    @contextmanager
    def process(self, pid):
        self.events.append(("open", pid))
        try:
            yield 0x123456789
        finally:
            self.events.append("close")

    def process_created(self, handle):
        self.events.append(("identity", handle))
        return self.created

    def trim_process(self, handle):
        self.events.append(("trim", handle))


def test_trim_process_checks_opened_identity_then_closes_handle_and_reports_bytes():
    native = FakeNative()
    result = MemoryActions(native=native).execute("trim_process", confirmed=True, pid=12, created=1234.5)
    assert (result.operation, result.before_available, result.after_available) == ("trim_process", 100, 140)
    assert native.events == [("open", 12), ("identity", 0x123456789), ("trim", 0x123456789), "close"]


def test_reused_pid_is_rejected_and_opened_handle_closed():
    native = FakeNative()
    with pytest.raises(ValueError, match="identity|reused"):
        MemoryActions(native=native).execute("trim_process", confirmed=True, pid=12, created=1230)
    assert native.events == [("open", 12), ("identity", 0x123456789), "close"]

@contextmanager
def fake_privilege(self, name):
    self.events.append(("enable", name))
    try:
        yield
    finally:
        self.events.append(("restore", name))


FakeNative.privilege = fake_privilege
FakeNative.is_admin = lambda self: self.admin
FakeNative.trim_system_cache = lambda self: self.events.append("cache")
FakeNative.purge_standby = lambda self, command: self.events.append(("purge", command))


@pytest.mark.parametrize("operation,privilege,action", [
    ("trim_system_cache", "SeIncreaseQuotaPrivilege", "cache"),
    ("purge_standby", "SeProfileSingleProcessPrivilege", ("purge", 4)),
    ("purge_standby_priority0", "SeProfileSingleProcessPrivilege", ("purge", 5)),
])
def test_machine_operations_scope_privilege_and_require_admin(operation, privilege, action):
    native = FakeNative()
    result = MemoryActions(native=native).execute(operation, confirmed=True)
    assert native.events == [("enable", privilege), action, ("restore", privilege)]
    assert (result.operation, result.before_available, result.after_available) == (operation, 100, 140)
    if operation.startswith("purge"):
        assert "unsupported" in result.message.lower()
    native = FakeNative()
    native.admin = False
    with pytest.raises(PermissionError, match="administrator"):
        MemoryActions(native=native).execute(operation, confirmed=True)
    assert native.events == []

@pytest.mark.parametrize("pid,created", [(None, None), (True, 1), (0, 1), (-1, 1), (2**32, 1), (12, None), (12, float("nan")), (12, float("inf")), (12, True), (12, -1)])
def test_invalid_process_identity_is_rejected_before_native_work(pid, created):
    with pytest.raises(ValueError):
        MemoryActions(native=UntouchableNative()).execute("trim_process", confirmed=True, pid=pid, created=created)


def test_machine_operation_rejects_process_payload_before_native_work():
    with pytest.raises(ValueError):
        MemoryActions(native=UntouchableNative()).execute("trim_system_cache", confirmed=True, pid=12)


def test_non_windows_default_adapter_is_guarded(monkeypatch):
    import vanta.infra.memory_actions as module
    with monkeypatch.context() as context:
        context.setattr(module.os, "name", "posix")
        with pytest.raises(NotImplementedError, match="Windows"):
            MemoryActions().execute("trim_system_cache", confirmed=True)


def test_operation_failure_restores_privilege_and_is_not_reported_as_success():
    native = FakeNative()
    def denied():
        raise PermissionError("access denied")
    native.trim_system_cache = denied
    with pytest.raises(PermissionError, match="access denied"):
        MemoryActions(native=native).execute("trim_system_cache", confirmed=True)
    assert native.events == [("enable", "SeIncreaseQuotaPrivilege"), ("restore", "SeIncreaseQuotaPrivilege")]


# DLL fakes exercise the real ctypes adapter without calling any OS maintenance.
import ctypes
from ctypes import wintypes as w
import os
from types import SimpleNamespace
from vanta.infra.memory_actions import _WindowsNative, _TokenPrivileges


class FakeFunction:
    def __init__(self, name, owner):
        self.name, self.owner = name, owner

    def __call__(self, *args):
        return self.owner.call(self.name, args)


class FakeLibraries:
    process_handle = 0x123456789
    process_token = 0x223456789
    duplicate = 0x323456789

    def __init__(self):
        self.calls = []
        self.failures = {}
        self.impersonating = False
        self.status = 0
        names = {
            "kernel32": "GetCurrentProcess GetCurrentThread OpenProcess CloseHandle GetProcessTimes SetSystemFileCacheSize",
            "advapi32": "OpenProcessToken OpenThreadToken GetTokenInformation DuplicateTokenEx LookupPrivilegeValueW AdjustTokenPrivileges SetThreadToken RevertToSelf",
            "psapi": "EmptyWorkingSet",
            "ntdll": "NtSetSystemInformation",
        }
        self.libraries = {lib: SimpleNamespace(**{name: FakeFunction(name, self) for name in funcs.split()}) for lib, funcs in names.items()}

    @staticmethod
    def put(pointer, kind, value):
        ctypes.cast(pointer, ctypes.POINTER(kind))[0] = kind(value)

    def call(self, name, args):
        self.calls.append((name, args))
        if name in self.failures:
            ctypes.set_last_error(self.failures[name])
            return 0
        if name == "OpenThreadToken":
            if self.impersonating:
                self.put(args[3], w.HANDLE, 0x423456789)
                return 1
            ctypes.set_last_error(1008)
            return 0
        if name in ("GetCurrentProcess", "GetCurrentThread", "OpenProcess"):
            return self.process_handle
        if name == "OpenProcessToken":
            self.put(args[2], w.HANDLE, self.process_token)
        if name == "DuplicateTokenEx":
            self.put(args[5], w.HANDLE, self.duplicate)
        if name == "GetTokenInformation":
            self.put(args[2], w.DWORD, 1)
        if name == "GetProcessTimes":
            ticks = 116444736000000000 + 12345000000
            time = ctypes.cast(args[1], ctypes.POINTER(w.FILETIME)).contents
            time.dwHighDateTime, time.dwLowDateTime = ticks >> 32, ticks & 0xFFFFFFFF
        if name == "AdjustTokenPrivileges" and args[4] is not None:
            ctypes.cast(args[4], ctypes.POINTER(_TokenPrivileges)).contents.PrivilegeCount = 1
        if name == "NtSetSystemInformation":
            return self.status
        return 1

    def native(self):
        native = _WindowsNative(libraries=self.libraries)
        native.available = lambda: 100
        return native

    def named(self, name):
        return [args for called, args in self.calls if called == name]


native_fake_test = pytest.mark.skipif(os.name != "nt", reason="Windows ctypes last-error helpers")


@native_fake_test
def test_real_adapter_uses_pointer_width_handles_and_checks_identity_on_same_handle():
    libraries = FakeLibraries()
    MemoryActions(native=libraries.native()).execute("trim_process", confirmed=True, pid=12, created=1234.5)
    assert libraries.named("OpenProcess") == [(0x1100, False, 12)]
    assert libraries.named("GetProcessTimes")[0][0] == libraries.process_handle
    assert libraries.named("EmptyWorkingSet") == [(libraries.process_handle,)]
    assert libraries.named("CloseHandle") == [(libraries.process_handle,)]
    assert libraries.libraries["kernel32"].OpenProcess.restype is w.HANDLE
    assert libraries.libraries["psapi"].EmptyWorkingSet.argtypes == [w.HANDLE]


@native_fake_test
@pytest.mark.parametrize("operation", ["trim_system_cache", "purge_standby", "purge_standby_priority0"])
def test_real_adapter_impersonates_duplicate_only_and_cleans_up(operation):
    libraries = FakeLibraries()
    MemoryActions(native=libraries.native()).execute(operation, confirmed=True)
    adjusts = libraries.named("AdjustTokenPrivileges")
    assert len(adjusts) == 2
    assert all(args[0].value == libraries.duplicate for args in adjusts)
    assert libraries.named("SetThreadToken")[0][1].value == libraries.duplicate
    assert len(libraries.named("RevertToSelf")) == 1
    assert sorted(args[0].value for args in libraries.named("CloseHandle")) == sorted([libraries.process_token, libraries.process_token, libraries.duplicate])
    if operation == "trim_system_cache":
        assert libraries.named("SetSystemFileCacheSize") == [(ctypes.c_size_t(-1).value, ctypes.c_size_t(-1).value, 0)]
        assert libraries.libraries["kernel32"].SetSystemFileCacheSize.argtypes[:2] == [ctypes.c_size_t] * 2
    else:
        args = libraries.named("NtSetSystemInformation")[0]
        assert args[0] == 80
        assert ctypes.cast(args[1], ctypes.POINTER(w.ULONG)).contents.value == (4 if operation == "purge_standby" else 5)
        assert args[2] == 4
        assert libraries.libraries["ntdll"].NtSetSystemInformation.restype is ctypes.c_int32


@native_fake_test
def test_existing_impersonation_is_rejected_without_replacement():
    libraries = FakeLibraries()
    libraries.impersonating = True
    with pytest.raises(PermissionError, match="impersonation"):
        MemoryActions(native=libraries.native()).execute("trim_system_cache", confirmed=True)
    assert not libraries.named("SetThreadToken")
    assert not libraries.named("AdjustTokenPrivileges")
    assert not libraries.named("SetSystemFileCacheSize")


@native_fake_test
@pytest.mark.parametrize("status,error_type", [(0xC0000022, PermissionError), (0xC0000061, PermissionError), (0xC0000003, NotImplementedError), (0xC00000BB, NotImplementedError), (0xC0000001, OSError)])
def test_ntstatus_is_preserved_and_scoped_privilege_is_restored(status, error_type):
    libraries = FakeLibraries()
    libraries.status = ctypes.c_int32(status).value
    with pytest.raises(error_type, match=f"0x{status:08X}") as error:
        MemoryActions(native=libraries.native()).execute("purge_standby", confirmed=True)
    assert error.value.ntstatus == status
    assert len(libraries.named("RevertToSelf")) == 1
    assert len(libraries.named("AdjustTokenPrivileges")) == 2


@native_fake_test
@pytest.mark.parametrize("failed_call,error", [("EmptyWorkingSet", 5), ("GetProcessTimes", 5)])
def test_process_native_error_preserves_winerror_and_closes_handle(failed_call, error):
    libraries = FakeLibraries()
    libraries.failures[failed_call] = error
    with pytest.raises(PermissionError) as caught:
        MemoryActions(native=libraries.native()).execute("trim_process", confirmed=True, pid=12, created=1234.5)
    assert caught.value.winerror == error
    assert libraries.named("CloseHandle") == [(libraries.process_handle,)]

@native_fake_test
def test_not_all_assigned_success_return_does_not_impersonate_or_clear():
    libraries = FakeLibraries()
    original = libraries.call
    def call(name, args):
        result = original(name, args)
        if name == "AdjustTokenPrivileges" and args[4] is not None:
            ctypes.set_last_error(1300)  # Success return with ERROR_NOT_ALL_ASSIGNED.
        return result
    libraries.call = call
    with pytest.raises(OSError) as caught:
        MemoryActions(native=libraries.native()).execute("trim_system_cache", confirmed=True)
    assert caught.value.winerror == 1300
    assert not libraries.named("SetThreadToken")
    assert not libraries.named("SetSystemFileCacheSize")
    assert len(libraries.named("AdjustTokenPrivileges")) == 2
    assert any(args[0].value == libraries.duplicate for args in libraries.named("CloseHandle"))


@native_fake_test
@pytest.mark.parametrize("failed_call", ["SetThreadToken", "SetSystemFileCacheSize", "RevertToSelf"])
def test_thread_scope_failures_restore_duplicate_and_close_handles(failed_call):
    libraries = FakeLibraries()
    libraries.failures[failed_call] = 5
    with pytest.raises(PermissionError) as caught:
        MemoryActions(native=libraries.native()).execute("trim_system_cache", confirmed=True)
    assert caught.value.winerror == 5
    assert len(libraries.named("AdjustTokenPrivileges")) == 2
    assert any(args[0].value == libraries.duplicate for args in libraries.named("CloseHandle"))
    if failed_call == "SetThreadToken":
        assert not libraries.named("SetSystemFileCacheSize")
        assert not libraries.named("RevertToSelf")
    else:
        assert len(libraries.named("RevertToSelf")) == 1


@native_fake_test
def test_missing_native_export_reports_unsupported_and_restores_thread_scope():
    libraries = FakeLibraries()
    del libraries.libraries["ntdll"].NtSetSystemInformation
    with pytest.raises(NotImplementedError, match="NtSetSystemInformation"):
        MemoryActions(native=libraries.native()).execute("purge_standby", confirmed=True)
    assert len(libraries.named("RevertToSelf")) == 1
    assert len(libraries.named("AdjustTokenPrivileges")) == 2
