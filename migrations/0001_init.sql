-- lorvaq schema v1: settings, the single admin, sessions, scene settings, event history.

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value JSONB NOT NULL
);

CREATE TABLE admin (
    id            SMALLINT PRIMARY KEY CHECK (id = 1),
    password_hash TEXT NOT NULL,
    updated_ms    BIGINT NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    created_ms BIGINT NOT NULL,
    expires_ms BIGINT NOT NULL
);

CREATE TABLE scene_settings (
    scene      TEXT PRIMARY KEY,
    value      JSONB NOT NULL,
    updated_ms BIGINT NOT NULL
);

-- seq restarts with the server, so it is not part of the key.
CREATE TABLE events (
    id     BIGSERIAL PRIMARY KEY,
    scene  TEXT NOT NULL,
    seq    BIGINT NOT NULL,
    ts_ms  BIGINT NOT NULL,
    change JSONB NOT NULL
);
CREATE INDEX events_scene_ts ON events (scene, ts_ms);
