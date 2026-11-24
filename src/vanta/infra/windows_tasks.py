"""Windows-only task operations. Handles are always closed on this thread."""
from __future__ import annotations
import ctypes
from ctypes import wintypes as w
import os


def process_session(pid: int) -> int | None:
    session = w.DWORD()
    if ctypes.windll.kernel32.ProcessIdToSessionId(pid, ctypes.byref(session)):
        return session.value
    return None

from contextlib import contextmanager
from pathlib import Path
import subprocess
import psutil


def _dll(name):
    if os.name != "nt":
        raise OSError("This operation requires Windows.")
    return ctypes.WinDLL(name, use_last_error=True)


def _function(dll, name, args, result=w.BOOL):
    function = getattr(dll, name)
    function.argtypes = args
    function.restype = result
    return function


def _check(ok):
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())
    return ok


@contextmanager
def process_handle(identity, access):
    kernel = _dll("kernel32")
    open_process = _function(kernel, "OpenProcess", [w.DWORD, w.BOOL, w.DWORD], w.HANDLE)
    close = _function(kernel, "CloseHandle", [w.HANDLE])
    handle = open_process(access | 0x1000, False, identity.pid)
    _check(handle)
    try:
        # Bind the actual opened kernel object to the selected identity. A PID
        # could have been reused between the psutil check and OpenProcess.
        created, exited, kernel_time, user_time = (w.FILETIME() for _ in range(4))
        get_times = _function(kernel, "GetProcessTimes", [w.HANDLE] + [ctypes.POINTER(w.FILETIME)] * 4)
        _check(get_times(handle, ctypes.byref(created), ctypes.byref(exited), ctypes.byref(kernel_time), ctypes.byref(user_time)))
        unix_created = ((created.dwHighDateTime << 32) | created.dwLowDateTime) / 10_000_000 - 11644473600
        if abs(unix_created - identity.created) > .001:
            raise ValueError("PID was reused. Select the process again.")
        yield kernel, handle
    finally:
        close(handle)


class PowerState(ctypes.Structure):
    _fields_ = [("Version", w.DWORD), ("ControlMask", w.DWORD), ("StateMask", w.DWORD)]


def set_efficiency(identity, enabled: bool):
    with process_handle(identity, 0x0200) as (kernel, handle):
        state = PowerState(1, 1, 1 if enabled else 0)
        setter = _function(kernel, "SetProcessInformation", [w.HANDLE, ctypes.c_int, ctypes.c_void_p, w.DWORD])
        _check(setter(handle, 4, ctypes.byref(state), ctypes.sizeof(state)))


def create_process_dump(identity, destination: str):
    path = Path(destination)
    # Exclusive creation protects an existing file even after a file dialog.
    with path.open("xb") as stream:
        try:
            import msvcrt
            file_handle = w.HANDLE(msvcrt.get_osfhandle(stream.fileno()))
            with process_handle(identity, 0x0400 | 0x0010) as (_, handle):
                writer = _function(_dll("dbghelp"), "MiniDumpWriteDump",
                    [w.HANDLE, w.DWORD, w.HANDLE, w.DWORD, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p])
                _check(writer(handle, identity.pid, file_handle, 2, None, None, None))
        except Exception:
            stream.close()
            path.unlink(missing_ok=True)
            raise
    return str(path)


class LockInfo(ctypes.Structure):
    _fields_ = [("ObjectName", w.WCHAR * 128), ("Timeout", ctypes.c_longlong), ("Alertable", w.BOOL)]


class ThreadInfo(ctypes.Structure):
    _fields_ = [("ProcessId", w.DWORD), ("ThreadId", w.DWORD), ("WaitTime", w.DWORD),
                ("ContextSwitches", w.DWORD)]


class NodeUnion(ctypes.Union):
    _fields_ = [("LockObject", LockInfo), ("ThreadObject", ThreadInfo)]


class WaitNode(ctypes.Structure):
    _fields_ = [("ObjectType", ctypes.c_int), ("ObjectStatus", ctypes.c_int), ("Object", NodeUnion)]


def wait_chain(identity):
    process = psutil.Process(identity.pid)
    if abs(process.create_time() - identity.created) > .001:
        raise ValueError("Process changed. Select it again.")
    if process.status() == psutil.STATUS_STOPPED:
        raise ValueError("Wait-chain analysis is unavailable for suspended processes.")
    if process.name().lower() == "sqlservr.exe":
        raise ValueError("Microsoft advises against Task Manager wait-chain analysis on SQL Server.")
    advapi = _dll("advapi32")
    open_session = _function(advapi, "OpenThreadWaitChainSession", [w.DWORD, ctypes.c_void_p], w.HANDLE)
    close_session = _function(advapi, "CloseThreadWaitChainSession", [w.HANDLE], None)
    get_chain = _function(advapi, "GetThreadWaitChain", [w.HANDLE, ctypes.c_size_t, w.DWORD, w.DWORD,
        ctypes.POINTER(w.DWORD), ctypes.POINTER(WaitNode), ctypes.POINTER(w.BOOL)])
    session = open_session(0, None)
    _check(session)
    output = []
    try:
        threads = process.threads()
        for thread in threads[:64]:
            nodes = (WaitNode * 16)()
            count, cycle = w.DWORD(16), w.BOOL()
            if get_chain(session, 0, 0, thread.id, ctypes.byref(count), nodes, ctypes.byref(cycle)):
                descriptions = []
                for node in nodes[:count.value]:
                    if node.ObjectType == 8:
                        info = node.Object.ThreadObject
                        descriptions.append(f"PID {info.ProcessId} / TID {info.ThreadId} (state {node.ObjectStatus})")
                    else:
                        descriptions.append(f"Object type {node.ObjectType}, state {node.ObjectStatus}: {node.Object.LockObject.ObjectName}")
                output.append(f"Thread {thread.id}" + (" [CYCLE]" if cycle.value else "") + ": " + " -> ".join(descriptions))
            else:
                output.append(f"Thread {thread.id}: unavailable ({ctypes.get_last_error()})")
        if len(threads) > 64:
            output.append(f"Showing 64 of {len(threads)} threads.")
    finally:
        close_session(session)
    return "\n".join(output) or "No thread information is available."


class SessionInfo(ctypes.Structure):
    _fields_ = [("SessionId", w.DWORD), ("pWinStationName", w.LPWSTR), ("State", ctypes.c_int)]


def user_sessions():
    wts = _dll("wtsapi32")
    pointer = ctypes.POINTER(SessionInfo)()
    count = w.DWORD()
    enum = _function(wts, "WTSEnumerateSessionsW", [w.HANDLE, w.DWORD, w.DWORD,
                     ctypes.POINTER(ctypes.POINTER(SessionInfo)), ctypes.POINTER(w.DWORD)])
    free = _function(wts, "WTSFreeMemory", [ctypes.c_void_p], None)
    query = _function(wts, "WTSQuerySessionInformationW", [w.HANDLE, w.DWORD, ctypes.c_int,
                     ctypes.POINTER(w.LPWSTR), ctypes.POINTER(w.DWORD)])
    _check(enum(None, 0, 1, ctypes.byref(pointer), ctypes.byref(count)))
    result = []
    states = ["Active", "Connected", "Connect query", "Shadow", "Disconnected", "Idle", "Listen", "Reset", "Down", "Init"]
    try:
        for session in pointer[:count.value]:
            username = w.LPWSTR()
            size = w.DWORD()
            if query(None, session.SessionId, 5, ctypes.byref(username), ctypes.byref(size)):
                try:
                    user = username.value or ""
                finally:
                    free(username)
                if user:
                    result.append({"User": user, "Session": session.SessionId,
                        "Status": states[session.State] if session.State < len(states) else str(session.State),
                        "Station": session.pWinStationName or ""})
    finally:
        free(pointer)
    return result


def session_action(session_id: int, logoff: bool):
    wts = _dll("wtsapi32")
    action = _function(wts, "WTSLogoffSession" if logoff else "WTSDisconnectSession", [w.HANDLE, w.DWORD, w.BOOL])
    _check(action(None, session_id, False))


def service_action(name: str, action: str):
    if action not in {"start", "stop", "restart"}:
        raise ValueError("Unknown service action.")
    # No command shell; service names remain one argument.
    if action == "restart":
        service_action(name, "stop")
        import time
        deadline = time.monotonic() + 25
        while time.monotonic() < deadline:
            if psutil.win_service_get(name).status() == "stopped":
                break
            time.sleep(.25)
        else:
            raise TimeoutError("Service did not stop within 25 seconds.")
        return service_action(name, "start")
    result = subprocess.run([os.path.join(os.environ["WINDIR"], "System32", "sc.exe"), action, name],
        capture_output=True, text=True, errors="replace", timeout=30, creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode:
        raise OSError((result.stdout + result.stderr).strip())
    return result.stdout.strip()


def run_task(command: str, elevated: bool = False):
    import shlex
    parts = shlex.split(command, posix=False)
    parts = [part[1:-1] if part.startswith('"') and part.endswith('"') else part for part in parts]
    if not parts:
        raise ValueError("Enter a program and optional arguments.")
    shell = _dll("shell32")
    execute = _function(shell, "ShellExecuteW", [w.HWND, w.LPCWSTR, w.LPCWSTR, w.LPCWSTR, w.LPCWSTR, ctypes.c_int], ctypes.c_void_p)
    result = execute(None, "runas" if elevated else "open", parts[0], subprocess.list2cmdline(parts[1:]), None, 1)
    if not result or result <= 32:
        raise OSError(f"Windows could not start the task (ShellExecute {result}).")


def open_file_properties(path: str):
    class ShellExecuteInfo(ctypes.Structure):
        _fields_ = [("cbSize", w.DWORD), ("fMask", w.ULONG), ("hwnd", w.HWND),
            ("lpVerb", w.LPCWSTR), ("lpFile", w.LPCWSTR), ("lpParameters", w.LPCWSTR),
            ("lpDirectory", w.LPCWSTR), ("nShow", ctypes.c_int), ("hInstApp", w.HINSTANCE),
            ("lpIDList", ctypes.c_void_p), ("lpClass", w.LPCWSTR), ("hkeyClass", w.HKEY),
            ("dwHotKey", w.DWORD), ("hIcon", w.HANDLE), ("hProcess", w.HANDLE)]
    shell = _dll("shell32")
    execute = _function(shell, "ShellExecuteExW", [ctypes.POINTER(ShellExecuteInfo)])
    info = ShellExecuteInfo()
    info.cbSize, info.fMask = ctypes.sizeof(info), 0x0000000C
    info.lpVerb, info.lpFile, info.nShow = "properties", path, 1
    _check(execute(ctypes.byref(info)))


def switch_to_process(identity):
    proc = psutil.Process(identity.pid)
    if abs(proc.create_time() - identity.created) > .001:
        raise ValueError("Process changed.")
    user = _dll("user32")
    callback_type = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    get_pid = _function(user, "GetWindowThreadProcessId", [w.HWND, ctypes.POINTER(w.DWORD)], w.DWORD)
    visible = _function(user, "IsWindowVisible", [w.HWND])
    found = []
    @callback_type
    def callback(window, _):
        pid = w.DWORD()
        get_pid(window, ctypes.byref(pid))
        if pid.value == identity.pid and visible(window):
            found.append(window)
            return False
        return True
    enum = _function(user, "EnumWindows", [callback_type, w.LPARAM])
    enum(callback, 0)
    if not found:
        raise ValueError("This process has no visible top-level window.")
    _function(user, "ShowWindow", [w.HWND, ctypes.c_int])(found[0], 9)
    _check(_function(user, "SetForegroundWindow", [w.HWND])(found[0]))


def visible_processes() -> set[int]:
    user = _dll("user32")
    callback_type = ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    get_pid = _function(user, "GetWindowThreadProcessId", [w.HWND, ctypes.POINTER(w.DWORD)], w.DWORD)
    visible = _function(user, "IsWindowVisible", [w.HWND])
    result = set()
    @callback_type
    def callback(window, _):
        if visible(window):
            pid = w.DWORD()
            get_pid(window, ctypes.byref(pid))
            result.add(pid.value)
        return True
    _function(user, "EnumWindows", [callback_type, w.LPARAM])(callback, 0)
    return result


def memory_counters():
    class PerformanceInfo(ctypes.Structure):
        _fields_ = [("cb", w.DWORD)] + [(name, ctypes.c_size_t) for name in (
            "CommitTotal", "CommitLimit", "CommitPeak", "PhysicalTotal", "PhysicalAvailable",
            "SystemCache", "KernelTotal", "KernelPaged", "KernelNonpaged", "PageSize")] + [
            ("HandleCount", w.DWORD), ("ProcessCount", w.DWORD), ("ThreadCount", w.DWORD)]
    info = PerformanceInfo()
    info.cb = ctypes.sizeof(info)
    _check(_function(_dll("psapi"), "GetPerformanceInfo", [ctypes.POINTER(PerformanceInfo), w.DWORD])(ctypes.byref(info), info.cb))
    return {"Committed": info.CommitTotal * info.PageSize, "Commit limit": info.CommitLimit * info.PageSize,
        "Commit peak": info.CommitPeak * info.PageSize, "System cache": info.SystemCache * info.PageSize,
        "Paged kernel pool": info.KernelPaged * info.PageSize, "Nonpaged kernel pool": info.KernelNonpaged * info.PageSize}


def hardware_inventory():
    """One bounded, read-only CIM query. No PowerShell profile or shell interpolation."""
    import json
    script = r'''
$ErrorActionPreference = 'Stop'
$cpu = @(Get-CimInstance Win32_Processor | Select-Object Name,SocketDesignation,NumberOfCores,NumberOfLogicalProcessors,MaxClockSpeed,VirtualizationFirmwareEnabled,L2CacheSize,L3CacheSize)
$ram = @(Get-CimInstance Win32_PhysicalMemory | Select-Object DeviceLocator,Capacity,ConfiguredClockSpeed,Speed,FormFactor,Manufacturer,PartNumber)
$slots = @(Get-CimInstance Win32_PhysicalMemoryArray | Select-Object MemoryDevices)
$gpu = @(Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,PNPDeviceID)
$disks = @(Get-CimInstance Win32_DiskDrive | Select-Object Index,Model,Size,InterfaceType)
@{CPU=$cpu; RAM=$ram; Slots=$slots; GPU=$gpu; Disks=$disks} | ConvertTo-Json -Depth 4 -Compress
'''
    result = subprocess.run([str(Path(os.environ["WINDIR"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"),
        "-NoProfile", "-NonInteractive", "-Command", script], capture_output=True, text=True,
        encoding="utf-8", errors="replace", timeout=25, creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode:
        raise OSError(result.stderr.strip())
    return json.loads(result.stdout.lstrip("\ufeff"))


class CounterValueUnion(ctypes.Union):
    _fields_ = [("longValue", w.LONG), ("doubleValue", ctypes.c_double), ("largeValue", ctypes.c_longlong), ("AnsiStringValue", ctypes.c_char_p), ("WideStringValue", w.LPWSTR)]


class CounterValue(ctypes.Structure):
    _fields_ = [("CStatus", w.DWORD), ("Value", CounterValueUnion)]


class CounterArrayItem(ctypes.Structure):
    _fields_ = [("szName", w.LPWSTR), ("FmtValue", CounterValue)]


class GpuCounters:
    """Native WDDM performance counters, collected without a helper process."""
    PATHS = {
        "engines": r"\GPU Engine(*)\Utilization Percentage",
        "dedicated": r"\GPU Process Memory(*)\Dedicated Usage",
        "shared": r"\GPU Process Memory(*)\Shared Usage",
        "adapter_dedicated": r"\GPU Adapter Memory(*)\Dedicated Usage",
        "adapter_shared": r"\GPU Adapter Memory(*)\Shared Usage",
    }

    def __init__(self):
        self.dll = _dll("pdh")
        self.query = w.HANDLE()
        self.handles = {}
        opened = _function(self.dll, "PdhOpenQueryW", [w.LPCWSTR, ctypes.c_size_t, ctypes.POINTER(w.HANDLE)], w.LONG)
        if opened(None, 0, ctypes.byref(self.query)) != 0:
            raise OSError("Windows performance counters could not be opened.")
        add = _function(self.dll, "PdhAddEnglishCounterW", [w.HANDLE, w.LPCWSTR, ctypes.c_size_t, ctypes.POINTER(w.HANDLE)], w.LONG)
        for key, path in self.PATHS.items():
            handle = w.HANDLE()
            if add(self.query, path, 0, ctypes.byref(handle)) == 0:
                self.handles[key] = handle
        self.collect = _function(self.dll, "PdhCollectQueryData", [w.HANDLE], w.LONG)
        self.collect(self.query)

    def sample(self):
        self.collect(self.query)
        result = {}
        get_array = _function(self.dll, "PdhGetFormattedCounterArrayW", [w.HANDLE, w.DWORD, ctypes.POINTER(w.DWORD), ctypes.POINTER(w.DWORD), ctypes.c_void_p], w.LONG)
        for key, handle in self.handles.items():
            length, count = w.DWORD(), w.DWORD()
            status = get_array(handle, 0x200 | 0x8000, ctypes.byref(length), ctypes.byref(count), None)
            if length.value <= 0 or length.value > 16 * 1024 * 1024:
                continue
            buffer = ctypes.create_string_buffer(length.value)
            if get_array(handle, 0x200 | 0x8000, ctypes.byref(length), ctypes.byref(count), buffer) != 0:
                continue
            items = ctypes.cast(buffer, ctypes.POINTER(CounterArrayItem))
            result[key] = {items[i].szName: items[i].FmtValue.Value.doubleValue for i in range(count.value)
                           if items[i].FmtValue.CStatus in (0, 1)}
        return result

    def close(self):
        if self.query:
            _function(self.dll, "PdhCloseQuery", [w.HANDLE], w.LONG)(self.query)
            self.query = None
