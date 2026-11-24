"""Reversible startup management using Run keys and Startup folders.

Disabled entries are stored by VAnta, rather than editing undocumented
Explorer StartupApproved binary records. Existing Windows-disabled entries
are read and presented without pretending VAnta can safely rewrite them.
"""
from __future__ import annotations
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


def _token(text: str) -> str:
    return hashlib.sha256(text.encode()).hexdigest()[:24]


class StartupManager:
    def __init__(self, data_dir: Path):
        self.data_dir = data_dir / "disabled_startup"
        self.data_dir.mkdir(parents=True, exist_ok=True)

    def entries(self) -> list[dict]:
        if os.name != "nt":
            return []
        import winreg
        entries = []
        run = r"Software\Microsoft\Windows\CurrentVersion\Run"
        approved = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved"
        for hive_name, hive in (("HKCU", winreg.HKEY_CURRENT_USER), ("HKLM", winreg.HKEY_LOCAL_MACHINE)):
            for view_name, view in (("64", winreg.KEY_WOW64_64KEY), ("32", winreg.KEY_WOW64_32KEY)):
                try:
                    with winreg.OpenKey(hive, run, 0, winreg.KEY_READ | view) as key:
                        for index in range(winreg.QueryInfoKey(key)[1]):
                            name, command, value_type = winreg.EnumValue(key, index)
                            disabled = False
                            try:
                                section = "Run32" if view_name == "32" else "Run"
                                with winreg.OpenKey(hive, approved + "\\" + section) as approval:
                                    status = winreg.QueryValueEx(approval, name)[0]
                                    disabled = isinstance(status, bytes) and bool(status) and status[0] in (3, 7)
                            except OSError:
                                pass
                            entries.append({"id": _token(f"{hive_name}/{view_name}/{name}"), "Name": name,
                                "Command": str(command), "Status": "Disabled by Windows" if disabled else "Enabled",
                                "Location": f"{hive_name} Run ({view_name}-bit)", "kind": "registry",
                                "hive": hive_name, "view": view_name, "name": name, "command": command,
                                "value_type": value_type, "run_key": run, "external_disabled": disabled})
                except OSError:
                    pass
        folders = [("Current user", Path(os.environ["APPDATA"]) / r"Microsoft\Windows\Start Menu\Programs\Startup"),
                   ("All users", Path(os.environ["PROGRAMDATA"]) / r"Microsoft\Windows\Start Menu\Programs\Startup")]
        for label, folder in folders:
            try:
                for path in folder.iterdir():
                    if path.is_file() and path.name.lower() != "desktop.ini":
                        entries.append({"id": _token(str(path)), "Name": path.stem, "Command": str(path),
                            "Status": "Enabled", "Location": label + " Startup folder", "kind": "file", "path": str(path)})
            except OSError:
                pass
        for metadata in self.data_dir.glob("*.json"):
            try:
                entry = json.loads(metadata.read_text(encoding="utf-8"))
                entry["Status"] = "Disabled by VAnta"
                entry["backup"] = str(metadata)
                entries.append(entry)
            except (OSError, ValueError):
                pass
        # Windows shares HKCU Run across registry views. Present one entry for
        # the same hive/name/command, while retaining distinct HKLM views.
        unique, seen = [], set()
        for entry in entries:
            key = (entry.get("hive"), entry.get("name"), str(entry.get("command")), entry.get("Status")) if entry.get("hive") == "HKCU" else entry["id"]
            if key not in seen:
                seen.add(key)
                unique.append(entry)
        return unique

    def toggle(self, entry: dict, enable: bool):
        if enable:
            self._restore(entry)
        else:
            self._disable(entry)

    def _disable(self, entry: dict):
        if entry.get("external_disabled"):
            raise ValueError("This entry is disabled by Windows. Use Windows Startup apps to change it.")
        metadata = self.data_dir / (entry["id"] + ".json")
        if metadata.exists():
            raise ValueError("A backup already exists. Refresh the list.")
        metadata.write_text(json.dumps(entry), encoding="utf-8")
        try:
            if entry["kind"] == "file":
                source = Path(entry["path"])
                shutil.move(str(source), str(self.data_dir / (entry["id"] + source.suffix)))
            else:
                import winreg
                hive = winreg.HKEY_CURRENT_USER if entry["hive"] == "HKCU" else winreg.HKEY_LOCAL_MACHINE
                view = winreg.KEY_WOW64_64KEY if entry["view"] == "64" else winreg.KEY_WOW64_32KEY
                with winreg.OpenKey(hive, entry["run_key"], 0, winreg.KEY_READ | winreg.KEY_SET_VALUE | view) as key:
                    current = winreg.QueryValueEx(key, entry["name"])
                    if current != (entry["command"], entry["value_type"]):
                        raise ValueError("Startup entry changed. Refresh before trying again.")
                    winreg.DeleteValue(key, entry["name"])
        except Exception:
            metadata.unlink(missing_ok=True)
            raise

    def _restore(self, entry: dict):
        metadata = self.data_dir / (entry["id"] + ".json")
        # Read the authoritative backup, not editable table values.
        saved = json.loads(metadata.read_text(encoding="utf-8"))
        if saved["kind"] == "file":
            destination = Path(saved["path"])
            if destination.exists():
                raise ValueError("The original startup path already exists; it will not be overwritten.")
            shutil.move(str(self.data_dir / (saved["id"] + destination.suffix)), str(destination))
        else:
            import winreg
            hive = winreg.HKEY_CURRENT_USER if saved["hive"] == "HKCU" else winreg.HKEY_LOCAL_MACHINE
            view = winreg.KEY_WOW64_64KEY if saved["view"] == "64" else winreg.KEY_WOW64_32KEY
            with winreg.OpenKey(hive, saved["run_key"], 0, winreg.KEY_READ | winreg.KEY_SET_VALUE | view) as key:
                try:
                    winreg.QueryValueEx(key, saved["name"])
                except FileNotFoundError:
                    pass
                else:
                    raise ValueError("A startup value already exists; it will not be overwritten.")
                winreg.SetValueEx(key, saved["name"], 0, saved["value_type"], saved["command"])
        metadata.unlink()
