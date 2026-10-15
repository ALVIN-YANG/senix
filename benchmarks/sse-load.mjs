import http from 'node:http';
import http2 from 'node:http2';
import { writeFileSync } from 'node:fs';
import { StringDecoder } from 'node:string_decoder';
import { performance } from 'node:perf_hooks';

const url = new URL(process.env.URL);
const vus = Number(process.env.VUS || 32);
const duration = Number(process.env.DURATION_SECONDS || 20);
const events = Number(process.env.EVENTS || 100);
const interval = Number(process.env.INTERVAL_MS || 50);
const dataBytes = Number(process.env.DATA_BYTES || 512);
const output = process.env.OUTPUT_DIR || '/output';
if (![vus, events, interval, dataBytes].every(Number.isSafeInteger) || vus < 1 || events < 2
    || interval < 1 || dataBytes < 64 || !Number.isFinite(duration) || duration <= 0) throw new Error('invalid load parameters');
const requestTimeout = Math.max(5000, events * interval * 3 + 5000);
const agent = new http.Agent({ keepAlive: true, maxSockets: vus, maxFreeSockets: vus });
let session;
let remoteMaxConcurrentStreams = null;
const samples = { first_event_us: [], event_gap_us: [], complete_stream_us: [], delivery_batches: [] };
const errors = [];
let failed = 0;
let active = 0;
let peakActive = 0;
let completed = 0;
let bodyBytes = 0;
let receivedEvents = 0;
let stop = false;
const protocols = {};

function openResponse() {
  let request;
  const response = new Promise((resolve, reject) => {
    if (url.protocol === 'https:') {
      if (!session) {
        session = http2.connect(url.origin, { rejectUnauthorized: false, servername: 'bench.senix.test' });
        session.on('error', () => {}); // Each affected request reports its own error.
        session.on('remoteSettings', settings => { remoteMaxConcurrentStreams = settings.maxConcurrentStreams; });
      }
      request = session.request({ ':method': 'GET', ':path': url.pathname,
        ':authority': 'bench.senix.test', accept: 'text/event-stream' });
      request.once('response', headers => {
        if (session.alpnProtocol !== 'h2') reject(new Error('TLS did not negotiate h2'));
        else resolve({ stream: request, status: headers[':status'], type: headers['content-type'], protocol: 'h2' });
      });
      request.end();
    } else {
      request = http.get(url, { agent, headers: { Host: 'bench.senix.test', Accept: 'text/event-stream' } }, stream => {
        resolve({ stream, status: stream.statusCode, type: stream.headers['content-type'], protocol: stream.httpVersion });
      });
    }
    request.once('error', reject);
  });
  return { response, abort: () => request?.destroy(new Error('stream timeout or invalid response')) };
}

async function oneStream() {
  const started = performance.now();
  const transport = openResponse();
  const timer = setTimeout(transport.abort, requestTimeout);
  let complete = false;
  try {
    const { stream, status, type, protocol } = await transport.response;
    if (status !== 200 || type !== 'text/event-stream') throw new Error(`invalid response: ${status}, ${type}`);
    const decoder = new StringDecoder('utf8');
    let buffer = '';
    let count = 0;
    let bytes = 0;
    let batches = 0;
    let first;
    let previous;
    const gaps = [];
    for await (const chunk of stream) {
      bytes += chunk.length;
      buffer += decoder.write(chunk);
      let delivered = false;
      for (let end; (end = buffer.indexOf('\n\n')) !== -1;) {
        const event = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const lines = event.split('\n');
        if (lines.length !== 2 || lines[0] !== `id: ${count}` || !lines[1].startsWith('data: ')) throw new Error('event framing/order mismatch');
        const data = lines[1].slice(6);
        const message = JSON.parse(data);
        if (Buffer.byteLength(data) !== dataBytes || message.seq !== count || message.token !== '你好'
            || typeof message.padding !== 'string' || !/^x*$/.test(message.padding)) throw new Error('event payload mismatch');
        const now = performance.now();
        if (first === undefined) first = Math.round((now - started) * 1000);
        if (previous !== undefined) gaps.push(Math.round((now - previous) * 1000));
        previous = now;
        count += 1;
        delivered = true;
      }
      if (delivered) batches += 1;
      if (buffer.length > dataBytes * 2 + 1024 || count > events) throw new Error('unexpected event count/size');
    }
    buffer += decoder.end();
    if (buffer !== '' || count !== events) throw new Error('truncated event stream');
    samples.first_event_us.push(first);
    samples.event_gap_us.push(...gaps);
    samples.complete_stream_us.push(Math.round((performance.now() - started) * 1000));
    samples.delivery_batches.push(batches);
    protocols[protocol] = (protocols[protocol] || 0) + 1;
    completed += 1;
    receivedEvents += count;
    bodyBytes += bytes;
    complete = true;
  } finally {
    clearTimeout(timer);
    if (!complete) transport.abort();
  }
}

function distribution(values, scale = 1) {
  if (!values.length) return null;
  const sorted = [...values].sort((a, b) => a - b);
  const percentile = p => {
    const index = (sorted.length - 1) * p;
    const lower = Math.floor(index), upper = Math.ceil(index);
    return (sorted[lower] + (sorted[upper] - sorted[lower]) * (index - lower)) / scale;
  };
  return { count: sorted.length, min: sorted[0] / scale, p50: percentile(.5), p95: percentile(.95), p99: percentile(.99), max: sorted.at(-1) / scale };
}

const started = performance.now();
const deadline = started + duration * 1000;
await Promise.all(Array.from({ length: vus }, async () => {
  while (!stop && performance.now() < deadline) {
    active += 1;
    peakActive = Math.max(peakActive, active);
    try { await oneStream(); }
    catch (error) {
      failed += 1;
      stop = true;
      if (errors.length < 8) errors.push(error.message);
    } finally { active -= 1; }
  }
}));
const measuredSeconds = (performance.now() - started) / 1000;
agent.destroy();
session?.destroy();
const summary = { configured_seconds: duration, measured_seconds: measuredSeconds, vus,
  client_peak_outstanding: peakActive, streams_completed: completed, streams_failed: failed,
  events_received: receivedEvents, response_body_bytes: bodyBytes, protocols, errors,
  http2_remote_max_concurrent_streams: remoteMaxConcurrentStreams,
  streams_per_second: completed / measuredSeconds, events_per_second: receivedEvents / measuredSeconds,
  first_event_ms: distribution(samples.first_event_us, 1000), event_gap_ms: distribution(samples.event_gap_us, 1000),
  complete_stream_ms: distribution(samples.complete_stream_us, 1000), delivery_batches: distribution(samples.delivery_batches) };
writeFileSync(`${output}/samples.json`, JSON.stringify(samples));
writeFileSync(`${output}/summary.json`, `${JSON.stringify(summary, null, 2)}\n`);
console.log(JSON.stringify(summary));
if (failed || !completed) process.exitCode = 1;
