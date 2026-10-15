#!/usr/bin/env python3
"""Check real Docker demo startup and saved configuration in disposable Compose projects."""
import argparse
import copy
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
COMPOSE = ['docker', 'compose', '--project-directory', str(ROOT / 'examples/demo'),
           '--file', str(ROOT / 'examples/demo/compose.yml')]


def checked(condition, message):
    if not condition:
        raise RuntimeError(message)


def run_case(args, output, number):
    project = 'senix-check-' + uuid.uuid4().hex[:12]
    password = None
    env = {**os.environ, 'COMPOSE_PROJECT_NAME': project, 'SENIX_DEMO_PULL': '0',
           'SENIX_DEMO_HOST': 'demo.senix.local',
           'SENIX_DEMO_PROXY_PORT': str(args.proxy_port), 'SENIX_DEMO_ADMIN_PORT': str(args.admin_port)}
    env.pop('SENIX_IMAGE', None)
    env.pop('SENIX_DEMO_PASSWORD', None)
    if args.image:
        env['SENIX_IMAGE'] = args.image
    log = output / f'run-{number}.log'
    result = {'run': number, 'project': project, 'status': 'running', 'checks': []}
    started = time.monotonic()

    def command(arguments, timeout=120, expect_failure=False):
        nonlocal password
        try:
            completed = subprocess.run(arguments, cwd=ROOT, env=env, text=True,
                                       capture_output=True, timeout=timeout)
            captured = completed.stdout + completed.stderr
        except subprocess.TimeoutExpired as error:
            captured = ''.join(value.decode(errors='replace') if isinstance(value, bytes) else value or ''
                               for value in (error.stdout, error.stderr))
            completed = None
        if completed is not None and password is None and arguments == [str(ROOT / 'scripts/demo.sh')]:
            match = re.search(r'^Password: (.+)$', completed.stdout, re.MULTILINE)
            if match:
                password = match.group(1)
        if password:
            captured = captured.replace(password, '<REDACTED>')
        captured = re.sub(r'(?im)^(.*password\s*:\s*).+$', r'\1<REDACTED>', captured)
        captured = re.sub(r'\bsnxs?_[A-Za-z0-9_-]+', '<REDACTED>', captured)
        captured = captured.replace(str(ROOT), '<REPO>')
        with log.open('a') as handle:
            handle.write('$ ' + shlex.join(arguments).replace(str(ROOT), '<REPO>') + '\n' + captured + '\n')
        checked(completed is not None, 'command timed out; see sanitized run log')
        checked((completed.returncode != 0) if expect_failure else (completed.returncode == 0),
                f'unexpected command exit {completed.returncode}; see sanitized run log')
        return completed.stdout

    def request(path, body=None, headers=None, proxy=False):
        port = args.proxy_port if proxy else args.admin_port
        req = urllib.request.Request(f'http://127.0.0.1:{port}{path}',
                                     data=None if body is None else json.dumps(body).encode(),
                                     headers={'Content-Type': 'application/json', **(headers or {})})
        try:
            response = urllib.request.urlopen(req, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            raw = response.read().decode()
            with log.open('a') as handle:
                handle.write(f'HTTP {"POST" if body is not None else "GET"} {path}: {response.status}\n')
            data = raw if proxy else json.loads(raw)
            return response.status, data, response.headers

    def assert_proxy(host, expected=200):
        status, body, _ = request('/', headers={'Host': host}, proxy=True)
        checked(status == expected, f'{host}: expected HTTP {expected}, received {status}')
        if expected == 200:
            checked('Hostname: backend-' in body, 'proxy did not return a real demo backend')

    def login():
        status, body, headers = request('/api/v1/auth/login', {'username': 'admin', 'password': password})
        checked(status == 200 and body.get('username') == 'admin', 'original Owner password no longer works')
        return {'Cookie': headers['Set-Cookie'].split(';')[0], 'X-Senix-CSRF': '1'}

    def pass_check(name):
        result['checks'].append(name)
        print(f'PASS run {number}: {name}', flush=True)

    try:
        result['phase'] = 'preflight'
        config = json.loads(command([*COMPOSE, 'config', '--format', 'json']))
        result['images'] = {}
        for name in dict.fromkeys(service['image'] for service in config['services'].values()):
            info = json.loads(command(['docker', 'image', 'inspect', name, '--format',
                                      '{"Id":{{json .Id}},"RepoDigests":{{json .RepoDigests}},"Architecture":{{json .Architecture}}}']))
            result['images'][name] = {key: info.get(key) for key in ('Id', 'RepoDigests', 'Architecture')}
        if args.startup_failure:
            result['phase'] = 'startup-failure'
            with socket.socket() as blocked:
                blocked.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
                blocked.bind(('0.0.0.0', args.proxy_port))
                blocked.listen(1)
                command([str(ROOT / 'scripts/demo.sh')], timeout=75, expect_failure=True)
            checked(password is not None, 'failed startup did not preserve the generated Owner password')
            pass_check('bounded startup failure preserves the generated Owner password')
        result['phase'] = 'fresh-start'
        command([str(ROOT / 'scripts/demo.sh')])
        checked(password is not None and len(password) >= 12, 'demo did not print a generated Owner password')
        assert_proxy('demo.senix.local')
        owner = login()
        status, initial, _ = request('/api/v1/config', headers=owner)
        checked(status == 200 and initial['version'] == 1, 'fresh project did not start at Snapshot v1')
        pass_check(('retry after failed startup' if args.startup_failure else 'fresh start') + ', real proxy and Owner login')

        result['phase'] = 'running-repeat'
        command([str(ROOT / 'scripts/demo.sh')])
        login()
        assert_proxy('demo.senix.local')
        pass_check('repeat while running preserves the Owner and route')

        result['phase'] = 'saved-change'
        candidate = copy.deepcopy(initial['config'])
        next(route for route in candidate['routes'] if route['id'] == 'demo-route')['host'] = 'check.senix.local'
        status, plan, _ = request('/api/v1/changes/plan', candidate, owner)
        checked(status == 201 and plan['base_version'] == 1, 'could not plan the test hostname change')
        change_id = plan['change_id']
        status, approved, _ = request(f'/api/v1/changes/{change_id}/approve', {}, owner)
        checked(status == 200 and approved['status'] == 'APPROVED', 'test Owner could not approve the exact plan')
        status, applied, _ = request(f'/api/v1/changes/{change_id}/apply', {}, owner)
        checked(status == 200 and applied['version'] == 2, 'approved plan did not publish Snapshot v2')
        assert_proxy('check.senix.local')
        assert_proxy('demo.senix.local', 404)
        env['SENIX_DEMO_HOST'] = 'check.senix.local'
        command([str(ROOT / 'scripts/demo.sh')])
        pass_check('approved hostname change and repeat with the saved hostname')

        result['phase'] = 'stopped-restart'
        command([*COMPOSE, 'down'])
        command([str(ROOT / 'scripts/demo.sh')])
        owner = login()
        status, current, _ = request('/api/v1/config', headers=owner)
        checked(status == 200 and current['version'] == 2 and current['config'] == candidate,
                'restart changed the saved Snapshot or candidate')
        assert_proxy('check.senix.local')
        assert_proxy('demo.senix.local', 404)
        pass_check('restart preserves v2, the exact config and the original Owner')
        result['status'] = 'passed'
    except (Exception, KeyboardInterrupt) as error:
        result['status'] = 'failed'
        result['error'] = str(error) or type(error).__name__
        try:
            command([*COMPOSE, 'ps', '--all'], timeout=10)
            command([*COMPOSE, 'logs', '--no-color', '--tail', '30', 'senix'], timeout=10)
        except Exception as diagnostic_error:
            result['diagnostic_error'] = str(diagnostic_error)
    finally:
        try:
            command([*COMPOSE, 'down', '--volumes'], timeout=60)
            result['cleanup'] = 'passed'
        except Exception as error:
            result['cleanup'] = 'failed'
            result['cleanup_error'] = str(error)
            result['status'] = 'failed'
        result['elapsed_seconds'] = round(time.monotonic() - started, 3)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', help='cached source image; otherwise use the published Compose default')
    parser.add_argument('--runs', type=int, default=1, help='number of independent clean projects (sequential)')
    parser.add_argument('--startup-failure', action='store_true', help='occupy the proxy port first; require bounded failure and recovery with the generated password')
    parser.add_argument('--proxy-port', type=int, default=18080)
    parser.add_argument('--admin-port', type=int, default=19080)
    parser.add_argument('--output', type=Path, help='new output directory; defaults to target/demo-check-<random>')
    args = parser.parse_args()
    if args.runs < 1 or any(port < 1 or port > 65535 for port in (args.proxy_port, args.admin_port)):
        parser.error('runs must be positive and ports must be in 1..65535')
    if args.proxy_port == args.admin_port:
        parser.error('proxy and admin ports must differ')
    output = (args.output or ROOT / 'target' / ('demo-check-' + uuid.uuid4().hex[:12])).resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    report = {'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'harness_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              'options': {'image': args.image, 'runs': args.runs, 'proxy_port': args.proxy_port,
                          'admin_port': args.admin_port, 'password_source': 'demo-generated',
                          'startup_failure': args.startup_failure},
              'note': 'Cached Linux images; clean data per run. Uses test Owner REST APIs, not an AI client.',
              'runs': []}
    for number in range(1, args.runs + 1):
        result = run_case(args, output, number)
        report['runs'].append(result)
        (output / 'results.json').write_text(json.dumps(report, indent=2) + '\n')
        if result['status'] != 'passed':
            print(f"FAIL {result['phase']}: {result.get('error', result.get('cleanup_error'))}; {output}", file=sys.stderr)
            return 1
    print(f'PASS {args.runs} clean project(s); sanitized logs and results: {output}', flush=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
