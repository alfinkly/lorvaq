-- Hosts that run `lorvaq agent`: each joins with its own token (only its hash is kept).
CREATE TABLE agents (
    name         TEXT PRIMARY KEY,
    token_hash   TEXT NOT NULL UNIQUE,
    created_ms   BIGINT NOT NULL,
    last_seen_ms BIGINT,
    host         TEXT,
    version      TEXT
);
