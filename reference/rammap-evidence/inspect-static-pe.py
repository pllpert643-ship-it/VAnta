import pathlib, json, struct, re, hashlib
out=pathlib.Path(r'C:\Users\MSI GF63 Thin\VAnta\reference\rammap-evidence')
results=[]
for stem in ('RAMMap','RAMMap64a'):
 e=json.loads((out/(stem+'.pe-resources.json')).read_text(encoding='utf-8-sig'))
 b=pathlib.Path(e['subject']['local_path']).read_bytes(); p=struct.unpack_from('<I',b,0x3c)[0]
 machine,nsec=struct.unpack_from('<HH',b,p+4); szopt=struct.unpack_from('<H',b,p+20)[0]; op=p+24; magic=struct.unpack_from('<H',b,op)[0]; dd=op+(112 if magic==0x20b else 96)
 sections=[]
 for i in range(nsec):
  q=op+szopt+i*40; name=b[q:q+8].split(b'\0')[0].decode('ascii'); vs,va,rs,rp=struct.unpack_from('<IIII',b,q+8); sections.append(dict(name=name,rva=va,virtual_size=vs,raw_bytes=rs,offset=rp))
 def off(rva):
  for s in sections:
   if s['rva']<=rva<s['rva']+max(s['virtual_size'],s['raw_bytes']): return s['offset']+rva-s['rva']
  raise ValueError(rva)
 def cstr(q): return b[q:b.index(b'\0',q)].decode('ascii',errors='replace')
 irva,ibytes=struct.unpack_from('<II',b,dd+8); imports=[]
 if irva:
  q=off(irva)
  while any(b[q:q+20]):
   orig,ts,fw,nrva,first=struct.unpack_from('<IIIII',b,q); dll=cstr(off(nrva)); tq=off(orig or first); stride=8 if magic==0x20b else 4; syms=[]
   while True:
    val=int.from_bytes(b[tq:tq+stride],'little'); tq+=stride
    if not val:break
    if val>>(stride*8-1):syms.append('#ordinal:'+str(val&65535))
    else:syms.append(cstr(off(val)+2))
   imports.append(dict(dll=dll,symbols=syms));q+=20
 resources=[]
 for r in e['normalized_result']['resources']:
  loc=r['payload']['location']; data=b[loc['offset']:loc['offset']+loc['bytes']]; typ=r['type'].get('id',r['type'].get('name')); texts=[]
  if typ in (4,5,6,16):
   for parity in (0,1):
    for m in re.finditer(rb'(?:[\x20-\x7e]\x00){4,}',data[parity:]): texts.append(dict(offset=loc['offset']+parity+m.start(),text=m.group().decode('utf-16le')))
   texts=list({(x['offset'],x['text']):x for x in texts}.values()); texts.sort(key=lambda x:x['offset'])
  item=dict(index=r['index'],type=typ,name=r['name'].get('id',r['name'].get('name')),language=r['language'].get('id'),offset=loc['offset'],bytes=loc['bytes'],sha256=r['payload']['sha256'],utf16_ascii_runs=texts)
  if typ==24:item['manifest_utf8']=data.decode('utf-8',errors='replace')
  if typ=='BINRES':item['prefix_hex']=data[:24].hex()
  resources.append(item)
 results.append(dict(path=e['subject']['local_path'],sha256=hashlib.sha256(b).hexdigest(),parent_evidence_id=e['evidence_id'],machine=hex(machine),sections=sections,imports=imports,resources=resources))
result=dict(evidence_id='LOCAL-PE-DECODE-001',method='Read-only Python struct PE import table parsing and ASCII-subset UTF-16LE run enumeration of exact REA resource payload ranges. Text runs are not a semantic dialog/menu decoder; presence does not establish runtime behavior. Manifest decoded as UTF-8. No target loading/execution.',artifacts=results)
(out/'pe-static-details.json').write_text(json.dumps(result,indent=2),encoding='utf-8')
for a in results:
 print(pathlib.Path(a['path']).name,a['machine'],'resources',len(a['resources']))
 for r in a['resources']:
  if r['type'] in ('BINRES',4,5,6,24): print(json.dumps(r))
 print('FOCUSED IMPORTS',[(i['dll'],[s for s in i['symbols'] if re.search('Memory|WorkingSet|Privilege|Service|DeviceIo|NtQuery|NtSet|Token|Process',s)]) for i in a['imports']])

