-- A core's own records (`crates/core-store/src/store.rs`): those of the one
-- principal the core serves, the user of the server whose directory it is,
-- or the program that started it. No row names its principal: the file
-- does.

CREATE TABLE user_preferences (
  id     INTEGER PRIMARY KEY CHECK (id = 1),
  preferences TEXT NOT NULL
) STRICT;

CREATE TABLE user_instructions (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  text    TEXT NOT NULL
) STRICT;

CREATE TABLE user_subagents (
  id             INTEGER PRIMARY KEY CHECK (id = 1),
  enabled             INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  max_live            INTEGER NOT NULL CHECK (max_live >= 1),
  max_live_unlimited  INTEGER NOT NULL CHECK (max_live_unlimited IN (0, 1)),
  max_depth           INTEGER NOT NULL CHECK (max_depth >= 1),
  max_depth_unlimited INTEGER NOT NULL CHECK (max_depth_unlimited IN (0, 1))
) STRICT;

CREATE TABLE subagent_profiles (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  description  TEXT NOT NULL,
  model        TEXT,
  instructions TEXT,
  can_spawn    INTEGER NOT NULL CHECK (can_spawn IN (0, 1)),
  enabled      INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  -- The profile's own Context in tokens, its model's full window or a
  -- limit; null for the user's limit, and always without a model.
  context_limit INTEGER CHECK (context_limit >= 1),
  UNIQUE (name)
) STRICT;

CREATE TABLE devices (
  id           TEXT PRIMARY KEY,
  kind         TEXT NOT NULL CHECK (kind IN ('user', 'managed')),
  name         TEXT NOT NULL,
  platform     TEXT NOT NULL,
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
  home TEXT,
  -- Why the device will not start its runner by itself after a restart, as
  -- its runner last reported it, JSON, autostart reason format 1, the
  -- runner wire's `AutostartUnavailable` (runner.md § Starting with the
  -- computer); none when it does, or before its runner first said.
  autostart_unavailable TEXT
) STRICT;

CREATE UNIQUE INDEX devices_one_machine ON devices (machine) WHERE kind = 'managed';

CREATE TABLE workspaces (
  id         TEXT PRIMARY KEY,
  name       TEXT CHECK (name <> ''),
  icon       TEXT NOT NULL,
  color      TEXT NOT NULL,
  sort_order INTEGER NOT NULL,
  created_at INTEGER NOT NULL
) STRICT;

CREATE INDEX workspaces_order ON workspaces (sort_order);

CREATE TABLE projects (
  id         TEXT PRIMARY KEY,
  device_id  TEXT NOT NULL REFERENCES devices (id),
  path       TEXT NOT NULL,
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
    CHECK ((last_workflow_kind IS 'workflow') = (last_workflow_id IS NOT NULL)),
  -- Whose list a message saved for later in one of its conversations joins:
  -- the conversation's own, or the project's, which all of them share.
  saved_scope TEXT NOT NULL DEFAULT 'conversation' CHECK (saved_scope IN ('conversation', 'project')),
  -- When the user deleted it; null for a project in the sidebar.
  deleted_at INTEGER,
  workspace_id TEXT NOT NULL REFERENCES workspaces (id)
) STRICT;

CREATE INDEX projects_order ON projects (sort_order);

CREATE UNIQUE INDEX projects_directory ON projects (workspace_id, device_id, path);

CREATE UNIQUE INDEX projects_of_workspace ON projects (id, workspace_id);

CREATE TABLE directory_worktrees (
  device_id     TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  path          TEXT NOT NULL,
  name          TEXT NOT NULL,
  worktree_path TEXT NOT NULL,
  PRIMARY KEY (device_id, path, name)
) STRICT;

CREATE INDEX directory_worktrees_path ON directory_worktrees (device_id, worktree_path);

CREATE TABLE port_forwards (
  id                TEXT PRIMARY KEY,
  listen_device_id  TEXT NOT NULL REFERENCES devices (id),
  listen_port       INTEGER NOT NULL CHECK (listen_port BETWEEN 1 AND 65535),
  connect_device_id TEXT REFERENCES devices (id),
  connect_address   TEXT NOT NULL,
  enabled           INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  created_at        INTEGER NOT NULL
) STRICT;

CREATE INDEX port_forwards_order ON port_forwards (created_at);

CREATE TABLE host_workflows (
  device_id TEXT NOT NULL,
  directory TEXT NOT NULL,
  list      TEXT NOT NULL,
  read_at   INTEGER NOT NULL,
  revision  INTEGER NOT NULL CHECK (revision >= 1),
  PRIMARY KEY (device_id, directory)
) STRICT;

CREATE TABLE user_plugins (
  plugin  TEXT NOT NULL,
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  PRIMARY KEY (plugin)
) STRICT, WITHOUT ROWID;

CREATE TABLE user_plugin_features (
  plugin  TEXT NOT NULL,
  feature TEXT NOT NULL,
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  PRIMARY KEY (plugin, feature)
) STRICT, WITHOUT ROWID;

CREATE TABLE plugin_values (
  plugin   TEXT NOT NULL,
  key      TEXT NOT NULL,
  document TEXT NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  blobs    TEXT NOT NULL,
  PRIMARY KEY (plugin, key)
) STRICT, WITHOUT ROWID;

CREATE TABLE plugin_directories (
  plugin       TEXT NOT NULL,
  name         TEXT NOT NULL,
  digest       TEXT NOT NULL,
  files        TEXT NOT NULL,
  workspace_id TEXT REFERENCES workspaces (id),
  PRIMARY KEY (plugin, name)
) STRICT, WITHOUT ROWID;

CREATE TABLE plugin_secrets (
  plugin   TEXT NOT NULL,
  name     TEXT NOT NULL,
  sealed   BLOB NOT NULL,
  revision INTEGER NOT NULL CHECK (revision >= 1),
  PRIMARY KEY (plugin, name)
) STRICT, WITHOUT ROWID;

CREATE TABLE plugin_oauth_clients (
  plugin       TEXT NOT NULL,
  issuer       TEXT NOT NULL,
  redirect_uri TEXT NOT NULL,
  client_id    TEXT NOT NULL,
  PRIMARY KEY (plugin, issuer)
) STRICT, WITHOUT ROWID;

CREATE TABLE user_credentials (
  id           TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  note         TEXT NOT NULL,
  fields       TEXT NOT NULL,
  sealed       BLOB NOT NULL,
  keep_until   INTEGER,
  last_used_at INTEGER,
  created_at   INTEGER NOT NULL,
  updated_at   INTEGER NOT NULL,
  UNIQUE (name)
) STRICT;

CREATE INDEX user_credentials_by_expiry ON user_credentials (keep_until) WHERE keep_until IS NOT NULL;

CREATE TABLE schedules (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  name             TEXT NOT NULL,
  trigger          TEXT NOT NULL,
  time_zone        TEXT NOT NULL,
  max_runs_per_day INTEGER CHECK (max_runs_per_day > 0),
  if_busy          TEXT NOT NULL CHECK (if_busy IN ('queue', 'steer')),
  prompt           TEXT NOT NULL,
  paused           INTEGER NOT NULL CHECK (paused IN (0, 1)),
  since            INTEGER NOT NULL,
  secret           BLOB,
  watch_stdout     BLOB,
  watch_digest     BLOB,
  watch_exit       INTEGER,
  watch_checked_at INTEGER,
  created_at       INTEGER NOT NULL,
  updated_at       INTEGER NOT NULL
) STRICT;

CREATE INDEX schedules_by_conversation ON schedules (conversation_id, created_at);

CREATE TABLE schedule_runs (
  id          TEXT PRIMARY KEY,
  schedule_id TEXT NOT NULL REFERENCES schedules (id) ON DELETE CASCADE,
  cause       TEXT NOT NULL CHECK (cause IN ('time', 'change', 'url', 'now')),
  at          INTEGER NOT NULL,
  state       TEXT NOT NULL CHECK (state IN ('running', 'done', 'notified', 'skipped', 'failed')),
  note        TEXT,
  line        TEXT,
  turn_id     TEXT,
  notified    INTEGER NOT NULL CHECK (notified IN (0, 1)),
  task_id     TEXT
) STRICT;

CREATE INDEX schedule_runs_by_schedule ON schedule_runs (schedule_id, at);

CREATE TABLE schedule_calls (
  id          INTEGER PRIMARY KEY,
  schedule_id TEXT NOT NULL REFERENCES schedules (id) ON DELETE CASCADE,
  at          INTEGER NOT NULL,
  delivery_id TEXT,
  state       TEXT NOT NULL CHECK (state IN ('pending', 'taken', 'held')),
  method      TEXT,
  headers     TEXT,
  body        BLOB,
  CHECK ((state = 'pending') = (method IS NOT NULL AND headers IS NOT NULL AND body IS NOT NULL))
) STRICT;

CREATE INDEX schedule_calls_by_state ON schedule_calls (schedule_id, state, id);

CREATE INDEX schedule_calls_by_delivery ON schedule_calls (schedule_id, delivery_id)
  WHERE delivery_id IS NOT NULL;

CREATE TABLE conversations (
  id                  TEXT PRIMARY KEY COLLATE NOCASE,
  -- title: empty exactly while title_origin is 'placeholder', a title that
  -- holds no words (conversations.md § Conversation titles).
  title               TEXT NOT NULL,
  title_origin        TEXT NOT NULL
                        CHECK (title_origin IN ('placeholder', 'message', 'generated', 'user')),
  -- When it was archived; none while it is not.
  archived_at         INTEGER,
  pinned              INTEGER NOT NULL CHECK (pinned IN (0, 1)),
  sort_order          INTEGER NOT NULL,
  read_revision       INTEGER NOT NULL,
  -- target: where it runs (sessions-and-targets.md § Resolve a target): the
  -- Cloud's directory of a conversation, its own without a path; a Host's
  -- home, by its device; a project, with the device and directory it
  -- names; or a device's directory as a release before projects by
  -- directory kept it, until that device's runner first reports its home,
  -- and of a conversation whose device was revoked.
  target_kind         TEXT NOT NULL CHECK (target_kind IN ('cloud', 'home', 'project', 'device')),
  target_device_id    TEXT,
  target_path         TEXT,
  target_project_id   TEXT,
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
  -- Its Workflow menu choice (subagents.md § Workflows of a project): the
  -- user's settings, No Subagents, or the workflow whose file is
  -- .demi/workflows/<workflow_id>.yaml.
  workflow_kind TEXT NOT NULL DEFAULT 'default' CHECK (workflow_kind IN ('default', 'none', 'workflow')),
  workflow_id   TEXT CHECK ((workflow_kind = 'workflow') = (workflow_id IS NOT NULL)),
  last_switch_revision INTEGER,
  -- The linked worktree of its project it works in, by git's name and its
  -- directory, in place of the project's directory; both null outside one
  -- (`crates/backend-database/src/selection.rs` § Worktrees).
  target_worktree_name TEXT,
  target_worktree_path TEXT
    CHECK ((target_worktree_name IS NULL) = (target_worktree_path IS NULL)
      AND (target_worktree_path IS NULL OR target_kind = 'project')),
  -- Whether its workflow is the worktree's file rather than the project's.
  workflow_worktree INTEGER NOT NULL DEFAULT 0
    CHECK (workflow_worktree IN (0, 1) AND (workflow_worktree = 0 OR workflow_kind = 'workflow')),
  -- The card that asks the user to turn notifications on, once its agent
  -- found that no browser takes them: shown, dismissed for good, or null
  -- (`crates/backend-database/src/push.rs` § The notifications card).
  notifications_card TEXT CHECK (notifications_card IN ('shown', 'dismissed')),
  -- The workspace it is in: its project's, when it is in one.
  workspace_id TEXT NOT NULL REFERENCES workspaces (id),
  CHECK (
    (target_kind = 'cloud'
      AND target_device_id IS NULL AND target_project_id IS NULL)
    OR (target_kind = 'home'
      AND target_device_id IS NOT NULL AND target_path IS NULL AND target_project_id IS NULL)
    OR (target_kind = 'device'
      AND target_device_id IS NOT NULL AND target_path IS NOT NULL AND target_project_id IS NULL)
    OR (target_kind = 'project'
      AND target_project_id IS NOT NULL AND target_device_id IS NOT NULL AND target_path IS NOT NULL)
  ),
  -- A project's move to another workspace takes its conversations along.
  FOREIGN KEY (target_project_id, workspace_id) REFERENCES projects (id, workspace_id) ON UPDATE CASCADE
) STRICT;

CREATE INDEX conversations_sidebar ON conversations (archived_at, pinned, sort_order);

CREATE INDEX conversations_project ON conversations (target_project_id);

CREATE INDEX conversations_of_workspace ON conversations (workspace_id);

CREATE TABLE running_jobs (
  job_id          TEXT PRIMARY KEY,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  sent            INTEGER NOT NULL DEFAULT 1 CHECK (sent IN (0, 1))
) STRICT;

CREATE INDEX running_jobs_of_device ON running_jobs (device_id);

CREATE TABLE conversation_hosts (
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id),
  device_id       TEXT NOT NULL REFERENCES devices (id),
  -- The directory its commands start in, fixed when it was attached; null
  -- for a device attached by name, whose commands start in its home.
  cwd             TEXT,
  attached_at     INTEGER NOT NULL,
  PRIMARY KEY (conversation_id, device_id)
) STRICT;

CREATE INDEX conversation_hosts_device ON conversation_hosts (device_id);

CREATE TABLE conversation_panels (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CHECK (revision >= 1),
  document        TEXT NOT NULL,
  updated_at      INTEGER NOT NULL
) STRICT;

CREATE TABLE plugin_reports (
  plugin_id       TEXT NOT NULL,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  device_id       TEXT NOT NULL REFERENCES devices (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CHECK (revision >= 0),
  value           TEXT NOT NULL,
  reported_at     INTEGER NOT NULL,
  PRIMARY KEY (plugin_id, conversation_id, device_id)
) STRICT;

CREATE TABLE conversation_drafts (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  revision        INTEGER NOT NULL CHECK (revision >= 1),
  document        TEXT NOT NULL,
  saved_revision  INTEGER NOT NULL CHECK (saved_revision >= 0),
  updated_at      INTEGER NOT NULL
) STRICT;

CREATE TABLE conversation_suggested_replies (
  id              INTEGER PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  block_id        TEXT NOT NULL,
  answered_at     INTEGER NOT NULL,
  -- The small model that answered, a hash of the instruction it read and its
  -- whole answer; null on a row a release before them kept.
  provider_id     TEXT,
  model           TEXT,
  instruction_hash TEXT,
  response        TEXT,
  replies         TEXT NOT NULL,
  -- The reply the user last took, by its place in replies, and how.
  used_reply      INTEGER,
  used_as         TEXT CHECK (used_as IN ('sent', 'edited')),
  used_at         INTEGER
) STRICT;

CREATE INDEX conversation_suggested_replies_latest ON conversation_suggested_replies (conversation_id, id);

CREATE TABLE saved_messages (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  project_id      TEXT REFERENCES projects (id) ON DELETE CASCADE,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL,
  CHECK ((conversation_id IS NULL) != (project_id IS NULL))
) STRICT;

CREATE INDEX saved_messages_of_conversation ON saved_messages (conversation_id, saved_at);

CREATE INDEX saved_messages_of_project ON saved_messages (project_id, saved_at);

CREATE TABLE saved_sends (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  turn_id         TEXT NOT NULL,
  saved_at        INTEGER NOT NULL,
  document        TEXT NOT NULL,
  sent_at         INTEGER NOT NULL
) STRICT;

CREATE INDEX saved_sends_by_age ON saved_sends (sent_at);

CREATE TABLE permission_requests (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind              TEXT NOT NULL CHECK (kind IN ('permission', 'credential')),
  categories        TEXT,
  credential_name   TEXT,
  ask               TEXT,
  command           TEXT NOT NULL,
  node_id           TEXT NOT NULL,
  agent_number      INTEGER,
  agent_description TEXT,
  created_at        INTEGER NOT NULL,
  -- 'reached', another conversation's message granting the category, only releases up to
  -- 0.1.5960 decided, and the migration from it removed their requests.
  decision          TEXT CHECK (decision IN ('allowed', 'denied', 'reached')),
  decided_at        INTEGER,
  read              INTEGER NOT NULL DEFAULT 0 CHECK (read IN (0, 1)),
  CHECK ((kind = 'permission') = (categories IS NOT NULL)),
  CHECK ((kind = 'permission') = (credential_name IS NULL)),
  CHECK ((kind = 'credential') = (ask IS NOT NULL)),
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

CREATE TABLE interrupted_turns (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE
) STRICT;

CREATE TABLE idle_compactions (
  conversation_id TEXT PRIMARY KEY COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  due_at          INTEGER NOT NULL,
  lifetime_ms     INTEGER NOT NULL CHECK (lifetime_ms > 0),
  carried         INTEGER NOT NULL CHECK (carried > 0)
) STRICT;

CREATE TABLE conversation_deletions (
  id         TEXT PRIMARY KEY COLLATE NOCASE,
  deleted_at INTEGER NOT NULL
) STRICT;

CREATE TABLE conversation_fork_operations (
  id        TEXT PRIMARY KEY COLLATE NOCASE,
  source_id TEXT NOT NULL,
  block_id  TEXT NOT NULL,
  metadata  TEXT NOT NULL
) STRICT;

CREATE TABLE providers (
  id                   TEXT PRIMARY KEY,
  provider_type        TEXT NOT NULL,
  credential_kind      TEXT NOT NULL CHECK (credential_kind IN ('api_key', 'subscription')),
  label                TEXT NOT NULL,
  config               BLOB,
  active_credential_id TEXT,
  created_at           INTEGER NOT NULL,
  -- Which account each agent uses (providers.md § Account rotation); an
  -- API-key entry's is always manual.
  rotation             TEXT NOT NULL DEFAULT 'manual'
    CHECK (rotation IN ('manual', 'in_order', 'smart')
      AND (credential_kind = 'subscription' OR rotation = 'manual')),
  -- Whether the entry makes images when its provider can (providers.md
  -- § Provider management); on from the start.
  image_generation     INTEGER NOT NULL DEFAULT 1 CHECK (image_generation IN (0, 1)),
  -- The entry's place in its owner's list of Settings › Providers, which
  -- generated media try entries in (media.md § Which entry makes a call).
  position             INTEGER NOT NULL DEFAULT 0 CHECK (position >= 0),
  -- Whether the entry makes videos when its provider can (providers.md
  -- § Provider management); on from the start.
  video_generation     INTEGER NOT NULL DEFAULT 1 CHECK (video_generation IN (0, 1)),
  -- Whether the entry makes audio when its provider can; on from the
  -- start.
  audio_generation     INTEGER NOT NULL DEFAULT 1 CHECK (audio_generation IN (0, 1)),
  -- The prompt cache lifetime its requests ask for, whole minutes or hours
  -- such as '5m' or '1h'; null asks for none.
  cache_ttl            TEXT,
  -- Whether the entry's models use its vendor's own web search and X search
  -- when the vendor has them; on from the start.
  web_search           INTEGER NOT NULL DEFAULT 1 CHECK (web_search IN (0, 1)),
  x_search             INTEGER NOT NULL DEFAULT 1 CHECK (x_search IN (0, 1)),
  -- Whether those searches look at the images and watch the videos they
  -- find, when the vendor's can; on from the start.
  search_images        INTEGER NOT NULL DEFAULT 1 CHECK (search_images IN (0, 1)),
  search_videos        INTEGER NOT NULL DEFAULT 1 CHECK (search_videos IN (0, 1)),
  -- The owner's device the entry's requests go out from
  -- (crates/core/src/device_requests.rs); null for the
  -- server. It names no row, so a device removed leaves it, and the entry's
  -- requests fail until another is chosen. Which families may name one is
  -- theirs to say (crates/provider-registry/src/families.rs).
  through_device_id    TEXT,
  CHECK ((credential_kind = 'api_key') = (config IS NOT NULL)),
  CHECK (credential_kind = 'subscription' OR active_credential_id IS NULL)
) STRICT;

CREATE INDEX providers_order ON providers (position);

CREATE UNIQUE INDEX providers_one_subscription ON providers (provider_type)
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

CREATE TABLE retired_models (
  provider_id TEXT NOT NULL REFERENCES providers (id) ON DELETE CASCADE,
  key_digest  TEXT NOT NULL,
  model_id    TEXT NOT NULL,
  answered_by TEXT,
  retired_at  INTEGER NOT NULL,
  PRIMARY KEY (provider_id, key_digest, model_id)
) STRICT;

CREATE TABLE usage_ledger (
  id                    TEXT PRIMARY KEY,
  conversation_id       TEXT NOT NULL,
  -- Null for a request outside a session, such as a title.
  agent                 INTEGER CHECK (agent >= 0),
  turn_id               TEXT,
  purpose               TEXT CHECK (purpose IN ('turn', 'compaction', 'title', 'suggested_replies', 'image', 'video', 'audio')),
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
  ended_at              INTEGER NOT NULL,
  -- The seconds of video or audio the call made, as its file states them;
  -- null for any other call or when nothing was made.
  media_seconds         REAL CHECK (media_seconds >= 0),
  -- What the vendor said the call cost, in the currency it names, never
  -- converted; null when it said nothing in a documented unit.
  cost                  REAL CHECK (cost >= 0),
  cost_currency         TEXT CHECK (cost_currency IN ('usd', 'cny')),
  -- What each kind of the call's tokens would cost on the vendor's API, in
  -- dollars, fixed as the row is written; all four null when the model has
  -- no price for a kind the call used. Their reader refuses a row that
  -- sets some of them only, since a migration adds no table CHECK.
  api_price_input       REAL CHECK (api_price_input >= 0),
  api_price_cache_write REAL CHECK (api_price_cache_write >= 0),
  api_price_cache_read  REAL CHECK (api_price_cache_read >= 0),
  api_price_output      REAL CHECK (api_price_output >= 0),
  -- Hashes of the system prompt and of the tool definitions the request
  -- sent; null for a request outside a session.
  system_hash           TEXT,
  tools_hash            TEXT,
  -- A turn call's number among its agent's turn calls, from 1, whether its
  -- agent answered a compaction since its previous one, and why it missed
  -- the cache, judged as it is written by how much less of the cache it
  -- read than its previous call; null for every other row.
  call_number           INTEGER CHECK (call_number >= 1),
  after_compaction      INTEGER CHECK (after_compaction IN (0, 1)),
  miss                  TEXT CHECK (miss IN ('model_changed', 'tools_changed', 'system_prompt_changed', 'cache_expired', 'not_cached')),
  -- The SHA-256 of the video a video call made, in hex; null for any other
  -- call, and for a video call written before the ledger kept it.
  media_digest          TEXT,
  CHECK ((cost IS NULL) = (cost_currency IS NULL))
) STRICT;

CREATE INDEX usage_ledger_time ON usage_ledger (ended_at);

CREATE INDEX usage_ledger_media_digest ON usage_ledger (media_digest) WHERE media_digest IS NOT NULL;

CREATE INDEX usage_ledger_conversation ON usage_ledger (conversation_id, ended_at);

CREATE INDEX usage_ledger_agent ON usage_ledger (conversation_id, agent);

CREATE INDEX usage_ledger_cache_writes ON usage_ledger (conversation_id, agent, model_id)
  WHERE call_number IS NOT NULL AND cache_write_tokens > 0;

CREATE INDEX usage_ledger_turn ON usage_ledger (conversation_id, turn_id)
  WHERE turn_id IS NOT NULL;

CREATE TABLE usage_costs (
  conversation_id    TEXT NOT NULL,
  agent              INTEGER CHECK (agent >= 0),
  input_tokens       INTEGER NOT NULL CHECK (input_tokens >= 0),
  cache_write_tokens INTEGER NOT NULL CHECK (cache_write_tokens >= 0),
  cache_read_tokens  INTEGER NOT NULL CHECK (cache_read_tokens >= 0),
  output_tokens      INTEGER NOT NULL CHECK (output_tokens >= 0),
  price_total        REAL CHECK (price_total >= 0),
  price_input        REAL CHECK (price_input >= 0),
  price_cache_write  REAL CHECK (price_cache_write >= 0),
  price_cache_read   REAL CHECK (price_cache_read >= 0),
  price_output       REAL CHECK (price_output >= 0),
  calls              INTEGER NOT NULL CHECK (calls >= 0)
) STRICT;

CREATE UNIQUE INDEX usage_costs_by_agent ON usage_costs (conversation_id, ifnull(agent, -1));

CREATE TABLE media_jobs (
  call_id         TEXT NOT NULL,
  ordinal         INTEGER NOT NULL CHECK (ordinal >= 0),
  conversation_id TEXT NOT NULL COLLATE NOCASE REFERENCES conversations (id) ON DELETE CASCADE,
  kind            TEXT NOT NULL CHECK (kind IN ('image', 'video', 'audio')),
  vendor_job      TEXT NOT NULL,
  asked_at        INTEGER NOT NULL,
  usage           TEXT NOT NULL,
  follow          TEXT NOT NULL,
  status          TEXT NOT NULL CHECK (status IN ('running', 'made', 'failed', 'cancelled')),
  blob            TEXT,
  made            TEXT,
  refusal         TEXT,
  PRIMARY KEY (call_id, ordinal),
  CHECK ((status = 'made') = (blob IS NOT NULL AND made IS NOT NULL)),
  CHECK ((status = 'failed') = (refusal IS NOT NULL))
) STRICT;

CREATE INDEX media_jobs_of_conversation ON media_jobs (conversation_id);

CREATE INDEX media_jobs_running ON media_jobs (status) WHERE status = 'running';

CREATE TABLE attachments (
  id         TEXT PRIMARY KEY,
  media_type TEXT NOT NULL,
  size_bytes INTEGER NOT NULL,
  sha256     TEXT NOT NULL,
  snippet    TEXT,
  created_at INTEGER NOT NULL
) STRICT;
