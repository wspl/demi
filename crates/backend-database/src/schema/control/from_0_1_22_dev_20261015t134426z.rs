//! From 0.1.22-dev.20261015T134426Z's control schema, which a server keeps data
//! of, to the one whose quota snapshots (format 2) give each window a kind, which
//! with its period and its scope the page names it from in the user's language
//! (`crates/shared-types/src/quota.rs`), instead of the English name format 1
//! held.
//!
//! Each window's kind is the one its old name says, read by its vendor's old
//! namer ([`old_window_kind`]); a name that namer did not make, or a vendor
//! with none, fails the migration.

use rusqlite::Transaction;

use super::super::{Migration, ReadBlob, corrupt, corrupt_text};

pub(super) const MIGRATION: Migration = Migration::Code(snapshots_with_window_kinds);

fn snapshots_with_window_kinds(transaction: &Transaction<'_>, _: &dyn ReadBlob) -> rusqlite::Result<()> {
    // The shares made before a share held only what was said, each of
    // which held every block and its subagents, are removed, and their
    // links answer as ones that never existed; each conversation's database
    // drops their lists ([`switches_with_window_names`]).
    transaction.execute("DELETE FROM shares", [])?;
    let rows = transaction
        .prepare(
            "SELECT c.provider_id, c.id, c.quota, p.provider_type FROM provider_credentials c
               JOIN providers p ON p.id = c.provider_id WHERE c.quota IS NOT NULL",
        )?
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut update = transaction.prepare("UPDATE provider_credentials SET quota = ?3 WHERE provider_id = ?1 AND id = ?2")?;
    for (provider, id, text, provider_type) in rows {
        let namer = match provider_type.as_str() {
            "codex" => OldNamer::Codex,
            "claude-code" => OldNamer::ClaudeCode,
            "grok-build" => OldNamer::GrokBuild,
            other => return Err(corrupt_text(&format!("a {other} account kept a quota snapshot, which no namer made"))),
        };
        let mut snapshot: serde_json::Value = serde_json::from_str(&text).map_err(corrupt)?;
        let windows = snapshot
            .get_mut("windows")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| corrupt_text("a stored quota snapshot has no windows"))?;
        for window in windows {
            let fields = window
                .as_object_mut()
                .ok_or_else(|| corrupt_text("a stored quota window is not an object"))?;
            let label = match fields.remove("label") {
                Some(serde_json::Value::String(label)) => label,
                _ => return Err(corrupt_text("a stored quota window of format 1 has no name")),
            };
            let kind = old_window_kind(&label, namer)?;
            fields.insert("kind".to_owned(), kind.into());
        }
        update.execute(rusqlite::params![provider, id, snapshot.to_string()])?;
    }
    Ok(())
}

/// The vendor whose old namer made a window's English name, when the
/// stored row says it: a snapshot's provider type. An account switch's
/// block does not say it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OldNamer {
    Codex,
    ClaudeCode,
    GrokBuild,
}

/// The length Codex's old namer wrote as `count` of a unit of `unit`
/// seconds, `1.5` of a minute: its seconds, or none, as Codex stored it,
/// when they are not whole; `None` when `count` is no length it wrote,
/// not a positive number in Rust's own form or too long to keep.
fn codex_length(text: &str, unit: u64) -> Option<Option<u64>> {
    let count: f64 = text.parse().ok().filter(|count: &f64| count.is_finite() && *count > 0.0)?;
    // The namer wrote the quotient in Rust's shortest form, so a count in
    // any other form, `1.50` or `1e3`, is not one it wrote.
    if count.to_string() != text {
        return None;
    }
    let seconds = count * unit as f64;
    // Past 2^53 - 1, JavaScript's largest exact integer.
    if seconds > ((1_u64 << 53) - 1) as f64 {
        return None;
    }
    if seconds.fract() != 0.0 || seconds < 1.0 {
        return Some(None);
    }
    // Whole and below 2^53, so the conversion is exact.
    Some(Some(seconds as u64))
}

/// The kind of the window that `namer`'s old code named `label` in English,
/// before a window had a kind; a label it did not make is corrupt data,
/// which fails the migration.
/// - Codex: `Primary` or `Secondary` for an unknown length, `Weekly`,
///   `Daily`, or the length in the largest unit that divides it, `3-day`,
///   `5-hour`, `1.5-minute`.
/// - Claude Code: `5h session`, `7d all models`, `7d <model>`, `5h <model>`,
///   `Other limit`, and the model's name alone for a limit on one model
///   whose period it could not tell.
/// - Grok Build: `Weekly credits`, `Monthly credits`, `On-demand cap`, and
///   `Requests (short window)` and `Tokens (short window)`.
fn old_window_kind(label: &str, namer: OldNamer) -> rusqlite::Result<&'static str> {
    let unknown = || Err(corrupt_text(&format!("no namer made the quota window name {label:?}")));
    match namer {
        OldNamer::Codex => {
            if matches!(label, "Primary" | "Secondary" | "Weekly" | "Daily") {
                return Ok("limit");
            }
            let length = label.rsplit_once('-').and_then(|(count, unit)| {
                let unit = match unit {
                    "week" => 7 * 24 * 3600,
                    "day" => 24 * 3600,
                    "hour" => 3600,
                    "minute" => 60,
                    _ => return None,
                };
                codex_length(count, unit)
            });
            match length {
                Some(_) => Ok("limit"),
                None => unknown(),
            }
        }
        OldNamer::GrokBuild => match label {
            "Weekly credits" | "Monthly credits" => Ok("credits"),
            "On-demand cap" => Ok("cap"),
            "Requests (short window)" => Ok("requests"),
            "Tokens (short window)" => Ok("tokens"),
            _ => unknown(),
        },
        OldNamer::ClaudeCode if label.is_empty() => unknown(),
        OldNamer::ClaudeCode => Ok("limit"),
    }
}
