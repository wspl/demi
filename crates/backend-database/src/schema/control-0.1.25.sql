
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
-- keys and the events it chose.
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
  created_at       INTEGER NOT NULL
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

CREATE TABLE user_preferences (
  user_id     TEXT PRIMARY KEY REFERENCES users (id),
  preferences TEXT NOT NULL
) STRICT;

-- Each user's personal instructions (`instructions.md` § Personal
-- instructions): a user with no row has none.
CREATE TABLE user_instructions (
  user_id TEXT PRIMARY KEY REFERENCES users (id),
  text    TEXT NOT NULL
) STRICT;

-- Each user's Subagents switch and limits (`subagents.md` § Profiles,
-- § Limits): a user with no row has subagents on, at most 10 live and 1
-- level deep. Each limit is the number last set and whether it is lifted
-- (Unlimited), which keeps the number.
CREATE TABLE user_subagents (
  user_id             TEXT PRIMARY KEY REFERENCES users (id),
  enabled             INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  max_live            INTEGER NOT NULL CHECK (max_live >= 1),
  max_live_unlimited  INTEGER NOT NULL CHECK (max_live_unlimited IN (0, 1)),
  max_depth           INTEGER NOT NULL CHECK (max_depth >= 1),
  max_depth_unlimited INTEGER NOT NULL CHECK (max_depth_unlimited IN (0, 1))
) STRICT;

-- Each user's subagent profiles: the model settings as JSON, null for the
-- parent's model, and the instructions added to a child's system prompt
-- after the product's, null for none. A name is unique among its user's
-- profiles.
CREATE TABLE subagent_profiles (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  description  TEXT NOT NULL,
  model        TEXT,
  instructions TEXT,
  can_spawn    INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  enabled      INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  -- The profile's own Context in tokens, its model's full window or a
  -- limit; null for the user's limit, and always without a model.
  context_limit INTEGER CHECK (context_limit >= 1),
  UNIQUE (user_id, name)
) STRICT;

-- Devices and workspaces. The token hash is absent until the backend issues
-- a token; one managed device per user.
CREATE TABLE devices (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  kind         TEXT NOT NULL CHECK (kind IN ('user', 'managed')),
  name         TEXT NOT NULL,
  platform     TEXT NOT NULL,
  token_hash   TEXT UNIQUE,
  claimed_at   INTEGER NOT NULL,
  last_seen_at INTEGER,
  installed    TEXT NOT NULL DEFAULT '[]',
  -- The operating system its runner last reported, JSON, and the runner
  -- release it reported with it; none before its runner first connected.
  os           TEXT,
  runner_version TEXT,
  -- Whether the backend's last shutdown ended its runner's connection; the
  -- next start reads it and clears it.
  ended_by_shutdown INTEGER NOT NULL DEFAULT 0 CHECK (ended_by_shutdown IN (0, 1)),
  -- How pages reach it: Automatic, Prefer Direct or Server Only, which the
  -- user picks on the device's page.
  route        TEXT NOT NULL DEFAULT 'automatic' CHECK (route IN ('automatic', 'direct', 'server')),
  -- The instance its runner last named, a number each start of the runner
  -- draws; none before its runner first connected.
  runner_instance INTEGER,
  -- When a Cloud's latest boot completed, as its runner connected; the
  -- lifetime cap counts from it. None for a paired device and a Cloud that
  -- never booted.
  cloud_started_at INTEGER,
  -- A paired device's number among its user's devices of the same logo
  -- (conversations.md § Conversations and projects), given when its runner first
  -- reports its system and again when its logo changes; none before, and
  -- for a Cloud.
  mark_number INTEGER CHECK (mark_number > 0),
  -- Which managed machine a managed device is: the user's Cloud, or the
  -- Claude Code machine, which only the instance's master holds
  -- (managed-hosts.md § The Claude Code machine); none for a paired device.
  machine      TEXT CHECK (machine IN ('cloud', 'claude_code')),
  -- The home its runner last reported (sessions-and-targets.md § Resolve a
  -- target); none before its runner first reported one since the release
  -- that began keeping it.
  home TEXT
) STRICT;
CREATE UNIQUE INDEX devices_one_machine ON devices (user_id, machine) WHERE kind = 'managed';

CREATE TABLE workspaces (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  device_id  TEXT NOT NULL REFERENCES devices (id),
  path       TEXT NOT NULL,
  name       TEXT NOT NULL,
  sort_order INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  -- What the project's next conversation starts with (web-api.md § User
  -- preferences), as the user last chose it there: model settings as JSON,
  -- and a Workflow menu choice typed as a conversation's; each none before
  -- its first choice.
  last_model TEXT,
  last_workflow_kind TEXT
    CHECK (last_workflow_kind IN ('default', 'none', 'workflow')),
  last_workflow_id TEXT
    CHECK ((last_workflow_kind IS 'workflow') = (last_workflow_id IS NOT NULL))
) STRICT;
CREATE INDEX workspaces_order ON workspaces (user_id, sort_order);
CREATE UNIQUE INDEX workspaces_directory ON workspaces (user_id, device_id, path);

-- Each choice a user made about a plugin (`plugins.md` § A user's plugins):
-- a plugin with no row is on.
CREATE TABLE user_plugins (
  user_id TEXT NOT NULL REFERENCES users (id),
  plugin  TEXT NOT NULL,
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  PRIMARY KEY (user_id, plugin)
) STRICT, WITHOUT ROWID;

-- Each plugin's values for a user (`plugins.md` § The contract): a JSON
-- document its plugin decodes, the revision a conditional write names, and
-- the blobs the value names, a JSON array of their SHA-256s.
CREATE TABLE plugin_values (
  user_id  TEXT NOT NULL REFERENCES users (id),
  plugin   TEXT NOT NULL,
  key      TEXT NOT NULL,
  document TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  blobs    TEXT NOT NULL,
  PRIMARY KEY (user_id, plugin, key)
) STRICT, WITHOUT ROWID;

-- Each plugin's Host directories for a user (`plugins.md` § Host
-- directories): the directory's name, its digest and its files, a JSON
-- array of each file's path, whether it is executable and its SHA-256.
CREATE TABLE plugin_directories (
  user_id TEXT NOT NULL REFERENCES users (id),
  plugin  TEXT NOT NULL,
  name    TEXT NOT NULL,
  digest  TEXT NOT NULL,
  files   TEXT NOT NULL,
  PRIMARY KEY (user_id, plugin, name)
) STRICT, WITHOUT ROWID;

-- Each plugin's secrets for a user (`plugins.md` § Secrets): sealed, with
-- the plugin, the user and the name as the additional data, and the
-- revision a conditional write names. The plugin host keeps a sign-in's
-- tokens here too, under a name no plugin can give.
CREATE TABLE plugin_secrets (
  user_id  TEXT NOT NULL REFERENCES users (id),
  plugin   TEXT NOT NULL,
  name     TEXT NOT NULL,
  sealed   BLOB NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  PRIMARY KEY (user_id, plugin, name)
) STRICT, WITHOUT ROWID;

-- The public client each plugin registered with its vendor's authorization
-- server (`plugins.md` § OAuth sign-in), and the redirect URI it was
-- registered with: a client of another URI is registered again.
CREATE TABLE plugin_oauth_clients (
  plugin       TEXT NOT NULL,
  issuer       TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  client_id    TEXT NOT NULL,
  PRIMARY KEY (plugin, issuer)
) STRICT, WITHOUT ROWID;

-- Each user's credentials (`credentials.md`): the fields as JSON, credential
-- fields format 1, `[{ name, label, kind }]`, never a value; the values, a
-- JSON object by field name, sealed with the user and the name as the
-- additional data, so a rename seals them again; when it is removed, null
-- while it is kept always; and when an agent last read it. A name is unique
-- among its user's credentials.
CREATE TABLE user_credentials (
  id           TEXT PRIMARY KEY,
  user_id      TEXT NOT NULL REFERENCES users (id),
  name         TEXT NOT NULL,
  note         TEXT NOT NULL,
  fields       TEXT NOT NULL,
  sealed       BLOB NOT NULL,
  keep_until   INTEGER,
  last_used_at INTEGER,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  UNIQUE (user_id, name)
) STRICT;
CREATE INDEX user_credentials_by_expiry ON user_credentials (keep_until) WHERE keep_until IS NOT NULL;

-- Each user's automations (`automations.md`): the trigger as JSON,
-- automation trigger format 1, the `Trigger` of the web contract; the
-- target as JSON, automation target format 1, with the existing
-- conversation it wakes again in `conversation_id`, null for a new one each
-- run; the model settings as JSON, null for none chosen; why it paused
-- itself, null while it is on or the user paused it; when its trigger was
-- last set or it was last turned on, before which no time of its schedule
-- fires; a URL trigger's secret and the Stripe signing secret the user
-- pasted, each sealed with the automation as the additional data; and a
-- change watch's last output, its standard output's last 16 KB, the
-- SHA-256 of the whole of it, which a check compares, and its exit status,
-- with when its command last ran, all null before its first.
CREATE TABLE automations (
  id               TEXT PRIMARY KEY,
  user_id          TEXT NOT NULL REFERENCES users (id),
  name             TEXT NOT NULL,
  trigger          TEXT NOT NULL,
  time_zone        TEXT NOT NULL,
  max_runs_per_day INTEGER CHECK (max_runs_per_day > 0),
  target           TEXT NOT NULL,
  conversation_id  TEXT,
  prompt           TEXT NOT NULL,
  model            TEXT,
  paused           INTEGER NOT NULL CHECK (paused IN (0, 1)),
  paused_because   TEXT CHECK (paused_because IN ('conversation_archived', 'conversation_deleted')),
  schedule_since   INTEGER NOT NULL,
  secret           BLOB,
  stripe_secret    BLOB,
  watch_stdout     BLOB,
  watch_digest     BLOB,
  watch_exit       INTEGER,
  watch_checked_at INTEGER,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
) STRICT;
CREATE INDEX automations_by_user ON automations (user_id, created_at);
CREATE INDEX automations_by_conversation ON automations (conversation_id) WHERE conversation_id IS NOT NULL;
-- Each automation's runs (`automations.md` § Runs): what fired it, when,
-- its state with a note as JSON, automation run note format 1, the `RunNote`
-- of the web contract, the start of its agent's last answer, the
-- conversation it ran in with the turn its message started, whether that
-- new conversation is still filed under the automation, out of the
-- sidebar, whether it notified and its conversation was not opened since,
-- and whether its agent notified at all, which makes it end Notified. A
-- run's conversation outlives it.
CREATE TABLE automation_runs (
  id              TEXT PRIMARY KEY,
  automation_id   TEXT NOT NULL REFERENCES automations (id) ON DELETE CASCADE,
  cause           TEXT NOT NULL CHECK (cause IN ('time', 'change', 'url', 'now')),
  at              INTEGER NOT NULL,
  state           TEXT NOT NULL CHECK (state IN ('running', 'done', 'notified', 'skipped', 'failed')),
  note            TEXT,
  line            TEXT,
  conversation_id TEXT,
  turn_id         TEXT,
  filed           INTEGER NOT NULL CHECK (filed IN (0, 1)),
  unread          INTEGER NOT NULL CHECK (unread IN (0, 1)),
  notified        INTEGER NOT NULL CHECK (notified IN (0, 1))
) STRICT;
CREATE INDEX automation_runs_by_automation ON automation_runs (automation_id, at);
CREATE INDEX automation_runs_by_conversation ON automation_runs (conversation_id) WHERE conversation_id IS NOT NULL;
-- A URL's calls (`automations.md` § Triggers), each kept as it is taken,
-- with its delivery ID: pending until a run takes it, so a restart loses
-- none; taken; or held, for one a paused automation or its daily limit
-- answered and one a pause or an edit dropped. A pending call keeps its
-- method, its headers as JSON and its body; the others keep only what
-- dedupes a repeated delivery ID, for an hour.
CREATE TABLE automation_calls (
  id            INTEGER PRIMARY KEY,
  automation_id TEXT NOT NULL REFERENCES automations (id) ON DELETE CASCADE,
  at            INTEGER NOT NULL,
  delivery_id   TEXT,
  state         TEXT NOT NULL CHECK (state IN ('pending', 'taken', 'held')),
  method        TEXT,
  headers       TEXT,
  body          BLOB,
  CHECK ((state = 'pending') = (method IS NOT NULL AND headers IS NOT NULL AND body IS NOT NULL))
) STRICT;
CREATE INDEX automation_calls_by_state ON automation_calls (automation_id, state, id);
CREATE INDEX automation_calls_by_delivery ON automation_calls (automation_id, delivery_id)
  WHERE delivery_id IS NOT NULL;

-- The conversation index. The web app chooses a conversation's id and its
-- case is kept, but no two ids differ only in case: each names a database
-- file, and a file system may ignore case. The target is Cloud (optionally a
-- directory on it), a device directory or a workspace, checked per kind. The
-- target columns are not foreign keys: a conversation that targets a device
-- directly does not block revoking that device.
CREATE TABLE conversations (
  id                  TEXT PRIMARY KEY COLLATE NOCASE,
  user_id             TEXT NOT NULL REFERENCES users (id),
  -- title: empty exactly while title_origin is 'placeholder', a title that
  -- holds no words (conversations.md § Conversation titles).
  title               TEXT NOT NULL,
  title_origin        TEXT NOT NULL
                        CHECK (title_origin IN ('placeholder', 'message', 'generated', 'user')),
  archived            INTEGER NOT NULL CHECK (archived IN (0, 1)),
  pinned              INTEGER NOT NULL CHECK (pinned IN (0, 1)),
  sort_order          INTEGER NOT NULL,
  read_revision       INTEGER NOT NULL,
  -- target: where it runs (sessions-and-targets.md § Resolve a target): the
  -- Cloud's directory of a conversation, its own without a path; a Host's
  -- home, by its device; a workspace, with the device and directory it
  -- names; or a device's directory as a release before projects by
  -- directory kept it, until that device's runner first reports its home,
  -- and of a conversation whose device was revoked.
  target_kind         TEXT NOT NULL CHECK (target_kind IN ('cloud', 'home', 'workspace', 'device')),
  target_device_id    TEXT,
  target_path         TEXT,
  target_workspace_id TEXT,
  -- last_switch: the latest target switch, TargetSwitch as JSON; with
  -- last_switch_revision, the execution-context revision it made, both null
  -- before the first. A CHECK would tie them, but SQLite tests one an added
  -- column brings against the rows before the migration fills it.
  last_switch         TEXT,
  cloud_reset_id      TEXT,
  context_version     INTEGER NOT NULL,
  model               TEXT,
  created_at          INTEGER NOT NULL,
  updated_at          INTEGER NOT NULL,
  -- The revision of its attached hosts, raised by each change of them; 0
  -- before the first.
  hosts_revision      INTEGER NOT NULL DEFAULT 0 CHECK (hosts_revision >= 0),
  -- The pending move an agent asked for (sessions-and-targets.md § Switch
  -- the primary target): its id and its target, typed as the target is; all
  -- null while none waits.
  pending_id           TEXT,
  pending_kind         TEXT CHECK (pending_kind IN ('cloud', 'device', 'workspace')),
  pending_device_id    TEXT,
  pending_path         TEXT,
  pending_workspace_id TEXT CHECK (
    (pending_kind IS NULL
      AND pending_id IS NULL AND pending_device_id IS NULL AND pending_path IS NULL
      AND pending_workspace_id IS NULL)
    OR (pending_kind = 'cloud' AND pending_id IS NOT NULL
      AND pending_device_id IS NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'device' AND pending_id IS NOT NULL
      AND pending_device_id IS NOT NULL AND pending_path IS NOT NULL AND pending_workspace_id IS NULL)
    OR (pending_kind = 'workspace' AND pending_id IS NOT NULL
      AND pending_workspace_id IS NOT NULL AND pending_device_id IS NULL AND pending_path IS NULL)
  ),
  -- Its Workflow menu choice (subagents.md § Workflows of a project): the
  -- user's settings, No Subagents, or the workflow whose file is
  -- .demi/workflows/<workflow_id>.yaml.
  workflow_kind TEXT NOT NULL DEFAULT 'default' CHECK (workflow_kind IN ('default', 'none', 'workflow')),
  workflow_id   TEXT CHECK ((workflow_kind = 'workflow') = (workflow_id IS NOT NULL)),
  last_switch_revision INTEGER,
  CHECK (
    (target_kind = 'cloud'
      AND target_device_id IS NULL AND target_workspace_id IS NULL)
    OR (target_kind = 'home'
      AND target_device_id IS NOT NULL AND target_path IS NULL AND target_workspace_id IS NULL)
    OR (target_kind = 'device'
      AND target_device_id IS NOT NULL AND target_path IS NOT NULL AND target_workspace_id IS NULL)
    OR (target_kind = 'workspace'
      AND target_workspace_id IS NOT NULL AND target_device_id IS NOT NULL AND target_path IS NOT NULL)
  )
) STRICT;
CREATE INDEX conversations_sidebar ON conversations (user_id, archived, pinned, sort_order);
CREATE INDEX conversations_workspace ON conversations (target_workspace_id);

-- Each job of a command that runs, with its device, whose runner's hello
-- names the job, and its conversation, whose tree takes the command up
-- again (`storage.md` § Command outputs). Written before the job starts and
-- deleted with the command's end; written before then too while the job's
-- start waits for its device, with sent 0, until it is sent.
CREATE TABLE running_jobs (
  job_id          TEXT PRIMARY KEY,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  sent            INTEGER NOT NULL DEFAULT 1 CHECK (sent IN (0, 1))
) STRICT;
CREATE INDEX running_jobs_of_device ON running_jobs (device_id);

-- A device attached to a conversation, which goes by its device's name; the
-- row id keeps the order of hosts attached at one time.
CREATE TABLE conversation_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id),
  device_id       TEXT NOT NULL REFERENCES devices (id),
  -- The directory its commands start in, fixed when it was attached; null
  -- for a device attached by name, whose commands start in its home.
  cwd             TEXT,
  attached_at     INTEGER NOT NULL,
  -- An agent's detach, which waits for the tree to be idle.
  detaching       INTEGER NOT NULL DEFAULT 0 CHECK (detaching IN (0, 1)),
  PRIMARY KEY (conversation_id, device_id)
) STRICT;
CREATE INDEX conversation_hosts_device ON conversation_hosts (device_id);

-- A conversation's work panel: its tabs and the ids it ever had, one JSON
-- document, with the revision that counts its changes.
CREATE TABLE conversation_panels (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CHECK (revision >= 1),
  document        TEXT NOT NULL,
  updated_at      INTEGER NOT NULL
) STRICT;

-- A conversation's draft: its text and files with the revision they were
-- written at, and the version a save replaced with the revision that version
-- had. The row stays when the draft is emptied, so a revision is never used
-- twice. It holds the revision of the conversation's messages saved for
-- later too, which every save, edit, send and removal of one raises.
CREATE TABLE conversation_drafts (
  conversation_id   TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision          INTEGER NOT NULL CHECK (revision >= 1),
  document          TEXT NOT NULL,
  written           INTEGER NOT NULL CHECK (written BETWEEN 1 AND revision),
  replaced_revision INTEGER,
  replaced          TEXT,
  saved_revision    INTEGER NOT NULL CHECK (saved_revision >= 0),
  updated_at        INTEGER NOT NULL,
  CHECK ((replaced IS NULL) = (replaced_revision IS NULL))
) STRICT;

-- A conversation's messages saved for later: each one's text and files as
-- one JSON document, as a draft holds them.
CREATE TABLE saved_messages (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL
) STRICT;
CREATE INDEX saved_messages_of_conversation ON saved_messages (conversation_id, saved_at);

-- An edit of a message saved for later, by the page's id for it, with its
-- answer, kept a day so the same edit sent again answers as it first did.
-- answer: the edit's answer, `{ draft, revisions }` as `web-api.md`
-- § Messages saved for later names it, as JSON.
CREATE TABLE saved_edits (
  op              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  answer          TEXT NOT NULL,
  made_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX saved_edits_by_age ON saved_edits (made_at);

-- A message saved for later that a send took, with the turn the send became,
-- kept a day so the same send, or a save of it, sent again is known.
-- document: its text and files as saved_messages keeps them.
CREATE TABLE saved_sends (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  turn_id         TEXT NOT NULL,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL,
  sent_at         INTEGER NOT NULL
) STRICT;
CREATE INDEX saved_sends_by_age ON saved_sends (sent_at);

-- A conversation's requests (permissions.md § Requests), each of a kind: a
-- permission request is a command an agent ran without the grants of its
-- categories, a JSON array of their ids sorted so that the same categories
-- compare equal; a credential request asks the user for a credential of a
-- name, its ask a JSON document, credential request format 1, `{ reason,
-- fields: [{ name, label, kind }], keep }`, and is gone once answered, since
-- the command that asked waits for the answer; an allowance request asks to
-- use a saved credential in the conversation, which names it while it is
-- saved. A request is undecided until the user decides and then kept until
-- the decision's message is in the agent's checkpoint, and read once the
-- user has opened the conversation with the request waiting (the sidebar's
-- yellow mark settles then). The agent is the root when its number and
-- description are null. Each category the user allowed for a conversation,
-- and each credential, are the conversation's grants and allowances.
CREATE TABLE permission_requests (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('permission', 'credential', 'allowance')),
  categories        TEXT,
  credential_name   TEXT,
  credential_id     TEXT REFERENCES user_credentials (id) ON DELETE SET NULL,
  ask               TEXT,
  command           TEXT NOT NULL,
  node_id           TEXT NOT NULL,
  agent_number      INTEGER,
  agent_description TEXT,
  created_at        INTEGER NOT NULL,
  decision          TEXT CHECK (decision IN ('allowed', 'denied')),
  decided_at        INTEGER,
  read              INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1)),
  -- The host a permission request asks to reach (permissions.md § Hosts):
  -- a device, or the pairing code of a computer `demi host pair` pairs; none
  -- for a request of categories alone, whose categories are then one or more.
  host_device_id    TEXT CHECK (host_device_id IS NULL OR kind = 'permission'),
  pairing_code      TEXT CHECK (pairing_code IS NULL OR (kind = 'permission' AND host_device_id IS NULL)),
  CHECK ((kind = 'permission') = (categories IS NOT NULL)),
  CHECK ((kind = 'permission') = (credential_name IS NULL)),
  CHECK ((kind = 'credential') = (ask IS NOT NULL)),
  CHECK (kind = 'allowance' OR credential_id IS NULL),
  CHECK (kind <> 'credential' OR decision IS NULL),
  CHECK ((agent_number IS NULL) = (agent_description IS NULL)),
  CHECK ((decision IS NULL) = (decided_at IS NULL))
) STRICT;
CREATE INDEX permission_requests_of_conversation ON permission_requests (conversation_id, created_at);

CREATE TABLE permission_grants (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  category        TEXT NOT NULL,
  granted_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, category)
) STRICT;

-- The hosts a conversation may reach besides its primary one
-- (sessions-and-targets.md § Hosts a conversation may reach): each primary
-- target it left, each host attached to it, and each host the user allowed
-- when an agent asked; kept for the conversation's life, as a grant is.
CREATE TABLE reachable_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  added_at        INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, device_id)
) STRICT;
CREATE INDEX reachable_hosts_of_device ON reachable_hosts (device_id);

-- The pairing codes the user let a conversation pair (permissions.md
-- § Hosts): its next `demi host pair` of the code is dispatched, once.
CREATE TABLE pairing_passes (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  code            TEXT NOT NULL,
  allowed_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, code)
) STRICT;

CREATE TABLE credential_allowances (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  credential_id   TEXT NOT NULL REFERENCES user_credentials (id) ON DELETE CASCADE,
  allowed_at      INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, credential_id)
) STRICT;
CREATE INDEX credential_allowances_of_credential ON credential_allowances (credential_id);

-- A conversation whose root's running turn a graceful shutdown began to
-- interrupt, marked before the turn was aborted, which the next start resumes
-- once (`failures-and-recovery.md` § The unfinished turn).
CREATE TABLE interrupted_turns (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE
) STRICT;

-- Each share of a conversation or of one answer of it, behind its token,
-- the only key to it (`sharing.md` § Storage and routes): its owner, its
-- conversation, its kind, the answer's block for an answer's share, and when
-- its snapshot, in the conversation's database, was taken. A conversation
-- has at most one share of its own and one of each answer; its shares go
-- with it.
CREATE TABLE shares (
  token           TEXT PRIMARY KEY,
  user_id         TEXT NOT NULL REFERENCES users (id),
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind            TEXT NOT NULL CHECK (kind IN ('conversation', 'answer')),
  block_id        TEXT,
  -- The conversation's title when the snapshot was taken; null while it
  -- was the placeholder.
  title           TEXT,
  taken_at        INTEGER NOT NULL,
  CHECK ((kind = 'answer') = (block_id IS NOT NULL))
) STRICT;
CREATE UNIQUE INDEX shares_of_conversation ON shares (conversation_id) WHERE kind = 'conversation';
CREATE UNIQUE INDEX shares_of_answer ON shares (conversation_id, block_id) WHERE kind = 'answer';

-- A conversation's deletion that has not reached its last step: its record
-- is gone, and a start finishes what is left (`storage.md` § Deleting a
-- conversation). The id is compared as the index compares it.
CREATE TABLE conversation_deletions (
  id         TEXT PRIMARY KEY COLLATE NOCASE,
  user_id    TEXT NOT NULL REFERENCES users (id),
  deleted_at INTEGER NOT NULL
) STRICT;

-- Records that make interrupted multi-step work discoverable. A Fork
-- reserves its destination's conversation id, compared as the index
-- compares it.
-- conversation_fork_operations.metadata: Fork metadata format 3, ForkMetadata
-- as JSON, whose attached hosts are their devices and directories without
-- names and which holds the source's execution-context revision at the Fork
-- and its workflow (format 1 named the hosts and held no revision, format 2
-- held no workflow).
CREATE TABLE conversation_fork_operations (
  id        TEXT PRIMARY KEY COLLATE NOCASE,
  user_id   TEXT NOT NULL REFERENCES users (id),
  source_id TEXT NOT NULL,
  block_id  TEXT NOT NULL,
  metadata  TEXT NOT NULL
) STRICT;

CREATE TABLE managed_operations (
  device_id    TEXT NOT NULL REFERENCES devices (id),
  operation_id TEXT NOT NULL,
  base_version TEXT NOT NULL,
  phase        TEXT NOT NULL
                 CHECK (phase IN ('stopping', 'saving', 'rebuilding', 'booting', 'ready', 'failed')),
  error        TEXT,
  updated_at   INTEGER NOT NULL,
  PRIMARY KEY (device_id, operation_id)
) STRICT;

-- Providers: an API-key entry's sealed configuration, a subscription entry's
-- active account, a sealed secret per subscription account, and one
-- subscription entry per owner and family. The active account is not a
-- foreign key: removing an account clears it in the same transaction.
-- providers.config: provider configuration format 2, sealed.
CREATE TABLE providers (
  id                   TEXT PRIMARY KEY,
  owner_user_id        TEXT NOT NULL REFERENCES users (id),
  provider_type        TEXT NOT NULL,
  credential_kind      TEXT NOT NULL CHECK (credential_kind IN ('api_key', 'subscription')),
  label                TEXT NOT NULL,
  config               BLOB,
  active_credential_id TEXT,
  created_at           INTEGER NOT NULL,
  -- Which account each agent uses (providers.md § Account rotation); an
  -- API-key entry's is always manual.
  rotation             TEXT NOT NULL DEFAULT 'manual'
    CHECK (rotation IN ('manual', 'in_order', 'balanced')
      AND (credential_kind = 'subscription' OR rotation = 'manual')),
  CHECK ((credential_kind = 'api_key') = (config IS NOT NULL)),
  CHECK (credential_kind = 'subscription' OR active_credential_id IS NULL)
) STRICT;
CREATE INDEX providers_owner ON providers (owner_user_id, created_at);
CREATE UNIQUE INDEX providers_one_subscription ON providers (owner_user_id, provider_type)
  WHERE credential_kind = 'subscription';

CREATE TABLE provider_credentials (
  provider_id  TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  id           TEXT NOT NULL,
  identity_key TEXT,
  label        TEXT NOT NULL,
  detail       TEXT,
  source       TEXT NOT NULL,
  -- The sealed secret document; none for a Claude Code account, whose
  -- sign-in its CLI keeps (providers.md § Subscription secrets).
  secret       BLOB,
  version      INTEGER NOT NULL CHECK (version >= 1),
  -- A quota snapshot, format 2, whose windows have a kind (format 1 named
  -- each window in English).
  quota        TEXT,
  updated_at   INTEGER NOT NULL,
  -- The account's place in its entry's list, which In Order follows.
  position     INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0),
  -- The vendor refused the account's sign-in; a login of it clears this.
  signed_out   INTEGER NOT NULL DEFAULT 0 CHECK (signed_out IN (0, 1)),
  PRIMARY KEY (provider_id, id)
) STRICT;
CREATE UNIQUE INDEX provider_credentials_identity ON provider_credentials (provider_id, identity_key)
  WHERE identity_key IS NOT NULL;

CREATE TABLE model_catalogs (
  provider_id TEXT PRIMARY KEY REFERENCES providers (id) ON DELETE CASCADE,
  record      TEXT NOT NULL
) STRICT;

-- Usage and attachments; neither holds attachment bytes.
-- One row per vendor call. A row written before the ledger kept a call's
-- agent, turn, purpose, account, ids, hour-long cache write, outcome, quota
-- and start keeps null for them; a call the vendor reported no counts for
-- has null counts.
-- usage_ledger.quota: [{ kind, periodSeconds, scope, usedPercent, resetsAt }], format 1
CREATE TABLE usage_ledger (
  id                    TEXT PRIMARY KEY,
  user_id               TEXT NOT NULL REFERENCES users (id),
  conversation_id       TEXT NOT NULL,
  -- Null for a request outside a session, such as a title.
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title', 'image')),
  provider_id           TEXT NOT NULL,
  -- Null for an API key.
  account_id            TEXT,
  model_id              TEXT NOT NULL,
  request_id            TEXT,
  vendor_id             TEXT,
  input_tokens          INTEGER,
  output_tokens         INTEGER,
  cache_read_tokens     INTEGER,
  cache_write_tokens    INTEGER,
  cache_write_1h_tokens INTEGER,
  -- 'answered', 'cancelled' or the failure code of a refused call.
  outcome               TEXT,
  quota                 TEXT,
  started_at            INTEGER,
  ended_at              INTEGER NOT NULL
) STRICT;
CREATE INDEX usage_ledger_user ON usage_ledger (user_id, ended_at);

CREATE TABLE attachments (
  id         TEXT PRIMARY KEY,
  user_id    TEXT NOT NULL REFERENCES users (id),
  media_type TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  sha256     TEXT NOT NULL,
  snippet    TEXT,
  created_at INTEGER NOT NULL
) STRICT;
