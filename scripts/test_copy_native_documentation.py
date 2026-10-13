#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('copy_native_documentation', Path(__file__).with_name('copy_native_documentation.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class NativeCopyTests(unittest.TestCase):
    def fixture(self, base):
        source = base / 'docs'
        bundle = source / 'generated' / ('a' * 64)
        bundle.mkdir(parents=True)
        (source / 'index.html').write_text('<title>Native</title>')
        (bundle / 'root-overview.html').write_bytes((source / 'index.html').read_bytes())
        (bundle / 'bindings.json').write_text(json.dumps({'schema': 'codeclew-documentation-bindings/1.4',
            'outputHashes': {'root-overview.html': module.digest(source / 'index.html')}}))
        (bundle / 'publication.json').write_text(json.dumps({'schema': 'codeclew-documentation-publication/1.0',
            'id': bundle.name, 'files': {name: module.digest(bundle / name) for name in ('root-overview.html', 'bindings.json')}}))
        return source, bundle

    def test_native_bytes_and_manifest_are_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source, bundle = self.fixture(base)
            result = module.stage(source, base / 'site')
            self.assertFalse(result['nativeBytesChanged'])
            for path in source.rglob('*'):
                if path.is_file():
                    self.assertEqual(path.read_bytes(), (base / 'site' / path.relative_to(source)).read_bytes())

    def test_corruption_is_refused_before_destination_creation(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source, bundle = self.fixture(base)
            (bundle / 'root-overview.html').write_text('changed')
            with self.assertRaisesRegex(ValueError, 'digest mismatch'):
                module.stage(source, base / 'site')
            self.assertFalse((base / 'site').exists())

    def test_private_state_symlinks_and_overwrites_are_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            source, bundle = self.fixture(base)
            (source / 'alias').symlink_to(source / 'index.html')
            with self.assertRaisesRegex(ValueError, 'non-regular'):
                module.stage(source, base / 'site')
            (source / 'alias').unlink()
            (source / '.git').mkdir()
            with self.assertRaisesRegex(ValueError, 'private state'):
                module.stage(source, base / 'site')
            (base / 'site').mkdir()
            with self.assertRaisesRegex(ValueError, 'already exists'):
                module.stage(source, base / 'site')


if __name__ == '__main__':
    unittest.main()
