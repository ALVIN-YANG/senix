"""CLI regressions using the archived, independently measured SSE dataset."""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / 'docs/validation/sse-performance/2026-10-15/raw-results.zip'


class ArchivedSseReport(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='senix-report-test-')
        self.addCleanup(temporary.cleanup)
        self.folder = Path(temporary.name)
        self.source = self.folder / 'raw'
        with zipfile.ZipFile(FIXTURE) as archive:
            for name in archive.namelist():
                path = Path(name)
                self.assertFalse(path.is_absolute() or '..' in path.parts)
            archive.extractall(self.source)

    def report(self, output):
        return subprocess.run([sys.executable, str(ROOT / 'benchmarks/report.py'),
                               '--input', str(self.source), '--output', str(output)],
                              text=True, capture_output=True, timeout=20)

    def test_exports_the_measured_baseline_with_valid_checksums(self):
        output = self.folder / 'report'
        result = self.report(output)
        self.assertEqual(result.returncode, 0, result.stderr)
        verification = json.loads((output / 'verification.json').read_text())
        self.assertEqual(verification['verified_measurements'], 27)
        self.assertEqual(verification['streams_completed'], 6498)
        self.assertEqual(verification['events_received'], 649800)
        for line in (output / 'SHA256SUMS').read_text().splitlines():
            digest, name = line.split('  ')
            self.assertEqual(hashlib.sha256((output / name).read_bytes()).hexdigest(), digest)

    def test_rejects_modified_raw_samples_before_export(self):
        first = json.loads((self.source / 'runs.json').read_text())[0]
        path = self.source / f"{first['case']}-vus{first['vus']}-run{first['repeat']}" / 'samples.json'
        samples = json.loads(path.read_text())
        samples['first_event_us'][0] += 1_000_000
        path.write_text(json.dumps(samples))
        output = self.folder / 'rejected-report'
        result = self.report(output)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('AssertionError', result.stderr)
        self.assertFalse(output.exists())

    def test_rejects_nested_output_before_copying(self):
        output = self.source / 'report'
        result = self.report(output)
        self.assertEqual(result.returncode, 2)
        self.assertIn('output must be outside the input directory', result.stderr)
        self.assertFalse(output.exists())


if __name__ == '__main__':
    unittest.main()
