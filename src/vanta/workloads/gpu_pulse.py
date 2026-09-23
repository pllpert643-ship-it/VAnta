from __future__ import annotations

import ctypes
import time


def _function(driver: ctypes.WinDLL, name: str, argtypes: list[object]) -> object:
    function = getattr(driver, name)
    function.restype = ctypes.c_int
    function.argtypes = argtypes
    return function


def run(memory_mb: int = 256, duration: int = 20) -> int:
    if memory_mb < 32 or memory_mb > 2048:
        raise ValueError("memory_mb must be between 32 and 2048")
    if duration < 1 or duration > 3600:
        raise ValueError("duration must be between 1 and 3600 seconds")

    driver = ctypes.WinDLL("nvcuda.dll")
    cu_init = _function(driver, "cuInit", [ctypes.c_uint])
    cu_device_get = _function(driver, "cuDeviceGet", [ctypes.POINTER(ctypes.c_int), ctypes.c_int])
    cu_ctx_create = _function(
        driver,
        "cuCtxCreate_v2",
        [ctypes.POINTER(ctypes.c_void_p), ctypes.c_uint, ctypes.c_int],
    )
    cu_mem_alloc = _function(
        driver,
        "cuMemAlloc_v2",
        [ctypes.POINTER(ctypes.c_uint64), ctypes.c_size_t],
    )
    cu_memset = _function(
        driver,
        "cuMemsetD8_v2",
        [ctypes.c_uint64, ctypes.c_ubyte, ctypes.c_size_t],
    )
    cu_sync = _function(driver, "cuCtxSynchronize", [])
    cu_mem_free = _function(driver, "cuMemFree_v2", [ctypes.c_uint64])
    cu_ctx_destroy = _function(driver, "cuCtxDestroy_v2", [ctypes.c_void_p])

    if cu_init(0) != 0:
        return 2
    device = ctypes.c_int()
    context = ctypes.c_void_p()
    device_ptr = ctypes.c_uint64()
    if cu_device_get(ctypes.byref(device), 0) != 0:
        return 3
    if cu_ctx_create(ctypes.byref(context), 0, device) != 0:
        return 4

    size = memory_mb * 1024 * 1024
    if cu_mem_alloc(ctypes.byref(device_ptr), size) != 0:
        cu_ctx_destroy(context)
        return 5

    try:
        deadline = time.monotonic() + duration
        value = 0
        while time.monotonic() < deadline:
            if cu_memset(device_ptr.value, value % 256, size) != 0:
                return 6
            cu_sync()
            value += 1
            time.sleep(0.015)
    finally:
        cu_mem_free(device_ptr.value)
        cu_ctx_destroy(context)
    return 0
