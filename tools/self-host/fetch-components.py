#!/usr/bin/env python3
"""Fetch only the preview's public C inputs; no ESP-IDF installation required.

Registry ZIP timestamps vary, so pin the sorted paths and SHA-256 of each file,
not ZIP metadata. Selected bytes were compared with the firmware lock's installed
components. Refuse changed inputs or a moved firmware lock; never replace a local
component tree silently.
"""
import hashlib
from pathlib import Path, PurePosixPath
import re
import subprocess
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]
COMPONENTS = [
    ('lvgl/lvgl', '9.5.0', 'c626fb4a-df30-4428-9542-e2db06f6f97a',
     'aa276a4187c15bf53c95c7b3cb00d72d9069d9abf773f3cf840f3740fae3cffb'),
    ('espressif/cbor', '0.6.1~4', 'fe6d9469-4302-4401-a45b-fc7b6e9ed541',
     'aa74651d8471ad854aea559b670bd3f9c35fda5046ef675bac8fb49e154a152e'),
]

def wanted(name, path):
    if name == 'lvgl/lvgl':
        return path.startswith('src/') or path in ('lvgl.h', 'lvgl_private.h', 'lv_version.h', 'LICENCE.txt')
    return path.startswith('tinycbor/src/') or path in ('LICENSE', 'tinycbor/LICENSE')

def digest(files):
    result = hashlib.sha256()
    for path, data in sorted(files.items()):
        result.update(path.encode() + b'\0' + hashlib.sha256(data).digest())
    return result.hexdigest()

lock = (ROOT / 'firmware/dependencies.lock').read_text()
parent = ROOT / 'firmware/managed_components'
parent.mkdir(exist_ok=True)
for name, version, object_id, expected in COMPONENTS:
    block = re.search(r'^  ' + re.escape(name) + r':\n(.*?)(?=^  \S|\Z)', lock, re.M | re.S)
    if not block or not re.search(r'^    version: ' + re.escape(version) + r'$', block[1], re.M):
        raise SystemExit(f'{name}: firmware lock changed; review/update preview pins first')
    dest = parent / name.replace('/', '__')
    if dest.exists():
        files = {p.relative_to(dest).as_posix(): p.read_bytes() for p in dest.rglob('*')
                 if p.is_file() and wanted(name, p.relative_to(dest).as_posix())}
        if digest(files) != expected:
            raise SystemExit(f'{dest}: differs from pinned preview inputs; move it aside explicitly before retrying')
        print(f'{name}: verified existing sources')
        continue
    with tempfile.TemporaryDirectory(prefix='.preview-', dir=parent) as tmp:
        archive = Path(tmp) / 'component.zip'
        url = 'https://components.espressif.com/api/downloads/?object_type=component&object_id=' + object_id
        subprocess.run(['curl', '--fail', '--location', '--silent', '--show-error',
                        '--connect-timeout', '30', '--max-time', '600', url, '-o', str(archive)], check=True)
        files = {}
        with zipfile.ZipFile(archive) as package:
            for info in package.infolist():
                if info.is_dir() or '/' not in info.filename:
                    continue
                path = info.filename.split('/', 1)[1]
                if not wanted(name, path):
                    continue
                if PurePosixPath(path).is_absolute() or '..' in PurePosixPath(path).parts or path in files:
                    raise SystemExit('Unsafe or duplicate component path')
                files[path] = package.read(info)
        if digest(files) != expected:
            raise SystemExit(f'{name}: downloaded sources do not match pinned SHA-256')
        stage = Path(tmp) / 'sources'
        for path, data in files.items():
            target = stage / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
        stage.rename(dest)
        print(f'{name}: fetched and verified {len(files)} files')
