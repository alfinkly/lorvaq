import { useState, type FormEvent } from "react";
import { ApiError, setup } from "../api";
import { t } from "../i18n";
import { passwordProblem } from "../settings/forms";

export function Setup({ onDone }: { onDone: () => void }) {
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    const problem = passwordProblem(password, confirm);
    if (problem) return setError(problem);
    setBusy(true);
    try {
      await setup(code, password);
      onDone();
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="card narrow" onSubmit={submit}>
      <h2>{t.setupTitle}</h2>
      <p className="hint">{t.setupHint}</p>
      <label>
        {t.setupCode}
        <input value={code} onChange={(e) => setCode(e.target.value)} autoComplete="off" placeholder="ABCD-EFGH-JKMN-PQRS" required />
      </label>
      <label>
        {t.password}
        <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" required />
      </label>
      <label>
        {t.confirmPassword}
        <input type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="new-password" required />
      </label>
      {error && <p className="error">{error}</p>}
      <button type="submit" disabled={busy}>{t.createAdmin}</button>
    </form>
  );
}
