//! A conversation's sequences (`storage.md` § Conversation state and
//! transactions, `runtime.md` § Identifiers the model sees): the next number
//! of each sequence the model sees, commands, shells and agents. A number is
//! advanced past in its own statement before it is given out, so a crash
//! leaves a gap and never gives a number twice; a Fork's destination goes on
//! from its source's next numbers.

use demi_core::Sequence;
use rusqlite::{Connection, params};

use super::StorageError;
use super::columns::decode;

/// The next number of `sequence`, from 1, which the database has advanced
/// past when it answers.
pub(crate) fn next(connection: &Connection, sequence: Sequence) -> Result<u64, StorageError> {
    let number: i64 = connection
        .prepare_cached(
            "INSERT INTO sequences (name, next) VALUES (?1, 2)
             ON CONFLICT (name) DO UPDATE SET next = next + 1
             RETURNING next - 1",
        )?
        .query_row([sequence.to_string()], |row| row.get(0))?;
    decode("sequences", "next", u64::try_from(number))
}

/// Each sequence that gave a number out, with its next one.
pub(crate) fn all(connection: &Connection) -> Result<Vec<(Sequence, u64)>, StorageError> {
    let mut statement = connection.prepare_cached("SELECT name, next FROM sequences ORDER BY name")?;
    let mut rows = statement.query([])?;
    let mut sequences = Vec::new();
    while let Some(row) = rows.next()? {
        let sequence = decode("sequences", "name", row.get::<_, String>(0)?.parse::<Sequence>())?;
        let next = decode("sequences", "next", u64::try_from(row.get::<_, i64>(1)?))?;
        sequences.push((sequence, next));
    }
    Ok(sequences)
}

/// Starts a Fork's destination at its source's next numbers, so a number
/// the copied history names is never given to something new.
pub(crate) fn continue_from(connection: &Connection, sequences: &[(Sequence, u64)]) -> Result<(), StorageError> {
    let mut insert = connection.prepare_cached(
        "INSERT INTO sequences (name, next) VALUES (?1, ?2)
         ON CONFLICT (name) DO UPDATE SET next = max(next, excluded.next)",
    )?;
    for (sequence, next) in sequences {
        let next = i64::try_from(*next).expect("a sequence's next number fits the column");
        insert.execute(params![sequence.to_string(), next])?;
    }
    Ok(())
}
