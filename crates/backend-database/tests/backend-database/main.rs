//! The databases' migrations: the crate's test binary. Every step of each
//! database's history, taken from an empty database of its schema, must
//! leave the database a new one of the current schema would be, so that a
//! server that upgraded and one installed afresh hold the same tables.

use demi_backend_database::DatabaseKind;
use demi_backend_database::testing::{current, history, migrate, version_of};
use rusqlite::Connection;

#[test]
fn every_step_of_the_control_history_leads_to_the_current_schema() {
    every_step_leads_to_the_current_schema(DatabaseKind::Control);
}

#[test]
fn every_step_of_the_conversation_history_leads_to_the_current_schema() {
    every_step_leads_to_the_current_schema(DatabaseKind::Conversation);
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

