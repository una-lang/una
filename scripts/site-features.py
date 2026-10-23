#!/usr/bin/env python3
"""Validate the reviewed feature catalog and bundle editions (Python 3.11+).

This is build metadata, not a language ABI. Conformance directives and their runtime
expectations are owned by the existing conformance runner. This tool checks the
positive packages again with the exact binary used to build the bundle.
"""
import argparse
import json
import re
import subprocess
import sys
try:
    import tomllib
except ModuleNotFoundError:
    sys.exit("site-features: Python 3.11 or newer is required")
from pathlib import Path


def local_path(root, name):
    if not isinstance(name, str) or not name or Path(name).is_absolute() or '..' in Path(name).parts:
        raise ValueError(f'invalid repository-relative path: {name!r}')
    path = root / name
    if not path.exists() or not path.resolve().is_relative_to(root.resolve()):
        raise ValueError(f'missing or external path: {name}')
    return path


def load_catalog(root, editions):
    catalog = json.loads((root / 'docs/features.json').read_text())
    if catalog.get('schema') != 'una.features/1' or not isinstance(catalog.get('features'), list):
        raise ValueError('unsupported feature catalog schema')
    ids = set()
    for row in catalog['features']:
        ident = row.get('id')
        if not isinstance(ident, str) or not re.fullmatch(r'[a-z][a-z0-9]*(?:[.-][a-z0-9]+)*', ident) or ident in ids:
            raise ValueError(f'invalid or duplicate feature id: {ident!r}')
        ids.add(ident)
        if not isinstance(row.get('title'), str) or not row['title'] or '|' in row['title'] or '\n' in row['title']:
            raise ValueError(f'{ident}: invalid title')
        if row.get('status') not in ('supported', 'preview', 'proposed', 'excluded'):
            raise ValueError(f'{ident}: invalid status')
        if row.get('minimum_release', 'missing') is not None:
            raise ValueError(f'{ident}: minimum releases need historical evidence; use null')
        if not isinstance(row.get('normative'), list) or not row['normative']:
            raise ValueError(f'{ident}: no source references')
        for ref in row['normative']:
            if not ref.startswith(('docs/spec/', 'docs/adr/', 'docs/plan/')):
                raise ValueError(f'{ident}: invalid source reference {ref}')
            local_path(root, ref)
        fixtures = row.get('fixtures')
        if not isinstance(fixtures, list) or len(fixtures) != len(set(fixtures)):
            raise ValueError(f'{ident}: invalid fixtures')
        if row['status'] in ('proposed', 'excluded'):
            if row.get('edition') is not None or fixtures:
                raise ValueError(f'{ident}: unavailable features cannot advertise executable support')
            continue
        if not fixtures or row.get('edition') not in editions:
            raise ValueError(f'{ident}: no compatible fixture or unsupported edition {row.get("edition")}')
        if row['edition'] in ('0.3', '0.4') and row['status'] != 'preview':
            raise ValueError(f'{ident}: a preview edition cannot advertise stable support')
        for fixture in fixtures:
            package = local_path(root, 'tests/conformance/' + fixture)
            if len(Path(fixture).parts) != 2 or not package.is_dir():
                raise ValueError(f'{ident}: fixture must be a conformance package')
            # The reviewed set uses una.toml. Ita packages need an explicit future
            # extension rather than assuming their edition from their directory.
            manifest = tomllib.loads((package / 'una.toml').read_text())
            if manifest.get('package', {}).get('edition', '0.1') != row['edition']:
                raise ValueError(f'{ident}: fixture edition differs: {fixture}')
    catalog['features'].sort(key=lambda row: row['id'])
    return catalog


def validate_document_editions(root, editions):
    paths = sorted(path.relative_to(root).as_posix() for path in (root / 'docs/guide').rglob('*.md'))
    named = []
    for name in paths:
        text = (root / name).read_text()
        if not text.startswith('---\n'):
            continue
        front = text.split('---', 2)[1]
        match = re.search(r'^editions:\s*(\[[^\n]*\])\s*$', front, re.M)
        if match:
            named.extend((name, edition) for edition in json.loads(match[1]))
        elif re.search(r'^editions:', front, re.M):
            raise ValueError(f'{name}: malformed inline editions list')
    for name in ('docs/std/listing.json', 'docs/std/builtins.json'):
        doc = json.loads((root / name).read_text())
        for item in doc['items']:
            for variant in item['variants']:
                named.extend((name, edition) for edition in variant['editions'])
    for name, edition in sorted(set(named)):
        if edition not in editions:
            raise ValueError(f'{name} names edition {edition}, absent from UNA_BIN; build una from this commit')


def render(catalog):
    lines = ['# Reviewed feature availability', '',
             '<!-- Generated by scripts/site-features.py --write-table; edit docs/features.json. -->', '',
             'This selected catalog records checked evidence, not every language feature. `edition` is the edition of the evidence, not a claim of first availability. `supported` means implemented in that edition; `preview` retains its edition terms. A proposed or excluded feature is not executable support. The minimum supporting release is unknown unless historical release evidence is established; all current records use `null`.', '',
             '| Feature | State | Evidence edition | Minimum release | Sources | Conformance packages |',
             '| --- | --- | --- | --- | --- | --- |']
    for row in catalog['features']:
        refs = ', '.join(f'[{Path(p).stem}](../{p.removeprefix("docs/")})' for p in row['normative'])
        fixtures = ', '.join(f'`{p}`' for p in row['fixtures']) or 'none (unavailable)'
        lines.append(f'| {row["title"]} (`{row["id"]}`) | {row["status"]} | {row["edition"] or "none"} | unknown | {refs} | {fixtures} |')
    return '\n'.join(lines) + '\n'


def check_fixtures(root, catalog, binary):
    fixtures = sorted({f for row in catalog['features'] for f in row['fixtures']})
    for fixture in fixtures:
        result = subprocess.run([str(binary), '--json', 'check', str(root / 'tests/conformance' / fixture)], capture_output=True, text=True, timeout=120)
        if result.returncode:
            raise ValueError(f'{fixture}: feature evidence does not check with UNA_BIN\n{result.stdout}{result.stderr}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument('--editions', required=True, help='comma-separated editions served by UNA_BIN')
    parser.add_argument('--una', type=Path)
    parser.add_argument('--write-table', action='store_true')
    args = parser.parse_args()
    root = args.root.resolve()
    editions = args.editions.replace(' ', '').split(',')
    try:
        validate_document_editions(root, editions)
        catalog = load_catalog(root, editions)
        table = root / 'docs/guide/availability.md'
        if args.write_table:
            table.write_text(render(catalog))
        elif table.read_text() != render(catalog):
            raise ValueError('stale availability table: run scripts/site-features.py --editions <served editions> --write-table')
        if args.una:
            check_fixtures(root, catalog, args.una.resolve())
        print(json.dumps(catalog, sort_keys=True, separators=(',', ':')))
    except (ValueError, KeyError, TypeError, OSError, subprocess.SubprocessError) as error:
        print(f'site-features: FAIL ({error})', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
