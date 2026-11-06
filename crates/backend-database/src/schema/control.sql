
-- Identity. One master: two concurrent setups cannot both create one.
CREATE TABLE users (
  id            TEXT PRIMARY KEY,
  email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
  nickname      TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  role          TEXT NOT NULL CHECK (role IN ('master', 'admin', 'user')),
  created_at    INTEGER NOT NULL,
  -- A disabled account is signed out and refused every sign-in; nothing of it is removed.
  disabled      INTEGER NOT NULL DEFAULT 0 CHECK (disabled IN (0, 1)),
  -- True until the owner of an account an administrator made or reset chooses its own password.
  chooses_password INTEGER NOT NULL DEFAULT 0 CHECK (chooses_password IN (0, 1)),
  -- Written with a session's last activity, so it outlives the sessions; none before the first sign-in.
  last_active_at INTEGER,
  -- The size of the account's Cloud disks in whole GB, which its Cloud's next start takes.
  cloud_system_gb INTEGER NOT NULL DEFAULT 16 CHECK (cloud_system_gb BETWEEN 1 AND 1024),
  cloud_home_gb   INTEGER NOT NULL DEFAULT 16 CHECK (cloud_home_gb BETWEEN 1 AND 1024)
) STRICT;
CREATE UNIQUE INDEX users_one_master ON users (role) WHERE role = 'master';

-- A signed-in browser: the SHA-256 of its cookie's token, and its id in the
-- account's list. Its last activity is written at most once an hour; it was
-- verified when its user last proved it was them on it; a handoff code that
-- made it is named, though the code's row is replaced by the next one; the
-- administrator whose sign-in code made it is named too.
CREATE TABLE web_sessions (
  token_hash     TEXT PRIMARY KEY,
  id             TEXT NOT NULL UNIQUE,
  user_id        TEXT NOT NULL REFERENCES users (id),
  expires_at     INTEGER NOT NULL,
  name           TEXT NOT NULL,
  created_at     INTEGER NOT NULL,
  last_active_at INTEGER NOT NULL,
  verified_at    INTEGER,
  handoff_id     TEXT,
  issued_by      TEXT REFERENCES users (id)
) STRICT;
CREATE INDEX web_sessions_expiry ON web_sessions (expires_at);
CREATE INDEX web_sessions_of_user ON web_sessions (user_id, created_at);

-- A browser's push subscription of a user (turns.md § Notifications), one
-- per session, which it serves and which takes it along when it ends: its
-- push service's endpoint, which names the browser's subscription, its two
-- keys, the events it chose and whether it stays quiet while the user is
-- on another device, which has no default: a subscription says it.
CREATE TABLE push_subscriptions (
  session_id       TEXT PRIMARY KEY REFERENCES web_sessions (id) ON DELETE CASCADE,
  endpoint         TEXT NOT NULL UNIQUE,
  user_id          TEXT NOT NULL REFERENCES users (id),
  p256dh           TEXT NOT NULL,
  auth             TEXT NOT NULL,
  turn_finishes    INTEGER NOT NULL CHECK (turn_finishes IN (0, 1)),
  turn_fails       INTEGER NOT NULL CHECK (turn_fails IN (0, 1)),
  needs_permission INTEGER NOT NULL CHECK (needs_permission IN (0, 1)),
  agent_notifies   INTEGER NOT NULL CHECK (agent_notifies IN (0, 1)),
  created_at       INTEGER NOT NULL,
  quiet_while_elsewhere INTEGER NOT NULL CHECK (quiet_while_elsewhere IN (0, 1))
) STRICT;
CREATE INDEX push_subscriptions_of_user ON push_subscriptions (user_id);

-- Each user's one handoff code and one sign-in code, by the SHA-256 of the
-- code, with the path it opens; a sign-in code is made by an administrator
-- for the account, who is named. A spent one stays until the next of its
-- kind replaces it.
CREATE TABLE handoff_codes (
  user_id    TEXT NOT NULL REFERENCES users (id),
  kind       TEXT NOT NULL CHECK (kind IN ('handoff', 'sign_in')),
  issued_by  TEXT REFERENCES users (id),
  id         TEXT NOT NULL UNIQUE,
  code_hash  TEXT NOT NULL UNIQUE,
  to_path    TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  spent      INTEGER NOT NULL CHECK (spent IN (0, 1)),
  PRIMARY KEY (user_id, kind),
  CHECK ((kind = 'sign_in') = (issued_by IS NOT NULL))
) STRICT;

-- One pending address change per user.
CREATE TABLE email_challenges (
  user_id       TEXT PRIMARY KEY REFERENCES users (id),
  id            TEXT NOT NULL UNIQUE,
  email         TEXT NOT NULL,
  password_hash TEXT NOT NULL,
  code_hash     TEXT NOT NULL,
  expires_at    INTEGER NOT NULL,
  sent_at       INTEGER NOT NULL,
  attempts      INTEGER NOT NULL
) STRICT;

-- Each user's passkeys, by credential id in base64url; a public key is no
-- secret, so it is stored as it is.
CREATE TABLE passkeys (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  -- passkeys.credential: webauthn-rs 0.5's Passkey
  credential   TEXT NOT NULL,
  created_at   INTEGER NOT NULL,
  last_used_at INTEGER
) STRICT;
CREATE INDEX passkeys_of_user ON passkeys (user_id, created_at);

-- The challenges of passkey sign-ins and additions, until used or expired;
-- an addition's is the user's.
CREATE TABLE passkey_challenges (
  id         TEXT PRIMARY KEY,
  kind       TEXT NOT NULL CHECK (kind IN ('sign_in', 'registration')),
  user_id    TEXT REFERENCES users (id),
  -- passkey_challenges.state: webauthn-rs 0.5's DiscoverableAuthentication
  -- for a sign-in, PasskeyRegistration for a registration
  state      TEXT NOT NULL,
  expires_at INTEGER NOT NULL,
  CHECK ((kind = 'registration') = (user_id IS NOT NULL))
) STRICT;
CREATE INDEX passkey_challenges_expiry ON passkey_challenges (expires_at);

-- Each share of a conversation or of one answer of it, behind its token,
-- the only key to it (`sharing.md` § Storage and routes): its owner, its
-- conversation, its kind, the answer's block for an answer's share, and when
-- its snapshot, in the conversation's database, was taken. A conversation
-- has at most one share of its own and one of each answer; its shares go
-- with it.
CREATE TABLE shares (
  token           TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users (id),
  conversation_id TEXT NOT NULL COLLATE NOCASE,
  kind            TEXT NOT NULL CHECK (kind IN ('conversation', 'answer', 'reply_block')),
  block_id        TEXT,
  -- A reply block's share: the tag of its line and its ID, which its
  -- plugin keeps.
  reply_block_tag TEXT,
  reply_block_id  TEXT,
  -- The conversation's title when the snapshot was taken; null while it
  -- was the placeholder. A reply block's share holds the title its
  -- component gave it.
  title           TEXT,
  taken_at        INTEGER NOT NULL,
  CHECK ((kind = 'answer') = (block_id IS NOT NULL)),
  CHECK ((kind = 'reply_block') = (reply_block_tag IS NOT NULL)),
  CHECK ((kind = 'reply_block') = (reply_block_id IS NOT NULL)),
  CHECK (kind <> 'reply_block' OR title IS NOT NULL)
) STRICT;
CREATE UNIQUE INDEX shares_of_conversation ON shares (user_id, conversation_id) WHERE kind = 'conversation';
CREATE UNIQUE INDEX shares_of_answer ON shares (user_id, conversation_id, block_id) WHERE kind = 'answer';
CREATE UNIQUE INDEX shares_of_reply_block ON shares (user_id, conversation_id, reply_block_tag, reply_block_id)
  WHERE kind = 'reply_block';

CREATE TABLE managed_operations (
  device_id    TEXT NOT NULL,
  operation_id TEXT NOT NULL,
  base_version TEXT NOT NULL,
  phase        TEXT NOT NULL
                 CHECK (phase IN ('stopping', 'saving', 'rebuilding', 'booting', 'ready', 'failed')),
  error        TEXT,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (device_id, operation_id)
) STRICT;

-- The keys another server borrows an entry with: each one's SHA-256, and
-- once it was used, the server that used it last, by the host it gave, and
-- when.
CREATE TABLE provider_share_keys (
  token_hash  TEXT PRIMARY KEY,
  id          TEXT NOT NULL UNIQUE,
  -- The entry is a record of its owner's core.
  owner_id    TEXT NOT NULL REFERENCES users (id),
  provider_id TEXT NOT NULL,
  created_by  TEXT NOT NULL REFERENCES users (id),
  created_at  INTEGER NOT NULL,
  used_by     TEXT,
  used_at     INTEGER,
  CHECK ((used_by IS NULL) = (used_at IS NULL))
) STRICT;
CREATE INDEX provider_share_keys_provider ON provider_share_keys (owner_id, provider_id, created_at);

-- Which user's core holds each device, and the hash of its current token,
-- which finds the device of a runner's hello or pipe request through one
-- index lookup. A Cloud has no token until it boots.
CREATE TABLE device_owners (
  device_id  TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  token_hash TEXT UNIQUE
) STRICT, WITHOUT ROWID;

-- Which user's core holds each schedule, for its URL's calls, which carry no
-- session. A row whose schedule is gone finds nothing there and goes.
CREATE TABLE schedule_owners (
  schedule_id TEXT PRIMARY KEY,
  user_id     TEXT NOT NULL REFERENCES users (id)
) STRICT, WITHOUT ROWID;

-- The server's own settings, which its master and administrators change:
-- one row, absent until one is changed. `upgrade_channel` names the releases
-- Settings › Updates offers.
CREATE TABLE server_settings (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  -- The channel an administrator chose in Settings › Updates; none keeps the installation's.
  upgrade_channel TEXT CHECK (upgrade_channel IN ('beta', 'canary')),
  -- Whether every user uses the master's providers, which the master chooses in Settings › Users.
  instance_mode   TEXT NOT NULL DEFAULT 'isolated' CHECK (instance_mode IN ('shared', 'isolated')),
  -- Whether a check that finds a newer release gets it ready at once, which an administrator turns on in Settings › Updates.
  prepare_ahead   INTEGER NOT NULL DEFAULT 0 CHECK (prepare_ahead IN (0, 1)),
  -- How Clouds are isolated, which the master chooses in Settings › Cloud.
  isolation       TEXT NOT NULL DEFAULT 'simple' CHECK (isolation IN ('simple', 'gvisor')),
  -- What Clouds reach, which the master chooses in Settings › Cloud.
  network_policy  TEXT NOT NULL DEFAULT 'internetOnly'
    CHECK (network_policy IN ('noNetwork', 'developerServices', 'internetOnly', 'internetAndPrivateNetwork', 'custom')),
  -- What Custom allows, kept while another policy is chosen: its groups, and the domains and addresses
  -- as a JSON array of the strings the master wrote.
  custom_internet INTEGER NOT NULL DEFAULT 1 CHECK (custom_internet IN (0, 1)),
  custom_private_network INTEGER NOT NULL DEFAULT 0 CHECK (custom_private_network IN (0, 1)),
  custom_developer_services INTEGER NOT NULL DEFAULT 0 CHECK (custom_developer_services IN (0, 1)),
  custom_allowed  TEXT NOT NULL DEFAULT '[]'
) STRICT;
