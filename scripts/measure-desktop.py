"""Local smoke/measurement: launches only the supplied application, then closes its window."""
import ctypes
from ctypes import wintypes
import json, os, subprocess, sys, tempfile, time
from pathlib import Path
import psutil
from PySide6.QtGui import QGuiApplication
u=ctypes.WinDLL('user32',use_last_error=True)
CB=ctypes.WINFUNCTYPE(wintypes.BOOL,wintypes.HWND,wintypes.LPARAM)
u.EnumWindows.argtypes=[CB,wintypes.LPARAM]
u.GetWindowThreadProcessId.argtypes=[wintypes.HWND,ctypes.POINTER(wintypes.DWORD)]
u.GetWindowTextW.argtypes=[wintypes.HWND,wintypes.LPWSTR,ctypes.c_int]
u.IsWindowVisible.argtypes=[wintypes.HWND]
u.IsHungAppWindow.argtypes=[wintypes.HWND]
u.PostMessageW.argtypes=[wintypes.HWND,wintypes.UINT,wintypes.WPARAM,wintypes.LPARAM]
exe=Path(sys.argv[1]).resolve(); output=Path(sys.argv[2]).resolve(); output.mkdir(parents=True,exist_ok=True)
qt=QGuiApplication([])
with tempfile.TemporaryDirectory(prefix='vanta-native-check-') as data:
 env=os.environ.copy(); env['VANTA_NATIVE_DATA_DIR']=str(Path(data)/'native');env['LOCALAPPDATA']=str(Path(data)/'legacy')
 started=time.monotonic(); p=subprocess.Popen([str(exe)],cwd=exe.parent,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 proc=psutil.Process(p.pid); hwnd=None
 try:
  while time.monotonic()-started<40:
   assert p.poll() is None,f'Exited before window: {p.returncode}, {p.communicate()}'
   owned={p.pid}|{c.pid for c in proc.children(recursive=True)};found=[]
   @CB
   def visit(window,param):
    pid=wintypes.DWORD();u.GetWindowThreadProcessId(window,ctypes.byref(pid))
    if pid.value in owned and u.IsWindowVisible(window):
     title=ctypes.create_unicode_buffer(512);u.GetWindowTextW(window,title,512)
     if 'VAnta' in title.value:found.append((window,title.value))
    return True
   u.EnumWindows(visit,0)
   if found:hwnd,title=found[0];break
   time.sleep(.1)
  assert hwnd,'No main window'
  startup=time.monotonic()-started
  samples=[];cpu_begin={};begin=time.monotonic()
  while time.monotonic()-begin<18:
   assert p.poll() is None,'Exited during observation'
   assert not u.IsHungAppWindow(hwnd),'Window stopped responding'
   memory=private=0
   for process in [proc]+proc.children(recursive=True):
    try:
     memory+=process.memory_info().rss
     private+=getattr(process.memory_info(),'private',process.memory_info().vms)
     t=process.cpu_times();cpu_begin.setdefault(process.pid,t.user+t.system)
    except psutil.Error:pass
   samples.append((memory,private));time.sleep(.5)
  cpu_used=0
  for process in [proc]+proc.children(recursive=True):
   try:
    t=process.cpu_times();cpu_used+=max(0,t.user+t.system-cpu_begin.get(process.pid,t.user+t.system))
   except psutil.Error:pass
  measured_seconds=time.monotonic()-begin
  screenshot=output/'overview.png';screen=qt.primaryScreen();pixmap=screen.grabWindow(hwnd);assert not pixmap.isNull(),'Empty window capture';assert pixmap.save(str(screenshot))
  u.PostMessageW(hwnd,0x10,0,0);code=p.wait(timeout=30);assert code==0,f'Close failed: {code}'
  result={'exe_name':exe.name,'window_title':title,'startup_to_visible_seconds':round(startup,3),'observation_seconds':round(measured_seconds,3),'rss_mean_mib':round(sum(x[0]for x in samples)/len(samples)/1048576,2),'rss_peak_mib':round(max(x[0]for x in samples)/1048576,2),'private_mean_mib':round(sum(x[1]for x in samples)/len(samples)/1048576,2),'cpu_seconds':round(cpu_used,3),'cpu_percent_one_core':round(cpu_used/measured_seconds*100,2),'clean_exit':True,'screenshot':screenshot.name}
  (output/'result.json').write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result,indent=2))
 finally:
  if p.poll() is None:
   if hwnd:u.PostMessageW(hwnd,0x10,0,0)
   try:p.wait(timeout=10)
   except subprocess.TimeoutExpired:p.terminate();p.wait(timeout=10)
