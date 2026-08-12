import { StrictMode, useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";

const ACTIONS = [
  ["instance.read", "查看实例"],
  ["instance.drain", "摘除流量"],
  ["instance.rejoin", "回接实例"],
  ["instance.set_weight", "调整权重"],
  ["instance.disable", "禁用实例"],
  ["diagnostics.read", "运行诊断"],
  ["change.plan", "规划变更"],
  ["change.read", "查看变更"],
  ["change.apply", "应用已批准变更"],
  ["audit.read", "查看审计"]
];

async function api(path, options = {}) {
  const method = options.method || "GET";
  const headers = { ...(options.headers || {}) };
  if (options.body !== undefined) headers["Content-Type"] = "application/json";
  if (!["GET", "HEAD"].includes(method)) headers["X-Senix-CSRF"] = "1";
  const response = await fetch(path, {
    method,
    headers,
    credentials: "same-origin",
    body: options.body === undefined ? undefined : JSON.stringify(options.body)
  });
  const text = await response.text();
  const data = text ? JSON.parse(text) : null;
  if (!response.ok) {
    const error = new Error(data?.message || `请求失败（${response.status}）`);
    error.code = data?.code;
    error.status = response.status;
    throw error;
  }
  return data;
}

function formatTime(value) {
  if (!value) return "不过期";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit"
  }).format(new Date(value));
}

function stateTone(value) {
  if (["SERVING", "HEALTHY", "SUCCEEDED"].includes(value)) return "good";
  if (["DRAINING", "UNKNOWN"].includes(value)) return "warn";
  return "bad";
}

function BrandMark({ compact = false }) {
  return <div className={`brand-mark${compact ? " compact" : ""}`} aria-hidden="true"><i /><i /><i /></div>;
}

function Login({ onLogin }) {
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(event) {
    event.preventDefault();
    const formElement = event.currentTarget;
    setBusy(true);
    setError("");
    const form = new FormData(formElement);
    try {
      const session = await api("/api/v1/auth/login", {
        method: "POST",
        body: { username: form.get("username"), password: form.get("password") }
      });
      formElement.reset();
      onLogin(session.username);
    } catch (requestError) {
      setError(requestError.code === "OWNER_ACCOUNT_NOT_INITIALIZED"
        ? "尚未初始化所有者账号，请先在服务器本机运行引导命令。"
        : "用户名或密码不正确。");
    } finally {
      setBusy(false);
    }
  }

  return <div className="login-shell">
    <section className="login-panel" aria-labelledby="login-title">
      <BrandMark />
      <p className="eyebrow">Gateway control desk</p>
      <h1 id="login-title">Senix</h1>
      <p className="login-copy">用所有者账号进入管理面。部署脚本和 MCP 继续使用独立的受限 Key。</p>
      <form onSubmit={submit}>
        <label>用户名<input name="username" autoComplete="username" required maxLength="64" autoFocus /></label>
        <label>密码<input name="password" type="password" autoComplete="current-password" required /></label>
        <p className="form-error" role="alert">{error}</p>
        <button className="primary-button wide" type="submit" disabled={busy}>{busy ? "正在验证…" : "进入控制台"}</button>
      </form>
      <p className="setup-hint">尚未初始化账号时，请在网关服务器本机运行 <code>senixd owner bootstrap</code>。</p>
    </section>
    <aside className="login-rail" aria-label="Senix 管理边界">
      <div className="rail-line"><span className="rail-light live" /><b>业务流量</b><small>Pingora 数据面持续运行</small></div>
      <div className="rail-line"><span className="rail-light guarded" /><b>管理入口</b><small>Owner 会话与 Key 使用同一权限边界</small></div>
      <div className="rail-line"><span className="rail-light recorded" /><b>控制动作</b><small>关键操作留下审计记录</small></div>
    </aside>
  </div>;
}

function Sidebar({ owner, page, onPage, onLogout }) {
  return <aside className="sidebar">
    <div className="sidebar-brand"><BrandMark compact /><div><strong>Senix</strong><span>Control desk</span></div></div>
    <nav aria-label="控制台导航">
      {[["overview", "流量状态", "状态", "1"], ["changes", "配置变更", "变更", "2"], ["credentials", "访问 Key", "Key", "3"], ["audit", "审计记录", "审计", "4"]].map(([id, label, short, key]) =>
        <button key={id} className={`nav-item${page === id ? " active" : ""}`} onClick={() => onPage(id)} type="button"><span data-short={short}>{label}</span><kbd>{key}</kbd></button>)}
    </nav>
    <div className="sidebar-foot">
      <span className="connection-dot" />
      <div><b>{owner}</b><small>管理会话</small></div>
      <button className="text-button" onClick={onLogout} type="button">退出</button>
    </div>
  </aside>;
}

function Overview({ instances }) {
  const serving = instances.filter((item) => item.traffic === "SERVING").length;
  const attention = instances.filter((item) => item.traffic !== "SERVING" || item.health === "UNHEALTHY").length;
  return <section className="page">
    <div className="summary-strip">
      <div><span>实例</span><strong>{instances.length}</strong></div>
      <div><span>接流中</span><strong>{serving}</strong></div>
      <div><span>需处理</span><strong>{attention}</strong></div>
    </div>
    <div className="section-heading"><div><h2>实例轨道</h2><p>流量状态和健康状态分别显示，避免把健康恢复误当成自动回接。</p></div></div>
    <div className="traffic-board" aria-live="polite">
      {instances.length ? instances.map((item) => <article className="traffic-row" key={item.id}>
        <div className="traffic-rail" aria-label={`流量 ${item.traffic}，健康 ${item.health}`}>
          <i className={stateTone(item.traffic)} /><i className={stateTone(item.health)} />
        </div>
        <div className="traffic-name"><strong>{item.id}</strong><code>generation {item.generation}</code></div>
        <div className="metric"><span>流量</span><b className={`state-${stateTone(item.traffic)}`}>{item.traffic}</b></div>
        <div className="metric"><span>健康</span><b className={`state-${stateTone(item.health)}`}>{item.health}</b></div>
        <div className="metric"><span>权重</span><b>{item.weight}</b></div>
      </article>) : <div className="empty-state">还没有实例。先通过配置快照加入后端实例。</div>}
    </div>
  </section>;
}

function ChangeCard({ change, onAction, busy }) {
  const stages = ["PLANNED", "APPROVED", "APPLIED"];
  const reached = stages.indexOf(change.status);
  const diffCount = change.diff.added_routes.length + change.diff.changed_routes.length + change.diff.removed_routes.length;
  const approvalExpired = change.status === "APPROVED" && (!change.approval_expires_at_ms || change.approval_expires_at_ms <= Date.now());
  return <article className="change-card">
    <div className="approval-rail" aria-label={`变更状态 ${change.status}`}>
      {stages.map((stage, index) => <span className={index <= reached ? "reached" : ""} key={stage}><i />{stage}</span>)}
    </div>
    <div className="change-body">
      <div className="change-heading">
        <div><strong>{change.kind === "ROLLBACK" ? `回滚至 Snapshot ${change.rollback_target_version}` : `Change ${change.change_id.slice(0, 8)}`}</strong><code>{change.candidate_digest.slice(0, 16)}…</code></div>
        <div className="change-version">v{change.base_version} → {change.applied_version ? `v${change.applied_version}` : "待发布"}</div>
      </div>
      <p className="change-meta">{change.created_by.label} 创建于 {formatTime(change.created_at_ms)} · {diffCount} 个路由差异</p>
      {change.status === "APPROVED" && <p className={`approval-expiry${approvalExpired ? " expired" : ""}`}>{approvalExpired ? "批准已过期，需由 Owner 重新批准" : `批准有效至 ${formatTime(change.approval_expires_at_ms)}`}</p>}
      <div className="diff-pills">
        {change.diff.added_routes.map((id) => <span className="diff-added" key={`a-${id}`}>+ {id}</span>)}
        {change.diff.changed_routes.map((id) => <span className="diff-changed" key={`c-${id}`}>~ {id}</span>)}
        {change.diff.removed_routes.map((id) => <span className="diff-removed" key={`r-${id}`}>− {id}</span>)}
        {!diffCount && <span>配置内容未变化</span>}
      </div>
      {change.issues.length > 0 && <div className="issue-list" role="alert">{change.issues.map((issue) => <p key={`${issue.code}-${issue.message}`}><b>{issue.code}</b>{issue.message}</p>)}</div>}
      <details><summary>查看完整候选配置</summary><pre>{JSON.stringify(change.candidate, null, 2)}</pre></details>
      <div className="change-actions">
        {change.status === "PLANNED" && change.issues.length === 0 && <button className="primary-button" disabled={busy} onClick={() => onAction(change, "approve")} type="button">批准这份内容</button>}
        {change.status === "PLANNED" && change.issues.length > 0 && <span className="blocked-label">修正校验问题后重新规划</span>}
        {change.status === "APPROVED" && approvalExpired && <button className="primary-button" disabled={busy} onClick={() => onAction(change, "approve")} type="button">重新批准这份内容</button>}
        {change.status === "APPROVED" && !approvalExpired && <button className="primary-button" disabled={busy} onClick={() => onAction(change, "apply")} type="button">应用已批准计划</button>}
        {change.status === "APPLIED" && change.base_version > 0 && <button className="quiet-button" disabled={busy} onClick={() => onAction(change, "rollback")} type="button">以 v{change.base_version} 创建回滚计划</button>}
      </div>
    </div>
  </article>;
}

function Changes({ current, changes, onReload, notify }) {
  const [candidate, setCandidate] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState("");

  useEffect(() => {
    if (current) setCandidate(JSON.stringify(current.config, null, 2));
  }, [current?.version]);

  async function plan(event) {
    event.preventDefault();
    let parsed;
    try {
      parsed = JSON.parse(candidate);
    } catch {
      setError("JSON 格式不正确，先修正后再规划。");
      return;
    }
    setBusy("plan");
    setError("");
    try {
      const change = await api("/api/v1/changes/plan", { method: "POST", body: parsed });
      notify(change.issues.length ? `计划已保存，发现 ${change.issues.length} 个问题` : "变更计划已保存，等待批准");
      await onReload();
    } catch (requestError) {
      setError(requestError.message);
    } finally {
      setBusy("");
    }
  }

  async function action(change, name) {
    setBusy(`${change.change_id}-${name}`);
    try {
      if (name === "rollback") {
        await api(`/api/v1/snapshots/${change.base_version}/rollback-plan`, { method: "POST" });
        notify(`已生成回滚到 v${change.base_version} 的计划，仍需批准`);
      } else {
        await api(`/api/v1/changes/${change.change_id}/${name}`, { method: "POST" });
        notify(name === "approve" ? "计划已批准" : "已发布为新配置快照");
      }
      await onReload();
    } catch (requestError) {
      notify(requestError.message);
    } finally {
      setBusy("");
    }
  }

  return <section className="page changes-page">
    <div className="snapshot-banner"><div><span>当前 Snapshot</span><strong>v{current?.version ?? "—"}</strong></div><p>候选配置必须先生成不可修改的计划。批准只绑定计划中显示的摘要和完整内容。</p></div>
    <form className="config-editor" onSubmit={plan}>
      <div className="section-heading split"><div><h2>规划配置</h2><p>编辑完整配置只会生成计划，不会直接影响正在处理的流量。</p></div><button className="primary-button" disabled={Boolean(busy)} type="submit">{busy === "plan" ? "正在校验…" : "生成变更计划"}</button></div>
      <textarea aria-label="完整候选配置 JSON" spellCheck="false" value={candidate} onChange={(event) => setCandidate(event.target.value)} />
      <p className="form-error" role="alert">{error}</p>
    </form>
    <div className="section-heading"><div><h2>批准队列</h2><p>AI 和脚本可以规划；只有 Owner 可以批准；带 change.apply 的 Key 只能应用已经批准的精确计划。</p></div></div>
    <div className="change-list">{changes.length ? changes.map((change) => <ChangeCard change={change} busy={Boolean(busy)} onAction={action} key={change.change_id} />) : <div className="empty-state">还没有配置变更计划。</div>}</div>
  </section>;
}

function CredentialRow({ item, onRevoke }) {
  const revoked = Boolean(item.revoked_at_ms);
  const isBootstrap = item.kind === "OWNER";
  const scope = isBootstrap ? "本机引导" : item.policy.all_resources ? "所有资源" : item.policy.instance_ids.join("、");
  return <article className={`credential-row${revoked ? " revoked" : ""}`}>
    <div className="credential-main"><strong>{item.label}</strong><code>{item.credential_id}</code></div>
    <div className="credential-actions">
      {isBootstrap
        ? <span className="permission-pill">账号创建后失效</span>
        : item.policy.actions.map((action) => <span className="permission-pill" key={action}>{action}</span>)}
    </div>
    <div><div className="credential-scope">{scope}</div><div className="credential-expiry">{revoked ? `已失效 ${formatTime(item.revoked_at_ms)}` : `有效至 ${formatTime(item.expires_at_ms)}`}</div></div>
    {!isBootstrap && !revoked && <button className="danger-button" onClick={() => onRevoke(item)} type="button">吊销</button>}
  </article>;
}

function KeyForm({ instances, onCancel, onIssued }) {
  const [allResources, setAllResources] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(event) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const actions = form.getAll("actions");
    const instanceIds = allResources ? [] : form.getAll("instance_ids");
    const needsGlobalScope = actions.some((action) => ["diagnostics.read", "change.plan", "change.read", "change.apply", "audit.read"].includes(action));
    if (!actions.length || (!allResources && !instanceIds.length)) {
      setError("至少选择一个动作和一个实例范围。");
      return;
    }
    if (needsGlobalScope && !allResources) {
      setError("诊断、配置变更和审计动作必须选择“所有当前和未来实例”。");
      return;
    }
    setBusy(true);
    setError("");
    const expiryHours = Number(form.get("expires"));
    try {
      const issued = await api("/api/v1/credentials", {
        method: "POST",
        body: {
          label: form.get("label"),
          actions,
          instance_ids: instanceIds,
          all_resources: allResources,
          expires_at_ms: expiryHours ? Date.now() + expiryHours * 60 * 60 * 1000 : null
        }
      });
      onIssued(issued.api_key);
    } catch (requestError) {
      setError(requestError.message);
    } finally {
      setBusy(false);
    }
  }

  return <div className="key-workbench">
    <form onSubmit={submit}>
      <div className="form-grid">
        <label>名称<input name="label" placeholder="deploy-instance-a" required maxLength="80" autoFocus /></label>
        <label>有效期<select name="expires" defaultValue="168"><option value="24">24 小时</option><option value="168">7 天</option><option value="720">30 天</option><option value="0">不过期</option></select></label>
      </div>
      <fieldset><legend>允许动作</legend><div className="permission-grid">
        {ACTIONS.map(([action, label]) => <label className="permission-option" key={action}><input type="checkbox" name="actions" value={action} /><span><b>{label}</b><small>{action}</small></span></label>)}
      </div></fieldset>
      <fieldset><legend>实例范围</legend>
        <label className="scope-toggle"><input type="checkbox" checked={allResources} onChange={(event) => setAllResources(event.target.checked)} /><span>所有当前和未来实例</span></label>
        <div className="instance-picker">{instances.length ? instances.map((item) => <label className="instance-option" key={item.id}><input type="checkbox" name="instance_ids" value={item.id} disabled={allResources} /><span>{item.id}</span></label>) : <span className="empty-state">暂无实例</span>}</div>
      </fieldset>
      <p className="form-error" role="alert">{error}</p>
      <div className="form-actions"><button className="quiet-button" onClick={onCancel} type="button">取消</button><button className="primary-button" type="submit" disabled={busy}>{busy ? "正在生成…" : "生成 Key"}</button></div>
    </form>
  </div>;
}

function Credentials({ credentials, instances, onReload, onSecret, notify }) {
  const [open, setOpen] = useState(false);

  async function revoke(item) {
    if (!window.confirm(`立即吊销 “${item.label}”？使用这个 Key 的脚本和 AI 会马上失去访问权限。`)) return;
    try {
      await api(`/api/v1/credentials/${encodeURIComponent(item.credential_id)}`, { method: "DELETE" });
      notify("Key 已吊销");
      onReload();
    } catch (error) {
      notify(error.message);
    }
  }

  return <section className="page">
    <div className="section-heading split"><div><h2>访问 Key</h2><p>给脚本和 AI 发最小权限。完整 Key 只出现一次。</p></div>{!open && <button className="primary-button" onClick={() => setOpen(true)} type="button">生成 Key</button>}</div>
    {open && <KeyForm instances={instances} onCancel={() => setOpen(false)} onIssued={(secret) => { setOpen(false); onSecret(secret); onReload(); }} />}
    <div className="credential-list" aria-live="polite">{credentials.length ? credentials.map((item) => <CredentialRow item={item} onRevoke={revoke} key={item.credential_id} />) : <div className="empty-state">还没有 Credential。</div>}</div>
  </section>;
}

function Audit({ events }) {
  return <section className="page">
    <div className="section-heading"><div><h2>审计记录</h2><p>这里记录操作者、动作、资源和结果，不保存 Key 或请求密文。</p></div></div>
    <div className="audit-list" aria-live="polite">{events.length ? events.map((event) => <article className="audit-row" key={event.event_id}>
      <time className="audit-time">{formatTime(event.occurred_at_ms)}</time>
      <div className="audit-action"><strong>{event.action}</strong><small>{event.credential_label}</small></div>
      <div className="audit-resource">{event.resource_type}{event.resource_id ? ` / ${event.resource_id}` : ""}</div>
      <span className={`outcome ${event.outcome.toLowerCase()}`}>{event.outcome}</span>
    </article>) : <div className="empty-state">还没有审计记录。</div>}</div>
  </section>;
}

function SecretDialog({ secret, onClose }) {
  const dialog = useRef(null);
  const [copyState, setCopyState] = useState("");
  useEffect(() => { if (secret) dialog.current?.showModal(); }, [secret]);

  async function copy() {
    try {
      await navigator.clipboard.writeText(secret);
      setCopyState("已复制到剪贴板");
    } catch {
      setCopyState("复制失败，请手动选择并复制");
    }
  }

  function close() {
    dialog.current?.close();
    setCopyState("");
    onClose();
  }

  return <dialog ref={dialog}>
    <div className="dialog-signal" aria-hidden="true" /><p className="eyebrow">Only once</p><h2>现在保存这个 Key</h2>
    <p>关闭后无法找回。遗失时请生成新 Key，再吊销旧 Key。</p><code className="secret-value">{secret}</code>
    <p className="copy-state" aria-live="polite">{copyState}</p><div className="form-actions"><button className="quiet-button" onClick={copy} type="button">复制 Key</button><button className="primary-button" onClick={close} type="button">我已保存</button></div>
  </dialog>;
}

function ControlDesk({ owner, onExpired }) {
  const [page, setPage] = useState("overview");
  const [instances, setInstances] = useState([]);
  const [current, setCurrent] = useState(null);
  const [changes, setChanges] = useState([]);
  const [credentials, setCredentials] = useState([]);
  const [audit, setAudit] = useState([]);
  const [updated, setUpdated] = useState(null);
  const [secret, setSecret] = useState(null);
  const [toast, setToast] = useState("");

  const notify = useCallback((message) => {
    setToast(message);
    window.clearTimeout(notify.timer);
    notify.timer = window.setTimeout(() => setToast(""), 2600);
  }, []);

  const loadAll = useCallback(async () => {
    try {
      const [nextInstances, nextCurrent, nextChanges, nextCredentials, nextAudit] = await Promise.all([
        api("/api/v1/instances"), api("/api/v1/config"), api("/api/v1/changes"), api("/api/v1/credentials"), api("/api/v1/audit-events")
      ]);
      setInstances(nextInstances);
      setCurrent(nextCurrent);
      setChanges(nextChanges);
      setCredentials(nextCredentials);
      setAudit(nextAudit);
      setUpdated(new Date());
    } catch (error) {
      if (error.status === 401) onExpired();
      else notify(error.message);
    }
  }, [notify, onExpired]);

  useEffect(() => { loadAll(); }, [loadAll]);
  useEffect(() => {
    function keyboard(event) {
      if (["INPUT", "SELECT", "TEXTAREA"].includes(document.activeElement?.tagName)) return;
      if (["1", "2", "3", "4"].includes(event.key)) setPage(["overview", "changes", "credentials", "audit"][Number(event.key) - 1]);
    }
    document.addEventListener("keydown", keyboard);
    return () => document.removeEventListener("keydown", keyboard);
  }, []);

  async function logout() {
    try { await api("/api/v1/auth/session", { method: "DELETE" }); } catch { /* Clear UI either way. */ }
    onExpired();
  }

  const labels = {
    overview: ["Live traffic", "流量状态"],
    changes: ["Approval queue", "配置变更"],
    credentials: ["Access boundary", "访问 Key"],
    audit: ["Recorded actions", "审计记录"]
  };

  return <div className="app-shell">
    <Sidebar owner={owner} page={page} onPage={setPage} onLogout={logout} />
    <main>
      <header className="topbar"><div><p className="eyebrow">{labels[page][0]}</p><h1>{labels[page][1]}</h1></div><div className="top-actions"><span className="last-updated">{updated ? `更新于 ${updated.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}` : "尚未刷新"}</span><button className="quiet-button" onClick={loadAll} type="button">刷新</button></div></header>
      {page === "overview" && <Overview instances={instances} />}
      {page === "changes" && <Changes current={current} changes={changes} onReload={loadAll} notify={notify} />}
      {page === "credentials" && <Credentials credentials={credentials} instances={instances} onReload={loadAll} onSecret={setSecret} notify={notify} />}
      {page === "audit" && <Audit events={audit} />}
    </main>
    <SecretDialog secret={secret} onClose={() => setSecret(null)} />
    <div className={`toast${toast ? " visible" : ""}`} role="status" aria-live="polite">{toast}</div>
  </div>;
}

function App() {
  const [owner, setOwner] = useState(undefined);
  useEffect(() => {
    api("/api/v1/auth/session").then((session) => setOwner(session.username)).catch(() => setOwner(null));
  }, []);
  if (owner === undefined) return <div className="empty-state">正在连接 Senix…</div>;
  if (!owner) return <Login onLogin={setOwner} />;
  return <ControlDesk owner={owner} onExpired={() => setOwner(null)} />;
}

createRoot(document.getElementById("root")).render(<StrictMode><App /></StrictMode>);
