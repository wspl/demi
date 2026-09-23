//! The replies of fs and working-tree calls: `fs_ok { id, op, result }` and
//! `git_ok { id, op, result }`, where `op` names the call and decides the
//! type of `result`. Their serde is written by hand, because the keys follow
//! the message tag in the order `id`, `op`, `result`, and `op` must be read
//! before `result` can be decoded.

use std::fmt;
use std::marker::PhantomData;

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{FileStat, GitChanges, Readdir};

/// A successful fs call's reply.
#[derive(Debug, Clone, PartialEq)]
pub struct FsOk {
    pub id: String,
    pub result: FsResult,
}

/// The result of each fs operation; operations that only succeed or fail
/// carry none.
#[derive(Debug, Clone, PartialEq)]
pub enum FsResult {
    ReadFile,
    WriteFile,
    Exists(bool),
    Stat(FileStat),
    Lstat(FileStat),
    Readdir(Readdir),
    Mkdir,
    Rm,
    Cp,
    Mv,
    Chmod,
    Symlink,
    Link,
    Readlink(String),
    Realpath(String),
    Utimes,
}

impl FsResult {
    /// The operation's name on the wire.
    pub fn op(&self) -> &'static str {
        match self {
            Self::ReadFile => "readFile",
            Self::WriteFile => "writeFile",
            Self::Exists(_) => "exists",
            Self::Stat(_) => "stat",
            Self::Lstat(_) => "lstat",
            Self::Readdir(_) => "readdir",
            Self::Mkdir => "mkdir",
            Self::Rm => "rm",
            Self::Cp => "cp",
            Self::Mv => "mv",
            Self::Chmod => "chmod",
            Self::Symlink => "symlink",
            Self::Link => "link",
            Self::Readlink(_) => "readlink",
            Self::Realpath(_) => "realpath",
            Self::Utimes => "utimes",
        }
    }
}

/// A successful working-tree call's reply.
#[derive(Debug, Clone, PartialEq, garde::Validate)]
pub struct GitOk {
    #[garde(skip)]
    pub id: String,
    #[garde(dive)]
    pub result: GitResult,
}

#[derive(Debug, Clone, PartialEq, garde::Validate)]
pub enum GitResult {
    Changes(#[garde(dive)] GitChanges),
    /// The file streams into the call's output pipe after the reply.
    Show,
}

impl GitResult {
    /// The operation's name on the wire.
    pub fn op(&self) -> &'static str {
        match self {
            Self::Changes(_) => "changes",
            Self::Show => "show",
        }
    }
}

impl Serialize for FsOk {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut reply = serializer.serialize_struct("FsOk", 3)?;
        reply.serialize_field("id", &self.id)?;
        reply.serialize_field("op", self.result.op())?;
        match &self.result {
            FsResult::Exists(exists) => reply.serialize_field("result", exists)?,
            FsResult::Stat(stat) | FsResult::Lstat(stat) => {
                reply.serialize_field("result", stat)?;
            }
            FsResult::Readdir(entries) => reply.serialize_field("result", entries)?,
            FsResult::Readlink(path) | FsResult::Realpath(path) => {
                reply.serialize_field("result", path)?;
            }
            FsResult::ReadFile
            | FsResult::WriteFile
            | FsResult::Mkdir
            | FsResult::Rm
            | FsResult::Cp
            | FsResult::Mv
            | FsResult::Chmod
            | FsResult::Symlink
            | FsResult::Link
            | FsResult::Utimes => reply.serialize_field("result", &())?,
        }
        reply.end()
    }
}

impl Serialize for GitOk {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut reply = serializer.serialize_struct("GitOk", 3)?;
        reply.serialize_field("id", &self.id)?;
        reply.serialize_field("op", self.result.op())?;
        match &self.result {
            GitResult::Changes(changes) => reply.serialize_field("result", changes)?,
            GitResult::Show => reply.serialize_field("result", &())?,
        }
        reply.end()
    }
}

impl<'de> Deserialize<'de> for FsOk {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (id, result) = deserializer.deserialize_map(ReplyVisitor(PhantomData))?;
        Ok(Self { id, result })
    }
}

impl<'de> Deserialize<'de> for GitOk {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let (id, result) = deserializer.deserialize_map(ReplyVisitor(PhantomData))?;
        Ok(Self { id, result })
    }
}

/// A reply's result, read once its operation is known.
trait ReplyResult: Sized {
    const REPLY: &'static str;

    /// Reads the result of operation `op` from the map's next value.
    fn read<'de, A: MapAccess<'de>>(op: &str, map: &mut A) -> Result<Self, A::Error>;
}

impl ReplyResult for FsResult {
    const REPLY: &'static str = "an fs_ok reply";

    fn read<'de, A: MapAccess<'de>>(op: &str, map: &mut A) -> Result<Self, A::Error> {
        Ok(match op {
            "readFile" => none(map, Self::ReadFile)?,
            "writeFile" => none(map, Self::WriteFile)?,
            "exists" => Self::Exists(map.next_value()?),
            "stat" => Self::Stat(map.next_value()?),
            "lstat" => Self::Lstat(map.next_value()?),
            "readdir" => Self::Readdir(map.next_value()?),
            "mkdir" => none(map, Self::Mkdir)?,
            "rm" => none(map, Self::Rm)?,
            "cp" => none(map, Self::Cp)?,
            "mv" => none(map, Self::Mv)?,
            "chmod" => none(map, Self::Chmod)?,
            "symlink" => none(map, Self::Symlink)?,
            "link" => none(map, Self::Link)?,
            "readlink" => Self::Readlink(map.next_value()?),
            "realpath" => Self::Realpath(map.next_value()?),
            "utimes" => none(map, Self::Utimes)?,
            other => return Err(de::Error::custom(format!("unknown fs operation {other}"))),
        })
    }
}

impl ReplyResult for GitResult {
    const REPLY: &'static str = "a git_ok reply";

    fn read<'de, A: MapAccess<'de>>(op: &str, map: &mut A) -> Result<Self, A::Error> {
        Ok(match op {
            "changes" => Self::Changes(map.next_value()?),
            "show" => none(map, Self::Show)?,
            other => {
                return Err(de::Error::custom(format!(
                    "unknown working-tree operation {other}"
                )));
            }
        })
    }
}

/// The result of an operation that carries none must be nil.
fn none<'de, A: MapAccess<'de>, T>(map: &mut A, result: T) -> Result<T, A::Error> {
    let Nil = map.next_value()?;
    Ok(result)
}

/// Exactly nil. Decoding `()` would also accept an empty map once the
/// internally tagged message has buffered its fields.
struct Nil;

impl<'de> Deserialize<'de> for Nil {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NilVisitor;
        impl<'de> Visitor<'de> for NilVisitor {
            type Value = Nil;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("nil")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Nil, E> {
                Ok(Nil)
            }
            fn visit_none<E: de::Error>(self) -> Result<Nil, E> {
                Ok(Nil)
            }
        }
        deserializer.deserialize_any(NilVisitor)
    }
}

/// Visits a reply's `id`, `op` and `result`; `op` precedes `result`.
struct ReplyVisitor<R>(PhantomData<R>);

impl<'de, R: ReplyResult> Visitor<'de> for ReplyVisitor<R> {
    type Value = (String, R);

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str(R::REPLY)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut id = None;
        let mut op: Option<String> = None;
        let mut result = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "id" if id.is_none() => id = Some(map.next_value()?),
                "op" if op.is_none() => op = Some(map.next_value()?),
                "result" if result.is_none() => {
                    let op = op
                        .as_deref()
                        .ok_or_else(|| de::Error::custom("op must precede result"))?;
                    result = Some(R::read(op, &mut map)?);
                }
                "id" | "op" | "result" => return Err(de::Error::duplicate_field("reply field")),
                other => return Err(de::Error::unknown_field(other, &["id", "op", "result"])),
            }
        }
        let id = id.ok_or_else(|| de::Error::missing_field("id"))?;
        let result = result.ok_or_else(|| de::Error::missing_field("result"))?;
        Ok((id, result))
    }
}
