#!/usr/bin/env python3
"""Negative controls for feature evidence and early edition checks."""
import copy
import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location('features', Path(__file__).with_name('site-features.py'))
features = importlib.util.module_from_spec(spec)
spec.loader.exec_module(features)


class CatalogTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for name, body in {
            'docs/spec/25-units.md': '# Units\n',
            'docs/guide/units.md': '---\neditions: ["0.3"]\n---\n# Units\n',
            'docs/std/listing.json': '{"items":[{"variants":[{"editions":["0.3"]}]}]}',
            'docs/std/builtins.json': '{"items":[]}',
            'tests/conformance/types/units/una.toml': '[package]\nedition = "0.3"\n',
        }.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(body)
        self.catalog = {'schema': 'una.features/1', 'scope': 'Test', 'features': [{
            'id': 'units.integer', 'title': 'Integer units', 'status': 'preview', 'edition': '0.3',
            'minimum_release': None, 'normative': ['docs/spec/25-units.md'], 'fixtures': ['types/units'],
        }]}
        subprocess.run(['git', 'init', '-q', str(self.root)], check=True)
        subprocess.run(['git', 'add', '.'], cwd=self.root, check=True)
        self.save()

    def save(self):
        (self.root / 'docs/features.json').write_text(json.dumps(self.catalog))

    def load(self):
        self.save()
        return features.load_catalog(self.root, ['0.1', '0.2', '0.3', '0.4'])

    def test_supported_evidence_and_deterministic_rendering(self):
        catalog = self.load()
        self.assertEqual(features.render(catalog), features.render(self.load()))
        self.assertIn('| preview | 0.3 | unknown |', features.render(catalog))

    def test_missing_fixture_is_rejected(self):
        self.catalog['features'][0]['fixtures'] = ['types/missing']
        with self.assertRaisesRegex(ValueError, 'missing'):
            self.load()

    def test_fixture_edition_must_match(self):
        self.catalog['features'][0]['edition'] = '0.2'
        with self.assertRaisesRegex(ValueError, 'fixture edition differs'):
            self.load()

    def test_preview_cannot_claim_stability(self):
        self.catalog['features'][0]['status'] = 'supported'
        with self.assertRaisesRegex(ValueError, 'preview edition'):
            self.load()

    def test_unavailable_feature_cannot_claim_support(self):
        self.catalog['features'][0]['status'] = 'proposed'
        with self.assertRaisesRegex(ValueError, 'unavailable'):
            self.load()

    def test_unknown_release_is_not_guessed(self):
        self.catalog['features'][0]['minimum_release'] = '0.3.16'
        with self.assertRaisesRegex(ValueError, 'historical evidence'):
            self.load()

    def test_duplicate_identity_and_invalid_status_fail(self):
        self.catalog['features'].append(copy.deepcopy(self.catalog['features'][0]))
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            self.load()
        self.catalog['features'].pop()
        self.catalog['features'][0]['status'] = 'maybe'
        with self.assertRaisesRegex(ValueError, 'invalid status'):
            self.load()

    def test_external_paths_fail(self):
        self.catalog['features'][0]['fixtures'] = ['../../outside']
        with self.assertRaisesRegex(ValueError, 'invalid repository-relative'):
            self.load()

    def test_guide_and_std_cannot_name_a_missing_binary_edition(self):
        with self.assertRaisesRegex(ValueError, 'docs/guide/units.md.*edition 0.3.*build una from this commit'):
            features.validate_document_editions(self.root, ['0.1', '0.2'])
        (self.root / 'docs/guide/units.md').write_text('# Units\n')
        with self.assertRaisesRegex(ValueError, 'docs/std/listing.json.*edition 0.3'):
            features.validate_document_editions(self.root, ['0.1', '0.2'])
        features.validate_document_editions(self.root, ['0.1', '0.2', '0.3'])

    def test_stale_table_fails_the_command(self):
        (self.root / 'docs/guide/availability.md').write_text('stale\n')
        result = subprocess.run(['python3', '-B', str(Path(__file__).with_name('site-features.py')), '--root', str(self.root), '--editions', '0.1,0.2,0.3'], capture_output=True, text=True)
        self.assertEqual(result.returncode, 1)
        self.assertIn('stale availability table', result.stderr)


if __name__ == '__main__':
    unittest.main()
