
CREATE TABLE nodes (
  id               TEXT PRIMARY KEY,
  number           INTEGER NOT NULL UNIQUE CHECK (number >= 0),
  parent_id        TEXT REFERENCES nodes (id) ON DELETE CASCADE,
  description      TEXT NOT NULL,
  profile          TEXT,
  -- The instructions the node's profile replaced its parent's with; null
  -- when it has its parent's.
  instructions     TEXT,
  round            INTEGER NOT NULL CHECK (round >= 1),
  started_at       INTEGER NOT NULL,
  can_spawn        INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  closed_phase     TEXT CHECK (closed_phase IN ('completed', 'aborted', 'error')),
  closed_at        INTEGER,
  result           TEXT,
  failure          TEXT,
  delivered        INTEGER NOT NULL CHECK (delivered IN (0, 1)),
  -- The session's checkpoint state as JSON, with whether a Stop holds its
  -- waiting input until the user's next action.
  state            TEXT NOT NULL,
  block_count      INTEGER NOT NULL CHECK (block_count >= 0),
  output_revision  INTEGER NOT NULL CHECK (output_revision >= 0),
  -- When the earliest wakeup the state saves is due, as the index of
  -- conversations holds it; null when it saves none.
  wakeup_at        INTEGER CHECK (wakeup_at >= 0),
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
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'shell', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;

CREATE TABLE blocks (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx     INTEGER NOT NULL CHECK (idx >= 0),
  block   TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
) STRICT;

-- Each ended command's whole output: stored as a blob, with the bytes at
-- its end the backend does not have and why; or not stored, and why.
CREATE TABLE command_outputs (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  -- How it ended, a JSON object: its exit code, stopped, or with its Host's
  -- connection; null for a row a release before 0.1.21 wrote.
  ending         TEXT,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  -- The command's media, a JSON array, beside a stored output.
  media          TEXT,
  not_stored     TEXT,
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL),
  CHECK ((media IS NULL) = (blob IS NULL))
) STRICT;

-- Each attachment the agent uploaded, by its number in the conversation's
-- attachment sequence: the file's name, the media type read from its bytes,
-- its size, an image's or a video's width and height in pixels when its
-- header gives them, and its blob in the owner's namespace. A row never
-- changes; a Fork's seed copies the rows, and the blobs stay shared.
CREATE TABLE attachments (
  number     INTEGER PRIMARY KEY CHECK (number >= 1),
  name       TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size >= 0),
  width      INTEGER CHECK (width >= 1),
  height     INTEGER CHECK (height >= 1),
  blob       TEXT NOT NULL,
  CHECK ((width IS NULL) = (height IS NULL))
) STRICT;
