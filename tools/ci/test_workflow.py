"""Keep non-firmware verification reachable when the IDF build fails."""
from pathlib import Path
import re
import unittest

WORKFLOW = Path(__file__).resolve().parents[2] / '.github/workflows/ci.yml'


def jobs():
    sections = re.split(r'^  ([a-z][a-z-]*):\n', WORKFLOW.read_text(), flags=re.M)
    return dict(zip(sections[1::2], sections[2::2]))


class WorkflowTests(unittest.TestCase):
    def test_native_jobs_fetch_their_own_pinned_sources(self):
        for name in ('companion', 'firmware-host-tests'):
            with self.subTest(job=name):
                job = jobs()[name]
                self.assertNotIn('    needs:', job)
                self.assertIn('python3 tools/self-host/fetch-components.py', job)
                self.assertNotIn('download-artifact', job)

    def test_firmware_still_builds_and_verifies_installed_sources(self):
        job = jobs()['firmware']
        build = job.index('command: idf.py build')
        lock = job.index('git diff --exit-code -- firmware/dependencies.lock')
        verify = job.index('python3 tools/self-host/fetch-components.py')
        self.assertLess(build, lock)
        self.assertLess(lock, verify)
        self.assertNotIn('firmware-managed-components', WORKFLOW.read_text())

    def test_producer_tests_have_an_independent_job_with_pillow(self):
        job = jobs()['tools']
        self.assertNotIn('    needs:', job)
        install = job.index('python3 -m pip install pillow')
        tests = job.index("python3 -m unittest discover -s tools/picture-producers -p 'test_*.py'")
        self.assertLess(install, tests)


if __name__ == '__main__':
    unittest.main()
