import json,pathlib,re
out=pathlib.Path(r'C:\Users\MSI GF63 Thin\VAnta\reference\rammap-evidence'); src=json.loads((out/'pe-static-details.json').read_text()); rows=[]
pattern=re.compile(r'Use Counts|Processes|Priority Summary|Physical Pages|Physical Ranges|File Summary|File Details|NtQuerySystemInformation|NtSetSystemInformation|NtQueryVirtualMemory|SeProfileSingleProcessPrivilege|SeDebugPrivilege|SeIncreaseQuotaPrivilege|GlobalMemoryStatusEx|GetPerformanceInfo|QueryWorkingSet|\\Device\\|RAMMap\.sys',re.I)
for a in src['artifacts']:
 b=pathlib.Path(a['path']).read_bytes(); hits=[]
 for enc,rx in [('ascii',rb'[\x20-\x7e]{5,}'),('utf-16le',rb'(?:[\x20-\x7e]\x00){5,}')]:
  for m in re.finditer(rx,b):
   text=m.group().decode(enc)
   if pattern.search(text):hits.append(dict(offset=m.start(),bytes=len(m.group()),encoding=enc,text=text))
 rows.append(dict(path=a['path'],sha256=a['sha256'],hits=hits))
r=dict(evidence_id='LOCAL-STRING-SCAN-001',method='Bounded exact-file static printable ASCII/UTF-16LE run search; selected names only, not exhaustive; string presence is not a call or observed behavior',artifacts=rows)
(out/'focused-string-hits.json').write_text(json.dumps(r,indent=2),encoding='utf-8')
for a in rows:
 print(pathlib.Path(a['path']).name)
 for h in a['hits']:print(hex(h['offset']),h['encoding'],h['text'])
