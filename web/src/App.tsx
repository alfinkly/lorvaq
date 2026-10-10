import { useEffect, useState } from "react";
import { fetchScenes } from "./api";
import { t } from "./i18n";
import type { SceneInfo } from "./types";
import { CiView } from "./scenes/ci/CiView";

type Status = "connecting" | "live" | "reconnecting" | "offline";

export function App() {
  const [scenes, setScenes] = useState<SceneInfo[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>("connecting");
  const [stale, setStale] = useState<Record<string, number>>({});
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    fetchScenes()
      .then((list) => {
        setScenes(list);
        setActive(list[0]?.id ?? null);
      })
      .catch(() => setStatus("offline"));
  }, []);

  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="app">
      <header>
        <h1>lorvaq</h1>
        <nav className="tabs">
          {scenes.map((s) => (
            <button key={s.id} className={s.id === active ? "on" : ""} onClick={() => setActive(s.id)}>
              {s.title}
            </button>
          ))}
        </nav>
        <span className="status">{t[status]}</span>
      </header>
      {Object.entries(stale).map(([source, since]) => (
        <div key={source} className="banner">
          {t.stale(source, Math.max(0, Math.round((now - since) / 1000)))}
        </div>
      ))}
      {active === "ci" && <CiView onStatus={setStatus} onStale={setStale} />}
    </div>
  );
}
