"""Read-only verification of the exact extracted Store payload; no account or system mutations."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time
import zipfile


def verify(package, output):
    package, output = Path(package).resolve(), Path(output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    extracted = output / 'extracted'
    if extracted.exists():
        raise ValueError('Use a new output directory to preserve prior evidence')
    with zipfile.ZipFile(package) as archive:
        assert archive.testzip() is None, 'MSIX CRC failed'
        archive.extractall(extracted)
    executable = extracted / 'App/VAntaNative.exe'
    env = os.environ.copy()
    env['VANTA_NATIVE_DATA_DIR'] = str(output / 'isolated-data')
    commands = []
    def run(arguments, expected=0):
        started = time.monotonic()
        result = subprocess.run([str(executable), *arguments], cwd=executable.parent, env=env,
                                capture_output=True, text=True, encoding='utf-8', timeout=40)
        commands.append({'arguments': arguments, 'exit_code': result.returncode,
                         'seconds': round(time.monotonic()-started, 3), 'stderr': result.stderr})
        assert result.returncode == expected, (arguments, result.returncode, result.stderr)
        if expected == 0:
            assert not result.stderr, (arguments, result.stderr)
        return result
    run(['--help'])
    version = run(['--version']).stdout.strip()
    info = json.loads(run(['--release-info']).stdout)
    assert info['edition'] == 'store' and info['system_mutations'] is False
    assert info['requires_elevation'] is False
    run(['--diagnostics', str(output / 'diagnostics.json')])
    snapshot = json.loads((output / 'diagnostics.json').read_text(encoding='utf-8'))['snapshot']
    assert snapshot['total_memory'] > 0 and snapshot['processes']
    run(['--inventory', str(output / 'inventory.json')])
    inventory = json.loads((output / 'inventory.json').read_text(encoding='utf-8'))
    assert inventory['services']
    assert all(row['writable'] is False and row['enabled'] is None for row in inventory['startup'])
    assert 'memory' in run(['--gpu-pulse', '0', '0'], expected=1).stderr.lower()
    assert 'Unrecognized' in run(['--unknown-command'], expected=1).stderr
    first = subprocess.Popen([str(executable), '--smoke-test'], cwd=executable.parent, env=env,
                             stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        deadline = time.monotonic() + 20
        while not (Path(env['VANTA_NATIVE_DATA_DIR']) / 'instance.lock').exists():
            assert first.poll() is None, 'First GUI exited early'
            assert time.monotonic() < deadline, 'GUI lock not acquired'
            time.sleep(0.05)
        second = run(['--smoke-test'], expected=1)
        assert 'already be running' in second.stderr, second.stderr
        stdout, stderr = first.communicate(timeout=40)
        assert first.returncode == 0 and not stderr, (first.returncode, stderr)
    finally:
        if first.poll() is None:
            first.terminate()
            first.wait(timeout=10)
    # A second GUI session proves the previous process released its OS lock.
    run(['--smoke-test'])
    result = {'version': version, 'release_info': info, 'sha256': hashlib.sha256(package.read_bytes()).hexdigest(),
              'package_crc': 'passed', 'extracted_payload': 'passed', 'read_only_diagnostics': 'passed',
              'read_only_inventory': 'passed', 'store_startup_entries_read_only_and_unknown': True,
              'gui_seven_pages': 'passed', 'simultaneous_gui_rejected': True, 'restart_after_exit': 'passed',
              'host_token_elevated': inventory['elevated'], 'gpu_available': snapshot['gpu'] is not None,
              'commands': commands, 'installed_package_identity': 'not tested; extracted payload only',
              'wack': 'not run', 'clean_machine_lifecycle': 'not run'}
    (output / 'result.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
    print(json.dumps({key: value for key, value in result.items() if key != 'commands'}, indent=2))

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    verify(args.package, args.output)
