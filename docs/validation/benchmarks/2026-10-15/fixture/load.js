import http from 'k6/http';
import { check } from 'k6';
import { Counter } from 'k6/metrics';

const h1 = new Counter('http1_responses');
const h2 = new Counter('http2_responses');
export const options = {
  vus: Number(__ENV.VUS || 32),
  duration: __ENV.DURATION || '10s',
  discardResponseBodies: true,
  summaryTrendStats: ['avg', 'med', 'p(95)', 'p(99)', 'min', 'max'],
  insecureSkipTLSVerify: true, // Generated, disposable benchmark certificate only.
  thresholds: { http_req_failed: ['rate==0'], checks: ['rate==1'] },
};
export default function () {
  const response = http.get(__ENV.URL, { headers: { Host: 'bench.senix.test' }, timeout: '5s' });
  check(response, { '200 and 1024 bytes': r => r.status === 200 && Number(r.headers['Content-Length']) === 1024 });
  if (response.proto === 'HTTP/2.0') h2.add(1); else h1.add(1);
}
export function handleSummary(data) {
  return { '/output/summary.json': JSON.stringify(data, null, 2) };
}
