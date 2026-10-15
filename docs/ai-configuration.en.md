# Change gateway configuration with an AI client

[简体中文](./ai-configuration.md) | **English**

An AI client can read the complete configuration, propose a change and apply it after the human Owner approves the exact plan. The AI accesses Senix through MCP. Application deployment remains your responsibility.

> `get_current_config`, the planning version guard, configuration JSON Schema and Key presets are unreleased source features. The default Docker demo uses published v0.3.1. Build the current source to complete this workflow. The admin UI is currently in Chinese; exact labels are included below.

## 1. Start the current source

You need Git, Docker with Linux containers, Docker Compose v2, curl and access to build images and package registries. From a clone of this repository, run:

```bash
docker build -t senix:local .
SENIX_IMAGE=senix:local SENIX_DEMO_PULL=0 ./scripts/demo.sh
```

The first build downloads and compiles dependencies; allow time for it. The script prints the admin URL, Owner username, generated password and a real proxy response. The default ports `8080` and `9080` and Docker subnet `172.28.0.0/24` must be available. The admin port is bound to loopback; the proxy port accepts connections on all host interfaces.

If a port is occupied, prefix the demo command with `SENIX_DEMO_PROXY_PORT=18080 SENIX_DEMO_ADMIN_PORT=19080`. Update the browser, MCP and curl addresses below accordingly. Keep these overrides when restarting or stopping the demo.

Stop any older demo with the stop command printed by the script before starting the source image. Repeated starts keep the account and configuration; use the original password. Once the database contains a Snapshot, editing the example JSON does not replace the saved configuration.

## 2. Issue a configuration collaboration Key

Open `http://127.0.0.1:9080/admin/` and sign in with the Owner credentials printed by the demo. Go to **访问 Key** (access Keys), click **生成 Key** (generate Key), then choose a preset:

| UI label | Purpose | Granted actions |
| --- | --- | --- |
| 只读诊断 | Read state, diagnostics and current configuration | `instance.read`, `diagnostics.read`, `change.read` |
| 配置提案 | Read and create configuration or rollback plans | Read-only actions plus `change.plan` |
| 配置协作 | Also apply an exact plan already approved by the Owner | Proposal actions plus `change.apply` |

For this demo, choose **配置协作**, name it `local-config-agent` and select 24 hours. Configuration and diagnostic actions currently need global scope, so the preset selects **所有当前和未来实例** (all current and future instances). These presets do not provide instance isolation. Review the grants before issuance; the complete Key is shown only once. Editing actions or scope switches the form to custom permissions.

None of these presets can approve a plan, issue a certificate, drain or disable an instance, or run a server command. To separate proposal and execution, give one client **配置提案** and another a global Key with only `change.read` and `change.apply`.

## 3. Connect Cursor

Merge the `senix` entry into your global `~/.cursor/mcp.json`, preserving existing servers:

```json
{
  "mcpServers": {
    "senix": {
      "url": "http://127.0.0.1:9080/mcp",
      "headers": {
        "Authorization": "Bearer ${env:SENIX_API_KEY}"
      }
    }
  }
}
```

`SENIX_API_KEY` must be in the **Cursor process environment**. Read the Key privately in a terminal, export the variable, fully quit existing Cursor processes and launch Cursor from that same terminal. Exporting it in another terminal does not update an already running process. Do not put a real Key in this repository or a chat message.

Cursor supports HTTP headers and environment interpolation. Remote HTTP entries do not support `envFile`; see the [official Cursor MCP reference](https://prod.cursor.com/docs/mcp). UI locations may vary by client version. This configuration was checked against the documentation; a specific model running in Cursor has not been tested for this workflow.

Enable the server and confirm the tool catalog includes `get_current_config`, `plan_change`, `get_change` and `apply_approved_change`. Tools are filtered by the Key's grants. A client's confirmation of a tool call and the Owner's approval of a Senix Change Plan are separate checks.

## 4. Propose one change and preserve the rest

Ask your AI client:

```text
Use Senix MCP get_current_config to read the current configuration.
Change only demo-route's host from demo.senix.local to preview.senix.local.
Preserve every other route, backend and field. Call plan_change with
expected_version set to this read's version and candidate set to the complete config.
Show me change_id, base_version, candidate_digest, the complete candidate
configuration and the diff. Wait for me to approve that exact plan in
the Senix admin console. Do not apply it yet.
```

`get_current_config` returns `{ "version": 1, "config": { ... } }`; use the actual returned version. Copy the **config object**, modify only the requested field and pass it as `plan_change`'s `candidate`, with the read's `version` in the required `expected_version` parameter. Do not pass the entire Snapshot as the candidate. `tools/list` exposes a JSON Schema for routes, backends, TLS and health checks, derived from the actual configuration types.

Call structure; replace the placeholder with the complete edited configuration:

```text
plan_change({"expected_version": 1, "candidate": {"routes": [...]}})
```

Creating a plan does not change live routing. The original hostname should still work:

```bash
curl -f -H 'Host: demo.senix.local' http://127.0.0.1:8080/
```

If someone publishes configuration between read and plan, the server returns `CONFIG_VERSION_MISMATCH`, with `expected_version` and `current_version` in `evidence`, and creates no plan. Read the complete configuration again, merge your requested change and plan with that newly read version. Do not change only the version number on an old candidate. The check and plan persistence share the configuration publication lock.

**MCP compatibility:** existing callers must add `expected_version`. Missing parameters, incorrect field types and unknown configuration fields return an MCP SDK tool error (`isError: true`) describing parameter deserialization, before domain planning. Structurally valid configurations with business validation errors still create plans with `issues` and cannot be approved. REST v1 keeps its existing complete-config request body and has no new version precondition. `plan_rollback` keeps its existing contract.

## 5. Approve as Owner, then apply through MCP

Open **配置变更** (configuration changes) in the console. Find the exact `change_id` and review its digest, complete candidate, diff and validation results. Approve it only after review. Approval covers that exact content and expires after 15 minutes.

Then ask your AI client:

```text
I approved that exact plan in the Senix console.
Use get_change to check the same change_id, digest, status and approval expiry.
Then call apply_approved_change. Read get_current_config afterward and
verify the new Snapshot version and the intended configuration change.
```

Check the new hostname with a real request:

```bash
curl -f -H 'Host: preview.senix.local' http://127.0.0.1:8080/
```

The default demo has one route. After this change, the old hostname should return 404:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' \
  -H 'Host: demo.senix.local' http://127.0.0.1:8080/
```

If your configuration has other routes, check that they still reach their original backends. To undo the change, ask the AI to call `plan_rollback` with the original `target_version`, review the resulting plan and follow the same Owner approval and apply path.

When restarting or rerunning the demo after this change, also set `SENIX_DEMO_HOST=preview.senix.local` so the startup probe uses the saved hostname. Keep your image and port overrides.

## 6. Handle a rejected operation

| Result | Next step |
| --- | --- |
| `AUTHENTICATION_REQUIRED` | Check the Key, client process environment, expiry and revocation state. |
| `FORBIDDEN` | Check actions and global scope. Do not replace the Key with unrestricted credentials. |
| `CHANGE_APPROVAL_REQUIRED` | Ask the Owner to review and approve this same plan in the console. |
| `CHANGE_APPROVAL_EXPIRED` | The Owner must review and approve again. An expired approval cannot be reused. |
| `CONFIG_VERSION_MISMATCH` | Read again, merge the change and plan with that read version; no plan was created |
| `STALE_PLAN` | Read the latest configuration, merge the requested change, create a new plan and obtain new approval. |

`get_current_config` requires global `change.read`. A read-only Key cannot plan or apply; an Instance-scoped Key cannot read the global configuration. Revocation takes effect on the next request. A client may display a cached tool list, but every call still checks current authorization.

## Verify the behavior

The real-process E2E suite reads, plans and applies through MCP, approves through a test Owner's management API, and sends requests to real backends. It also covers unapproved application, a stale baseline, permission isolation and revocation:

```bash
cargo +1.88 test --locked -p senixd --test gateway_e2e
```

These tests establish protocol and gateway behavior. They do not show that every model can produce a correct candidate. Review the plan and complete the workflow in your own client. See the [English onboarding validation](./validation/2026-10-15-english-onboarding.md) and the earlier [MCP/Owner validation record, Chinese](./validation/2026-10-15-ai-configuration.md).

[Back to the English README](../README.en.md)
