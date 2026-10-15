# Docker demo failure recovery

Date: 2026-10-15. Before-fix source: `336a785`. Local macOS ARM64 with an OrbStack Linux ARM64 Docker VM. Images were already cached; the checks did not measure registry download reliability.

## Reproduced failure

A test TCP listener occupied the demo proxy port and did not send HTTP responses. The original `scripts/demo.sh` waited in an unbounded curl call: a direct probe did not finish within three seconds, and a full startup attempt exceeded 120 seconds. Its retry counter could not advance while that request was blocked.

Owner creation ran before startup and the proxy check, but the generated password was printed only after those checks passed. A failed or stuck first start could therefore leave an initialized account whose generated password had never been shown.

The permanent regression check was run before changing the demo script. Its 75-second startup limit failed with `command timed out`, and its diagnostic log showed the gateway container healthy while the local proxy probe was stuck. Test resources were cleaned up.

## Change and acceptance

The demo now limits each health/proxy request to two seconds, with a one-second connection timeout. It prints the generated password immediately after successful Owner creation, before starting the services. It retains the account and database on retries.

`scripts/check-demo.py --startup-failure` reserves the test proxy port, requires startup to fail within 75 seconds with the generated password available, releases the port and then checks:

1. Recovery using the same Owner password and a real backend response.
2. Repeated startup without changing the account or route.
3. A test Owner's REST plan, approval and apply of a hostname change.
4. New hostname 200, old hostname 404, and exact Snapshot v2 configuration after stopping and restarting.
5. Cleanup of only that randomly named project's containers, network and volume.

| Final check | Result |
| --- | --- |
| Published Compose image `ghcr.io/alvin-yang/senix:0.3.1`, occupied-port recovery | One run passed, including cleanup. |
| Current-source image `senix:english-docs`, occupied-port recovery | Two consecutive runs passed on the same test ports, including cleanup. The image identity is recorded in the raw JSON and its build was checked in the [onboarding record](2026-10-15-english-onboarding.md). |
| Normal clean start with a demo-generated password | One run passed, including repeat, configuration change, restart and cleanup. |
| Missing cached image, with `python3 -O` | Exited 1 at preflight, retained failure evidence and completed cleanup. This checks the verifier's failure path; it is not the startup bug reproduction. |
| Rust regression | 79 tests passed, including 26 real-process E2E tests; strict Clippy and formatting passed. Shell, Python and Compose checks passed. |

The failure fixture now allows normal TCP address reuse so consecutive runs can reserve a recently released port. The earlier and final harness copies are in the archive and match the SHA-256 fields in their reports; the failure and recovery requirements are unchanged.

## Evidence and reproduction

[Pre-fix failure](demo-recovery/2026-10-15/before.json), [published-image recovery](demo-recovery/2026-10-15/published.json), [source-image recovery](demo-recovery/2026-10-15/source.json), [normal start](demo-recovery/2026-10-15/healthy.json), [negative check](demo-recovery/2026-10-15/negative.json), [sanitized logs and source fixtures](demo-recovery/2026-10-15/logs-and-fixtures.zip), [SHA-256 manifest](demo-recovery/2026-10-15/SHA256SUMS).

The harness records exact image identities and its own source hash. Passwords and credential values are redacted, HTTP logs contain only method/path/status, and local repository paths are replaced with `<REPO>`. The archive's script copies are audit snapshots; use the repository's `scripts/check-demo.py` to run a check. `elapsed_seconds` covers the whole scenario and cleanup, not just startup latency.

From the repository root, with Python 3, Docker Compose v2, curl, free test ports and an available `172.28.0.0/24` subnet:

```bash
docker compose --file examples/demo/compose.yml pull
python3 scripts/check-demo.py --startup-failure --runs 2
```

To check a locally built source image, add `--image senix:local`; use `--proxy-port` and `--admin-port` to change the test ports. Each run uses empty data and generates its own Owner password. Logs and results remain in a new `target/demo-check-*` directory on both success and failure. CI also runs this recovery check against the published Compose image and preserves sanitized evidence after a failed check.

The earlier unexplained first-start failure remains unconfirmed: its original diagnostic output was lost. This newly reproduced failure and its fix do not establish that the earlier event had the same cause. The checks use test Owner REST APIs, not a model client, and do not establish long-term reliability or production acceptance. Existing services were preserved; test resources were removed.
