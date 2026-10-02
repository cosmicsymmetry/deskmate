"""Harness contracts; subprocess tests never build or contact a real server."""
import ast
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


class WebcheckTests(unittest.TestCase):
    def test_pixel_walk_includes_every_mock_scenario(self):
        tree = ast.parse((HERE / 'pixelab.py').read_text())
        scenarios = next(ast.literal_eval(node.value) for node in tree.body
                         if isinstance(node, ast.Assign) and any(
                             isinstance(t, ast.Name) and t.id == 'SCENARIOS' for t in node.targets))
        mock = (ROOT / 'companion/apps/deskmate/src/dev/mockBackend.ts').read_text()
        union = mock.split('export const SCENARIOS = [', 1)[1].split(']', 1)[0]
        self.assertEqual(set(scenarios), set(re.findall(r'"([a-z]+)"', union)))

    def run_smoke(self, mode):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for directory in ('bin', 'companion/target/debug', 'companion/apps/deskmate', 'firmware'):
                (root / directory).mkdir(parents=True)
            (root / 'firmware/version.txt').write_text('test-version\n')
            # Keep builds and probes isolated even though smoke prepends developer tools to PATH.
            script = re.sub(r'export PATH=.*?; export CARGO_TERM_COLOR=never',
                            'export CARGO_TERM_COLOR=never', (HERE / 'smoke.sh').read_text())
            (root / 'smoke.sh').write_text(script)
            for name, body in {
                'cargo': 'exit 0', 'bun': 'exit 0',
                'curl': 'case "$PROBE_MODE" in startup-*) exit 1;; esac',
                'sleep': 'exec /bin/sleep 0.01',
                'python3': 'exec "$REAL_PYTHON" "$@"',
            }.items():
                path = root / 'bin' / name
                path.write_text('#!/bin/sh\n' + body + '\n'); path.chmod(0o700)
            server = root / 'companion/target/debug/server'
            server.write_text('''#!/bin/sh
[ "$PROBE_MODE" != startup-exit ] || exit 7
[ -n "$DESKMATE_PUBLIC_URL" ] || exit 8
printf ' WARN stub server\n'
exec "$REAL_PYTHON" -c 'import signal, sys, time; signal.signal(signal.SIGTERM, lambda *_: sys.exit(0)); time.sleep(30)'
''')
            server.chmod(0o700)
            (root / 'smoke.py').write_text('''import os, pathlib, sys, time
out = pathlib.Path(sys.argv[3])
if os.environ['PROBE_MODE'] == 'browser-failure': sys.exit(23)
(out / 'page-open').touch()
while not (out / 'server-stopped').exists(): time.sleep(.01)
''')
            return subprocess.run(
                ['/bin/bash', str(root / 'smoke.sh'), str(root), 'probe', '5999'],
                env={**os.environ, 'PATH': str(root / 'bin') + ':/usr/bin:/bin',
                     'REAL_PYTHON': sys.executable, 'TMPDIR': str(root), 'PROBE_MODE': mode},
                capture_output=True, text=True, timeout=15,
            )

    def test_startup_failures_are_nonzero(self):
        for mode in ('startup-exit', 'startup-timeout'):
            with self.subTest(mode=mode):
                result = self.run_smoke(mode)
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn('SERVER STARTUP FAILED', result.stderr)

    def test_browser_failure_is_propagated(self):
        result = self.run_smoke('browser-failure')
        self.assertEqual(result.returncode, 23, result.stdout + result.stderr)

    def test_success_waits_for_browser_and_server_shutdown(self):
        result = self.run_smoke('success')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn('still-alive-after-15s=no', result.stdout)


if __name__ == '__main__':
    unittest.main()
