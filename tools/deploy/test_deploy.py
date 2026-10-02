"""Local deploy regressions; every remote command is replaced by a stub."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
DEPLOY = ROOT / 'companion/crates/server/deploy'


class DeployTests(unittest.TestCase):
    def test_example_leaves_faces_disabled_until_installed(self):
        result = subprocess.run(
            ['/bin/sh', '-c', '. "$1"; printf "%s" "${DESKMATE_FACES_DIR-unset}"',
             'example', str(DEPLOY / 'deskmate-server.env.example')],
            env={k: v for k, v in os.environ.items() if k != 'DESKMATE_FACES_DIR'},
            capture_output=True, text=True, check=True,
        )
        self.assertEqual(result.stdout, 'unset')

    def test_mail_example_can_be_sourced_by_launchd(self):
        example = (DEPLOY / 'deskmate-server.env.example').read_text()
        assignment = re.search(r'^# (DESKMATE_MAIL_FROM=.*)$', example, re.M)[1]
        result = subprocess.run(
            ['/bin/sh', '-c', assignment + '\nprintf "%s" "$DESKMATE_MAIL_FROM"'],
            capture_output=True, text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, '')
        self.assertEqual(result.stdout, 'Deskmate <mail@example.com>')

    def test_readme_lists_all_deploy_modes(self):
        readme = (DEPLOY / 'README.md').read_text().split('## 6.')[1].split('```sh')[1].split('```')[0]
        for mode in ('--ui-only', '--faces-only', '--dry-run', '--status'):
            self.assertIn('deploy.sh ' + mode, readme)
        self.assertIn('# binary + UI + faces', readme)

    def run_deploy(self, args, fail_gate=''):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'companion/faces').mkdir(parents=True)
            (root / 'bin').mkdir()
            (root / 'logs').mkdir()
            stubs = {
                'mktemp': '/usr/bin/mktemp "$STUB_ROOT/logs/gate.XXXXXX"',
                'git': 'case "$*" in "rev-parse --show-toplevel") echo "$STUB_ROOT";; "rev-parse HEAD") echo test-revision;; esac',
                'bun': '''echo "$*" >> "$STUB_ROOT/bun-calls"
if [ "$*" = "$FAIL_GATE" ]; then echo "diagnostic: $* failed" >&2; exit 17; fi''',
                'ssh': 'echo ssh >> "$STUB_ROOT/remote-calls"',
                'rsync': 'echo rsync >> "$STUB_ROOT/remote-calls"',
            }
            for name, body in stubs.items():
                path = root / 'bin' / name
                path.write_text('#!/bin/sh\n' + body + '\n')
                path.chmod(0o700)
            result = subprocess.run(
                ['/bin/bash', str(DEPLOY / 'deploy.sh'), *args], cwd=root,
                env={**os.environ, 'PATH': str(root / 'bin') + os.pathsep + os.environ['PATH'],
                     'STUB_ROOT': str(root), 'TMPDIR': str(root / 'logs'), 'FAIL_GATE': fail_gate},
                capture_output=True, text=True, timeout=10,
            )
            remote = (root / 'remote-calls').read_text() if (root / 'remote-calls').exists() else ''
            calls = (root / 'bun-calls').read_text() if (root / 'bun-calls').exists() else ''
            self.assertEqual(list((root / 'logs').iterdir()), [], 'gate log leaked')
            return result, remote, calls

    def test_conflicting_modes_refuse_before_any_work(self):
        for args in (['--ui-only', '--faces-only'], ['--faces-only', '--ui-only']):
            with self.subTest(args=args):
                result, remote, calls = self.run_deploy(args)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn('choose one of --ui-only / --faces-only', result.stderr)
                self.assertEqual(remote + calls, '')

    def test_each_failed_gate_keeps_diagnostics_and_refuses_remote_commands(self):
        for gate in ('install --frozen-lockfile', 'run check', 'test'):
            with self.subTest(gate=gate):
                result, remote, calls = self.run_deploy(['--faces-only'], gate)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(f'diagnostic: {gate} failed', result.stderr)
                self.assertIn('refusing to ship', result.stderr)
                self.assertEqual(remote, '')
                self.assertEqual(calls.splitlines()[-1], gate)

    def test_successful_gates_clean_log_and_reach_stubbed_sync(self):
        result, remote, calls = self.run_deploy(['--faces-only', '--dry-run'])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, '')
        self.assertIn('rsync', remote)
        self.assertEqual(calls.splitlines(), [
            'run src/author/releases.ts verify', 'install --frozen-lockfile', 'run check', 'test'])


if __name__ == '__main__':
    unittest.main()
