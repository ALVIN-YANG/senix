#!/usr/bin/env python3
"""Reproducible Docker-network benchmark. Requires Python 3, Docker and openssl."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
K6 = 'grafana/k6:2.3.0'
BACKEND = 'node:24-bookworm-slim'


def run(*args, **kwargs):
    return subprocess.run(args, check=True, text=True, capture_output=True, **kwargs).stdout.strip()


def docker(*args, **kwargs):
    return run('docker', *args, **kwargs)


def inspect_image(image):
    info = json.loads(docker('image', 'inspect', image))[0]
    return {key: info.get(key) for key in ('Id', 'RepoDigests', 'Architecture', 'Os')}


def source_hash():
    digest = hashlib.sha256()
    paths = [ROOT / 'Cargo.toml', ROOT / 'Cargo.lock']
    paths += sorted((ROOT / 'crates').rglob('*.rs'))
    paths += sorted((ROOT / 'crates/senixd/assets').glob('*'))
    for path in paths:
        digest.update(str(path.relative_to(ROOT)).encode())
        digest.update(path.read_bytes())
    return digest.hexdigest()


def resources(container):
    raw = docker('exec', container, 'cat', '/sys/fs/cgroup/cpu.stat', '/proc/1/status')
    values = {}
    for line in raw.splitlines():
        fields = line.replace(':', '').split()
        if fields and fields[0] in ('usage_usec', 'nr_throttled', 'throttled_usec', 'VmRSS', 'VmHWM'):
            values[fields[0]] = int(fields[1])
    assert all(key in values for key in ('usage_usec', 'VmRSS', 'VmHWM')), 'Linux cgroup v2 and procfs are required'
    return values


def k6(network, prefix, url, vus, duration, output):
    output.mkdir(parents=True, exist_ok=True)
    command = ['docker', 'run', '--rm', '--name', prefix + '-load', '--network', network,
           '--cpus=2', '--memory=1g', '--user', '0:0',
           '-v', f'{ROOT / "benchmarks/load.js"}:/load.js:ro',
           '-v', f'{output}:/output', '-e', f'URL={url}', '-e', f'VUS={vus}',
           '-e', f'DURATION={duration}', K6, 'run', '--quiet', '/load.js']
    result = subprocess.run(command, text=True, capture_output=True)
    (output / 'k6.log').write_text(result.stdout + result.stderr)
    if result.returncode:
        raise RuntimeError(f'k6 failed ({result.returncode}); see {output / "k6.log"}')
    return json.loads((output / 'summary.json').read_text())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', default='senix:observability')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/benchmarks')
    parser.add_argument('--vus', default='1,32,128')
    parser.add_argument('--repeats', type=int, default=3)
    parser.add_argument('--duration', default='10s')
    parser.add_argument('--warmup', default='3s')
    args = parser.parse_args()
    vus_values = [int(value) for value in args.vus.split(',')]
    if args.repeats < 1 or any(value < 1 for value in vus_values):
        parser.error('repeats and virtual users must be positive')
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    if (output / 'metadata.json').exists():
        parser.error('output already contains a run; use a new directory')
    prefix = 'senix-bench-' + uuid.uuid4().hex[:10]
    names = [prefix + '-backend', prefix + '-gateway', prefix + '-load']
    network = prefix + '-net'
    metadata = {
        'started_utc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
        'git_head': run('git', 'rev-parse', 'HEAD', cwd=ROOT),
        'dirty': bool(run('git', 'status', '--porcelain', cwd=ROOT)),
        'source_sha256': source_hash(), 'host': platform.platform(),
        'docker': json.loads(docker('info', '--format', '{{json .}}')),
        'images': {image: inspect_image(image) for image in (args.image, K6, BACKEND)},
        'options': {**vars(args), 'output': str(output)},
        'limits_per_container': {'cpus': 2, 'memory_bytes': 1073741824},
        'payload_bytes': 1024, 'backend_workers': 1,
        'fixture_sha256': {name: hashlib.sha256((ROOT / 'benchmarks' / name).read_bytes()).hexdigest()
                           for name in ('run.py', 'load.js', 'backend.mjs')},
        'resource_method': 'cgroup v2 CPU counters and /proc/1/status RSS/HWM, before and after each run',
        'access_logging': False, 'external_monitoring': False,
        'request_mode': 'closed loop, keep-alive, no pacing, downstream TLS verification disabled only for the self-signed fixture',
    }
    # Docker info can include local paths/proxies; publish only reproducibility fields.
    metadata['docker'] = {key: metadata['docker'].get(key) for key in
                          ('ServerVersion', 'OperatingSystem', 'Architecture', 'NCPU', 'MemTotal', 'KernelVersion')}
    (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    rows = []
    try:
        docker('network', 'create', '--label', 'senix.benchmark=true', network)
        docker('run', '-d', '--name', names[0], '--network', network, '--network-alias', 'backend',
               '--cpus=2', '--memory=1g', '-v', f'{ROOT / "benchmarks/backend.mjs"}:/backend.mjs:ro',
               BACKEND, 'node', '/backend.mjs')
        address = json.loads(docker('inspect', names[0]))[0]['NetworkSettings']['Networks'][network]['IPAddress']
        with tempfile.TemporaryDirectory(prefix='senix-benchmark-') as fixture:
            fixture = Path(fixture)
            run('openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1',
                '-subj', '/CN=bench.senix.test', '-keyout', str(fixture / 'tls.key'), '-out', str(fixture / 'tls.crt'))
            # The non-root Senix process must read the disposable bind-mounted fixture.
            os.chmod(fixture, 0o755)
            os.chmod(fixture / 'tls.key', 0o644)
            for count in (1, 100):
                routes = [{'id': f'route-{i}', 'host': 'bench.senix.test' if i == 0 else f'route-{i}.bench.test',
                           'path_prefix': '/', 'backends': [{'id': f'backend-{i}', 'address': f'{address}:4101',
                                                           'generation': 1, 'weight': 100}]} for i in range(count)]
                (fixture / 'gateway.json').write_text(json.dumps({'routes': routes}))
                docker('run', '-d', '--name', names[1], '--network', network, '--network-alias', 'senix',
                       '--cpus=2', '--memory=1g', '-v', f'{fixture}:/fixture:ro', args.image,
                       '--listen', '0.0.0.0:8080', '--admin-listen', '0.0.0.0:9080',
                       '--db', '/tmp/senix.db', '--config', '/fixture/gateway.json',
                       '--tls-listen', '0.0.0.0:8443', '--tls-cert', '/fixture/tls.crt', '--tls-key', '/fixture/tls.key')
                for _ in range(30):
                    if subprocess.run(['docker', 'exec', names[1], '/usr/local/bin/senixd', 'healthcheck'],
                                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0:
                        break
                    time.sleep(0.5)
                else:
                    raise RuntimeError('gateway did not start: ' + docker('logs', names[1]))
                cases = ([('direct-http', 'http://backend:4101/'), ('senix-http', 'http://senix:8080/'),
                          ('senix-https', 'https://senix:8443/')] if count == 1 else
                         [('senix-http-100-routes', 'http://senix:8080/')])
                for case, url in cases:
                    for vus in vus_values:
                        k6(network, prefix, url, vus, args.warmup, output / f'{case}-vus{vus}-warmup')
                        for repeat in range(1, args.repeats + 1):
                            path = output / f'{case}-vus{vus}-run{repeat}'
                            print(f'{case}: {vus} VUs, repeat {repeat}/{args.repeats}', flush=True)
                            path.mkdir()
                            before = {role: resources(name) for role, name in zip(('backend', 'gateway'), names[:2])}
                            started = time.monotonic()
                            data = k6(network, prefix, url, vus, args.duration, path)
                            elapsed = time.monotonic() - started
                            after = {role: resources(name) for role, name in zip(('backend', 'gateway'), names[:2])}
                            (path / 'resources.json').write_text(json.dumps({
                                'window_seconds': elapsed, 'before': before, 'after': after,
                                'note': 'CPU window includes load-container startup/exit; RSS is an end sample; HWM is process-lifetime peak.'
                            }, indent=2) + '\n')
                            metrics = data['metrics']
                            duration = metrics['http_req_duration']['values']
                            rows.append({'case': case, 'vus': vus, 'repeat': repeat,
                                         'requests': metrics['http_reqs']['values']['count'],
                                         'rps': metrics['http_reqs']['values']['rate'],
                                         'p50_ms': duration['med'], 'p95_ms': duration['p(95)'],
                                         'p99_ms': duration['p(99)'], 'failed_rate': metrics['http_req_failed']['values']['rate'],
                                         'checks_rate': metrics['checks']['values']['rate'],
                                         'http2_responses': metrics.get('http2_responses', {}).get('values', {}).get('count', 0)})
                            (output / 'runs.json').write_text(json.dumps(rows, indent=2) + '\n')
                docker('rm', '-f', names[1])
        summary = []
        for case in dict.fromkeys(row['case'] for row in rows):
            for vus in vus_values:
                group = [row for row in rows if row['case'] == case and row['vus'] == vus]
                summary.append({'case': case, 'vus': vus, 'repeats': len(group),
                                **{key: statistics.median(row[key] for row in group) for key in
                                   ('rps', 'p50_ms', 'p95_ms', 'p99_ms')},
                                'rps_min': min(row['rps'] for row in group),
                                'rps_max': max(row['rps'] for row in group),
                                'requests': sum(row['requests'] for row in group),
                                'failed_rate_max': max(row['failed_rate'] for row in group)})
        (output / 'summary.json').write_text(json.dumps(summary, indent=2) + '\n')
        metadata['finished_utc'] = time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())
        (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
        print(json.dumps(summary, indent=2))
    finally:
        for name in reversed(names):
            subprocess.run(['docker', 'rm', '-f', name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run(['docker', 'network', 'rm', network], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


if __name__ == '__main__':
    main()
