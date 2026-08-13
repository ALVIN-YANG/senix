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
  ["certificate.read", "查看证书"],
  ["certificate.issue", "签发证书"],
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
    error.evidence = data?.evidence;
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
  if (["SERVING", "HEALTHY", "SUCCEEDED", "READY", "PASSED", "DRAINED"].includes(value)) return "good";
  if (["DRAINING", "UNKNOWN", "BLOCKED"].includes(value)) return "warn";
  return "bad";
}

function idempotencyKey() {
  return globalThis.crypto?.randomUUID?.() || `admin-${Date.now()}-${Math.random().toString(16).slice(2)}`;
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
      {[["overview", "流量状态", "状态", "1"], ["changes", "配置变更", "变更", "2"], ["diagnostics", "请求诊断", "诊断", "3"], ["credentials", "访问 Key", "Key", "4"], ["certificates", "TLS 证书", "证书", "5"], ["audit", "审计记录", "审计", "6"]].map(([id, label, short, key]) =>
        <button key={id} className={`nav-item${page === id ? " active" : ""}`} onClick={() => onPage(id)} type="button"><span data-short={short}>{label}</span><kbd>{key}</kbd></button>)}
    </nav>
    <div className="sidebar-foot">
      <span className="connection-dot" />
      <div><b>{owner}</b><small>管理会话</small></div>
      <button className="text-button" onClick={onLogout} type="button">退出</button>
    </div>
  </aside>;
}

function MaintenancePanel({ instance, onReload, notify, onClose }) {
  const [weight, setWeight] = useState(String(instance.weight));
  const [timeoutSeconds, setTimeoutSeconds] = useState("60");
  const [forceDrain, setForceDrain] = useState(false);
  const [generation, setGeneration] = useState(String(instance.generation + 1));
  const [rejoinWeight, setRejoinWeight] = useState(String(instance.weight || 100));
  const [forceRejoin, setForceRejoin] = useState(false);
  const [confirmDisable, setConfirmDisable] = useState(false);
  const [operation, setOperation] = useState(null);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");

  useEffect(() => {
    setWeight(String(instance.weight));
    setGeneration(String(instance.generation + 1));
    setRejoinWeight(String(instance.weight || 100));
    if (instance.traffic !== "DRAINING") setOperation(null);
  }, [instance.id, instance.generation, instance.weight, instance.traffic]);

  useEffect(() => {
    if (!operation || operation.status !== "DRAINING") return undefined;
    let cancelled = false;
    const timer = window.setInterval(async () => {
      try {
        const next = await api(`/api/v1/operations/${encodeURIComponent(operation.operation_id)}`);
        if (cancelled) return;
        setOperation(next);
        if (next.status !== "DRAINING") {
          window.clearInterval(timer);
          notify(next.status === "DRAINED" ? "实例已完成摘流" : "摘流等待超时，实例仍不会接收新请求");
          await onReload();
        }
      } catch (requestError) {
        if (!cancelled) setError(requestError.message);
      }
    }, 800);
    return () => { cancelled = true; window.clearInterval(timer); };
  }, [operation?.operation_id, operation?.status, notify, onReload]);

  function explain(requestError) {
    if (requestError.code === "LAST_AVAILABLE_BACKEND") {
      return `这是路由 ${requestError.evidence?.route_id || ""} 最后一个可用后端。确认业务可中断后，勾选 force 再摘流。`;
    }
    if (requestError.code === "INVALID_STATE" && requestError.message.includes("not healthy")) {
      return "健康检查尚未通过。人工确认实例可用后，可以勾选强制回接。";
    }
    return requestError.message;
  }

  async function run(name, path, method, body) {
    setBusy(name);
    setError("");
    try {
      const result = await api(path, {
        method,
        body,
        headers: { "Idempotency-Key": idempotencyKey() }
      });
      await onReload();
      return result;
    } catch (requestError) {
      setError(explain(requestError));
      return null;
    } finally {
      setBusy("");
    }
  }

  async function beginDrain(event) {
    event.preventDefault();
    const seconds = Number(timeoutSeconds);
    if (!Number.isFinite(seconds) || seconds < 1 || seconds > 86400) {
      setError("摘流等待时间必须在 1 秒到 24 小时之间。");
      return;
    }
    const result = await run(
      "drain",
      `/api/v1/instances/${encodeURIComponent(instance.id)}/drain`,
      "POST",
      { timeout_ms: Math.round(seconds * 1000), force: forceDrain }
    );
    if (result) {
      setOperation(result);
      notify(result.status === "DRAINED" ? "实例已摘流" : "已停止分配新请求，正在等待在途请求结束");
    }
  }

  async function changeWeight(event) {
    event.preventDefault();
    const nextWeight = Number(weight);
    if (!Number.isInteger(nextWeight) || nextWeight < 0 || nextWeight > 10000) {
      setError("权重必须是 0 到 10000 之间的整数。");
      return;
    }
    if (await run("weight", `/api/v1/instances/${encodeURIComponent(instance.id)}/weight`, "PATCH", { weight: nextWeight })) {
      notify("权重已更新");
    }
  }

  async function rejoin(event) {
    event.preventDefault();
    const nextGeneration = Number(generation);
    const nextWeight = Number(rejoinWeight);
    if (!Number.isSafeInteger(nextGeneration) || nextGeneration <= instance.generation) {
      setError(`新代次必须大于 ${instance.generation}。`);
      return;
    }
    if (!Number.isInteger(nextWeight) || nextWeight < 0 || nextWeight > 10000) {
      setError("回接权重必须是 0 到 10000 之间的整数。");
      return;
    }
    if (await run(
      "rejoin",
      `/api/v1/instances/${encodeURIComponent(instance.id)}/rejoin`,
      "POST",
      { generation: nextGeneration, weight: nextWeight, force: forceRejoin }
    )) notify("实例已按新代次回接");
  }

  async function disable() {
    if (await run("disable", `/api/v1/instances/${encodeURIComponent(instance.id)}/disable`, "POST")) {
      setConfirmDisable(false);
      notify("实例已禁用，只有显式回接才会恢复流量");
    }
  }

  const canRejoin = ["DRAINED", "DISABLED"].includes(instance.traffic);
  return <div className="maintenance-panel">
    <div className="maintenance-heading">
      <div><p className="eyebrow">Manual control</p><h3>实例维护 / {instance.id}</h3></div>
      <button className="panel-close" onClick={onClose} type="button" aria-label="关闭实例维护">关闭</button>
    </div>
    {operation && <div className={`drain-progress ${stateTone(operation.status)}`}>
      <div><span>摘流进度</span><strong>{operation.status}</strong></div>
      <div><span>普通在途</span><strong>{operation.ordinary_in_flight}</strong></div>
      <div><span>长连接</span><strong>{operation.long_lived_in_flight}</strong></div>
      <div><span>截止时间</span><strong>{formatTime(operation.deadline_at_ms)}</strong></div>
    </div>}
    {instance.traffic === "DRAINING" && !operation && <p className="maintenance-note">实例正在摘流。当前页面没有发起这次操作，因此不持有 operation_id；脚本仍可用原 operation_id 查询进度。</p>}
    <div className="maintenance-grid">
      {instance.traffic === "SERVING" && <form className="maintenance-unit" onSubmit={changeWeight}>
        <div><span className="unit-number">01</span><h4>调整权重</h4><p>只影响后续新请求，不改变现有连接。</p></div>
        <label>新权重<input type="number" min="0" max="10000" step="1" value={weight} onChange={(event) => setWeight(event.target.value)} /></label>
        <button className="quiet-button" disabled={Boolean(busy) || Number(weight) === instance.weight} type="submit">保存权重</button>
      </form>}
      {instance.traffic === "SERVING" && <form className="maintenance-unit drain-unit" onSubmit={beginDrain}>
        <div><span className="unit-number">02</span><h4>开始摘流</h4><p>立即停止新请求，等待当前请求自然结束。</p></div>
        <label>最长等待（秒）<input type="number" min="1" max="86400" step="1" value={timeoutSeconds} onChange={(event) => setTimeoutSeconds(event.target.value)} /></label>
        <label className="maintenance-check"><input type="checkbox" checked={forceDrain} onChange={(event) => setForceDrain(event.target.checked)} /><span><b>force</b> 允许最后一个后端进入维护</span></label>
        <button className="primary-button" disabled={Boolean(busy)} type="submit">{busy === "drain" ? "正在摘流…" : "开始摘流"}</button>
      </form>}
      {canRejoin && <form className="maintenance-unit rejoin-unit" onSubmit={rejoin}>
        <div><span className="unit-number">01</span><h4>以新代次回接</h4><p>不会因健康恢复自动接流，必须在这里或通过脚本显式回接。</p></div>
        <div className="maintenance-fields"><label>新代次<input type="number" min={instance.generation + 1} step="1" value={generation} onChange={(event) => setGeneration(event.target.value)} /></label><label>权重<input type="number" min="0" max="10000" step="1" value={rejoinWeight} onChange={(event) => setRejoinWeight(event.target.value)} /></label></div>
        <label className="maintenance-check"><input type="checkbox" checked={forceRejoin} onChange={(event) => setForceRejoin(event.target.checked)} /><span>人工确认健康，强制回接</span></label>
        <button className="primary-button" disabled={Boolean(busy)} type="submit">{busy === "rejoin" ? "正在回接…" : "以新代次回接"}</button>
      </form>}
      {instance.traffic !== "DISABLED" && <div className="maintenance-unit danger-unit">
        <div><span className="unit-number">{instance.traffic === "SERVING" ? "03" : "02"}</span><h4>禁用实例</h4><p>持续阻止新请求，健康恢复也不会自动启用。</p></div>
        {confirmDisable ? <div className="danger-confirm"><p>确定禁用 {instance.id}？</p><button className="quiet-button" onClick={() => setConfirmDisable(false)} type="button">取消</button><button className="danger-button" disabled={Boolean(busy)} onClick={disable} type="button">确认禁用</button></div>
          : <button className="danger-button" disabled={Boolean(busy)} onClick={() => setConfirmDisable(true)} type="button">禁用实例</button>}
      </div>}
    </div>
    <p className="maintenance-error" role="alert">{error}</p>
  </div>;
}

function Overview({ instances, onReload, notify }) {
  const [openInstance, setOpenInstance] = useState("");
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
      {instances.length ? instances.map((item) => <article className={`instance-bay${openInstance === item.id ? " open" : ""}`} key={item.id}>
        <div className="traffic-row">
          <div className="traffic-rail" aria-label={`流量 ${item.traffic}，健康 ${item.health}`}>
            <i className={stateTone(item.traffic)} /><i className={stateTone(item.health)} />
          </div>
          <div className="traffic-name"><strong>{item.id}</strong><code>generation {item.generation}{item.health_override ? " · health override" : ""}</code></div>
          <div className="metric metric-traffic"><span>流量</span><b className={`state-${stateTone(item.traffic)}`}>{item.traffic}</b></div>
          <div className="metric metric-health"><span>健康</span><b className={`state-${stateTone(item.health)}`}>{item.health}</b></div>
          <div className="metric metric-weight"><span>权重</span><b>{item.weight}</b></div>
          <button className={`control-button${openInstance === item.id ? " active" : ""}`} aria-expanded={openInstance === item.id} onClick={() => setOpenInstance(openInstance === item.id ? "" : item.id)} type="button">维护</button>
        </div>
        {openInstance === item.id && <MaintenancePanel instance={item} onReload={onReload} notify={notify} onClose={() => setOpenInstance("")} />}
      </article>) : <div className="empty-state">还没有实例。先通过配置快照加入后端实例。</div>}
    </div>
  </section>;
}

function Diagnostics({ current, notify }) {
  const [host, setHost] = useState("");
  const [path, setPath] = useState("/");
  const [report, setReport] = useState(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    const firstRoute = current?.config?.routes?.[0];
    if (!host && firstRoute) {
      setHost(firstRoute.host);
      setPath(firstRoute.path_prefix || "/");
    }
  }, [current?.version, host]);

  async function diagnose(event) {
    event.preventDefault();
    const targetHost = host.trim();
    const targetPath = path.trim();
    if (!targetHost || !targetPath.startsWith("/")) {
      setError("Host 不能为空，路径必须以 / 开头。");
      return;
    }
    setBusy(true);
    setError("");
    try {
      setReport(await api("/api/v1/diagnostics/requests", {
        method: "POST",
        body: { host: targetHost, path: targetPath }
      }));
    } catch (requestError) {
      setError(requestError.message);
    } finally {
      setBusy(false);
    }
  }

  async function copyReport() {
    try {
      await navigator.clipboard.writeText(JSON.stringify(report, null, 2));
      notify("原始诊断 JSON 已复制");
    } catch {
      notify("复制失败，请展开原始证据后手动复制");
    }
  }

  const outcomeCopy = {
    READY: "当前运行快照中有后端可以接收新请求。",
    ROUTE_NOT_FOUND: "当前配置没有匹配这个 Host 与路径。",
    NO_AVAILABLE_BACKEND: "路由已命中，但所有后端都被流量状态、健康状态或权重挡住。"
  };
  const stageLabels = {
    route_match: "路由匹配",
    backend_state: "后端状态",
    backend_selection: "后端选择"
  };

  return <section className="page diagnostics-page">
    <form className="diagnostic-console" onSubmit={diagnose}>
      <div className="diagnostic-intro"><p className="eyebrow">Evidence, not guesses</p><h2>模拟一个请求</h2><p>使用当前正在接流的 Snapshot 检查路由命中和后端资格，不会向业务后端发送请求。</p></div>
      <div className="diagnostic-inputs">
        <label>Host<input value={host} onChange={(event) => setHost(event.target.value)} placeholder="api.example.com" required /></label>
        <label>路径<input value={path} onChange={(event) => setPath(event.target.value)} placeholder="/api/users" required /></label>
        <button className="primary-button" disabled={busy} type="submit">{busy ? "正在检查…" : "请求诊断"}</button>
      </div>
      <p className="form-error" role="alert">{error}</p>
    </form>
    {report ? <div className="diagnostic-report" aria-live="polite">
      <div className={`diagnostic-outcome ${stateTone(report.outcome)}`}>
        <div><span>诊断结果</span><strong>{report.outcome}</strong></div>
        <p>{outcomeCopy[report.outcome]}</p>
        <code>{report.host}{report.path}</code>
      </div>
      <div className="evidence-list">
        {report.steps.map((step, index) => <article className={`evidence-step ${stateTone(step.status.toUpperCase())}`} key={`${step.stage}-${index}`}>
          <span className="evidence-index">{String(index + 1).padStart(2, "0")}</span>
          <i aria-hidden="true" />
          <div><b>{stageLabels[step.stage] || step.stage}</b><small>{step.status}</small><code>{step.detail}</code></div>
        </article>)}
      </div>
      <div className="raw-evidence"><details><summary>查看原始证据 JSON</summary><pre>{JSON.stringify(report, null, 2)}</pre></details><button className="quiet-button" onClick={copyReport} type="button">复制原始 JSON</button></div>
    </div> : <div className="diagnostic-empty"><span>01</span><p>输入 Host 和路径后，证据链会按实际决策顺序出现在这里。</p></div>}
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
    const needsGlobalScope = actions.some((action) => ["diagnostics.read", "change.plan", "change.read", "change.apply", "certificate.read", "certificate.issue", "audit.read"].includes(action));
    if (!actions.length || (!allResources && !instanceIds.length)) {
      setError("至少选择一个动作和一个实例范围。");
      return;
    }
    if (needsGlobalScope && !allResources) {
      setError("诊断、配置变更、证书和审计动作必须选择“所有当前和未来实例”。");
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

function certificateState(item) {
  if (!item.active) return { label: "历史版本", tone: "muted" };
  const remaining = item.not_after_ms - Date.now();
  if (remaining <= 0) return { label: "已过期", tone: "bad" };
  if (remaining <= 30 * 24 * 60 * 60 * 1000) return { label: "即将到期", tone: "warn" };
  return { label: "使用中", tone: "good" };
}

function CertificateRow({ item }) {
  const state = certificateState(item);
  const lifetime = Math.max(1, item.not_after_ms - item.not_before_ms);
  const elapsed = Math.min(100, Math.max(0, ((Date.now() - item.not_before_ms) / lifetime) * 100));
  return <article className={`certificate-row ${state.tone}`}>
    <div className="certificate-domains"><strong>{item.domains[0]}</strong>{item.domains.slice(1).map((domain) => <code key={domain}>{domain}</code>)}</div>
    <div className="certificate-horizon">
      <div className="horizon-track" style={{ "--elapsed": `${elapsed}%` }} role="progressbar" aria-label="证书有效期进度" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(elapsed)}><i /></div>
      <div><span>签发 {formatTime(item.not_before_ms)}</span><span>到期 {formatTime(item.not_after_ms)}</span></div>
    </div>
    <span className={`certificate-state ${state.tone}`}>{state.label}</span>
  </article>;
}

function Certificates({ certificates, enabled, onReload, notify }) {
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function issue(event) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const domains = String(form.get("domains") || "").split(/[\s,]+/).map((domain) => domain.trim()).filter(Boolean);
    if (!domains.length) {
      setError("至少填写一个域名。");
      return;
    }
    setBusy(true);
    setError("");
    try {
      const result = await api("/api/v1/certificates/issue", { method: "POST", body: { domains, timeout_seconds: 90 } });
      notify(`证书已签发并切换到 TLS generation ${result.tls_generation}`);
      setOpen(false);
      await onReload();
    } catch (requestError) {
      setError(requestError.code === "ACME_DISABLED" ? "服务端尚未配置 ACME 目录、联系人和条款确认。" : requestError.message);
    } finally {
      setBusy(false);
    }
  }

  if (!enabled) return <section className="page"><div className="certificate-disabled"><p className="eyebrow">Encrypted storage required</p><h2>证书存储尚未启用</h2><p>先在服务端配置 <code>--secret-key-file</code>。主密钥只放在外部文件，数据库只保存密文。</p></div></section>;
  return <section className="page certificates-page">
    <div className="section-heading split"><div><h2>TLS 证书</h2><p>查看当前与历史版本。签发由用户或脚本触发，Senix 不会自行安排续期。</p></div>{!open && <button className="primary-button" onClick={() => setOpen(true)} type="button">签发证书</button>}</div>
    {open && <form className="certificate-issue" onSubmit={issue}>
      <div><p className="eyebrow">HTTP-01</p><h3>签发并热切换</h3><p>域名必须先解析到这台网关的 HTTP 入口；每行一个域名，不支持通配符。</p></div>
      <label>域名<textarea name="domains" placeholder={"api.example.com\nwww.example.com"} required autoFocus spellCheck="false" /></label>
      <p className="form-error" role="alert">{error}</p>
      <div className="form-actions"><button className="quiet-button" onClick={() => { setOpen(false); setError(""); }} type="button">取消</button><button className="primary-button" disabled={busy} type="submit">{busy ? "正在验证并签发…" : "开始签发"}</button></div>
    </form>}
    <div className="certificate-list" aria-live="polite">{certificates.length ? certificates.map((item) => <CertificateRow item={item} key={item.certificate_id} />) : <div className="empty-state">还没有托管证书。配置 ACME 后可在这里手动签发。</div>}</div>
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
  const [certificates, setCertificates] = useState([]);
  const [certificateStoreEnabled, setCertificateStoreEnabled] = useState(true);
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
      const [nextInstances, nextCurrent, nextChanges, nextCredentials, nextCertificates, nextAudit] = await Promise.all([
        api("/api/v1/instances"), api("/api/v1/config"), api("/api/v1/changes"), api("/api/v1/credentials"),
        api("/api/v1/certificates").then((items) => ({ enabled: true, items })).catch((error) => {
          if (error.code === "CERTIFICATE_STORE_DISABLED") return { enabled: false, items: [] };
          throw error;
        }),
        api("/api/v1/audit-events")
      ]);
      setInstances(nextInstances);
      setCurrent(nextCurrent);
      setChanges(nextChanges);
      setCredentials(nextCredentials);
      setCertificates(nextCertificates.items);
      setCertificateStoreEnabled(nextCertificates.enabled);
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
      if (["1", "2", "3", "4", "5", "6"].includes(event.key)) setPage(["overview", "changes", "diagnostics", "credentials", "certificates", "audit"][Number(event.key) - 1]);
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
    diagnostics: ["Routing evidence", "请求诊断"],
    credentials: ["Access boundary", "访问 Key"],
    certificates: ["Certificate horizon", "TLS 证书"],
    audit: ["Recorded actions", "审计记录"]
  };

  return <div className="app-shell">
    <Sidebar owner={owner} page={page} onPage={setPage} onLogout={logout} />
    <main>
      <header className="topbar"><div><p className="eyebrow">{labels[page][0]}</p><h1>{labels[page][1]}</h1></div><div className="top-actions"><span className="last-updated">{updated ? `更新于 ${updated.toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })}` : "尚未刷新"}</span><button className="quiet-button" onClick={loadAll} type="button">刷新</button></div></header>
      {page === "overview" && <Overview instances={instances} onReload={loadAll} notify={notify} />}
      {page === "changes" && <Changes current={current} changes={changes} onReload={loadAll} notify={notify} />}
      {page === "diagnostics" && <Diagnostics current={current} notify={notify} />}
      {page === "credentials" && <Credentials credentials={credentials} instances={instances} onReload={loadAll} onSecret={setSecret} notify={notify} />}
      {page === "certificates" && <Certificates certificates={certificates} enabled={certificateStoreEnabled} onReload={loadAll} notify={notify} />}
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
