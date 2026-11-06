//! The databases' migrations: the crate's test binary. Every step of each
//! database's history, taken from an empty database of its schema, must
//! leave the database a new one of the current schema would be, so that a
//! server that upgraded and one installed afresh hold the same tables.

use demi_backend_database::DatabaseKind;
use demi_backend_database::schema::ControlSteps;
use demi_backend_database::testing::{current, history, migrate, migrate_control_to, version_of};
use demi_web_api_protocol::ids::ProviderId;
use rusqlite::Connection;

/// The last control schema that held every user's records, which the tests
/// of the steps before it read.
const SHARED: &str = include_str!("../../src/schema/control/from_0_1_65_one_core_per_user.sql");

#[test]
fn every_step_of_the_control_history_leads_to_the_current_schema() {
    every_step_leads_to_the_current_schema(DatabaseKind::Control);
}

#[test]
fn every_step_of_the_conversation_history_leads_to_the_current_schema() {
    every_step_leads_to_the_current_schema(DatabaseKind::Conversation);
}

#[test]
fn the_grants_and_requests_of_the_other_conversations_go() {
    let shipped = include_str!("../../src/schema/control/from_0_1_5960.sql");
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(shipped).unwrap();
    connection.pragma_update(None, "user_version", version_of(shipped)).unwrap();
    // The rows need no conversation of their own.
    connection.pragma_update(None, "foreign_keys", false).unwrap();
    connection
        .execute_batch(
            "INSERT INTO permission_grants VALUES ('c1', 'conversation.use', 1), ('c1', 'credentials.use', 1);
             INSERT INTO permission_requests (id, conversation_id, kind, categories, command, node_id, created_at, decision, decided_at) VALUES
               ('waits', 'c1', 'permission', '[\"conversation.use\"]', 'demi conversation list', 'n', 1, NULL, NULL),
               ('reached', 'c1', 'permission', '[\"conversation.use\"]', 'demi conversation send', 'n', 1, 'reached', 2),
               ('other', 'c1', 'permission', '[\"credentials.use\"]', 'demi credentials list', 'n', 1, NULL, NULL);",
        )
        .unwrap();
    migrate_control_to(&mut connection, SHARED, None).unwrap();
    let grants = query(&connection, "SELECT category FROM permission_grants", |row| row.get::<_, String>(0));
    let requests = query(&connection, "SELECT id FROM permission_requests", |row| row.get::<_, String>(0));
    assert_eq!((grants, requests), (vec!["credentials.use".to_owned()], vec!["other".to_owned()]));
}

#[test]
fn an_archived_conversation_takes_its_last_activity_as_its_archive_time() {
    let shipped = include_str!("../../src/schema/control/from_0_1_5979_port_forward_routes.sql");
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(shipped).unwrap();
    connection.pragma_update(None, "user_version", version_of(shipped)).unwrap();
    // The rows need no owner of their own.
    connection.pragma_update(None, "foreign_keys", false).unwrap();
    connection
        .execute_batch(
            "INSERT INTO conversations (id, user_id, workspace_id, title, title_origin, archived, pinned, sort_order, read_revision,
               target_kind, context_version, created_at, updated_at) VALUES
               ('active', 'u1', 'w1', 'Draft the release notes', 'user', 0, 0, 1, 0, 'cloud', 1, 1, 10),
               ('archived', 'u1', 'w1', 'Plan the launch', 'user', 1, 0, 2, 0, 'cloud', 1, 1, 20);",
        )
        .unwrap();
    migrate_control_to(&mut connection, SHARED, None).unwrap();
    let times = query(&connection, "SELECT id, archived_at FROM conversations ORDER BY id", |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
    });
    assert_eq!(times, vec![("active".to_owned(), None), ("archived".to_owned(), Some(20))]);
}

#[test]
fn a_panel_loses_its_widget_tabs_and_keeps_the_others() {
    let shipped = include_str!("../../src/schema/control/from_0_1_5979.sql");
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(shipped).unwrap();
    connection.pragma_update(None, "user_version", version_of(shipped)).unwrap();
    // The rows need no conversation of their own.
    connection.pragma_update(None, "foreign_keys", false).unwrap();
    connection
        .execute_batch(
            r#"INSERT INTO conversation_panels VALUES
                 ('c1', 4, '{"tabs":[{"id":"t1","kind":"file","data":{"path":"a.md"}},{"id":"t2","kind":"widget","data":{"id":"w1","title":"Orders"}}],"retired":["t0"]}', 1),
                 ('c2', 2, '{"tabs":[{"id":"t1","kind":"file","data":{}}],"retired":[]}', 1);
               INSERT INTO shares (token, user_id, conversation_id, kind, block_id, title, taken_at) VALUES
                 ('s1', 'u1', 'c1', 'answer', 'b1', 'Orders', 1);"#,
        )
        .unwrap();
    migrate_control_to(&mut connection, SHARED, None).unwrap();
    let panels = query(&connection, "SELECT conversation_id, revision, document FROM conversation_panels ORDER BY 1", |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?))
    });
    assert_eq!(
        panels,
        vec![
            ("c1".to_owned(), 5, r#"{"tabs":[{"id":"t1","kind":"file","data":{"path":"a.md"}}],"retired":["t0","t2"]}"#.to_owned()),
            ("c2".to_owned(), 2, r#"{"tabs":[{"id":"t1","kind":"file","data":{}}],"retired":[]}"#.to_owned()),
        ]
    );
    let shares = query(&connection, "SELECT token, kind, block_id FROM shares", |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
    });
    assert_eq!(shares, vec![("s1".to_owned(), "answer".to_owned(), "b1".to_owned())]);
}

#[test]
fn a_widget_keeps_its_title_in_place_of_its_file_name() {
    let shipped = include_str!("../../src/schema/control/from_0_1_5979.sql");
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(shipped).unwrap();
    connection.pragma_update(None, "user_version", version_of(shipped)).unwrap();
    // The rows need no user of their own.
    connection.pragma_update(None, "foreign_keys", false).unwrap();
    connection
        .execute_batch(
            r#"INSERT INTO plugin_values VALUES
                 ('u1', 'widget', 'w1', '{"number":1,"name":"Orders per week.html","frame":{}}', 2, '[]'),
                 ('u1', 'widget', 'w2', '{"number":2,"name":"widget-w2.html","frame":{}}', 1, '[]'),
                 ('u1', 'widget', 'w3', '{"number":3,"name":"widget-w2 sales.html","frame":{}}', 1, '[]'),
                 ('u1', 'widget', 'w4', '{"number":4,"name":"x","frame":{}}', 1, '[]'),
                 ('u1', 'widget', 'count', '2', 3, '[]'),
                 ('u1', 'web', 'w1', '{"name":"Kept.html"}', 1, '[]');"#,
        )
        .unwrap();
    migrate_control_to(&mut connection, SHARED, None).unwrap();
    let values = query(&connection, "SELECT plugin, key, document, revision FROM plugin_values ORDER BY 1, 2", |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?))
    });
    let value = |plugin: &str, key: &str, document: &str, revision: i64| {
        (plugin.to_owned(), key.to_owned(), document.to_owned(), revision)
    };
    assert_eq!(
        values,
        vec![
            value("web", "w1", r#"{"name":"Kept.html"}"#, 1),
            value("widget", "count", "2", 3),
            value("widget", "w1", r#"{"number":1,"frame":{},"title":"Orders per week"}"#, 3),
            value("widget", "w2", r#"{"number":2,"frame":{},"title":null}"#, 2),
            value("widget", "w3", r#"{"number":3,"frame":{},"title":"widget-w2 sales"}"#, 2),
            value("widget", "w4", r#"{"number":4,"frame":{},"title":null}"#, 2),
        ]
    );
}

#[test]
fn each_configuration_of_format_2_is_sealed_again_in_format_3() {
    /// A vault whose sealed values are their format's name: it seals one of
    /// format 2 again as format 3, leaves one of format 3, and cannot read
    /// any other.
    struct Formats;

    impl ControlSteps for Formats {
        fn data_directory(&self) -> &std::path::Path {
            std::path::Path::new("test")
        }

        fn provider_config(&self, id: &ProviderId, _: &str, _: &[u8]) -> Result<Vec<u8>, String> {
            Err(format!("entry {id} is no configuration of format 1"))
        }

        fn schedule_secret(&self, id: &str, _: &[u8]) -> Result<Vec<u8>, String> {
            Err(format!("schedule {id} has no secret here"))
        }

        fn provider_endpoint_on_device(
            &self,
            id: &ProviderId,
            _: &[u8],
            _: &dyn Fn(&str) -> bool,
        ) -> Result<Option<(String, Vec<u8>)>, String> {
            Err(format!("entry {id} is read by no earlier step here"))
        }

        fn provider_config_format_3(&self, id: &ProviderId, sealed: &[u8]) -> Result<Option<Vec<u8>>, String> {
            match sealed {
                b"format 2" => Ok(Some(b"format 3".to_vec())),
                b"format 3" => Ok(None),
                _ => Err(format!("entry {id} does not open")),
            }
        }
    }

    let shipped = include_str!("../../src/schema/control/from_0_1_5979_reply_block_shares.sql");
    let mut connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(shipped).unwrap();
    connection.pragma_update(None, "user_version", version_of(shipped)).unwrap();
    connection
        .execute_batch(
            "INSERT INTO users (id, email, nickname, password_hash, role, created_at) VALUES ('alex', 'alex@example.com', 'Alex', 'x', 'master', 1);
             INSERT INTO providers (id, owner_user_id, provider_type, credential_kind, label, config, created_at) VALUES
               ('compatible', 'alex', 'openai', 'api_key', 'Compatible API', CAST('format 2' AS BLOB), 1),
               ('resealed', 'alex', 'openai', 'api_key', 'xAI', CAST('format 3' AS BLOB), 1),
               ('subscription', 'alex', 'codex', 'subscription', 'Codex', NULL, 1);",
        )
        .unwrap();
    migrate_control_to(&mut connection, SHARED, Some(&Formats)).unwrap();
    let configs = query(&connection, "SELECT id, CAST(config AS TEXT) FROM providers ORDER BY id", |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    });
    let formats = |id: &str, config: Option<&str>| (id.to_owned(), config.map(str::to_owned));
    assert_eq!(
        configs,
        vec![formats("compatible", Some("format 3")), formats("resealed", Some("format 3")), formats("subscription", None)]
    );
}

fn every_step_leads_to_the_current_schema(kind: DatabaseKind) {
    let fresh = Connection::open_in_memory().unwrap();
    fresh.execute_batch(current(kind)).unwrap();
    let wanted = shape(&fresh);
    let mut failures = Vec::new();
    for (index, sql) in history(kind).into_iter().enumerate() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "foreign_keys", true).unwrap();
        connection.execute_batch(sql).unwrap();
        connection.pragma_update(None, "user_version", version_of(sql)).unwrap();
        if let Err(error) = migrate(kind, &mut connection) {
            failures.push(format!("step {index} ({}): {error}", version_of(sql)));
            continue;
        }
        let got = shape(&connection);
        let missing: Vec<&String> = wanted.iter().filter(|line| !got.contains(line)).collect();
        let extra: Vec<&String> = got.iter().filter(|line| !wanted.contains(line)).collect();
        if !missing.is_empty() || !extra.is_empty() {
            failures.push(format!(
                "step {index} ({}) differs from a new database\n  lacks: {missing:#?}\n  has besides: {extra:#?}",
                version_of(sql)
            ));
        }
    }
    assert!(failures.is_empty(), "{kind:?}:\n{}", failures.join("\n"));
}

/// What SQLite reports of the database's schema, one fact a line: each
/// table's columns, keys and `STRICT`, each index with its columns'
/// collations, each trigger and view. A `CHECK`, and the collation of a
/// column no index covers, live only in the table's text, which differs
/// between a table a migration altered and one made new, so they are left
/// out.
fn shape(connection: &Connection) -> Vec<String> {
    let mut lines = Vec::new();
    let objects: Vec<(String, String, String, Option<String>)> = query(
        connection,
        "SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    );
    for (kind, name, table, sql) in objects {
        match kind.as_str() {
            "table" => {
                let strict: bool = connection
                    .query_row("SELECT strict FROM pragma_table_list WHERE name = ?1", [&name], |row| row.get(0))
                    .unwrap();
                lines.push(format!("table {name} strict={strict}"));
                lines.extend(query(
                    connection,
                    &format!("SELECT name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo('{name}')"),
                    |row| {
                        Ok(format!(
                            "table {name} column {} {} not_null={} default={:?} key={} hidden={}",
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, bool>(2)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    },
                ));
                // A key of several columns is one line, its columns in order.
                lines.extend(query(
                    connection,
                    &format!(
                        "SELECT \"table\", group_concat(\"from\", ',' ORDER BY seq), group_concat(ifnull(\"to\", '?'), ',' ORDER BY seq),
                           on_update, on_delete, \"match\"
                         FROM pragma_foreign_key_list('{name}') GROUP BY id"
                    ),
                    |row| {
                        Ok(format!(
                            "table {name} key ({}) references {} ({}) on_update={} on_delete={} match={}",
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    },
                ));
                // Each index with its columns' order and collations, which
                // a column's own collation gives an index on it. One that
                // SQLite makes for a table's own constraint has no text and
                // a name of its choosing, so it is named by its origin.
                for (index, unique, origin) in query(
                    connection,
                    &format!("SELECT name, \"unique\", origin FROM pragma_index_list('{name}')"),
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?, row.get::<_, String>(2)?)),
                ) {
                    let columns = query(
                        connection,
                        &format!("SELECT name, \"desc\", coll FROM pragma_index_xinfo('{index}') WHERE key ORDER BY seqno"),
                        |row| Ok(format!("{:?} desc={} {}", row.get::<_, Option<String>>(0)?, row.get::<_, bool>(1)?, row.get::<_, String>(2)?)),
                    );
                    let named = if origin == "c" { index } else { format!("({origin})") };
                    lines.push(format!("table {name} index {named} unique={unique} on {columns:?}"));
                }
            }
            _ => lines.push(format!("{kind} {name} on {table}: {}", normalized(sql.as_deref().unwrap_or("")))),
        }
    }
    lines.sort();
    lines
}

/// `sql` with its whitespace and its identifiers' quotes made uniform, as
/// a table's rename leaves the statements that name it; a string literal
/// stays as it is.
fn normalized(sql: &str) -> String {
    let (mut out, mut literal, mut space) = (String::new(), false, false);
    for character in sql.chars() {
        if literal {
            out.push(character);
            literal = character != '\'';
            continue;
        }
        match character {
            '\'' => {
                if space && !out.is_empty() {
                    out.push(' ');
                }
                space = false;
                literal = true;
                out.push(character);
            }
            '"' => {}
            character if character.is_whitespace() => space = true,
            character => {
                if space && !out.is_empty() {
                    out.push(' ');
                }
                space = false;
                out.push(character);
            }
        }
    }
    out
}

fn query<T>(connection: &Connection, sql: &str, row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>) -> Vec<T> {
    connection.prepare(sql).unwrap().query_map([], row).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

