"""Build a fully packaged x64 MSIX from the verified Store-edition payload.

No signing, trust-store changes, account access or submission is performed.
"""
from __future__ import annotations
import argparse
from dataclasses import dataclass
import hashlib
import html
import json
from pathlib import Path
import re
import shutil
import subprocess
import struct
import tempfile
import xml.etree.ElementTree as ET
import zipfile

REPO = Path(__file__).resolve().parent.parent
FOUNDATION = 'http://schemas.microsoft.com/appx/manifest/foundation/windows10'
UAP = 'http://schemas.microsoft.com/appx/manifest/uap/windows10'
UAP10 = 'http://schemas.microsoft.com/appx/manifest/uap/windows10/10'
RESCAP = 'http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities'
for prefix, namespace in [('', FOUNDATION), ('uap', UAP), ('uap10', UAP10), ('rescap', RESCAP)]:
    ET.register_namespace(prefix, namespace)

@dataclass(frozen=True)
class Identity:
    name: str
    publisher: str
    publisher_display_name: str
    version: str
    preview: bool = False

    def __post_init__(self):
        if not re.fullmatch(r'[A-Za-z0-9.-]{3,50}', self.name):
            raise ValueError('Identity name must be 3..50 letters, digits, dots or hyphens from Partner Center')
        if not self.publisher.startswith('CN=') or len(self.publisher) > 8192:
            raise ValueError('Publisher must be the complete X.500 CN= value from Partner Center')
        if not self.publisher_display_name.strip() or len(self.publisher_display_name) > 256:
            raise ValueError('Publisher display name is required (maximum 256 characters)')
        if any(ord(c) < 32 for c in self.publisher + self.publisher_display_name):
            raise ValueError('Publisher values must not contain control characters')
        if not re.fullmatch(r'(0|[1-9][0-9]{0,4})(\.(0|[1-9][0-9]{0,4})){3}', self.version):
            raise ValueError('Version must contain four numeric components')
        numbers = list(map(int, self.version.split('.')))
        if numbers[0] == 0 or max(numbers) > 65535 or numbers[3] != 0:
            raise ValueError('Store version needs major > 0, components <= 65535, and revision 0; use 1.0.0.0 or later')
        if not self.preview and (self.name == 'VAnta.StorePreview' or self.publisher == 'CN=VAntaPreview'):
            raise ValueError('Preview identity cannot be used for a Store release; copy the real Partner Center identity')


def manifest(identity: Identity) -> bytes:
    q = lambda name: f'{{{FOUNDATION}}}{name}'
    root = ET.Element(q('Package'), {'IgnorableNamespaces': 'uap uap10 rescap'})
    ET.SubElement(root, q('Identity'), Name=identity.name, Publisher=identity.publisher,
                  Version=identity.version, ProcessorArchitecture='x64')
    properties = ET.SubElement(root, q('Properties'))
    for tag, text in [('DisplayName', 'VAnta Preview' if identity.preview else 'VAnta'),
                      ('PublisherDisplayName', identity.publisher_display_name),
                      ('Description', 'Local resource observations and GPU workload management'),
                      ('Logo', r'Assets\StoreLogo.png')]:
        ET.SubElement(properties, q(tag)).text = text
    resources = ET.SubElement(root, q('Resources'))
    ET.SubElement(resources, q('Resource'), Language='en-us')
    dependencies = ET.SubElement(root, q('Dependencies'))
    ET.SubElement(dependencies, q('TargetDeviceFamily'), Name='Windows.Desktop',
                  MinVersion='10.0.19041.0', MaxVersionTested='10.0.26100.0')
    applications = ET.SubElement(root, q('Applications'))
    app = ET.SubElement(applications, q('Application'), {
        'Id': 'VAnta', 'Executable': r'App\VAntaNative.exe',
        f'{{{UAP10}}}RuntimeBehavior': 'packagedClassicApp', f'{{{UAP10}}}TrustLevel': 'mediumIL'})
    ET.SubElement(app, f'{{{UAP}}}VisualElements', {
        'DisplayName': 'VAnta Preview' if identity.preview else 'VAnta',
        'Description': 'Resource observatory', 'BackgroundColor': '#11141E',
        'Square44x44Logo': r'Assets\Square44x44Logo.png',
        'Square150x150Logo': r'Assets\Square150x150Logo.png'})
    capabilities = ET.SubElement(root, q('Capabilities'))
    ET.SubElement(capabilities, f'{{{RESCAP}}}Capability', Name='runFullTrust')
    ET.indent(root)
    return ET.tostring(root, encoding='utf-8', xml_declaration=True)


# Windows inbox DLLs used by this desktop application and its supported adapters.
# Unknown DLLs must be bundled and recursively checked; a developer's PATH cannot
# silently make a broken release pass. API-set contracts are supplied by Windows.
WINDOWS_DLLS = set("""advapi32.dll avrt.dll bcrypt.dll bcryptprimitives.dll cfgmgr32.dll
comctl32.dll comdlg32.dll crypt32.dll d2d1.dll d3d11.dll d3d12.dll dbghelp.dll dbgcore.dll
 dnsapi.dll dwmapi.dll dwrite.dll dxgi.dll gdi32.dll glu32.dll imm32.dll iphlpapi.dll
kernel32.dll kernelbase.dll mfplat.dll msimg32.dll msvcrt.dll msvcp_win.dll ncrypt.dll
netapi32.dll normaliz.dll ntdll.dll ntmarta.dll ole32.dll oleaut32.dll opengl32.dll
pdh.dll powrprof.dll profapi.dll propsys.dll psapi.dll rpcrt4.dll secur32.dll setupapi.dll
shell32.dll shcore.dll shlwapi.dll uiautomationcore.dll sspicli.dll ucrtbase.dll user32.dll userenv.dll uxtheme.dll
version.dll windowscodecs.dll winhttp.dll wininet.dll winmm.dll winspool.drv
winsta.dll wldap32.dll ws2_32.dll wtsapi32.dll authz.dll""".split())


def pe_imports(path: Path) -> list[str]:
    data = path.read_bytes()
    try:
        if data[:2] != b'MZ': raise ValueError('Missing DOS header')
        pe = struct.unpack_from('<I', data, 0x3c)[0]
        if data[pe:pe+4] != b'PE\0\0': raise ValueError('Missing PE header')
        machine, count = struct.unpack_from('<HH', data, pe + 4)
        optional_size = struct.unpack_from('<H', data, pe + 20)[0]
        optional = pe + 24
        if machine != 0x8664 or struct.unpack_from('<H', data, optional)[0] != 0x20b:
            raise ValueError('Store payload must contain x64 PE32+ binaries')
        if count > 96 or optional_size < 240: raise ValueError('Invalid PE section/optional header')
        sections = optional + optional_size
        def offset(rva):
            if rva < struct.unpack_from('<I', data, optional + 60)[0]: return rva
            for index in range(count):
                virtual_size, virtual, raw_size, raw = struct.unpack_from('<IIII', data, sections + index*40 + 8)
                if virtual <= rva < virtual + max(virtual_size, raw_size):
                    result = raw + rva - virtual
                    if result >= len(data): raise ValueError('PE RVA points beyond file')
                    return result
            raise ValueError('PE RVA has no section')
        def name_at(rva):
            start = offset(rva)
            end = data.find(b'\0', start, min(start + 260, len(data)))
            if end < 0: raise ValueError('Invalid imported DLL name')
            name = data[start:end].decode('ascii')
            if not re.fullmatch(r'[A-Za-z0-9_.-]+', name): raise ValueError('Imported DLL name contains a path')
            return name
        imports = []
        for table, descriptor_size in [(1, 20), (13, 32)]:
            rva, size = struct.unpack_from('<II', data, optional + 112 + table*8)
            if not rva: continue
            base = offset(rva)
            for index in range(min(size // descriptor_size + 1, 4096)):
                fields = struct.unpack_from('<' + 'I'*(descriptor_size//4), data, base + index*descriptor_size)
                if not any(fields): break
                if table == 13 and fields[0] != 1: raise ValueError('Unsupported VA-based delay import table')
                imports.append(name_at(fields[3] if table == 1 else fields[1]))
            else: raise ValueError('Unterminated PE import table')
        return sorted(set(imports), key=str.lower)
    except (struct.error, UnicodeDecodeError, IndexError) as error:
        raise ValueError(f'Invalid PE file {path.name}: {error}') from error


def validate_native_dependencies(directory: Path) -> dict[str, list[str]]:
    binaries = [p for p in directory.iterdir() if p.suffix.lower() in {'.exe', '.dll'}]
    available = {p.name.lower() for p in binaries}
    result = {}
    for binary in binaries:
        imports = pe_imports(binary)
        for name in imports:
            lower = name.lower()
            if lower not in WINDOWS_DLLS and lower not in available and not lower.startswith(('api-ms-win-', 'ext-ms-')):
                raise ValueError(f'{binary.name} requires missing native dependency {name}; bundle and license it or use a static CRT build')
        result[binary.name] = imports
    if 'vantanative.exe' not in available:
        raise ValueError('Missing VAntaNative.exe')
    return result


def release_info(executable: Path) -> dict:
    result = subprocess.run([str(executable.resolve()), '--release-info'], capture_output=True,
                            text=True, encoding='utf-8', timeout=30, check=True)
    info = json.loads(result.stdout)
    if info.get('edition') != 'store' or info.get('system_mutations') is not False or info.get('requires_elevation') is not False:
        raise ValueError('Refusing to package a portable build or a build requiring system mutations/elevation')
    return info


def package(source: Path, output: Path, identity: Identity, makeappx: Path) -> Path:
    source = source.resolve(strict=True)
    executable = source / 'VAntaNative.exe'
    info = release_info(executable)
    if not makeappx.is_file():
        raise ValueError('Windows SDK MakeAppx.exe is required; pass --makeappx')
    output.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix='vanta-msix-', dir=output))
    app = staging / 'App'
    app.mkdir()
    # Copy only the application payload, local policies and dependency license notices.
    for path in source.rglob('*'):
        if path.is_symlink() or path.is_junction():
            raise ValueError(f'Package source must not contain links: {path.name}')
        if not path.is_file():
            continue
        relative = path.relative_to(source)
        if relative.parts[0] == 'licenses' or path.suffix.lower() in {'.exe', '.dll', '.md'}:
            target = app / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, target)
    for required in ['LICENSE', 'PRIVACY.md', 'THIRD_PARTY_NOTICES.md']:
        path = source / required
        if not path.is_file():
            raise ValueError(f'Missing release notice: {required}')
        shutil.copy2(path, app / required)
    native_imports = validate_native_dependencies(app)
    shutil.copytree(REPO / 'packaging/store/Assets', staging / 'Assets')
    (staging / 'AppxManifest.xml').write_bytes(manifest(identity))
    name = f'{identity.name}_{identity.version}_x64.msix'
    destination = output / name
    if destination.exists():
        raise ValueError(f'Package already exists; preserve it and use a new output directory: {destination.name}')
    completed = subprocess.run([str(makeappx.resolve()), 'pack', '/d', str(staging.resolve()),
                                '/p', str(destination.resolve()), '/o'], capture_output=True, text=True,
                               encoding='utf-8', errors='replace', timeout=120)
    (output / 'makeappx.log').write_text(completed.stdout + completed.stderr, encoding='utf-8')
    if completed.returncode:
        raise RuntimeError(f'MakeAppx validation failed; see {output / "makeappx.log"}')
    with zipfile.ZipFile(destination) as archive:
        bad = archive.testzip()
        if bad:
            raise ValueError(f'Package CRC failed for {bad}')
        packaged_exe = archive.read('App/VAntaNative.exe')
        if packaged_exe != executable.read_bytes():
            raise ValueError('Packaged executable differs from validated source')
        if archive.read('AppxManifest.xml') != (staging / 'AppxManifest.xml').read_bytes():
            # MakeAppx may normalize the XML; compare parsed manifests.
            packaged_manifest = ET.fromstring(archive.read('AppxManifest.xml'))
            if packaged_manifest.find(f'{{{FOUNDATION}}}Identity').attrib != ET.fromstring(manifest(identity)).find(f'{{{FOUNDATION}}}Identity').attrib:
                raise ValueError('Packaged identity differs from requested identity')
    files = {str(p.relative_to(staging)).replace('\\', '/'): hashlib.sha256(p.read_bytes()).hexdigest()
             for p in sorted(staging.rglob('*')) if p.is_file()}
    evidence = {'schema_version': 1, 'preview_only': identity.preview, 'identity': identity.__dict__,
                'release_info': info, 'package': destination.name,
                'sha256': hashlib.sha256(destination.read_bytes()).hexdigest(), 'size_bytes': destination.stat().st_size,
                'files': files, 'native_imports': native_imports, 'signing': 'unsigned; Store submission workflow; not a sideload installer',
                'validation': {'makeappx': 'passed', 'zip_crc': 'passed', 'executable_matches': True,
                               'wack': 'not run', 'clean_machine_install_update_uninstall': 'not run'}}
    (output / (name + '.json')).write_text(json.dumps(evidence, indent=2), encoding='utf-8')
    policy = (REPO / 'docs/PRIVACY.md').read_text(encoding='utf-8')
    page = '<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>VAnta privacy policy</title><style>body{font:17px/1.7 system-ui;max-width:760px;margin:48px auto;padding:0 24px;color:#18202e}h1,h2{line-height:1.25}</style><main>'
    for paragraph in policy.split('\n\n'):
        if paragraph.startswith('## '): page += '<h2>' + html.escape(paragraph[3:]) + '</h2>'
        elif paragraph.startswith('# '): page += '<h1>' + html.escape(paragraph[2:]) + '</h1>'
        else: page += '<p>' + html.escape(paragraph) + '</p>'
    (output / 'privacy.html').write_text(page + '</main></html>', encoding='utf-8')
    print(destination)
    print('Preview only: ' + str(identity.preview) + '; no Store submission or machine installation performed.')
    return destination


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app-dir', type=Path, default=REPO / 'dist/VAnta-Store')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--identity-name')
    parser.add_argument('--publisher')
    parser.add_argument('--publisher-display-name')
    parser.add_argument('--version', required=True)
    parser.add_argument('--preview', action='store_true')
    parser.add_argument('--makeappx', required=True, type=Path)
    args = parser.parse_args()
    if not args.preview and not all([args.identity_name, args.publisher, args.publisher_display_name]):
        parser.error('Real Partner Center identity-name, publisher and publisher-display-name are required; use --preview for development only')
    identity = Identity(args.identity_name or 'VAnta.StorePreview', args.publisher or 'CN=VAntaPreview',
                        args.publisher_display_name or 'VAnta Preview', args.version, args.preview)
    package(args.app_dir, args.output, identity, args.makeappx)

if __name__ == '__main__':
    main()
