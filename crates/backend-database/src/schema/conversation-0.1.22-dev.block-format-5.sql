
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
  -- nodes.state: checkpoint state format 2
  state            TEXT NOT NULL,
  block_count      INTEGER NOT NULL CHECK (block_count >= 0),
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
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'agent', 'tab', 'attachment')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;

-- blocks.block: transcript block format 5.
CREATE TABLE blocks (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx     INTEGER NOT NULL CHECK (idx >= 0),
  -- blocks.block: transcript block format 5
  block   TEXT NOT NULL,
  PRIMARY KEY (node_id, idx)
) STRICT;
-- A page of a transcript is read by its requests, which start at its user
-- blocks; a block is found by its id, and the call that started a command
-- by the command its view names (web-api.md § Conversation history).
CREATE INDEX blocks_by_type ON blocks (node_id, json_extract(block, '$.type'), idx);
CREATE INDEX blocks_by_id ON blocks (json_extract(block, '$.id'));
CREATE INDEX blocks_by_command ON blocks (json_extract(block, '$.view.commandId'));

-- Each ended command's whole output: stored as a blob, with the bytes at
-- its end the backend does not have and why; or not stored, and why; and
-- the title of the call that started it, empty for a row written before
-- rows kept it.
CREATE TABLE command_outputs (
  command_id     TEXT PRIMARY KEY,
  ended_at       INTEGER NOT NULL,
  -- How it ended, a JSON object: its exit code, stopped, or with its Host's
  -- connection; null for a row a release before 0.1.21 wrote.
  -- command_outputs.ending: command ending format 2
  ending         TEXT,
  blob           TEXT,
  missing_bytes  INTEGER CHECK (missing_bytes >= 0),
  missing_reason TEXT,
  not_stored     TEXT,
  title          TEXT NOT NULL DEFAULT '',
  CHECK ((blob IS NOT NULL) + (not_stored IS NOT NULL) = 1),
  CHECK ((missing_bytes IS NULL) = (missing_reason IS NULL)),
  CHECK (missing_bytes IS NULL OR blob IS NOT NULL)
) STRICT;

-- Each command that runs, until its end writes its command_outputs row in
-- the same transaction: the node that ran it, its device, its job's id on
-- that device, the call that started it, when it started, how far each
-- node that looked at it has seen its output, a JSON object of each node's
-- bytes of stdout and stderr by the node's id, and the title of the call
-- that started it, empty for a command recorded before records kept it.
CREATE TABLE running_commands (
  command_id  TEXT PRIMARY KEY,
  node_id     TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  device_id   TEXT NOT NULL,
  job_id      TEXT NOT NULL,
  tool_use_id TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  -- running_commands.places: command places format 1
  places      TEXT NOT NULL,
  title       TEXT NOT NULL DEFAULT ''
) STRICT;
CREATE INDEX running_commands_of_node ON running_commands (node_id);

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
