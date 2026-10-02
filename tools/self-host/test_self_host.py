"""Exercise failures and exec in temporary bundles, with no builds or servers."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


class SelfHostTests(unittest.TestCase):
    def test_missing_build_prerequisite_names_the_tool(self):
        required = ('python3', 'curl', 'cargo', 'bun')
        for missing in required:
            with self.subTest(tool=missing), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                bindir = root / 'bin'; bindir.mkdir()
                # build.sh needs dirname to locate its root before checking prerequisites.
                (bindir / 'dirname').symlink_to(shutil.which('dirname'))
                for name in required:
                    if name != missing:
                        stub = bindir / name
                        stub.write_text('#!/bin/sh\nexit 99\n'); stub.chmod(0o700)
                result = subprocess.run(
                    ['/bin/bash', str(HERE / 'build.sh'), str(root / 'bundle')],
                    env={**os.environ, 'PATH': str(bindir)}, capture_output=True, text=True, timeout=5,
                )
                self.assertEqual(result.returncode, 1)
                self.assertEqual(result.stderr.strip(), f'build.sh needs {missing} on PATH')
                self.assertFalse((root / 'bundle').exists())

    def run_bundle(self, binary):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            bundle = root / 'bundle'; bundle.mkdir()
            instance = root / 'instance'; instance.mkdir()
            shutil.copy2(HERE / 'instance.py', bundle / 'instance.py')
            config = instance / 'server.env'
            config.write_text('DESKMATE_PUBLIC_URL=http://localhost:8443\nDESKMATE_MAIL_FROM="Deskboy <mail@example.com>"\n')
            config.chmod(0o600)
            if binary:
                (bundle / 'bin').mkdir()
                server = bundle / 'bin/deskmate-server'
                server.write_text('''#!/bin/sh
printf '%s\\n' "$DESKMATE_PUBLIC_URL" "$DESKMATE_MAIL_FROM" "${DESKMATE_INHERITED-unset}"
exit 37
''')
                server.chmod(0o700)
            result = subprocess.run(
                [sys.executable, str(bundle / 'instance.py'), 'run', str(instance)],
                env={**os.environ, 'DESKMATE_INHERITED': 'must not leak'},
                capture_output=True, text=True, timeout=5,
            )
            return result, bundle.resolve()

    def test_missing_bundle_binary_has_one_line_rebuild_advice(self):
        result, bundle = self.run_bundle(False)
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stderr, f'{bundle}/bin/deskmate-server is missing; rebuild the bundle.\n')
        self.assertEqual(result.stdout, '')

    def test_runner_execs_binary_with_parsed_instance_environment(self):
        result, _ = self.run_bundle(True)
        self.assertEqual(result.returncode, 37)
        self.assertEqual(result.stderr, '')
        self.assertEqual(result.stdout, 'http://localhost:8443\nDeskboy <mail@example.com>\nunset\n')

    def test_guide_version_follows_bundle_instead_of_a_stale_literal(self):
        guide = (ROOT / 'docs/self-host.md').read_text()
        line = next(line for line in guide.splitlines() if line.startswith('DESKMATE_FIRMWARE_VERSION='))
        self.assertEqual(line, "DESKMATE_FIRMWARE_VERSION=<the checkout's firmware/version.txt value>")
        self.assertNotIn('Current main is already contained in this branch.', guide)
        self.assertIn('panel provisioning and OTA were not exercised.', guide)


if __name__ == '__main__':
    unittest.main()
