import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import {
  changePassword,
  connectGitlabToken,
  disconnectGitlab,
  getGitlab,
  getAccess,
  getHistory,
  getSceneSettings,
  putAccess,
  putHistory,
  putSceneSettings,
  startGitlabOauth,
  type Access,
  type CiSettings,
  type ExitRule,
  type GitlabStatus,
  type HistoryInfo,
} from "../api";
import { t } from "../i18n";
import { failure } from "../settings/errors";
import {
  accessProblem,
  exitsProblem,
  formatCapacities,
  gitlabUrlProblem,
  normalizeGitlabUrl,
  oauthOutcome,
  parseCapacities,
  parseList,
  passwordProblem,
  retentionProblem,
  tokenPageUrl,
} from "../settings/forms";

type Props = { onAuthLost: () => void };

function Section({ title, children, onSubmit, status }: { title: string; children: ReactNode; onSubmit: (e: FormEvent) => void; status: string | null }) {
  return (
    <form className="card" onSubmit={onSubmit}>
      <h2>{title}</h2>
      {children}
      <div className="row">
        <button type="submit">{t.save}</button>
        {status && <span className={status === t.saved ? "ok" : "error"}>{status}</span>}
      </div>
    </form>
  );
}

function AccessSection({ onAuthLost }: Props) {
  const [mode, setMode] = useState<Access["mode"]>("admin");
  const [header, setHeader] = useState("x-forwarded-user");
  const [trusted, setTrusted] = useState("");
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    getAccess()
      .then((a) => {
        setMode(a.mode);
        if (a.mode === "proxy") {
          setHeader(a.header);
          setTrusted(a.trusted.join("\n"));
        }
      })
      .catch((e) => setStatus(failure(e, onAuthLost)));
  }, []);

  async function save(e: FormEvent) {
    e.preventDefault();
    const access: Access = mode === "proxy" ? { mode, header: header.trim(), trusted: parseList(trusted) } : { mode };
    const problem = accessProblem(access);
    if (problem) return setStatus(problem);
    try {
      await putAccess(access);
      setStatus(t.saved);
    } catch (err) {
      setStatus(failure(err, onAuthLost));
    }
  }

  return (
    <Section title={t.accessTitle} onSubmit={save} status={status}>
      {([["open", t.accessOpen], ["proxy", t.accessProxy], ["admin", t.accessAdmin]] as const).map(([value, label]) => (
        <label key={value} className="choice">
          <input type="radio" name="mode" checked={mode === value} onChange={() => setMode(value)} />
          {label}
        </label>
      ))}
      {mode === "proxy" && (
        <>
          <label>
            {t.proxyHeader}
            <input value={header} onChange={(e) => setHeader(e.target.value)} />
          </label>
          <label>
            {t.proxyTrusted}
            <textarea rows={3} value={trusted} onChange={(e) => setTrusted(e.target.value)} placeholder="10.0.0.0/8" />
          </label>
        </>
      )}
    </Section>
  );
}

function CiSection({ onAuthLost }: Props) {
  const [settings, setSettings] = useState<CiSettings | null>(null);
  const [exits, setExits] = useState<ExitRule[]>([]);
  const [capacities, setCapacities] = useState("");
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    getSceneSettings<CiSettings>("ci")
      .then((s) => {
        setSettings(s);
        setExits(s.exits);
        setCapacities(formatCapacities(s.capacities));
      })
      .catch((e) => setStatus(failure(e, onAuthLost)));
  }, []);

  if (!settings) return status ? <p className="error">{status}</p> : null;

  const setExit = (i: number, patch: Partial<ExitRule>) => setExits(exits.map((x, j) => (j === i ? { ...x, ...patch } : x)));

  async function save(e: FormEvent) {
    e.preventDefault();
    if (!settings) return;
    const cleaned = exits.map((x) => ({ label: x.label.trim(), stage: x.stage?.trim() || null, name: x.name?.trim() || null }));
    const parsed = parseCapacities(capacities);
    const problem = exitsProblem(cleaned) ?? (typeof parsed === "string" ? parsed : null);
    if (problem || typeof parsed === "string") return setStatus(problem);
    try {
      const applied = await putSceneSettings<CiSettings>("ci", { ...settings, exits: cleaned, capacities: parsed });
      setSettings(applied);
      setStatus(t.saved);
    } catch (err) {
      setStatus(failure(err, onAuthLost));
    }
  }

  return (
    <Section title={t.ciTitle} onSubmit={save} status={status}>
      <p className="hint">{t.exits}</p>
      {exits.map((x, i) => (
        <div className="exit" key={i}>
          <input aria-label={t.exitLabel} placeholder={t.exitLabel} value={x.label} onChange={(e) => setExit(i, { label: e.target.value })} />
          <input aria-label={t.exitStage} placeholder={t.exitStage} value={x.stage ?? ""} onChange={(e) => setExit(i, { stage: e.target.value })} />
          <input aria-label={t.exitName} placeholder={t.exitName} value={x.name ?? ""} onChange={(e) => setExit(i, { name: e.target.value })} />
          <button type="button" onClick={() => setExits(exits.filter((_, j) => j !== i))}>{t.remove}</button>
        </div>
      ))}
      <button type="button" onClick={() => setExits([...exits, { label: "", stage: "", name: "" }])}>{t.addExit}</button>
      <label>
        {t.maxProjects}
        <input type="number" min={1} value={settings.max_projects} onChange={(e) => setSettings({ ...settings, max_projects: Number(e.target.value) })} />
      </label>
      <label>
        {t.defaultCapacity}
        <input type="number" min={1} value={settings.default_capacity} onChange={(e) => setSettings({ ...settings, default_capacity: Number(e.target.value) })} />
      </label>
      <label>
        {t.capacities}
        <textarea rows={3} value={capacities} onChange={(e) => setCapacities(e.target.value)} placeholder="shared=5" />
      </label>
    </Section>
  );
}

function HistorySection({ onAuthLost }: Props) {
  const [info, setInfo] = useState<HistoryInfo | null>(null);
  const [days, setDays] = useState(14);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    getHistory()
      .then((h) => {
        setInfo(h);
        setDays(h.retention_days);
      })
      .catch((e) => setStatus(failure(e, onAuthLost)));
  }, []);

  async function save(e: FormEvent) {
    e.preventDefault();
    const problem = retentionProblem(days);
    if (problem) return setStatus(problem);
    try {
      await putHistory(days);
      setStatus(t.saved);
    } catch (err) {
      setStatus(failure(err, onAuthLost));
    }
  }

  return (
    <Section title={t.historyTitle} onSubmit={save} status={status}>
      <label>
        {t.retention}
        <input type="number" min={1} max={365} value={days} onChange={(e) => setDays(Number(e.target.value))} />
      </label>
      {info?.scenes.map((s) => (
        <p key={s.id} className={s.last_error ? "error" : "hint"}>
          {t.historyQueue(s.id, s.queued, s.dropped)}
          {s.last_error ? ` — ${s.last_error}` : ""}
        </p>
      ))}
    </Section>
  );
}

function PasswordSection({ onAuthLost }: Props) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [status, setStatus] = useState<string | null>(null);

  async function save(e: FormEvent) {
    e.preventDefault();
    const problem = passwordProblem(next, confirm);
    if (problem) return setStatus(problem);
    try {
      await changePassword(current, next);
      setCurrent("");
      setNext("");
      setConfirm("");
      setStatus(t.saved);
    } catch (err) {
      setStatus(failure(err, onAuthLost));
    }
  }

  return (
    <Section title={t.passwordTitle} onSubmit={save} status={status}>
      <label>
        {t.currentPassword}
        <input type="password" value={current} onChange={(e) => setCurrent(e.target.value)} autoComplete="current-password" required />
      </label>
      <label>
        {t.newPassword}
        <input type="password" value={next} onChange={(e) => setNext(e.target.value)} autoComplete="new-password" required />
      </label>
      <label>
        {t.confirmPassword}
        <input type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="new-password" required />
      </label>
    </Section>
  );
}

function GitlabSection({ onAuthLost }: Props) {
  const [st, setSt] = useState<GitlabStatus | null>(null);
  const [url, setUrl] = useState("");
  const [mode, setMode] = useState<"oauth" | "token">("oauth");
  const [clientId, setClientId] = useState("");
  const [secret, setSecret] = useState("");
  const [token, setToken] = useState("");
  const [status, setStatus] = useState<string | null>(() => oauthOutcome(location.hash));

  const load = () =>
    getGitlab()
      .then((g) => {
        setSt(g);
        if (g.url) setUrl(g.url);
      })
      .catch((e) => setStatus(failure(e, onAuthLost)));

  useEffect(() => {
    void load();
    if (location.hash.includes("?")) history.replaceState(null, "", "#/settings");
    const id = setInterval(() => void load(), 5000);
    return () => clearInterval(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function connect(e: FormEvent) {
    e.preventDefault();
    const problem = gitlabUrlProblem(url);
    if (problem) return setStatus(problem);
    try {
      if (mode === "token") {
        await connectGitlabToken(normalizeGitlabUrl(url), token.trim());
        setToken("");
        setStatus(t.gitlabOutcome.connected);
        void load();
      } else {
        const { authorize_url } = await startGitlabOauth(normalizeGitlabUrl(url), clientId.trim(), secret.trim() || null);
        location.href = authorize_url;
      }
    } catch (err) {
      setStatus(failure(err, onAuthLost));
    }
  }

  async function disconnect() {
    try {
      await disconnectGitlab();
      void load();
    } catch (err) {
      setStatus(failure(err, onAuthLost));
    }
  }

  const seconds = st?.last_ok_ms ? Math.max(0, Math.round((Date.now() - st.last_ok_ms) / 1000)) : null;
  return (
    <form className="card" onSubmit={connect}>
      <h2>{t.gitlabTitle}</h2>
      {st?.connected && (
        <div className="row">
          <span className={st.state === "ok" ? "ok" : "error"}>
            {t.gitlabConnectedAs(st.user ?? "?", st.url ?? "?")} — {t.gitlabState[st.state]}
            {seconds !== null ? `, ${t.gitlabLastOk(seconds)}` : ""}
          </span>
          <button type="button" onClick={disconnect}>{t.gitlabDisconnect}</button>
        </div>
      )}
      {st?.last_error && <p className="error">{st.last_error}</p>}
      <label>
        {t.gitlabUrl}
        <input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://gitlab.example.com" />
      </label>
      <label className="choice">
        <input type="radio" name="gitlab-mode" checked={mode === "oauth"} onChange={() => setMode("oauth")} />
        {t.gitlabByOauth}
      </label>
      <label className="choice">
        <input type="radio" name="gitlab-mode" checked={mode === "token"} onChange={() => setMode("token")} />
        {t.gitlabByToken}
      </label>
      {mode === "oauth" ? (
        <>
          <p className="hint">{t.gitlabOauthHint}</p>
          <p className="hint">
            {t.gitlabRedirectUri}: {st?.redirect_uri ? <code>{st.redirect_uri}</code> : t.gitlabNoRedirect}
          </p>
          <label>
            {t.gitlabClientId}
            <input value={clientId} onChange={(e) => setClientId(e.target.value)} autoComplete="off" />
          </label>
          <label>
            {t.gitlabClientSecret}
            <input type="password" value={secret} onChange={(e) => setSecret(e.target.value)} autoComplete="off" />
          </label>
          <button type="submit">{t.gitlabAuthorize}</button>
        </>
      ) : (
        <>
          <p className="hint">{t.gitlabTokenHint}</p>
          {!gitlabUrlProblem(url) && (
            <a href={tokenPageUrl(url)} target="_blank" rel="noopener noreferrer">
              {t.gitlabCreateToken}
            </a>
          )}
          <label>
            {t.gitlabToken}
            <input type="password" value={token} onChange={(e) => setToken(e.target.value)} autoComplete="off" />
          </label>
          <button type="submit">{t.gitlabConnect}</button>
        </>
      )}
      {status && <p className={status === t.gitlabOutcome.connected ? "ok" : "error"}>{status}</p>}
    </form>
  );
}

export function Settings({ onAuthLost }: Props) {
  return (
    <div className="settings">
      <GitlabSection onAuthLost={onAuthLost} />
      <AccessSection onAuthLost={onAuthLost} />
      <CiSection onAuthLost={onAuthLost} />
      <HistorySection onAuthLost={onAuthLost} />
      <PasswordSection onAuthLost={onAuthLost} />
    </div>
  );
}
