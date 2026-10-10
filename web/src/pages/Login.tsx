import { useState, type FormEvent } from "react";
import { ApiError, AuthError, login } from "../api";
import { t } from "../i18n";

export function Login({ onDone }: { onDone: () => void }) {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      await login(password);
      onDone();
    } catch (err) {
      setError(err instanceof AuthError ? t.wrongPassword : err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="card narrow" onSubmit={submit}>
      <h2>{t.signInTitle}</h2>
      <label>
        {t.password}
        <input type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" autoFocus required />
      </label>
      {error && <p className="error">{error}</p>}
      <button type="submit" disabled={busy}>{t.signIn}</button>
    </form>
  );
}
