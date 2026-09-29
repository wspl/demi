
CREATE TABLE nodes (
  id               TEXT PRIMARY KEY,
  number           INTEGER NOT NULL UNIQUE CHECK (number >= 0),
  parent_id        TEXT REFERENCES nodes (id) ON DELETE CASCADE,
  description      TEXT NOT NULL,
  profile          TEXT,
  round            INTEGER NOT NULL CHECK (round >= 1),
  started_at       INTEGER NOT NULL,
  can_spawn        INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  closed_phase     TEXT CHECK (closed_phase IN ('completed', 'aborted', 'error')),
  closed_at        INTEGER,
  result           TEXT,
  failure          TEXT,
  delivered        INTEGER NOT NULL CHECK (delivered IN (0, 1)),
  state            TEXT NOT NULL,
  block_count      INTEGER NOT NULL CHECK (block_count >= 0),
  command_revision INTEGER NOT NULL CHECK (command_revision >= 0),
  output_revision  INTEGER NOT NULL CHECK (output_revision >= 0),
  CHECK ((closed_phase IS NULL) = (closed_at IS NULL)),
  CHECK (result IS NULL OR closed_phase = 'completed'),
  CHECK (failure IS NULL OR closed_phase = 'error')
) STRICT;
-- One root per tree.
CREATE UNIQUE INDEX nodes_root ON nodes ((parent_id IS NULL)) WHERE parent_id IS NULL;
CREATE INDEX nodes_children ON nodes (parent_id, number);

-- The next number of each sequence the model sees in the conversation: a
-- number is advanced past in its own transaction before it is given out,
-- so a crash leaves a gap and never gives a number twice.
CREATE TABLE sequences (
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'shell', 'agent', 'tab')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;

CREATE TABLE blocks (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx     INTEGER NOT NULL CHECK (idx >= 0),
  block   TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
) STRICT;

-- An index of the blobs the blocks reference, one row per reference, media
-- and edit copies: derived from each block in the transaction that writes
-- it, never written on its own. The retention pass finds expired tool media
-- and referenced blobs here, which SQLite cannot index inside a block's JSON.
CREATE TABLE blob_refs (
  node_id TEXT NOT NULL,
  idx     INTEGER NOT NULL,
  part    INTEGER NOT NULL CHECK (part >= 0),
  blob    TEXT NOT NULL,
  holder  TEXT NOT NULL CHECK (holder IN ('message', 'tool_result', 'edit_copy')),
  at      INTEGER NOT NULL,
  PRIMARY KEY (node_id, idx, part),
  FOREIGN KEY (node_id, idx) REFERENCES blocks (node_id, idx) ON DELETE CASCADE
) STRICT;
CREATE INDEX blob_refs_expiry ON blob_refs (holder, at);

-- Each version holds the node's complete command-storage map.
CREATE TABLE command_snapshots (
  node_id  TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  revision INTEGER NOT NULL CHECK (revision >= 0),
  entries  TEXT NOT NULL,
  PRIMARY KEY (node_id, revision)
) STRICT;

-- Each ended command's whole output: stored as a blob, with the bytes at
-- its end the backend does not have and why; not stored, and why; or
-- removed by the retention pass, and when.
CREATE TABLE command_outputs (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  not_stored     TEXT,
  removed_at     INTEGER,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) + (removed_at IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL)
) STRICT;
CREATE INDEX command_outputs_expiry ON command_outputs (ended_at) WHERE blob IS NOT NULL;

-- A boundary names the version that was current at its edge of a block; a
-- version a boundary names cannot go.
CREATE TABLE session_boundaries (
  node_id          TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  block_id         TEXT NOT NULL,
  edge             TEXT NOT NULL CHECK (edge IN ('before_user', 'after_assistant', 'after_block')),
  command_revision INTEGER NOT NULL,
  PRIMARY KEY (node_id, block_id, edge),
  FOREIGN KEY (node_id, command_revision) REFERENCES command_snapshots (node_id, revision)
) STRICT;
