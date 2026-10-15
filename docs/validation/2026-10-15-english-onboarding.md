# English onboarding validation

Date: 2026-10-15. Source baseline: `36a9160`, with documentation changes only. Local macOS ARM64 host with a Linux ARM64 Docker VM. The checks used isolated Compose projects and ports 18080/19080 to preserve an existing service on 8080.

## Executed checks

| Path | Result |
| --- | --- |
| Published `ghcr.io/alvin-yang/senix:0.3.1` | Pulled the image; two independent clean projects completed Owner creation, a real proxy response, Owner login and repeated startup. |
| Current-source image | `docker build` completed with Rust 1.88; image ID `sha256:5e5bd91982d7f6c1b5b8a02378af7a4213ad0a8d1135ca9f65e58c36d97f7a56`, tagged locally as `senix:english-docs`. |
| Preserve saved state when switching images | Started the source image with the published demo's saved volume; the original Owner account and Snapshot v1 remained available. |
| MCP workflow | Issued a limited configuration Key, read v1, changed only `demo-route.host`, planned against v1, confirmed the old hostname still worked and observed `CHANGE_APPROVAL_REQUIRED` before approval. |
| Owner approval and apply | A test Owner approved the exact plan through the management API. MCP read the same plan and digest, applied it and read Snapshot v2. The complete resulting configuration matched the candidate. |
| Real routing | `preview.senix.local` returned 200 with a real demo backend response; `demo.senix.local` returned 404. |
| Restart with the saved hostname | Repeated the demo with `SENIX_DEMO_HOST=preview.senix.local`, stopped it without deleting data, restarted and confirmed the route and v2 remained. |
| Regression | 79 Rust workspace tests passed, including 26 real-process E2E tests; formatting and strict Clippy passed. The suite separately checks preservation of an unaffected route, stale plans, scoped reads and revocation. |
| Documentation checks | Local links and heading anchors resolved; fenced JSON and shell examples parsed. The English benchmark table and request totals matched archived data. Cursor configuration was byte-identical to the Chinese guide and checked against the [official reference](https://prod.cursor.com/docs/mcp). |
| Rendering | English README and AI guide inspected at desktop and 360px mobile widths. Neither page overflowed horizontally; all three reused README images loaded. |

One initial isolated demo invocation returned exit status 1. The temporary validation helper removed its diagnostic output during cleanup, so its cause is unconfirmed. The helper was corrected to retain sanitized failure output before the two later clean-project checks passed. No product fix is claimed for that first failure.

A [follow-up failure-recovery check](2026-10-15-demo-recovery.md) reproduced and fixed an unbounded local probe and delayed password output during failed startup. It does not establish that the earlier unexplained event had the same cause.

The HTTP checks exercised MCP and test Owner APIs; they did not operate a model inside Cursor or establish that a new user can complete the workflow unaided. The admin UI remains Chinese, with exact labels explained in the English guide. Other deployment, monitoring and performance references are marked as Chinese where applicable.

The tests created only local credentials and isolated demo resources. Their containers, networks, volumes and credential files were removed; pre-existing services were preserved. No Release, container image or model-client configuration was published or changed by these checks.

[English README](../../README.en.md) · [English AI guide](../ai-configuration.en.md)
