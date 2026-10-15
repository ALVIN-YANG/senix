#!/usr/bin/env python3
"""Archive raw benchmark evidence and render a fixed-concurrency chart with matplotlib."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import statistics

NAMES = {'direct-http': 'Direct HTTP', 'senix-http': 'Senix HTTP',
         'senix-https': 'Senix HTTPS / H2', 'senix-http-100-routes': 'Senix HTTP / 100 routes'}


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
    if output.exists():
        parser.error('output must be a new directory')
    summary = json.loads((source / 'summary.json').read_text())
    rows = json.loads((source / 'runs.json').read_text())
    assert all(row['failed_rate'] == 0 and row['checks_rate'] == 1 for row in rows)
    output.mkdir(parents=True)
    for path in source.iterdir():
        if path.is_dir():
            shutil.copytree(path, output / path.name)
        elif path.name != 'metadata.json':
            shutil.copy2(path, output / path.name)
    metadata = json.loads((source / 'metadata.json').read_text())
    # Remove the local absolute output path. Measurement fields remain unchanged.
    metadata['options']['output'] = source.name
    metadata['publication_note'] = 'Only the local absolute output path was replaced with its directory name.'
    (output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
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
    checksums = []
    for path in sorted(output.rglob('*')):
        if path.is_file():
            checksums.append(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.relative_to(output)}')
    (output / 'SHA256SUMS').write_text('\n'.join(checksums) + '\n')
    print('\n'.join(table))


if __name__ == '__main__':
    main()
