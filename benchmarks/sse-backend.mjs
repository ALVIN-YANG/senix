import http from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';

const events = Number(process.env.EVENTS || 100);
const interval = Number(process.env.INTERVAL_MS || 50);
const dataBytes = Number(process.env.DATA_BYTES || 512);
if (!Number.isSafeInteger(events) || events < 2 || !Number.isSafeInteger(interval) || interval < 1
    || !Number.isSafeInteger(dataBytes) || dataBytes < 64) throw new Error('invalid SSE fixture parameters');
const frames = Array.from({ length: events }, (_, seq) => {
  const empty = JSON.stringify({ seq, token: '你好', padding: '' });
  const data = JSON.stringify({ seq, token: '你好', padding: 'x'.repeat(dataBytes - Buffer.byteLength(empty)) });
  return Buffer.from(`id: ${seq}\ndata: ${data}\n\n`);
});
const model = { events_per_stream: events, interval_ms: interval, data_bytes_per_event: dataBytes,
  body_bytes_per_stream: frames.reduce((sum, frame) => sum + frame.length, 0) };
let stats;
const reset = () => { stats = { active: 0, peak_active: 0, started: 0, completed: 0, aborted: 0, backpressure_waits: 0 }; };
reset();

async function waitForDrainOrClose(response) {
  await new Promise(resolve => {
    const done = () => { response.off('drain', done); response.off('close', done); resolve(); };
    response.once('drain', done);
    response.once('close', done);
  });
}

http.createServer(async (request, response) => {
  if (request.url === '/stats') {
    if (request.method === 'POST') {
      if (stats.active) { response.writeHead(409); response.end('streams still active'); return; }
      reset();
    }
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ ...stats, model }));
    return;
  }
  if (request.url !== '/events') { response.writeHead(404); response.end(); return; }
  stats.started += 1;
  stats.active += 1;
  stats.peak_active = Math.max(stats.peak_active, stats.active);
  response.once('close', () => {
    stats.active -= 1;
    if (response.writableFinished) stats.completed += 1; else stats.aborted += 1;
  });
  response.on('error', () => {}); // The close handler records aborted streams.
  response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' });
  response.flushHeaders();
  for (let i = 0; i < frames.length; i += 1) {
    if (response.destroyed) return;
    if (!response.write(frames[i])) {
      stats.backpressure_waits += 1;
      await waitForDrainOrClose(response);
    }
    if (i + 1 < frames.length) await delay(interval);
  }
  if (!response.destroyed) response.end();
}).listen(4101, '0.0.0.0');
