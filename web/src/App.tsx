import { useCallback, useEffect, useState, type ReactNode } from "react";
import { fetchMe, fetchScenes, logout, type FollowStatus, type Me } from "./api";
import { t } from "./i18n";
import type { SceneInfo } from "./types";
import { CiView } from "./scenes/ci/CiView";
import { Login } from "./pages/Login";
import { Settings } from "./pages/Settings";
import { Setup } from "./pages/Setup";

type Status = "connecting" | "offline" | FollowStatus;
type Route = "scenes" | "settings";

const routeOf = (hash: string): Route => (hash === "#/settings" ? "settings" : "scenes");

export function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [scenes, setScenes] = useState<SceneInfo[]>([]);
  const [active, setActive] = useState<string | null>(null);
  const [status, setStatus] = useState<Status>("connecting");
  const [stale, setStale] = useState<Record<string, number>>({});
  const [now, setNow] = useState(Date.now());
  const [route, setRoute] = useState<Route>(routeOf(location.hash));

  const refresh = useCallback(() => {
    fetchMe()
      .then((m) => {
        setMe(m);
        if (m.viewer) {
          fetchScenes()
            .then((list) => {
              setScenes(list);
              setActive((current) => current ?? list[0]?.id ?? null);
            })
            .catch(() => setStatus("offline"));
        }
      })
      .catch(() => setStatus("offline"));
  }, []);

  useEffect(refresh, [refresh]);

  useEffect(() => {
    const onHash = () => setRoute(routeOf(location.hash));
    window.addEventListener("hashchange", onHash);
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => {
      window.removeEventListener("hashchange", onHash);
      clearInterval(id);
    };
  }, []);

  const onStatus = useCallback(
    (s: FollowStatus) => {
      setStatus(s);
      if (s === "locked") refresh();
    },
    [refresh],
  );

  const signOut = () => {
    void logout().finally(() => {
      location.hash = "";
      refresh();
    });
  };

  let main: ReactNode = null;
  if (!me) main = <p className="hint pad">{t[status]}</p>;
  else if (me.setup_needed) main = <Setup onDone={refresh} />;
  else if (route === "settings" && !me.admin) main = <Login onDone={refresh} />;
  else if (route === "settings") main = <Settings />;
  else if (!me.viewer) main = <Login onDone={refresh} />;
  else if (active === "ci") main = <CiView onStatus={onStatus} onStale={setStale} />;

  return (
    <div className="app">
      <header>
        <h1>lorvaq</h1>
        <nav className="tabs">
          {scenes.map((s) => (
            <button
              key={s.id}
              className={route === "scenes" && s.id === active ? "on" : ""}
              onClick={() => {
                setActive(s.id);
                location.hash = "";
              }}
            >
              {s.title}
            </button>
          ))}
        </nav>
        <span className="status">{t[status]}</span>
        {me && !me.demo && me.admin && (
          <>
            <a href="#/settings">{t.settings}</a>
            <button className="link" onClick={signOut}>{t.signOut}</button>
          </>
        )}
        {me && !me.demo && !me.admin && !me.setup_needed && <a href="#/settings">{t.signIn}</a>}
      </header>
      {route === "scenes" &&
        Object.entries(stale).map(([source, since]) => (
          <div key={source} className="banner">
            {t.stale(source, Math.max(0, Math.round((now - since) / 1000)))}
          </div>
        ))}
      {main}
    </div>
  );
}
