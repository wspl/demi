//! The media jobs vendors took, kept from the vendor's answer until the call
//! that asked for each collects what it made
//! (`crates/backend-user-shard/src/shard/media_jobs.rs`).
//!
//! A row is written with the job's id, before the call is told of the job;
//! it holds the call's open usage ledger row, which whoever follows the job
//! writes when the job ends. The end keeps the file in the user's blobs, or
//! why the job failed, until the call collects it and deletes the row. A row
//! no call collects goes with its conversation, and its file with it.
//!
//! A job's end and its usage row are one transaction, written only while
//! the row is running: a job two followers end, or a follower that the
//! backend's stop cuts short, records its cost once. A job whose
//! conversation was deleted records its cost all the same.

use demi_plugin_interface::{MadeAudio, MadeImage, MadeVideo, MediaRefusal};
use demi_shared_types::{AudioKind, BlobRef, Cost, MediaKind, Timestamp, TokenUsage};
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId, UserId};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, to_json};
use super::control::ControlService;
use super::usage::{ObservedWindow, UsageRow, UsageWritten, write_usage};

const TABLE: &str = "media_jobs";

/// How a running job ended: its `status`, and its call's usage row, ended.
/// A job no call collects any more, as one whose call was stopped, leaves
/// no row unless `kept`.
pub struct JobEnd {
    pub of: JobOfCall,
    pub status: MediaJobStatus,
    pub usage: UsageRow,
    pub kept: bool,
}

/// A job's place among the jobs of the call that asked for it, the first
/// asked 0.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JobOfCall {
    /// The id of the `rpc_call` that asked for it, which a call the runner
    /// sends again after a restart keeps.
    pub call: String,
    pub ordinal: u32,
}

/// A job a vendor took.
#[derive(Debug, Clone, PartialEq)]
pub struct MediaJob {
    pub of: JobOfCall,
    pub kind: MediaKind,
    /// The vendor's id, which every read of the job sends.
    pub job: String,
    pub asked_at: Timestamp,
    /// The call's open ledger row: the conversation, entry, account and
    /// model it was asked on, and what it observed so far.
    pub usage: UsageRow,
    pub follow: Follow,
    pub status: MediaJobStatus,
}

/// What a job's end says of who made it, besides its usage row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Follow {
    /// The maker's name, such as `Google AI Studio`.
    pub vendor: String,
    /// The account's label, for whoever manages the entry.
    pub label: Option<String>,
    pub manages: bool,
    /// What its call answers once it is made, as far as the ask knows it.
    pub answer: Made,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MediaJobStatus {
    Running,
    /// Made: its file in the user's blobs, and what its call answers
    /// besides.
    Made { blob: BlobRef, made: Made },
    Failed { refusal: MediaRefusal },
    /// Stopped by its call, and cancelled at the vendor where it cancels.
    Cancelled,
}

/// What a job's call answers, as the port answers it, its file's bytes
/// empty: the file is in the row's blob.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
pub enum Made {
    Image { image: MadeImage },
    Video { video: MadeVideo },
    /// Of `audio`, the kind it was asked as, which says which of the
    /// vendor's requests makes it.
    Audio { audio: MadeAudio, of: AudioKind },
}

impl Made {
    /// The kind of media it is.
    pub fn kind(&self) -> MediaKind {
        match self {
            Self::Image { .. } => MediaKind::Image,
            Self::Video { .. } => MediaKind::Video,
            Self::Audio { .. } => MediaKind::Audio,
        }
    }
}

/// A usage row as `media_jobs.usage` keeps it, format 1.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct KeptUsage {
    user: UserId,
    conversation: ConversationId,
    agent: Option<u64>,
    turn: Option<String>,
    provider: ProviderId,
    account: Option<CredentialId>,
    model: String,
    request: String,
    vendor_id: Option<String>,
    usage: Option<TokenUsage>,
    quota: Option<Vec<ObservedWindow>>,
    started_at: Timestamp,
    media_seconds: Option<f64>,
    cost: Option<Cost>,
    media_digest: Option<String>,
}

impl KeptUsage {
    fn of(row: &UsageRow) -> Self {
        Self {
            user: row.user.clone(),
            conversation: row.conversation.clone(),
            agent: row.agent,
            turn: row.turn.clone(),
            provider: row.provider.clone(),
            account: row.account.clone(),
            model: row.model.clone(),
            request: row.request.clone(),
            vendor_id: row.vendor_id.clone(),
            usage: row.usage,
            quota: row.quota.clone(),
            started_at: row.started_at,
            media_seconds: row.media_seconds,
            cost: row.cost,
            media_digest: row.media_digest.clone(),
        }
    }

    /// The open row of a call of `kind`, cancelled until its end says.
    fn row(self, kind: MediaKind) -> UsageRow {
        UsageRow {
            user: self.user,
            conversation: self.conversation,
            agent: self.agent,
            turn: self.turn,
            purpose: purpose(kind),
            provider: self.provider,
            account: self.account,
            model: self.model,
            request: self.request,
            vendor_id: self.vendor_id,
            usage: self.usage,
            cache_write_1h_tokens: 0,
            outcome: "cancelled".to_owned(),
            quota: self.quota,
            started_at: self.started_at,
            ended_at: self.started_at,
            media_seconds: self.media_seconds,
            cost: self.cost,
            media_digest: self.media_digest,
            price: None,
            system_hash: None,
            tools_hash: None,
        }
    }
}

/// The ledger's purpose of a call of `kind`, which is also the kind's
/// stored name.
fn purpose(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
    }
}

impl ControlService {
    /// Keeps `job`, running. A conversation that is gone gets no row, and
    /// false.
    pub async fn insert_media_job(&self, job: MediaJob) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let inserted = connection.execute(
                "INSERT INTO media_jobs (call_id, ordinal, conversation_id, kind, vendor_job, asked_at, usage, follow, status)
                   SELECT ?1, ?2, id, ?4, ?5, ?6, ?7, ?8, 'running' FROM conversations WHERE id = ?3",
                params![
                    job.of.call,
                    job.of.ordinal,
                    job.usage.conversation.as_str(),
                    purpose(job.kind),
                    job.job,
                    job.asked_at.as_millisecond(),
                    to_json(&KeptUsage::of(&job.usage)),
                    to_json(&job.follow),
                ],
            )?;
            Ok(inserted == 1)
        })
        .await
    }

    /// The job `of` names; none once collected, or never asked.
    pub async fn media_job(&self, of: JobOfCall) -> Result<Option<MediaJob>, StorageError> {
        self.call(move |connection, _| {
            connection
                .query_row(
                    "SELECT * FROM media_jobs WHERE call_id = ?1 AND ordinal = ?2",
                    params![of.call, of.ordinal],
                    |row| Ok(read_row(row)),
                )
                .optional()?
                .transpose()
        })
        .await
    }

    /// `owner`'s running jobs, oldest first.
    pub async fn running_media_jobs(&self, owner: UserId) -> Result<Vec<MediaJob>, StorageError> {
        self.call(move |connection, _| {
            let mut statement = connection.prepare(
                "SELECT media_jobs.* FROM media_jobs
                   JOIN conversations ON conversations.id = media_jobs.conversation_id
                   WHERE conversations.user_id = ?1 AND status = 'running'
                   ORDER BY asked_at",
            )?;
            let mut rows = statement.query([owner.as_str()])?;
            let mut jobs = Vec::new();
            while let Some(row) = rows.next()? {
                jobs.push(read_row(row)?);
            }
            Ok(jobs)
        })
        .await
    }

    /// The owners of the running jobs.
    pub async fn media_job_owners(&self) -> Result<Vec<UserId>, StorageError> {
        self.call(|connection, _| {
            let mut statement = connection.prepare(
                "SELECT DISTINCT conversations.user_id FROM media_jobs
                   JOIN conversations ON conversations.id = media_jobs.conversation_id
                   WHERE status = 'running'
                   ORDER BY conversations.user_id",
            )?;
            let mut rows = statement.query([])?;
            let mut owners = Vec::new();
            while let Some(row) = rows.next()? {
                owners.push(decode("conversations", "user_id", UserId::try_from(row.get::<_, String>(0)?))?);
            }
            Ok(owners)
        })
        .await
    }

    /// Ends the running job as `end` says, with its usage row; false, and
    /// no usage row, when it ended already.
    pub async fn end_media_job(&self, end: JobEnd) -> Result<bool, StorageError> {
        let JobEnd { of, status, usage, kept } = end;
        let (user, conversation, agent) = (usage.user.clone(), usage.conversation.clone(), usage.agent);
        let written = self
            .call(move |connection, _| {
                let (name, blob, made, refusal) = match &status {
                    MediaJobStatus::Running => return Ok(None),
                    MediaJobStatus::Made { blob, made } => ("made", Some(blob.to_string()), Some(to_json(made)), None),
                    MediaJobStatus::Failed { refusal } => ("failed", None, None, Some(to_json(refusal))),
                    MediaJobStatus::Cancelled => ("cancelled", None, None, None),
                };
                let transaction = connection.transaction()?;
                let ended = if kept {
                    transaction.execute(
                        "UPDATE media_jobs SET status = ?3, blob = ?4, made = ?5, refusal = ?6
                           WHERE call_id = ?1 AND ordinal = ?2 AND status = 'running'",
                        params![of.call, of.ordinal, name, blob, made, refusal],
                    )?
                } else {
                    transaction.execute(
                        "DELETE FROM media_jobs WHERE call_id = ?1 AND ordinal = ?2 AND status = 'running'",
                        params![of.call, of.ordinal],
                    )?
                };
                // A job whose conversation was deleted, and its row with it,
                // still records what it cost.
                let gone = ended == 0
                    && !transaction
                        .prepare_cached("SELECT EXISTS (SELECT 1 FROM conversations WHERE id = ?1)")?
                        .query_row([usage.conversation.as_str()], |row| row.get::<_, bool>(0))?;
                if ended != 1 && !gone {
                    return Ok(None);
                }
                let (_, written) = write_usage(&transaction, &usage)?;
                transaction.commit()?;
                Ok(Some(written))
            })
            .await?;
        let Some(written) = written else {
            return Ok(false);
        };
        self.announce_usage(UsageWritten { user, conversation, agent, cost: written.cost, chart: written.chart, first: written.first });
        Ok(true)
    }

    /// Deletes the job `of` once it ended, as its call collects it or is
    /// stopped; a running job's row stays for its follower's end.
    pub async fn delete_media_job(&self, of: JobOfCall) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM media_jobs WHERE call_id = ?1 AND ordinal = ?2 AND status <> 'running'",
                params![of.call, of.ordinal],
            )?;
            Ok(())
        })
        .await
    }
}

/// The files of `owner`'s finished jobs, which their rows keep.
pub(crate) fn media_job_blobs(connection: &rusqlite::Connection, owner: &UserId) -> Result<Vec<BlobRef>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT media_jobs.blob FROM media_jobs
           JOIN conversations ON conversations.id = media_jobs.conversation_id
           WHERE conversations.user_id = ?1 AND media_jobs.blob IS NOT NULL",
    )?;
    let mut rows = statement.query([owner.as_str()])?;
    let mut blobs = Vec::new();
    while let Some(row) = rows.next()? {
        blobs.push(decode(TABLE, "blob", BlobRef::try_from(row.get::<_, String>(0)?))?);
    }
    Ok(blobs)
}

fn corrupt(column: &'static str, reason: String) -> StorageError {
    StorageError::Corrupt { table: TABLE, column, reason }
}

fn read_row(row: &Row<'_>) -> Result<MediaJob, StorageError> {
    let kind = match row.get::<_, String>("kind")?.as_str() {
        "image" => MediaKind::Image,
        "video" => MediaKind::Video,
        "audio" => MediaKind::Audio,
        other => return Err(corrupt("kind", format!("unknown kind {other}"))),
    };
    let usage: KeptUsage = decode(TABLE, "usage", serde_json::from_str(&row.get::<_, String>("usage")?))?;
    let follow = decode(TABLE, "follow", serde_json::from_str(&row.get::<_, String>("follow")?))?;
    let status = match row.get::<_, String>("status")?.as_str() {
        "running" => MediaJobStatus::Running,
        "made" => MediaJobStatus::Made {
            blob: decode(TABLE, "blob", BlobRef::try_from(row.get::<_, String>("blob")?))?,
            made: decode(TABLE, "made", serde_json::from_str(&row.get::<_, String>("made")?))?,
        },
        "failed" => MediaJobStatus::Failed {
            refusal: decode(TABLE, "refusal", serde_json::from_str(&row.get::<_, String>("refusal")?))?,
        },
        "cancelled" => MediaJobStatus::Cancelled,
        other => return Err(corrupt("status", format!("unknown status {other}"))),
    };
    let ordinal: i64 = row.get("ordinal")?;
    Ok(MediaJob {
        of: JobOfCall { call: row.get("call_id")?, ordinal: decode(TABLE, "ordinal", u32::try_from(ordinal))? },
        kind,
        job: row.get("vendor_job")?,
        asked_at: instant(row, TABLE, "asked_at")?,
        usage: usage.row(kind),
        follow,
        status,
    })
}
