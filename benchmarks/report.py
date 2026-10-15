#!/usr/bin/env python3
"""Archive and verify benchmark results; optionally render a chart with matplotlib."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import shutil
import statistics

NAMES = {'direct-http': 'Direct HTTP', 'senix-http': 'Senix HTTP',
         'senix-https': 'Senix HTTPS / H2', 'senix-http-100-routes': 'Senix HTTP / 100 routes'}
STREAM_NAMES = {'direct-sse': 'Direct HTTP/1.1', 'senix-sse-http': 'Senix HTTP/1.1',
                'senix-sse-https-h2': 'Senix TLS/H2 (one connection)'}


def checksums(output):
    lines = [f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(output)}'
             for path in sorted(output.rglob('*')) if path.is_file() and path.name != 'SHA256SUMS']
    (output / 'SHA256SUMS').write_text('\n'.join(lines) + '\n')


def verify_streaming(source, rows, summary, metadata):
    from streaming import summarize
    assert metadata['status'] == 'passed' and rows, 'only completed, successful measurements can be reported'
    expected_rows = 3 * len(metadata['options']['vus'].split(',')) * metadata['options']['repeats']
    assert len(rows) == expected_rows, 'incomplete SSE measurement matrix'
    model = metadata['model']
    for row in rows:
        path = source / f"{row['case']}-vus{row['vus']}-run{row['repeat']}"
        samples = json.loads((path / 'samples.json').read_text())
        resources = json.loads((path / 'resources.json').read_text())
        raw_summary = json.loads((path / 'summary.json').read_text())
        assert all(row[key] == value for key, value in raw_summary.items()), 'cached row differs from load summary'
        count = row['streams_completed']
        assert row['streams_failed'] == 0 and count > 0 and row['case'] in STREAM_NAMES
        assert row['events_received'] == count * model['events_per_stream']
        assert row['response_body_bytes'] == count * model['body_bytes_per_stream']
        assert len(samples['first_event_us']) == len(samples['complete_stream_us']) == len(samples['delivery_batches']) == count
        assert len(samples['event_gap_us']) == count * (model['events_per_stream'] - 1)
        assert all(1 <= value <= model['events_per_stream'] for value in samples['delivery_batches'])
        for sample_name, metric_name, scale in [('first_event_us', 'first_event_ms', 1000),
                ('event_gap_us', 'event_gap_ms', 1000), ('complete_stream_us', 'complete_stream_ms', 1000),
                ('delivery_batches', 'delivery_batches', 1)]:
            values = sorted(samples[sample_name])
            assert values and all(type(value) is int and value >= 0 for value in values)
            metric = row[metric_name]
            assert metric['count'] == len(values)
            assert metric['min'] == values[0] / scale and metric['max'] == values[-1] / scale
            for p, key in ((.5, 'p50'), (.95, 'p95'), (.99, 'p99')):
                index = (len(values) - 1) * p
                lower, upper = math.floor(index), math.ceil(index)
                expected = (values[lower] + (values[upper] - values[lower]) * (index - lower)) / scale
                assert math.isclose(metric[key], expected, abs_tol=1e-6), f'{path.name}: invalid {metric_name}.{key}'
        stats = resources['backend_stats']
        assert stats['active'] == stats['aborted'] == 0 and stats['completed'] == count
        assert row['backend_peak_active'] == stats['peak_active'] <= row['vus']
        protocol = 'h2' if row['case'].endswith('-h2') else '1.1'
        assert row['protocols'] == {protocol: count}
        if protocol == 'h2':
            assert 0 < stats['peak_active'] <= row['http2_remote_max_concurrent_streams']
        after, before = resources['after']['gateway'], resources['before']['gateway']
        cpu = (after['usage_usec'] - before['usage_usec']) / resources['window_seconds'] / 10000
        assert math.isclose(row['gateway_cpu_percent_of_window'], cpu, abs_tol=1e-6)
        assert row['gateway_rss_mib_end_sample'] == after['VmRSS'] / 1024
        assert row['gateway_process_hwm_mib'] == after['VmHWM'] / 1024
    assert summary == summarize(rows), 'cached aggregate differs from measurement rows'
    return {'verified_measurements': len(rows), 'streams_completed': sum(row['streams_completed'] for row in rows),
            'events_received': sum(row['events_received'] for row in rows),
            'method': 'independently recomputed Node latency quantiles from raw integer samples; cross-checked counts, protocol and resources'}


def report_streaming(args, summary, output, metadata):
    model = metadata['model']
    table = ['| 场景 | VU | 首事件 p95 ms | 事件间隔 p95 ms | 后端活动流峰值 | 完整流数 | 网关结束 RSS MiB |',
             '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for row in summary:
        rss = '—' if row['case'] == 'direct-sse' else f"{row['gateway_rss_mib_end_sample_max']:.1f}"
        table.append(f"| {STREAM_NAMES[row['case']]} | {row['vus']} | {row['first_event_p95_ms_median']:.3f} | {row['event_gap_p95_ms_median']:.3f} | {row['backend_peak_active_max']} | {row['streams_completed']:,} | {rss} |")
    (output / 'table.md').write_text('\n'.join(table) + '\n')
    if args.chart:
        import matplotlib
        matplotlib.use('Agg')
        import matplotlib.pyplot as plt
        selected = [row for row in summary if row['vus'] == args.vus]
        if not selected:
            raise ValueError('requested VU count is absent from the measurement')
        fig, ax = plt.subplots(figsize=(7, 5.5), dpi=180)
        fig.patch.set_facecolor('#edf3f2')
        ax.set_facecolor('#edf3f2')
        maximum = max(row['first_event_p95_ms_max'] for row in selected)
        for i, row in enumerate(selected):
            y = len(selected) - 1 - i
            value = row['first_event_p95_ms_median']
            ax.barh(y, value, height=.3, color=['#80969b', '#2d9c83', '#173a45'][i])
            ax.errorbar(value, y, xerr=[[value - row['first_event_p95_ms_min']],
                        [row['first_event_p95_ms_max'] - value]], color='#173a45', capsize=5)
            ax.text(0, y + .3, STREAM_NAMES[row['case']], fontsize=14, color='#173a45')
            ax.text(maximum * 1.23, y + .3, f'{value:.2f} ms', ha='right', fontsize=15, color='#173a45')
        ax.set_xlim(0, maximum * 1.27)
        ax.set_ylim(-.5, len(selected) - .25)
        ax.set_yticks([])
        ax.set_xlabel('First complete SSE event · lower is better', fontsize=11, color='#526a70')
        ax.tick_params(axis='x', labelsize=10, colors='#526a70')
        for spine in ax.spines.values():
            spine.set_visible(False)
        worker = 'worker' if args.vus == 1 else 'workers'
        ax.set_title(f'{args.vus} client {worker} · first-event p95', loc='left', fontsize=18, color='#173a45', pad=22)
        repeats = min(row['repeats'] for row in selected)
        run = 'run' if repeats == 1 else 'runs'
        source = metadata['git_head'][:7] + ('+dirty' if metadata['dirty'] else '')
        fig.text(.07, .035, f'Median of {repeats} {run}; whiskers show min–max p95. Includes initial connection wait.\n'
                 f'{args.environment_label}. H2 shares one connection; queueing is included.\n'
                 f"{model['events_per_stream']} events, {model['data_bytes_per_event']}-byte data, "
                 f"{model['interval_ms']} ms pacing. This is not a capacity benchmark.\n"
                 f"Source {source} · measured {metadata['started_utc'][:10]}.", fontsize=9, color='#526a70')
        fig.subplots_adjust(left=.07, right=.96, top=.82, bottom=.26)
        args.chart.parent.mkdir(parents=True, exist_ok=True)
        fig.savefig(args.chart, facecolor=fig.get_facecolor())
        plt.close(fig)
    print('\n'.join(table))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--chart', type=Path)
    parser.add_argument('--vus', type=int, default=32)
    parser.add_argument('--environment-label', default='Local Docker VM',
                        help='chart caption describing the measured environment; does not change data')
    args = parser.parse_args()
    source = args.input.resolve()
    output = args.output.resolve()
    if output == source or source in output.parents:
        parser.error('output must be outside the input directory')
    if output.exists():
        parser.error('output must be a new directory')
    summary = json.loads((source / 'summary.json').read_text())
    rows = json.loads((source / 'runs.json').read_text())
    metadata = json.loads((source / 'metadata.json').read_text())
    streaming = metadata.get('workload') == 'paced-sse'
    verification = verify_streaming(source, rows, summary, metadata) if streaming else None
    if not streaming:
        assert all(row['failed_rate'] == 0 and row['checks_rate'] == 1 for row in rows)
    output.mkdir(parents=True)
    for path in source.iterdir():
        if path.is_dir():
            shutil.copytree(path, output / path.name)
        elif path.name != 'metadata.json':
            shutil.copy2(path, output / path.name)
    # Remove the local absolute output path. Measurement fields remain unchanged.
    metadata['options']['output'] = source.name
    metadata['publication_note'] = 'Absolute output path replaced with the local input directory name; measurement fields unchanged.'
    (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    if streaming:
        (output / 'verification.json').write_text(json.dumps(verification, indent=2) + '\n')
        report_streaming(args, summary, output, metadata)
        checksums(output)
        return
    for row in summary:
        cpu, rss, peak_rss = [], [], []
        for run in rows:
            if run['case'] != row['case'] or run['vus'] != row['vus']:
                continue
            path = source / f"{run['case']}-vus{run['vus']}-run{run['repeat']}" / 'resources.json'
            resources = json.loads(path.read_text())
            before, after = resources['before']['gateway'], resources['after']['gateway']
            cpu.append((after['usage_usec'] - before['usage_usec']) / resources['window_seconds'] / 10000)
            rss.append(after['VmRSS'] / 1024)
            peak_rss.append(after['VmHWM'] / 1024)
        row['gateway_cpu_percent_median_of_windows'] = statistics.median(cpu)
        row['gateway_rss_mib_end_sample_max'] = max(rss)
        row['gateway_process_peak_rss_mib'] = max(peak_rss)
    (output / 'resource-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
    table = ['| 场景 | VU | RPS 中位数（范围） | p50 ms | p95 ms | p99 ms | 失败率 |',
             '| --- | ---: | ---: | ---: | ---: | ---: | ---: |']
    for row in summary:
        table.append(f"| {NAMES[row['case']]} | {row['vus']} | {row['rps']:,.0f} ({row['rps_min']:,.0f}–{row['rps_max']:,.0f}) | {row['p50_ms']:.3f} | {row['p95_ms']:.3f} | {row['p99_ms']:.3f} | {row['failed_rate_max']:.0%} |")
    (output / 'table.md').write_text('\n'.join(table) + '\n')
    if args.chart:
        import matplotlib
        matplotlib.use('Agg')
        import matplotlib.pyplot as plt
        selected = [row for row in summary if row['vus'] == args.vus]
        if not selected:
            parser.error('requested VU count is absent from the measurement')
        fig, ax = plt.subplots(figsize=(6, 6.5), dpi=160)
        fig.patch.set_facecolor('#edf3f2')
        ax.set_facecolor('#edf3f2')
        maximum = max(row['rps_max'] for row in selected)
        colors = ['#80969b', '#2d9c83', '#173a45', '#65b6c8']
        for i, row in enumerate(selected):
            y = 3 - i
            ax.barh(y, row['rps'], height=0.30, color=colors[i])
            ax.errorbar(row['rps'], y, xerr=[[row['rps'] - row['rps_min']], [row['rps_max'] - row['rps']]],
                        color='#173a45', capsize=6, linewidth=1.5)
            ax.text(0, y + .32, NAMES[row['case']], fontsize=18, color='#173a45', va='center')
            ax.text(maximum * 1.12, y + .32, f"{row['rps']:,.0f}", fontsize=20, color='#173a45', ha='right', va='center')
        ax.set_xlim(0, maximum * 1.16)
        ax.set_ylim(-.45, 3.7)
        ax.set_yticks([])
        ax.set_xticks([])
        for spine in ax.spines.values():
            spine.set_visible(False)
        ax.set_title(f'{args.vus} VUs · 1 KiB · requests/s', loc='left', fontsize=20, color='#173a45', pad=20)
        repeats = min(row['repeats'] for row in selected)
        fig.text(.07, .06, f'Median of {repeats} runs; whiskers show min–max.\n{args.environment_label}; not a capacity guarantee.', fontsize=12, color='#526a70')
        fig.subplots_adjust(left=.07, right=.96, top=.88, bottom=.14)
        args.chart.parent.mkdir(parents=True, exist_ok=True)
        fig.savefig(args.chart, facecolor=fig.get_facecolor())
        plt.close(fig)
        if args.chart.suffix.lower() == '.svg':
            args.chart.write_text('\n'.join(line.rstrip() for line in args.chart.read_text().splitlines()) + '\n')
    checksums(output)
    print('\n'.join(table))


if __name__ == '__main__':
    main()
