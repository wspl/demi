//! From 0.1.5751's control schema: the users' choices of their plugins'
//! features. Web search, which a new user finds off, stays on for a user who
//! signed in to Exa or saved an Exa API key: a sign-in is its tokens, which
//! the plugin host keeps as the plugin's secret, or the plugin's own mark of
//! one, which it writes only once it reads them.

use super::super::Migration;

pub(super) const MIGRATION: Migration = Migration::Sql(
    "CREATE TABLE user_plugin_features (
       user_id TEXT NOT NULL REFERENCES users (id),
       plugin  TEXT NOT NULL,
       feature TEXT NOT NULL,
       enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
       PRIMARY KEY (user_id, plugin, feature)
     ) STRICT, WITHOUT ROWID;
     INSERT INTO user_plugin_features (user_id, plugin, feature, enabled)
       SELECT user_id, 'web', 'search', 1 FROM plugin_secrets
         WHERE plugin = 'web' AND name IN ('api-key', 'oauth:tokens')
       UNION
       SELECT user_id, 'web', 'search', 1 FROM plugin_values WHERE plugin = 'web' AND key = 'signed-in';",
);
