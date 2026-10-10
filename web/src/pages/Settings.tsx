import { useEffect, useState, type FormEvent, type ReactNode } from "react";
import {
  ApiError,
  changePassword,
  getAccess,
  getHistory,
  getSceneSettings,
  putAccess,
  putHistory,
  putSceneSettings,
  type Access,
  type CiSettings,
  type ExitRule,
  type HistoryInfo,
} from "../api";
import { t } from "../i18n";
import { accessProblem, exitsProblem, formatCapacities, parseCapacities, parseList, passwordProblem, retentionProblem } from "../settings/forms";

const message = (err: unknown) => (err instanceof ApiError ? err.message : String(err));

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

function AccessSection() {
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
      .catch((e) => setStatus(message(e)));
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
      setStatus(message(err));
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

function CiSection() {
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
      .catch((e) => setStatus(message(e)));
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
      setStatus(message(err));
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

function HistorySection() {
  const [info, setInfo] = useState<HistoryInfo | null>(null);
  const [days, setDays] = useState(14);
  const [status, setStatus] = useState<string | null>(null);

  useEffect(() => {
    getHistory()
      .then((h) => {
        setInfo(h);
        setDays(h.retention_days);
      })
      .catch((e) => setStatus(message(e)));
  }, []);

  async function save(e: FormEvent) {
    e.preventDefault();
    const problem = retentionProblem(days);
    if (problem) return setStatus(problem);
    try {
      await putHistory(days);
      setStatus(t.saved);
    } catch (err) {
      setStatus(message(err));
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

function PasswordSection() {
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
      setStatus(message(err));
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

export function Settings() {
  return (
    <div className="settings">
      <AccessSection />
      <CiSection />
      <HistorySection />
      <PasswordSection />
    </div>
  );
}
