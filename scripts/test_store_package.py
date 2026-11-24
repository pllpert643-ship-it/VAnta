"""Release packaging contracts: identity, capability scope and actual binary selection."""
import importlib.util
from pathlib import Path
import sys
import unittest
import xml.etree.ElementTree as ET

spec = importlib.util.spec_from_file_location('store_package', Path(__file__).with_name('store_package.py'))
store = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = store
spec.loader.exec_module(store)

class StorePackagingTests(unittest.TestCase):
    def identity(self, **overrides):
        values = dict(name='Publisher.VAnta', publisher='CN=Publisher & Company',
                      publisher_display_name='Publisher & Company', version='1.0.0.0')
        values.update(overrides)
        return store.Identity(**values)

    def test_manifest_preserves_identity_and_requests_medium_integrity_full_trust_only(self):
        root = ET.fromstring(store.manifest(self.identity()))
        q = lambda name: f'{{{store.FOUNDATION}}}{name}'
        self.assertEqual(root.find(q('Identity')).get('Publisher'), 'CN=Publisher & Company')
        self.assertEqual(root.find(q('Identity')).get('ProcessorArchitecture'), 'x64')
        app = root.find(q('Applications'))[0]
        self.assertEqual(app.get(f'{{{store.UAP10}}}TrustLevel'), 'mediumIL')
        self.assertEqual(app.get(f'{{{store.UAP10}}}RuntimeBehavior'), 'packagedClassicApp')
        self.assertEqual([c.get('Name') for c in root.find(q('Capabilities'))], ['runFullTrust'])
        self.assertEqual(root.find(q('Dependencies'))[0].get('MinVersion'), '10.0.19041.0')

    def test_bad_identity_and_non_store_versions_are_refused_before_packaging(self):
        for changes in [dict(name='../payload'), dict(name='invalid name'), dict(name='x'),
                        dict(publisher='not a publisher'), dict(publisher_display_name=''),
                        dict(publisher='CN=Publisher\nInjected'), dict(version='1.0'),
                        dict(version='1.0.0.1'), dict(version='0.2.1.0'),
                        dict(version='1.65536.0.0'), dict(version='01.0.0.0')]:
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                self.identity(**changes)

    def test_preview_identity_cannot_accidentally_be_used_as_a_release(self):
        with self.assertRaises(ValueError):
            self.identity(name='VAnta.StorePreview')
        preview = self.identity(name='VAnta.StorePreview', publisher='CN=VAntaPreview', preview=True)
        root = ET.fromstring(store.manifest(preview))
        props = root.find(f'{{{store.FOUNDATION}}}Properties')
        self.assertEqual(props.find(f'{{{store.FOUNDATION}}}DisplayName').text, 'VAnta Preview')

    def test_payload_must_be_x64_and_have_no_missing_native_dll_dependencies(self):
        executable = store.REPO / 'dist/VAnta-Native/VAntaNative.exe'
        if not executable.exists():
            self.skipTest('Portable fixture is unavailable')
        imports = store.validate_native_dependencies(executable.parent)
        self.assertTrue(imports['VAntaNative.exe'])
        import tempfile
        with tempfile.TemporaryDirectory(prefix='vanta-import-check-') as directory:
            target = Path(directory) / 'VAntaNative.exe'
            original = executable.read_bytes()
            self.assertIn(b'KERNEL32.dll', original)
            target.write_bytes(original.replace(b'KERNEL32.dll', b'MISSING0.dll'))
            with self.assertRaisesRegex(ValueError, 'MISSING0.dll'):
                store.validate_native_dependencies(Path(directory))
            corrupt = bytearray(original)
            import struct
            pe = struct.unpack_from('<I', corrupt, 0x3c)[0]
            struct.pack_into('<H', corrupt, pe + 4, 0x14c)  # x86 instead of x64
            target.write_bytes(corrupt)
            with self.assertRaisesRegex(ValueError, 'x64'):
                store.validate_native_dependencies(Path(directory))

    def test_existing_portable_executable_is_rejected(self):
        executable = store.REPO / 'dist/VAnta-Native/VAntaNative.exe'
        if not executable.exists():
            self.skipTest('Original portable payload is not available in this checkout')
        # Old portable versions lack --release-info; newer ones report edition=portable.
        with self.assertRaises((ValueError, store.subprocess.CalledProcessError)):
            store.release_info(executable)

if __name__ == '__main__':
    unittest.main()
