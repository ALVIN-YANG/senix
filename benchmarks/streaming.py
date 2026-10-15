#!/usr/bin/env python3
"""Measure paced SSE delivery using Node's HTTP/1.1 and TLS/HTTP2 clients."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time
import uuid

from run import BACKEND, ROOT, docker, inspect_image, resources, run, source_hash


def backend_stats(container, reset=False):
    method = 'POST' if reset else 'GET'
    code = ("const r = await fetch('http://127.0.0.1:4101/stats', {method:'" + method + "'});"
            "if (!r.ok) throw new Error(await r.text()); console.log(await r.text());")
    return json.loads(docker('exec', container, 'node', '--input-type=module', '-e', code, timeout=15))


def measure(network, prefix, url, vus, seconds, args, output):
    output.mkdir(parents=True)
    command = ['docker', 'run', '--rm', '--name', prefix + '-load', '--network', network,
               '--label', 'senix.benchmark.sse=true', '--cpus=2', '--memory=1g', '--user', '0:0',
               '-v', f'{ROOT / "benchmarks/sse-load.mjs"}:/load.mjs:ro',
               '-v', f'{output}:/output', '-e', f'URL={url}', '-e', f'VUS={vus}',
               '-e', f'DURATION_SECONDS={seconds}', '-e', f'EVENTS={args.events}',
               '-e', f'INTERVAL_MS={args.interval_ms}', '-e', f'DATA_BYTES={args.data_bytes}',
               BACKEND, 'node', '/load.mjs']
    timeout = seconds + max(5, args.events * args.interval_ms * 3 / 1000 + 5) + 20
    try:
        result = subprocess.run(command, text=True, capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired as error:
        captured = ''.join(value.decode(errors='replace') if isinstance(value, bytes) else value or ''
                           for value in (error.stdout, error.stderr))
        (output / 'load.log').write_text(captured)
        raise RuntimeError(f'SSE load exceeded its time budget; see {output / "load.log"}') from error
    (output / 'load.log').write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f'SSE load failed ({result.returncode}); see {output / "load.log"}')
    return json.loads((output / 'summary.json').read_text())


def summarize(rows):
    summary = []
    for case, vus in dict.fromkeys((row['case'], row['vus']) for row in rows):
        group = [row for row in rows if row['case'] == case and row['vus'] == vus]
        summary.append({'case': case, 'vus': vus, 'repeats': len(group),
                        'streams_completed': sum(row['streams_completed'] for row in group),
                        'events_received': sum(row['events_received'] for row in group),
                        'streams_failed': sum(row['streams_failed'] for row in group),
                        'first_event_p95_ms_median': statistics.median(row['first_event_ms']['p95'] for row in group),
                        'first_event_p95_ms_min': min(row['first_event_ms']['p95'] for row in group),
                        'first_event_p95_ms_max': max(row['first_event_ms']['p95'] for row in group),
                        'event_gap_p95_ms_median': statistics.median(row['event_gap_ms']['p95'] for row in group),
                        'complete_stream_p95_ms_median': statistics.median(row['complete_stream_ms']['p95'] for row in group),
                        'backend_peak_active_max': max(row['backend_peak_active'] for row in group),
                        'gateway_rss_mib_end_sample_max': max(row['gateway_rss_mib_end_sample'] for row in group),
                        'gateway_process_hwm_mib': max(row['gateway_process_hwm_mib'] for row in group),
                        'gateway_cpu_percent_median_of_windows': statistics.median(row['gateway_cpu_percent_of_window'] for row in group)})
    return summary


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', default='senix:benchmark')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/streaming-benchmark')
    parser.add_argument('--vus', default='1,32,128')
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--duration-seconds', type=float, default=20)
    parser.add_argument('--warmup-seconds', type=float, default=3)
    parser.add_argument('--events', type=int, default=100)
    parser.add_argument('--interval-ms', type=int, default=50)
    parser.add_argument('--data-bytes', type=int, default=512)
    args = parser.parse_args()
    try:
        vus_values = [int(value) for value in args.vus.split(',')]
    except ValueError:
        parser.error('vus must be comma-separated positive integers')
    if (not vus_values or len(set(vus_values)) != len(vus_values) or min(vus_values) < 1
            or not all(math.isfinite(value) for value in (args.duration_seconds, args.warmup_seconds))
            or args.repeats < 1 or args.duration_seconds <= 0 or args.warmup_seconds <= 0
            or args.events < 2 or args.interval_ms < 1 or args.data_bytes < 64):
        parser.error('positive durations/repeats/users, at least two events and 64 data bytes are required')
    output = args.output.resolve()
    if output.exists() and any(output.iterdir()):
        parser.error('output is not empty; use a new directory')
    output.mkdir(parents=True, exist_ok=True)
    prefix = 'senix-sse-' + uuid.uuid4().hex[:10]
    names = [prefix + '-backend', prefix + '-gateway', prefix + '-load']
    network = prefix + '-net'
    info = json.loads(docker('info', '--format', '{{json .}}'))
    fixture_files = ['benchmarks/streaming.py', 'benchmarks/sse-load.mjs', 'benchmarks/sse-backend.mjs',
                     'benchmarks/run.py', 'Dockerfile', '.dockerignore']
    metadata = {
        'workload': 'paced-sse', 'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'git_head': run('git', 'rev-parse', 'HEAD', cwd=ROOT),
        'dirty': bool(run('git', 'status', '--porcelain', cwd=ROOT)), 'source_sha256': source_hash(),
        'host': platform.platform(), 'docker': {key: info.get(key) for key in
            ('ServerVersion', 'OperatingSystem', 'Architecture', 'NCPU', 'MemTotal', 'KernelVersion')},
        'images': {image: inspect_image(image) for image in (args.image, BACKEND)},
        'options': {**vars(args), 'output': str(output)},
        'limits_per_container': {'cpus': 2, 'memory_bytes': 1073741824}, 'backend_workers': 1,
        'fixture_sha256': {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in fixture_files},
        'latency_samples': 'microseconds rounded from performance.now(); percentiles use linear interpolation',
        'latency_scope': 'request start to complete validated first event; includes initial DNS/TCP/TLS wait and client scheduling/parsing',
        'connections': 'HTTP/1.1 keep-alive pool up to VUs; TLS/HTTP2 uses one multiplexed session per load process',
        'window': 'stop starting requests at duration; allow active streams to finish; rates use actual load-process window',
        'resource_method': 'cgroup v2 CPU counters and PID 1 RSS/HWM before/after; CPU window includes load-container startup/exit',
        'tls': 'disposable self-signed gateway certificate; benchmark H2 client disables verification',
        'access_logging': False, 'external_monitoring': False, 'container_prefix': prefix,
        'status': 'running',
    }
    (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    rows = []
    try:
        docker('network', 'create', '--label', 'senix.benchmark.sse=true', network)
        docker('run', '-d', '--name', names[0], '--network', network, '--network-alias', 'backend',
               '--label', 'senix.benchmark.sse=true', '--cpus=2', '--memory=1g',
               '-v', f'{ROOT / "benchmarks/sse-backend.mjs"}:/backend.mjs:ro',
               '-e', f'EVENTS={args.events}', '-e', f'INTERVAL_MS={args.interval_ms}',
               '-e', f'DATA_BYTES={args.data_bytes}', BACKEND, 'node', '/backend.mjs')
        for _ in range(30):
            try:
                metadata['model'] = backend_stats(names[0])['model']
                metadata['node_version'] = docker('exec', names[0], 'node', '--version')
                break
            except subprocess.CalledProcessError:
                time.sleep(.2)
        else:
            raise RuntimeError('SSE backend did not start: ' + docker('logs', names[0]))
        address = json.loads(docker('inspect', names[0]))[0]['NetworkSettings']['Networks'][network]['IPAddress']
        with tempfile.TemporaryDirectory(prefix='senix-sse-benchmark-') as fixture:
            fixture = Path(fixture)
            run('openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                '-subj', '/CN=bench.senix.test', '-keyout', str(fixture / 'tls.key'), '-out', str(fixture / 'tls.crt'))
            os.chmod(fixture, 0o755)
            os.chmod(fixture / 'tls.key', 0o644)
            (fixture / 'gateway.json').write_text(json.dumps({'routes': [{'id': 'stream', 'host': 'bench.senix.test',
                'path_prefix': '/', 'backends': [{'id': 'stream-backend', 'address': f'{address}:4101', 'generation': 1, 'weight': 100}]}]}))
            docker('run', '-d', '--name', names[1], '--network', network, '--network-alias', 'senix',
                   '--label', 'senix.benchmark.sse=true', '--cpus=2', '--memory=1g',
                   '-v', f'{fixture}:/fixture:ro', args.image, '--listen', '0.0.0.0:8080',
                   '--admin-listen', '0.0.0.0:9080', '--db', '/tmp/senix.db', '--config', '/fixture/gateway.json',
                   '--tls-listen', '0.0.0.0:8443', '--tls-cert', '/fixture/tls.crt', '--tls-key', '/fixture/tls.key')
            for _ in range(30):
                if subprocess.run(['docker', 'exec', names[1], '/usr/local/bin/senixd', 'healthcheck'],
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                    break
                time.sleep(.5)
            else:
                raise RuntimeError('gateway did not start: ' + docker('logs', names[1]))
            cases = [('direct-sse', 'http://backend:4101/events'), ('senix-sse-http', 'http://senix:8080/events'),
                     ('senix-sse-https-h2', 'https://senix:8443/events')]
            for case, url in cases:
                for vus in vus_values:
                    backend_stats(names[0], reset=True)
                    measure(network, prefix, url, vus, args.warmup_seconds, args,
                            output / f'{case}-vus{vus}-warmup')
                    for repeat in range(1, args.repeats + 1):
                        path = output / f'{case}-vus{vus}-run{repeat}'
                        backend_stats(names[0], reset=True)
                        before = {role: resources(name) for role, name in zip(('backend', 'gateway'), names[:2])}
                        started = time.monotonic()
                        print(f'{case}: {vus} VUs, repeat {repeat}/{args.repeats}', flush=True)
                        data = measure(network, prefix, url, vus, args.duration_seconds, args, path)
                        elapsed = time.monotonic() - started
                        after = {role: resources(name) for role, name in zip(('backend', 'gateway'), names[:2])}
                        stats = backend_stats(names[0])
                        (path / 'resources.json').write_text(json.dumps({'window_seconds': elapsed, 'before': before,
                            'after': after, 'backend_stats': stats}, indent=2) + '\n')
                        expected_protocol = 'h2' if case.endswith('-h2') else '1.1'
                        assert data['streams_failed'] == 0 and data['streams_completed'] > 0
                        assert data['protocols'] == {expected_protocol: data['streams_completed']}
                        assert data['events_received'] == data['streams_completed'] * args.events
                        assert data['response_body_bytes'] == data['streams_completed'] * metadata['model']['body_bytes_per_stream']
                        assert stats['active'] == 0 and stats['aborted'] == 0 and stats['completed'] == data['streams_completed']
                        row = {'case': case, 'vus': vus, 'repeat': repeat, **data,
                               'backend_peak_active': stats['peak_active'],
                               'gateway_cpu_percent_of_window': (after['gateway']['usage_usec'] - before['gateway']['usage_usec']) / elapsed / 10000,
                               'gateway_rss_mib_end_sample': after['gateway']['VmRSS'] / 1024,
                               'gateway_process_hwm_mib': after['gateway']['VmHWM'] / 1024}
                        rows.append(row)
                        (output / 'runs.json').write_text(json.dumps(rows, indent=2) + '\n')
        summary = summarize(rows)
        (output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
        metadata['status'] = 'passed'
        print(json.dumps(summary, indent=2))
    except BaseException as error:
        metadata['status'] = 'failed'
        metadata['error'] = str(error)
        raise
    finally:
        for name in reversed(names):
            subprocess.run(['docker', 'rm', '-f', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(['docker', 'network', 'rm', network], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        metadata['finished_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')


if __name__ == '__main__':
    main()
