
CREATE TABLE nodes (
  id               TEXT PRIMARY KEY,
  -- The agent's number, 0 for the root; null for a side question's node,
  -- which is no agent and has its row in side_questions.
  number           INTEGER UNIQUE CHECK (number >= 0),
  parent_id        TEXT REFERENCES nodes (id) ON DELETE CASCADE,
  description      TEXT NOT NULL,
  profile          TEXT,
  -- The instructions the node's profile added to its system prompt; null
  -- when it added none, or the node has its parent's prompt.
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
  -- nodes.state: checkpoint state format 7
  state            TEXT NOT NULL,
  output_revision  INTEGER NOT NULL CHECK (output_revision >= 0),
  -- The number of the agent the node was forked from; null for one that
  -- was not forked.
  forked_from      INTEGER CHECK (forked_from >= 1),
  -- A child's own working directory on the primary Host, and where the
  -- conversation worked when it was set: the primary Host's device, null
  -- for a Cloud not made yet, and the conversation's directory then. The
  -- child works there while the conversation still does, and in the
  -- conversation's directory otherwise. All null for the root, which works
  -- in the conversation's. Their reader refuses a row that sets some of
  -- them only, since a migration adds no table CHECK.
  cwd              TEXT CHECK (cwd <> ''),
  cwd_device       TEXT,
  cwd_under        TEXT CHECK (cwd_under <> ''),
  -- The Context its profile chose, in tokens, which its sessions use in
  -- place of the user's limit; null for the user's limit.
  context_limit    INTEGER CHECK (context_limit >= 1),
  -- The row number of the current path's last block, the current leaf;
  -- null while the path is empty (message-editing.md § Versions).
  leaf             INTEGER CHECK (leaf >= 0),
  -- The row a cut of the path left off its end that no row has replaced
  -- yet: the next row written at the path's end replaces it. Null when the
  -- path's end is no cut, or an edit or Regenerate cut it, whose next row
  -- is a version of its own.
  cut              INTEGER CHECK (cut >= 0),
  -- The id of the parent's call block that started this child, so the
  -- child belongs to the branch that call is on; null for the root and for
  -- a child stored before versions existed, which is on every path.
  started_by       TEXT,
  -- The model selection a child's rounds run on, the one owner of it: the
  -- checkpoint's is only what its history was last requested with. Null for
  -- the root, whose model the conversation's record holds, and for a side
  -- question, which keeps its checkpoint's. Its reader refuses a row that
  -- is a child without one, since a migration adds no table CHECK.
  -- nodes.model: model selection, as in checkpoint state format 7
  model            TEXT,
  CHECK ((closed_phase IS NULL) = (closed_at IS NULL)),
  CHECK (result IS NULL OR closed_phase = 'completed'),
  CHECK (failure IS NULL OR closed_phase = 'error')
) STRICT;
-- One root per tree.
CREATE UNIQUE INDEX nodes_root ON nodes ((parent_id IS NULL)) WHERE parent_id IS NULL;
CREATE INDEX nodes_children ON nodes (parent_id, number);

-- Each side question (comments-and-ask.md § Side questions), by its node, a
-- child of the root without a number: the root's text block its history
-- ends at, the character offsets of the asked words in the block's
-- rendered text, the words, the question, when it was asked, the title the
-- small model wrote, null until it wrote one, and whether its card is
-- folded. A side question about a picture (comments-and-ask.md § Images)
-- has the picture, a media source as JSON, its place among the block's
-- pictures as its offsets and its alt text as its words. The tree store
-- writes the row with its node; the title and the fold are the backend's.
CREATE TABLE side_questions (
  node_id      TEXT PRIMARY KEY REFERENCES nodes (id) ON DELETE CASCADE,
  block_id     TEXT NOT NULL CHECK (block_id <> ''),
  start_offset INTEGER NOT NULL CHECK (start_offset >= 0),
  end_offset   INTEGER NOT NULL CHECK (end_offset > start_offset),
  quote        TEXT NOT NULL CHECK (quote <> ''),
  picture      TEXT CHECK (json_valid(picture)),
  question     TEXT NOT NULL CHECK (question <> ''),
  asked_at     INTEGER NOT NULL,
  title        TEXT CHECK (title <> ''),
  folded       INTEGER NOT NULL DEFAULT 0 CHECK (folded IN (0, 1))
) STRICT, WITHOUT ROWID;
CREATE INDEX side_questions_in_order ON side_questions (asked_at, node_id);

-- The next number of each sequence the model sees in the conversation: a
-- number is advanced past in its own transaction before it is given out,
-- so a crash leaves a gap and never gives a number twice.
CREATE TABLE sequences (
  name TEXT PRIMARY KEY CHECK (name IN ('command', 'agent', 'tab', 'attachment', 'device')),
  next INTEGER NOT NULL CHECK (next >= 1)
) STRICT;

-- blocks.block: transcript block format 14.
-- A node's transcript is a tree of rows (message-editing.md § Versions):
-- `idx` is a row's number, its place in the order the node's rows were
-- written, which no other row is ever given. A row is never deleted.
CREATE TABLE blocks (
  node_id  TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  idx      INTEGER NOT NULL CHECK (idx >= 0),
  -- blocks.block: transcript block format 14
  block    TEXT NOT NULL,
  -- The row before it on its path; null for a path's first row.
  follows  INTEGER CHECK (follows < idx),
  -- The row it was written in place of, when the session's own rewrite (a
  -- retry, an unwind, a compaction inside a turn) began a branch with it;
  -- it is then of that row's version. Null for every other row.
  replaces INTEGER CHECK (replaces < idx),
  PRIMARY KEY (node_id, idx)
) STRICT;
-- A page of a transcript is read by its requests, which start at its user
-- blocks; a block is found by its id, and the call that started a command
-- by the command its view names (web-api-conversations.md § Conversation history).
CREATE INDEX blocks_by_type ON blocks (node_id, json_extract(block, '$.type'), idx);
CREATE INDEX blocks_by_id ON blocks (json_extract(block, '$.id'));
CREATE INDEX blocks_by_command ON blocks (json_extract(block, '$.view.commandId'));
-- The rows that follow a row: its versions and the branches the session's
-- rewrites left; and the rows that do not follow the row written before
-- them, where a place may hold versions.
CREATE INDEX blocks_by_follows ON blocks (node_id, follows);
CREATE INDEX blocks_branching ON blocks (node_id, follows) WHERE follows IS NOT idx - 1;
CREATE INDEX blocks_carrying ON blocks (node_id, replaces) WHERE replaces IS NOT NULL;
-- Each node's current path as runs of consecutive row numbers: the run at
-- path position `start` holds the rows `first` to `first + length - 1`.
-- Derived from the rows and the node's leaf, and written with the leaf in
-- the same transaction, so a page of the path reads an index range.
CREATE TABLE path_runs (
  node_id TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  start   INTEGER NOT NULL CHECK (start >= 0),
  first   INTEGER NOT NULL CHECK (first >= 0),
  length  INTEGER NOT NULL CHECK (length >= 1),
  PRIMARY KEY (node_id, start)
) STRICT, WITHOUT ROWID;
-- Each blob a block names, as a medium or a side of an edit, by the block's
-- node and index; written with the block in the same checkpoint and gone
-- with it, so what names a blob is found without reading a block. A
-- share's check of a blob and the collection of the user's namespace read
-- it (`storage.md` § Retention).
CREATE TABLE block_blobs (
  node_id TEXT NOT NULL,
  idx     INTEGER NOT NULL,
  blob    TEXT NOT NULL,
  PRIMARY KEY (node_id, idx, blob),
  FOREIGN KEY (node_id, idx) REFERENCES blocks (node_id, idx) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;
CREATE INDEX block_blobs_by_blob ON block_blobs (blob);
-- Each attachment a block's text delivers as an attachment block, by the block's
-- node and index; written with the block like block_blobs, so a share lists
-- the attachments it shows without reading a block (`storage.md`).
CREATE TABLE block_attachments (
  node_id    TEXT NOT NULL,
  idx        INTEGER NOT NULL,
  attachment INTEGER NOT NULL CHECK (attachment >= 1),
  PRIMARY KEY (node_id, idx, attachment),
  FOREIGN KEY (node_id, idx) REFERENCES blocks (node_id, idx) ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- Each share's list (`sharing.md` § Storage and routes): what it shows, by
-- the share's token and the position in it, from 0: a message or a text of
-- the root's transcript, by the row that holds it, its node and index, or,
-- with neither, by the share's copy in share_blocks at the same token and
-- position; or, with `steps`, a run of that many steps not shared. Every
-- read of a share goes through its list. The share's control record says
-- the token is valid; rows of a token it no longer has are never read.
CREATE TABLE share_entries (
  token    TEXT NOT NULL,
  position INTEGER NOT NULL CHECK (position >= 0),
  node_id  TEXT,
  idx      INTEGER CHECK (idx >= 0),
  steps    INTEGER CHECK (steps >= 1),
  CHECK ((node_id IS NULL) = (idx IS NULL)),
  CHECK (steps IS NULL OR node_id IS NULL),
  PRIMARY KEY (token, position)
) STRICT;
-- The entries that name a transcript row, for a block found by its id or
-- its type and for a write that replaces the row.
CREATE INDEX share_entries_by_row ON share_entries (node_id, idx, token);
-- A share's copies of the texts still being written when its list was
-- made, as they stood then, by the token and their position on the list.
CREATE TABLE share_blocks (
  token TEXT NOT NULL,
  idx   INTEGER NOT NULL CHECK (idx >= 0),
  -- share_blocks.block: transcript block format 14
  block TEXT NOT NULL,
  PRIMARY KEY (token, idx)
) STRICT;
-- Each blob a share's copies name, by the token.
CREATE TABLE share_blobs (
  token TEXT NOT NULL,
  blob  TEXT NOT NULL,
  PRIMARY KEY (token, blob)
) STRICT, WITHOUT ROWID;
-- Each attachment a share's text names that existed when its list was
-- made, copied from `attachments`; its blob stays shared with the
-- conversation's.
CREATE TABLE share_attachments (
  token      TEXT NOT NULL,
  number     INTEGER NOT NULL CHECK (number >= 1),
  name       TEXT NOT NULL,
  media_type TEXT NOT NULL,
  size       INTEGER NOT NULL CHECK (size >= 0),
  width      INTEGER CHECK (width >= 1),
  height     INTEGER CHECK (height >= 1),
  blob       TEXT NOT NULL,
  CHECK ((width IS NULL) = (height IS NULL)),
  PRIMARY KEY (token, number)
) STRICT;

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
-- bytes of stdout and stderr by the node's id, the title of the call that
-- started it, the interval in milliseconds that call asked it to report at,
-- null for one that reports only its end, and the call's script, which the
-- pages' view of it carries with its title (rendering.md § Frame protocol).
CREATE TABLE running_commands (
  command_id  TEXT PRIMARY KEY,
  node_id     TEXT NOT NULL REFERENCES nodes (id) ON DELETE CASCADE,
  device_id   TEXT NOT NULL,
  job_id      TEXT NOT NULL,
  tool_use_id TEXT NOT NULL,
  started_at  INTEGER NOT NULL,
  -- running_commands.places: command places format 1
  places      TEXT NOT NULL,
  title       TEXT NOT NULL DEFAULT '',
  interval_ms INTEGER CHECK (interval_ms >= 1),
  -- The turn whose shell call started it; null for a command recorded
  -- before records kept it.
  turn_id     TEXT,
  -- What it starts once its Host is back, while its start waits for it and
  -- was never sent; null once sent. running_commands.unsent: unsent start
  -- format 1.
  unsent      TEXT,
  script      TEXT NOT NULL DEFAULT ''
) STRICT;
CREATE INDEX running_commands_of_node ON running_commands (node_id);

-- Each attachment of the conversation, one the user sent or one the agent
-- uploaded, by its number in the conversation's attachment sequence: the
-- file's name, the media type read from its bytes, its size, an image's or a
-- video's width and height in pixels when its header gives them, its
-- blob in the owner's namespace, and the node of the agent whose
-- `demi attachment upload` stored it, a side question's included, null for
-- a file the user sent or uploaded themselves; only that agent's replies,
-- and those of the agents forked from it, show it (`file-previews.md`
-- § Attachment blocks). An upload also keeps the file it was uploaded from:
-- the device the command ran on and the file's full path as that Host
-- writes it, both null for a file the user sent and for an upload stored
-- before uploads kept them.
-- A row never changes; a Fork's seed copies the rows, and the blobs stay
-- shared.
CREATE TABLE attachments (
  number      INTEGER PRIMARY KEY CHECK (number >= 1),
  name        TEXT NOT NULL,
  media_type  TEXT NOT NULL,
  size        INTEGER NOT NULL CHECK (size >= 0),
  width       INTEGER CHECK (width >= 1),
  height      INTEGER CHECK (height >= 1),
  blob        TEXT NOT NULL,
  node_id     TEXT,
  source_host TEXT,
  source_path TEXT CHECK ((source_path IS NULL) = (source_host IS NULL)),
  CHECK ((width IS NULL) = (height IS NULL))
) STRICT;
