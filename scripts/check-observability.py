#!/usr/bin/env python3
"""Read-only acceptance check for the local observability example."""
import argparse
import json
from pathlib import Path
import time
import urllib.error
import urllib.parse
import urllib.request


def get(url, headers=None):
    with urllib.request.urlopen(urllib.request.Request(url, headers=headers or {}), timeout=10) as response:
        return response.read().decode()


def query(base, expression):
    data = json.loads(get(base + '/api/v1/query?' + urllib.parse.urlencode({'query': expression})))
    assert data['status'] == 'success', data
    return data['data']['result']


def eventually(action, description, timeout=90):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            result = action()
            if result:
                print('PASS ' + description, flush=True)
                return result
        except (urllib.error.URLError, ConnectionError, AssertionError, KeyError, ValueError) as error:
            last_error = error
        time.sleep(2)
    raise AssertionError(f'{description}: timed out; {last_error}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--key-file', type=Path, required=True)
    parser.add_argument('--admin', default='http://127.0.0.1:9080')
    parser.add_argument('--proxy', default='http://127.0.0.1:8080')
    parser.add_argument('--host', default='demo.senix.local')
    parser.add_argument('--prometheus', default='http://127.0.0.1:19090')
    parser.add_argument('--grafana', default='http://127.0.0.1:13000')
    parser.add_argument('--collectors', action='store_true')
    args = parser.parse_args()
    headers = {'Authorization': 'Bearer ' + args.key_file.read_text().strip()}
    try:
        get(args.admin + '/metrics')
    except urllib.error.HTTPError as error:
        assert error.code == 401, error.code
    else:
        raise AssertionError('anonymous metrics read succeeded')
    try:
        get(args.admin + '/api/v1/config', headers)
    except urllib.error.HTTPError as error:
        assert error.code == 403, error.code
    else:
        raise AssertionError('metrics key could read global config')
    print('PASS metrics authorization and least privilege', flush=True)
    for _ in range(20):
        get(args.proxy + '/', {'Host': args.host})
        time.sleep(0.1)
    eventually(lambda: query(args.prometheus, 'up{job="senix"} == 1'), 'Prometheus scrape is UP')
    eventually(lambda: query(args.prometheus, 'senix_proxy_request_duration_seconds_count{kind="ordinary"} > 0'), 'Prometheus histogram has real requests')

    def measured_p95():
        # A short burst can finish before the first scrape. Keep the counter moving
        # across scrape intervals so rate() has a measured increase to evaluate.
        get(args.proxy + '/', {'Host': args.host})
        return query(args.prometheus, 'histogram_quantile(0.95, sum by (le) (rate(senix_proxy_request_duration_seconds_bucket{kind="ordinary"}[1m]))) > 0')

    eventually(measured_p95, 'PromQL p95 returns a positive measured value')
    dashboard = eventually(lambda: json.loads(get(args.grafana + '/api/dashboards/uid/senix-overview')), 'Grafana provisioned dashboard')
    assert len(dashboard['dashboard']['panels']) == 8
    eventually(lambda: query(args.grafana + '/api/datasources/proxy/uid/senix-prometheus', 'senix_config_snapshot_version'), 'Grafana datasource queries real Senix series')
    rules = json.loads(get(args.prometheus + '/api/v1/rules'))['data']['groups']
    assert len(rules) == 1 and len(rules[0]['rules']) == 3
    print('PASS three alert rules loaded', flush=True)
    if args.collectors:
        eventually(lambda: 'senix_proxy_request_duration_seconds' in get('http://127.0.0.1:18889/metrics'), 'OTel Collector scrapes and exports Senix histogram')
        eventually(lambda: query('http://127.0.0.1:18428', 'up{job="senix"} == 1'), 'VictoriaMetrics direct scrape')
        eventually(lambda: query('http://127.0.0.1:18428', 'up{job="senix_alloy"} == 1'), 'Alloy scrape and remote_write to VictoriaMetrics')
        params = urllib.parse.urlencode({'query': '{job="senix"} |= "proxy_request_completed"', 'limit': 10})
        eventually(lambda: json.loads(get('http://127.0.0.1:13100/loki/api/v1/query_range?' + params))['data']['result'], 'Alloy file collection and Loki query of actual access events')


if __name__ == '__main__':
    main()
