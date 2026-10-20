//! The users' push subscriptions: one per session, which it serves.
//!
//! A subscription holds its push service's endpoint, the browser's keys and the
//! events it chose. A session's end deletes its subscription, and one whose
//! session expired is not sent to. Each is decoded as it is read, and encoded only
//! as it is written.

use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::push::{BrowserAuth, BrowserKey, PushEndpoint, PushEvents, PushSubscriptionKeys};
use demi_web_api_protocol::sessions::SessionId;
use rusqlite::{OptionalExtension as _, Row, params};

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;

/// A browser's subscription as it is kept: the session it serves, one per
/// session, its endpoint, its keys and the events it chose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredPushSubscription {
    pub session: SessionId,
    pub endpoint: PushEndpoint,
    pub keys: PushSubscriptionKeys,
    pub events: PushEvents,
}

/// What became of a subscription a session puts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutPush {
    /// The session's subscription is it now.
    Kept,
    /// Another live session's subscription has its endpoint, which stays
    /// that session's.
    Taken,
}

const COLUMNS: &str = "p.session_id, p.endpoint, p.p256dh, p.auth, p.turn_finishes, p.turn_fails,
     p.needs_permission, p.agent_notifies";

impl ControlService {
    /// Keeps `subscription` for `user`'s session, replacing what the session
    /// had, unless another session that has not expired holds its endpoint;
    /// an expired one's gives way.
    pub async fn put_push_subscription(
        &self,
        user: UserId,
        subscription: StoredPushSubscription,
    ) -> Result<PutPush, StorageError> {
        self.call(move |connection, now| {
            let StoredPushSubscription {
                session,
                endpoint,
                keys,
                events,
            } = subscription;
            let transaction = connection.transaction()?;
            let holder: Option<(String, Option<i64>)> = transaction
                .query_row(
                    "SELECT p.session_id, s.expires_at FROM push_subscriptions p
                       LEFT JOIN web_sessions s ON s.id = p.session_id
                      WHERE p.endpoint = ?1",
                    params![endpoint.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            if let Some((holder, expires_at)) = holder
                && holder != session.as_str()
            {
                if expires_at.is_some_and(|expires_at| expires_at > now.as_millisecond()) {
                    return Ok(PutPush::Taken);
                }
                transaction.execute("DELETE FROM push_subscriptions WHERE session_id = ?1", params![holder])?;
            }
            transaction.execute(
                "INSERT INTO push_subscriptions (session_id, endpoint, user_id, p256dh, auth,
                     turn_finishes, turn_fails, needs_permission, agent_notifies, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT (session_id) DO UPDATE SET endpoint = excluded.endpoint,
                     p256dh = excluded.p256dh, auth = excluded.auth,
                     turn_finishes = excluded.turn_finishes, turn_fails = excluded.turn_fails,
                     needs_permission = excluded.needs_permission,
                     agent_notifies = excluded.agent_notifies,
                     created_at = CASE WHEN push_subscriptions.endpoint = excluded.endpoint
                       THEN push_subscriptions.created_at ELSE excluded.created_at END",
                params![
                    session.as_str(),
                    endpoint.as_str(),
                    user.as_str(),
                    String::from(keys.p256dh),
                    String::from(keys.auth),
                    events.turn_finishes,
                    events.turn_fails,
                    events.needs_permission,
                    events.agent_notifies,
                    now.as_millisecond(),
                ],
            )?;
            transaction.commit()?;
            Ok(PutPush::Kept)
        })
        .await
    }

    /// Removes the session's subscription, if it has one.
    pub async fn delete_push_subscription(&self, session: SessionId) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM push_subscriptions WHERE session_id = ?1",
                params![session.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Removes the session's subscription of `endpoint`, which its push
    /// service says is gone, unless the session subscribed anew since.
    pub async fn delete_gone_push_subscription(
        &self,
        session: SessionId,
        endpoint: PushEndpoint,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM push_subscriptions WHERE session_id = ?1 AND endpoint = ?2",
                params![session.as_str(), endpoint.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// `user`'s subscriptions whose session has not expired, oldest first.
    pub async fn push_subscriptions(&self, user: UserId) -> Result<Vec<StoredPushSubscription>, StorageError> {
        self.call(move |connection, now| {
            let mut statement = connection.prepare_cached(&format!(
                "SELECT {COLUMNS} FROM push_subscriptions p
                   JOIN web_sessions s ON s.id = p.session_id
                  WHERE p.user_id = ?1 AND s.expires_at > ?2
                  ORDER BY p.created_at, p.session_id"
            ))?;
            let rows = statement.query_map(params![user.as_str(), now.as_millisecond()], |row| Ok(subscription(row)))?;
            rows.map(|row| row?).collect()
        })
        .await
    }
}

fn subscription(row: &Row<'_>) -> Result<StoredPushSubscription, StorageError> {
    let column = |name: &'static str| -> Result<String, StorageError> { Ok(row.get(name)?) };
    Ok(StoredPushSubscription {
        session: decode("push_subscriptions", "session_id", SessionId::try_from(column("session_id")?))?,
        endpoint: decode("push_subscriptions", "endpoint", PushEndpoint::try_from(column("endpoint")?))?,
        keys: PushSubscriptionKeys {
            p256dh: decode("push_subscriptions", "p256dh", BrowserKey::try_from(column("p256dh")?))?,
            auth: decode("push_subscriptions", "auth", BrowserAuth::try_from(column("auth")?))?,
        },
        events: PushEvents {
            turn_finishes: row.get("turn_finishes")?,
            turn_fails: row.get("turn_fails")?,
            needs_permission: row.get("needs_permission")?,
            agent_notifies: row.get("agent_notifies")?,
        },
    })
}
