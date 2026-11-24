"""Explicit Windows memory maintenance; no elevation or automatic/mass trimming.

``MemoryActions.execute`` requires confirmed=True (the bool, not a truthy value).
Success returns MemoryActionResult; validation errors raise ValueError, refused
confirmation/admin/permissions raise PermissionError, missing OS/exports raise
NotImplementedError, and native failures retain WinError or numeric NTSTATUS.
Available memory is in bytes (None if sampling fails), sampled sequentially;
it is not an attribution of reclaimed bytes and can decrease during an action.
``created`` is a Unix timestamp from the selected process. The opened process
object is checked before EmptyWorkingSet, and that same handle is used to trim.
The two-microsecond comparison allowance accommodates FILETIME/float conversion.

An explicitly injected native adapter supports safe tests. Default construction
is lazy: invalid/unconfirmed requests never load DLLs. Machine operations require
an already elevated process. Privileges are enabled only on a duplicated token,
impersonated on the calling worker thread, then reverted and restored. Existing
thread impersonation is rejected. Process-token privileges are never adjusted.
Do not run an action from a shared thread that has other impersonation duties.

Standby operations are advanced, unsupported native interfaces, not documented
Microsoft maintenance APIs. Verified primary header evidence (2026-11-16):
System Informer's own phnt ntexapi.h, commit
b636325a6a8f7b84cdb35b6b834ed09335ffd2ff, declares
SystemMemoryListInformation = 80 (SeProfileSingleProcessPrivilege) and the
zero-based SYSTEM_MEMORY_LIST_COMMAND enum: MemoryPurgeStandbyList = 4,
MemoryPurgeLowPriorityStandbyList = 5. No other command is implemented.
https://github.com/winsiderss/systeminformer/blob/b636325a6a8f7b84cdb35b6b834ed09335ffd2ff/phnt/include/ntexapi.h

Primary Microsoft documentation checked 2026-11-16:
* EmptyWorkingSet: QUERY_INFORMATION or QUERY_LIMITED_INFORMATION plus SET_QUOTA.
  https://learn.microsoft.com/en-us/windows/win32/api/psapi/nf-psapi-emptyworkingset
* GetProcessTimes: creation FILETIME in 100 ns units since 1601-01-01.
  https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-getprocesstimes
* SetSystemFileCacheSize: SIZE_T(-1), SIZE_T(-1), flags 0 flushes the cache;
  requires SeIncreaseQuotaPrivilege. This does not promise an exact byte gain.
  https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-setsystemfilecachesize
* AdjustTokenPrivileges: check last error even on success; restore PreviousState.
  https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-adjusttokenprivileges
* TokenElevation / TOKEN_ELEVATION identifies an already elevated process.
  https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-gettokeninformation
  https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-token_elevation
* Thread-scoped duplicate-token impersonation and cleanup:
  https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-duplicatetokenex
  https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-openthreadtoken
  https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-setthreadtoken
  https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-reverttoself
"""
from __future__ import annotations

from dataclasses import dataclass
import math
import os


@dataclass(frozen=True)
class MemoryActionResult:
    operation: str
    message: str
    before_available: int | None
    after_available: int | None


class MemoryActions:
    def __init__(self, *, native=None):
        self._native = native

    def execute(
        self, operation: str, *, confirmed: bool = False,
        pid: int | None = None, created: float | None = None,
    ) -> MemoryActionResult:
        if operation not in ("trim_process", "trim_system_cache", "purge_standby", "purge_standby_priority0"):
            raise ValueError(f"Unknown memory operation: {operation!r}")
        if confirmed is not True:
            raise PermissionError("Explicit confirmation is required for memory maintenance.")
        if operation == "trim_process":
            if type(pid) is not int or not 0 < pid <= 0xFFFFFFFF:
                raise ValueError("A positive 32-bit PID is required.")
            if isinstance(created, bool) or not isinstance(created, (int, float)) or not math.isfinite(created) or created <= 0:
                raise ValueError("A finite positive creation timestamp is required.")
        elif pid is not None or created is not None:
            raise ValueError("PID and creation time are only accepted for trim_process.")
        native = self._native
        if native is None:
            if os.name != "nt":
                raise NotImplementedError("Memory maintenance requires Windows.")
            native = _WindowsNative()
        if operation != "trim_process" and not native.is_admin():
            raise PermissionError("Run as administrator manually to use this operation; no automatic elevation is performed.")
        before = native.available()
        if operation == "trim_process":
            with native.process(pid) as handle:
                if abs(native.process_created(handle) - created) > 0.000002:
                    raise ValueError("Process identity changed (PID reused); select it again.")
                native.trim_process(handle)
            message = "Process working set trimmed; pages may be faulted back in."
        else:
            privilege = "SeIncreaseQuotaPrivilege" if operation == "trim_system_cache" else "SeProfileSingleProcessPrivilege"
            with native.privilege(privilege):
                if operation == "trim_system_cache":
                    native.trim_system_cache()
                    message = "System file cache working set trimmed; cache pages may be faulted back in."
                else:
                    native.purge_standby(4 if operation == "purge_standby" else 5)
                    message = "Standby purge completed using an unsupported native advanced operation."
        return MemoryActionResult(operation, message, before, native.available())

import ctypes
from ctypes import wintypes as w
from contextlib import contextmanager
import sys

import psutil


class _Luid(ctypes.Structure):
    _fields_ = [("LowPart", w.DWORD), ("HighPart", w.LONG)]


class _LuidAttributes(ctypes.Structure):
    _fields_ = [("Luid", _Luid), ("Attributes", w.DWORD)]


class _TokenPrivileges(ctypes.Structure):
    _fields_ = [("PrivilegeCount", w.DWORD), ("Privileges", _LuidAttributes * 1)]


def _cleanup(action):
    """Preserve the operation's error while making cleanup failures visible."""
    active = sys.exception()
    try:
        action()
    except OSError as error:
        if active is None:
            raise
        active.add_note(f"Cleanup failed: {error}")


class _WindowsNative:
    """Private adapter; libraries injection supports syscall-level safe tests."""

    def __init__(self, *, libraries=None):
        if libraries is None:
            if os.name != "nt":
                raise NotImplementedError("Memory maintenance requires Windows.")
            libraries = {name: ctypes.WinDLL(name, use_last_error=True)
                         for name in ("kernel32", "advapi32", "psapi", "ntdll")}
        self._libraries = libraries
        self._kernel = libraries["kernel32"]
        self._security = libraries["advapi32"]
        self._psapi = libraries["psapi"]
        self._ntdll = libraries["ntdll"]
        self._bind(self._kernel, "GetCurrentProcess", [], w.HANDLE)
        self._bind(self._kernel, "GetCurrentThread", [], w.HANDLE)
        self._bind(self._kernel, "OpenProcess", [w.DWORD, w.BOOL, w.DWORD], w.HANDLE)
        self._bind(self._kernel, "CloseHandle", [w.HANDLE])
        self._bind(self._kernel, "GetProcessTimes", [w.HANDLE] + [ctypes.POINTER(w.FILETIME)] * 4)
        self._bind(self._kernel, "SetSystemFileCacheSize", [ctypes.c_size_t, ctypes.c_size_t, w.DWORD])
        self._bind(self._psapi, "EmptyWorkingSet", [w.HANDLE])
        self._bind(self._security, "OpenProcessToken", [w.HANDLE, w.DWORD, ctypes.POINTER(w.HANDLE)])
        self._bind(self._security, "OpenThreadToken", [w.HANDLE, w.DWORD, w.BOOL, ctypes.POINTER(w.HANDLE)])
        self._bind(self._security, "GetTokenInformation", [w.HANDLE, ctypes.c_int, w.LPVOID, w.DWORD, ctypes.POINTER(w.DWORD)])
        self._bind(self._security, "DuplicateTokenEx", [w.HANDLE, w.DWORD, w.LPVOID, ctypes.c_int, ctypes.c_int, ctypes.POINTER(w.HANDLE)])
        self._bind(self._security, "LookupPrivilegeValueW", [w.LPCWSTR, w.LPCWSTR, ctypes.POINTER(_Luid)])
        self._bind(self._security, "AdjustTokenPrivileges", [w.HANDLE, w.BOOL, ctypes.POINTER(_TokenPrivileges), w.DWORD, ctypes.POINTER(_TokenPrivileges), ctypes.POINTER(w.DWORD)])
        self._bind(self._security, "SetThreadToken", [ctypes.POINTER(w.HANDLE), w.HANDLE])
        self._bind(self._security, "RevertToSelf", [])
        # Native NTSTATUS is signed 32 bits, including in a 64-bit process.
        # Bind lazily so documented operations still work if the export is absent.

    @staticmethod
    def _bind(library, name, args, result=w.BOOL):
        try:
            function = getattr(library, name)
        except AttributeError as error:
            raise NotImplementedError(f"Windows export {name} is unavailable.") from error
        function.argtypes = args
        function.restype = result
        return function

    @staticmethod
    def _check(ok):
        if not ok:
            raise ctypes.WinError(ctypes.get_last_error())

    def _close(self, handle):
        self._check(self._kernel.CloseHandle(handle))

    def available(self):
        try:
            return int(psutil.virtual_memory().available)
        except (OSError, psutil.Error):
            return None

    @contextmanager
    def process(self, pid):
        # PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, never debug privilege.
        handle = self._kernel.OpenProcess(0x1000 | 0x0100, False, pid)
        self._check(handle)
        try:
            yield handle
        finally:
            _cleanup(lambda: self._close(handle))

    def process_created(self, handle):
        created, exited, kernel, user = (w.FILETIME() for _ in range(4))
        self._check(self._kernel.GetProcessTimes(handle, ctypes.byref(created), ctypes.byref(exited), ctypes.byref(kernel), ctypes.byref(user)))
        ticks = (created.dwHighDateTime << 32) | created.dwLowDateTime
        return (ticks - 116444736000000000) / 10_000_000

    def trim_process(self, handle):
        self._check(self._psapi.EmptyWorkingSet(handle))

    @contextmanager
    def _process_token(self, access):
        token = w.HANDLE()
        self._check(self._security.OpenProcessToken(self._kernel.GetCurrentProcess(), access, ctypes.byref(token)))
        try:
            yield token
        finally:
            _cleanup(lambda: self._close(token))

    def is_admin(self):
        with self._process_token(0x0008) as token:  # TOKEN_QUERY
            elevated, length = w.DWORD(), w.DWORD()
            self._check(self._security.GetTokenInformation(token, 20, ctypes.byref(elevated), ctypes.sizeof(elevated), ctypes.byref(length)))
            return bool(elevated.value)

    @contextmanager
    def privilege(self, name):
        # Do not silently replace a caller's existing thread impersonation.
        existing = w.HANDLE()
        if self._security.OpenThreadToken(self._kernel.GetCurrentThread(), 0x0008, True, ctypes.byref(existing)):
            self._close(existing)
            raise PermissionError("Memory maintenance refuses existing thread impersonation.")
        error = ctypes.get_last_error()
        if error != 1008:  # ERROR_NO_TOKEN is the only acceptable failure.
            raise ctypes.WinError(error)
        with self._process_token(0x0002 | 0x0008) as process_token:
            token = w.HANDLE()
            # Duplicate as SecurityImpersonation / TokenImpersonation. Only this
            # duplicate receives changes; subprocesses inherit the process token.
            self._check(self._security.DuplicateTokenEx(process_token, 0x0004 | 0x0008 | 0x0020, None, 2, 2, ctypes.byref(token)))
        try:
            state, previous, length = _TokenPrivileges(), _TokenPrivileges(), w.DWORD()
            state.PrivilegeCount = 1
            self._check(self._security.LookupPrivilegeValueW(None, name, ctypes.byref(state.Privileges[0].Luid)))
            state.Privileges[0].Attributes = 2  # SE_PRIVILEGE_ENABLED
            ctypes.set_last_error(0)
            self._check(self._security.AdjustTokenPrivileges(token, False, ctypes.byref(state), ctypes.sizeof(previous), ctypes.byref(previous), ctypes.byref(length)))
            error = ctypes.get_last_error()
            try:
                if error:
                    raise ctypes.WinError(error)  # Includes ERROR_NOT_ALL_ASSIGNED.
                self._check(self._security.SetThreadToken(None, token))
                try:
                    yield
                finally:
                    _cleanup(lambda: self._check(self._security.RevertToSelf()))
            finally:
                def restore():
                    ctypes.set_last_error(0)
                    self._check(self._security.AdjustTokenPrivileges(token, False, ctypes.byref(previous), 0, None, None))
                    if ctypes.get_last_error():
                        raise ctypes.WinError(ctypes.get_last_error())
                _cleanup(restore)
        finally:
            _cleanup(lambda: self._close(token))

    def trim_system_cache(self):
        unlimited = ctypes.c_size_t(-1).value
        self._check(self._kernel.SetSystemFileCacheSize(unlimited, unlimited, 0))

    def purge_standby(self, command):
        if command not in (4, 5):
            raise ValueError("Only standby commands 4 and 5 are permitted.")
        function = self._bind(self._ntdll, "NtSetSystemInformation", [ctypes.c_int, w.LPVOID, w.ULONG], ctypes.c_int32)
        value = w.ULONG(command)
        status = function(80, ctypes.byref(value), ctypes.sizeof(value)) & 0xFFFFFFFF
        if status & 0x80000000:
            message = f"Unsupported native advanced standby purge failed: NTSTATUS 0x{status:08X}"
            if status in (0xC0000022, 0xC0000061):
                error = PermissionError(message + " (access denied or required privilege not held).")
            elif status in (0xC0000002, 0xC0000003, 0xC00000BB):
                error = NotImplementedError(message + " (operation unsupported by this Windows version).")
            else:
                error = OSError(message)
            error.ntstatus = status
            raise error
