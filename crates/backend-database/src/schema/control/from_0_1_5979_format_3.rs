//! From the control schema of provider configuration format 3, which no
//! release shipped: the media jobs vendors took are kept, in `media_jobs`.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(CONTROL_FROM_0_1_5979_FORMAT_3);

const CONTROL_FROM_0_1_5979_FORMAT_3: &str = "
-- The media jobs vendors took for the calls of `demi image`, `demi video`
-- and `demi audio`, each followed to its end across restarts
-- (`crates/backend-database/src/media_jobs.rs`): written with the vendor's
-- id of the job, ended with its file in the user's blobs or why it failed,
-- and deleted when its call collects it, or with its conversation. A call
-- the runner sends again after a restart finds its jobs by its id, in the
-- order it asked for them.
-- media_jobs.usage: the call's open usage ledger row, format 1
-- media_jobs.follow: { vendor, label, manages, answer }, format 1
-- media_jobs.made: what the call answers besides the file, format 1
-- media_jobs.refusal: the port's media refusal, format 1
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
-- The running jobs, which a backend that starts follows.
CREATE INDEX media_jobs_running ON media_jobs (status) WHERE status = 'running';
";
