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
      {[["overview", "流量状态", "状态", "1"], ["credentials", "访问 Key", "Key", "2"], ["audit", "审计记录", "审计", "3"]].map(([id, label, short, key]) =>
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
    if (!actions.length || (!allResources && !instanceIds.length)) {
      setError("至少选择一个动作和一个实例范围。");
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
      const [nextInstances, nextCredentials, nextAudit] = await Promise.all([
        api("/api/v1/instances"), api("/api/v1/credentials"), api("/api/v1/audit-events")
      ]);
      setInstances(nextInstances);
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
      if (["INPUT", "SELECT"].includes(document.activeElement?.tagName)) return;
      if (["1", "2", "3"].includes(event.key)) setPage(["overview", "credentials", "audit"][Number(event.key) - 1]);
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
    credentials: ["Access boundary", "访问 Key"],
    audit: ["Recorded actions", "审计记录"]
  };

  return <div className="app-shell">
    <Sidebar owner={owner} page={page} onPage={setPage} onLogout={logout} />
    <main>
      <header className="topbar"><div><p className="eyebrow">{labels[page][0]}</p><h1>{labels[page][1]}</h1></div><div className="top-actions"><span className="last-updated">{updated ? `更新于 ${updated.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}` : "尚未刷新"}</span><button className="quiet-button" onClick={loadAll} type="button">刷新</button></div></header>
      {page === "overview" && <Overview instances={instances} />}
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
