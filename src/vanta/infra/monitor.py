from __future__ import annotations

import psutil

from vanta.core.models import ResourceSnapshot


class SystemMonitor:
    def __init__(self) -> None:
        self._pynvml = None
        self._handle = None
        self.error: str | None = None
        try:
            import pynvml

            pynvml.nvmlInit()
            self._pynvml = pynvml
            self._handle = pynvml.nvmlDeviceGetHandleByIndex(0)
        except Exception as exc:  # hardware and driver failures vary by machine
            self.error = str(exc)

    def sample(self) -> ResourceSnapshot:
        snapshot = ResourceSnapshot(
            cpu_percent=psutil.cpu_percent(interval=None),
            ram_percent=psutil.virtual_memory().percent,
        )
        if self._pynvml is None or self._handle is None:
            return snapshot

        nvml = self._pynvml
        try:
            memory = nvml.nvmlDeviceGetMemoryInfo(self._handle)
            utilization = nvml.nvmlDeviceGetUtilizationRates(self._handle)
            raw_name = nvml.nvmlDeviceGetName(self._handle)
            snapshot.gpu_name = raw_name.decode() if isinstance(raw_name, bytes) else raw_name
            snapshot.gpu_utilization = float(utilization.gpu)
            snapshot.vram_used_mb = memory.used / (1024 * 1024)
            snapshot.vram_total_mb = memory.total / (1024 * 1024)
            snapshot.temperature_c = float(
                nvml.nvmlDeviceGetTemperature(self._handle, nvml.NVML_TEMPERATURE_GPU)
            )
            try:
                snapshot.power_w = nvml.nvmlDeviceGetPowerUsage(self._handle) / 1000
            except Exception:
                snapshot.power_w = None
            snapshot.telemetry_available = True
        except Exception as exc:
            self.error = str(exc)
        return snapshot

    def close(self) -> None:
        if self._pynvml is not None:
            try:
                self._pynvml.nvmlShutdown()
            except Exception:
                pass
