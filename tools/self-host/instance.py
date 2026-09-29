#!/usr/bin/env python3
"""Initialize or run a self-host instance; never overwrite existing credentials."""
import os
from pathlib import Path
import secrets
import shlex
import shutil
import sys

bundle = Path(__file__).resolve().parent
if len(sys.argv) != 3 or sys.argv[1] not in ('init', 'run'):
    sys.exit('usage: instance.py init|run /absolute/instance-directory')
instance = Path(sys.argv[2])
if not instance.is_absolute():
    sys.exit('Instance directory must be absolute.')
os.umask(0o077)
if sys.argv[1] == 'init':
    bun = shutil.which('bun')
    if not bun:
        sys.exit('Install Bun and put it on PATH first.')
    # Fail if anything exists: init is never a reset/upgrade operation.
    instance.mkdir(mode=0o700)
    (instance / 'configs').mkdir()
    (instance / 'firmware').mkdir()
    values = {
        'RUST_LOG': 'info',
        'DESKMATE_SERVER_BIND': '127.0.0.1:8443',
        'DESKMATE_PUBLIC_URL': 'http://localhost:8443',
        'DESKMATE_ADMIN_TOKEN': secrets.token_hex(32),
        'DESKMATE_CONFIG_DIR': str(instance / 'configs'),
        'DESKMATE_FIRMWARE_DIR': str(instance / 'firmware'),
        'DESKMATE_FIRMWARE_VERSION': (bundle / 'firmware-version.txt').read_text().strip(),
        'DESKMATE_WEB_DIR': str(bundle / 'web'),
        'DESKMATE_FACES_DIR': str(bundle / 'faces'),
        'DESKMATE_BUN': str(Path(bun).resolve()),
        'DESKMATE_SIGNUPS': 'closed',
    }
    with (instance / 'server.env').open('x') as config:
        config.write('# Data only: KEY=value, with optional shell-style quoting.\n')
        for key, value in values.items():
            config.write(f'{key}={shlex.quote(value)}\n')
    print(f'Created {instance}/server.env (0600). Run: python3 {bundle}/instance.py run {instance}')
else:
    config = instance / 'server.env'
    if config.stat().st_mode & 0o077:
        sys.exit('server.env must not be readable by group/others: chmod 600 it.')
    # Parse data, never source a shell script. Inherited optional Deskmate settings
    # must not accidentally enable OAuth/SMTP or target another instance.
    env = {k: v for k, v in os.environ.items() if not k.startswith('DESKMATE_') and k != 'RUST_LOG'}
    for number, line in enumerate(config.read_text().splitlines(), 1):
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        key, sep, value = line.partition('=')
        if not sep or not (key.startswith('DESKMATE_') or key == 'RUST_LOG') or not key.replace('_', '').isalnum():
            sys.exit(f'Invalid variable on server.env line {number}')
        words = shlex.split(value, comments=True)
        if len(words) != 1:
            sys.exit(f'Quote values containing spaces on server.env line {number}')
        env[key] = words[0]
    os.execve(bundle / 'bin/deskmate-server', ['deskmate-server'], env)
