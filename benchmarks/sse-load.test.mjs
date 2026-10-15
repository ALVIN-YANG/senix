import { test } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { execFile } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const cases = [
  ['valid', null],
  ['truncated', 'truncated event stream'],
  ['out-of-order', 'event framing/order mismatch'],
  ['bad-token', 'event payload mismatch'],
];
for (const [mode, expectedError] of cases) {
  test(`SSE load integrity: ${mode}`, { timeout: 10000 }, async () => {
    const folder = mkdtempSync(join(tmpdir(), 'senix-sse-load-test-'));
    const server = http.createServer((_request, response) => {
      const order = mode === 'truncated' ? [0, 1] : mode === 'out-of-order' ? [0, 2, 1] : [0, 1, 2];
      response.writeHead(200, { 'content-type': 'text/event-stream' });
      for (const seq of order) {
        const token = mode === 'bad-token' ? '你坏' : '你好';
        const empty = JSON.stringify({ seq, token, padding: '' });
        const data = JSON.stringify({ seq, token, padding: 'x'.repeat(128 - Buffer.byteLength(empty)) });
        response.write(`id: ${seq}\ndata: ${data}\n\n`);
      }
      response.end();
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    try {
      const child = await new Promise(resolve => {
        execFile(process.execPath, [fileURLToPath(new URL('./sse-load.mjs', import.meta.url))], {
          env: { ...process.env, URL: `http://127.0.0.1:${server.address().port}/events`, VUS: '1',
            EVENTS: '3', INTERVAL_MS: '1', DATA_BYTES: '128', DURATION_SECONDS: '.05', OUTPUT_DIR: folder },
          timeout: 5000,
        }, (error, stdout, stderr) => resolve({ error, stdout, stderr }));
      });
      const summary = JSON.parse(readFileSync(join(folder, 'summary.json'), 'utf8'));
      if (expectedError) {
        assert.equal(child.error?.code, 1);
        assert.equal(summary.streams_failed, 1);
        assert.equal(summary.streams_completed, 0);
        assert.deepEqual(summary.errors, [expectedError]);
      } else {
        assert.equal(child.error, null, child.stderr);
        assert.equal(summary.streams_failed, 0);
        assert.ok(summary.streams_completed > 0);
        assert.equal(summary.events_received, summary.streams_completed * 3);
        const samples = JSON.parse(readFileSync(join(folder, 'samples.json'), 'utf8'));
        assert.equal(samples.first_event_us.length, summary.streams_completed);
        assert.equal(samples.event_gap_us.length, summary.streams_completed * 2);
      }
    } finally {
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
      rmSync(folder, { recursive: true, force: true });
    }
  });
}
