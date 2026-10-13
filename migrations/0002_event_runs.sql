-- A run is one server start: seq restarts with it. (scene, run, seq) is unique, so a
-- batch written twice (a timed-out write that still committed) keeps one copy.
ALTER TABLE events ADD COLUMN run BIGINT;
-- Rows from before runs existed: each its own run, so the unique index can be built.
UPDATE events SET run = -id;
ALTER TABLE events ALTER COLUMN run SET NOT NULL;
CREATE UNIQUE INDEX events_scene_run_seq ON events (scene, run, seq);
-- Pruning by age across all scenes.
CREATE INDEX events_ts ON events (ts_ms);
