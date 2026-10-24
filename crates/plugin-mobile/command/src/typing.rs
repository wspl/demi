//! `type` (`crates/plugin-mobile/command/src/operations.rs`): the text goes
//! in at the focused field and lands there, or the command fails saying what
//! landed where.
//!
//! Keys reach an app faster than some apps take them. A row of fields that
//! take a character each, as a verification code's boxes, moves the focus to
//! the next box after each character, and a key that comes before the move
//! lands in a full box and is lost. So the first character goes alone, and
//! the tree tells whether the focus moved on:
//!
//! ```text
//!   the focused field ---- none ----> fails
//!     |  a password field: keys 50 ms apart, not read back
//!   the first character alone, the tree read until it shows
//!     +-- the focus moved to another field: a row; each next character
//!     |   alone once the focus settled, read back, sent once more when
//!     |   the app did not take it, else a failure
//!     +-- the focus stayed: the rest at once, the field read at the end;
//!         a field that holds something else gets its text set directly,
//!         and one that still lacks some of the text fails
//! ```
//!
//! Enter and Tab act on the app, a submit or a move of the focus, so the
//! text from the first of them on goes in unchecked. A screen whose tree
//! names no focused field while the software keyboard shows, as a web page
//! whose accessibility is still loading, has a field that takes keys: the
//! text goes in unchecked, and the command says so.

use std::{sync::Arc, time::Duration};

use tokio::time::Instant;

use crate::{
    driver::{Device, RawElement, Rect},
    error::MobileError,
    keys, text,
};

/// How long after a character shows the focus is watched for a move to the
/// next field of a row: an app moves it within a frame, or after an
/// animation of a few hundred milliseconds.
const FOCUS_WATCH: Duration = Duration::from_millis(500);
/// How long a character, or a field's focus, may take to show in the tree
/// before it counts as lost.
const LANDING: Duration = Duration::from_secs(2);
/// The least time between two reads of the tree, for a device that answers
/// at once.
const READ_GAP: Duration = Duration::from_millis(50);
/// The pace of a password's keys, which cannot be read back.
const PASSWORD_PACE: Duration = Duration::from_millis(50);

/// A text field as one read of the tree shows it.
#[derive(Debug, Clone)]
pub struct Field {
    pub role: String,
    pub label: Option<String>,
    pub identifier: Option<String>,
    pub frame: Rect,
    pub secure: bool,
    /// Its text, empty when it holds none.
    pub text: String,
    /// The text it holds instead, when the tree cannot tell: a field that
    /// says its hint holds nothing, or the hint's words.
    pub hint: Option<String>,
}

impl Field {
    fn of(element: &RawElement) -> Self {
        Self {
            role: element.role.clone(),
            label: element.label.clone(),
            identifier: element.identifier.clone(),
            frame: element.frame,
            secure: element.secure,
            text: element.value.clone().unwrap_or_default(),
            hint: element
                .hint
                .clone()
                .filter(|hint| !hint.is_empty() && element.value.is_none()),
        }
    }

    /// What the tree says of the field's text, which tells a change.
    fn shown(&self) -> &str {
        self.hint.as_deref().unwrap_or(&self.text)
    }

    /// Each text the field may hold.
    fn readings(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.text.as_str()).chain(self.hint.as_deref())
    }

    /// Whether `element` is a field like this one: its role, label and
    /// identifier, which stay as it fills and moves.
    fn alike(&self, element: &RawElement) -> bool {
        self.role == element.role && self.label == element.label && self.identifier == element.identifier
    }

    /// Whether `other`, of the same read of the tree, is this field.
    fn same(&self, other: &Field) -> bool {
        self.role == other.role
            && self.label == other.label
            && self.identifier == other.identifier
            && (self.frame.x - other.frame.x).abs() < 1.0
            && (self.frame.y - other.frame.y).abs() < 1.0
    }

    /// This field in `tree`, as it is now: of the fields alike, the nearest
    /// to where it was, as the boxes of a row are alike but for where they
    /// are, and a field moves when a message above it goes.
    fn in_tree(&self, tree: &[RawElement]) -> Option<Field> {
        let mut alike = Vec::new();
        collect(tree, &|element| self.alike(element), &mut alike);
        let distance = |field: &&RawElement| {
            (field.frame.x - self.frame.x).powi(2) + (field.frame.y - self.frame.y).powi(2)
        };
        alike
            .into_iter()
            .min_by(|a, b| distance(a).total_cmp(&distance(b)))
            .map(Field::of)
    }

    /// The field as a failure names it: `text field "Code"`.
    fn named(&self) -> String {
        match &self.label {
            Some(label) => format!("{} {}", self.role, text::quoted(label)),
            None => self.role.clone(),
        }
    }
}

/// The elements of `tree`, depth first, that `keep` says.
fn collect<'a>(tree: &'a [RawElement], keep: &dyn Fn(&RawElement) -> bool, into: &mut Vec<&'a RawElement>) {
    for element in tree {
        if keep(element) {
            into.push(element);
        }
        collect(&element.children, keep, into);
    }
}

/// What `type` did.
#[derive(Debug)]
pub struct Typed {
    /// Each field the text went into, in order, as it holds it at the end;
    /// a field of a row as it took its characters.
    pub fields: Vec<Field>,
    /// Whether the fields' text was read back: not a password field's,
    /// nor that of a field that is gone, as the app acted on the text.
    pub checked: bool,
    /// Whether the field is gone since: the app acted on the text, as by
    /// leaving the screen.
    pub gone: bool,
    /// What the keys left in the field when it was not the text, which was
    /// then set directly.
    pub keyed: Option<String>,
    /// The Enter or Tab from which the text went in unchecked.
    pub acted: Option<char>,
}

impl Typed {
    fn unchecked(fields: Vec<Field>) -> Self {
        Self {
            fields,
            checked: false,
            gone: false,
            keyed: None,
            acted: None,
        }
    }
}

/// Types `text` at the focused field of `device`.
pub async fn type_text(device: &Arc<dyn Device>, text: &str) -> Result<Typed, MobileError> {
    let Some(field) = focused_field(device).await? else {
        keys::enter(device, text).await?;
        return Ok(Typed::unchecked(Vec::new()));
    };
    if field.secure {
        for character in text.chars() {
            keys::enter(device, &character.to_string()).await?;
            tokio::time::sleep(PASSWORD_PACE).await;
        }
        return Ok(Typed::unchecked(vec![field]));
    }
    let (head, tail) = match text.find(['\n', '\t']) {
        Some(at) => text.split_at(at),
        None => (text, ""),
    };
    let mut characters = head.chars();
    let Some(first) = characters.next() else {
        keys::enter(device, tail).await?;
        return Ok(Typed {
            acted: tail.chars().next(),
            ..Typed::unchecked(vec![field])
        });
    };
    let rest = characters.as_str();
    let landed = land(device, &field, first, !rest.is_empty(), &[]).await?;
    let moved = match (&landed.now, &landed.next) {
        (Some(now), Some(next)) => !next.same(now),
        _ => false,
    };
    let mut typed = match landed.now {
        _ if moved => row(device, &field, landed, first, rest).await?,
        Some(now) => single(device, &field, now, head, rest).await?,
        None => {
            keys::enter(device, rest).await?;
            Typed {
                gone: true,
                ..Typed::unchecked(vec![field])
            }
        }
    };
    if !tail.is_empty() {
        keys::enter(device, tail).await?;
        typed.acted = tail.chars().next();
    }
    Ok(typed)
}

/// Types `rest` at once into `now`, which took the first character of
/// `head`, and reads back that `before`, the field as it was, holds `head`.
async fn single(
    device: &Arc<dyn Device>,
    before: &Field,
    now: Field,
    head: &str,
    rest: &str,
) -> Result<Typed, MobileError> {
    keys::enter(device, rest).await?;
    let Some(keyed) = settled(device, &now, before, head).await? else {
        return Ok(Typed {
            gone: true,
            ..Typed::unchecked(vec![now])
        });
    };
    if let Some(text) = took(before, head, &keyed) {
        let text = text.to_owned();
        return Ok(Typed {
            checked: true,
            ..Typed::unchecked(vec![Field { text, hint: None, ..keyed }])
        });
    }
    // Where the cursor was is unknown once the keys went wrong: the text
    // goes after the field's own.
    device
        .set_value(keyed.frame.centre(), &format!("{}{head}", before.text))
        .await?;
    let Some(set) = settled(device, &keyed, before, head).await? else {
        return Ok(Typed {
            gone: true,
            keyed: Some(keyed.text),
            ..Typed::unchecked(vec![now])
        });
    };
    // A field that formats what it takes, as a phone number with spaces,
    // holds every character typed, in order, among its own.
    if !before.readings().any(|text| within(&format!("{text}{head}"), &set.text)) {
        return Err(MobileError::Failed(format!(
            "{} did not take {}: the keys left {}, and setting the text left {}",
            set.named(),
            text::quoted(head),
            text::quoted(&keyed.text),
            text::quoted(&set.text)
        )));
    }
    Ok(Typed {
        checked: true,
        keyed: Some(keyed.text),
        ..Typed::unchecked(vec![set])
    })
}

/// The focused field, once one shows in the tree; none when the tree names
/// none while the software keyboard shows.
async fn focused_field(device: &Arc<dyn Device>) -> Result<Option<Field>, MobileError> {
    let started = Instant::now();
    loop {
        let read = Instant::now();
        let tree = device.tree().await?;
        if let Some(field) = focused(&tree) {
            return Ok(Some(Field::of(field)));
        }
        if started.elapsed() >= FOCUS_WATCH && crate::tree::find(&tree, &|element| element.keyboard_key).is_some() {
            return Ok(None);
        }
        if started.elapsed() >= LANDING {
            return Err(MobileError::Failed(
                "no field has the keyboard focus; tap one first, or name it with --into".into(),
            ));
        }
        tokio::time::sleep_until(read + READ_GAP).await;
    }
}

/// The focused element in `tree`: the one that says it has the focus with
/// none inside it saying so, the keyboard's keys aside. None when several
/// say so, as every field of a web page does in Safari while none has the
/// focus.
fn focused(tree: &[RawElement]) -> Option<&RawElement> {
    fn innermost<'a>(tree: &'a [RawElement], into: &mut Vec<&'a RawElement>) {
        for element in tree {
            let inside = into.len();
            innermost(&element.children, into);
            if into.len() == inside && element.focused == Some(true) && !element.keyboard_key {
                into.push(element);
            }
        }
    }
    let mut claims = Vec::new();
    innermost(tree, &mut claims);
    match claims.as_slice() {
        [element] => Some(element),
        _ => None,
    }
}

/// A character the app took: the field it was sent into as it is now,
/// none once it is gone, and the field that has the focus after it.
struct Landed {
    now: Option<Field>,
    next: Option<Field>,
}

/// A character the app did not take: the field it was sent into, unchanged
/// and still focused, as it is now.
struct Lost {
    now: Field,
}

/// Sends `character` alone into `field`, the focused field, and reads the
/// tree until the app took it; when `settle`, also until the focus moved on
/// or stayed for [`FOCUS_WATCH`] after the character showed. A character
/// that the app did not take is sent once more; `before` are the fields of
/// a row that took the characters before it, which a failure names.
async fn land(
    device: &Arc<dyn Device>,
    field: &Field,
    character: char,
    settle: bool,
    before: &[Field],
) -> Result<Landed, MobileError> {
    let lost = match send(device, field, character, settle).await? {
        Ok(landed) => return Ok(landed),
        Err(lost) => lost,
    };
    match send(device, &lost.now, character, settle).await? {
        Ok(landed) => Ok(landed),
        Err(lost) => {
            let mut message = format!(
                "{} did not land: {} holds {}",
                text::quoted(&character.to_string()),
                lost.now.named(),
                text::quoted(&lost.now.text)
            );
            if !before.is_empty() {
                message.push_str(&format!("; before it, {}", held(before)));
            }
            Err(MobileError::Failed(message))
        }
    }
}

/// One try of [`land`]. The app took the character when the field's text
/// changed, the focus moved to another field, or the field is gone: an app
/// that acts on a full code clears its boxes and moves the focus back, or
/// leaves the screen. It did not when, after [`LANDING`], the field is
/// still there, focused and unchanged, unless the tree cannot tell: a field
/// that says its hint before and after may hold the hint's words now. One
/// that lost the focus to nothing fails, as no field takes the character.
async fn send(
    device: &Arc<dyn Device>,
    field: &Field,
    character: char,
    settle: bool,
) -> Result<Result<Landed, Lost>, MobileError> {
    keys::enter(device, &character.to_string()).await?;
    let sent = Instant::now();
    let mut shown: Option<Instant> = None;
    loop {
        let read = Instant::now();
        let tree = device.tree().await?;
        let next = focused(&tree).map(Field::of);
        let Some(now) = field.in_tree(&tree) else {
            return Ok(Ok(Landed {
                now: None,
                next,
            }));
        };
        let moved = next.as_ref().is_some_and(|next| !next.same(&now));
        let changed = now.shown() != field.shown();
        if changed {
            shown.get_or_insert(read);
        }
        let watched = shown.is_some_and(|shown| shown.elapsed() >= FOCUS_WATCH);
        if moved || (changed && (!settle || watched)) {
            return Ok(Ok(Landed {
                now: Some(now),
                next,
            }));
        }
        if !changed && sent.elapsed() >= LANDING {
            let unknown = now.hint.as_ref().is_some_and(|hint| {
                field.readings().any(|text| format!("{text}{character}") == *hint)
            });
            if unknown {
                return Ok(Ok(Landed {
                    now: Some(now),
                    next,
                }));
            }
            if next.is_none() {
                return Err(MobileError::Failed(format!(
                    "no field has the keyboard focus for {}; {} holds {}",
                    text::quoted(&character.to_string()),
                    now.named(),
                    text::quoted(&now.text)
                )));
            }
            return Ok(Err(Lost { now }));
        }
        tokio::time::sleep_until(read + READ_GAP).await;
    }
}

/// What each field of a row holds, as a failure says it.
fn held(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|field| format!("{} holds {}", field.named(), text::quoted(&field.text)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Types `rest` into a row of fields whose first, `field`, took `first`
/// and moved the focus on: each character alone, into the field that has
/// the focus once it settled. Each field of the answer holds what it read
/// once it took its character, or, where the app cleared it at once, as it
/// does with a full code, the characters it took.
async fn row(
    device: &Arc<dyn Device>,
    field: &Field,
    landed: Landed,
    first: char,
    rest: &str,
) -> Result<Typed, MobileError> {
    let mut fields = Vec::new();
    let mut focus = took_in(&mut fields, field.clone(), &landed, first);
    let count = rest.chars().count();
    for (index, character) in rest.chars().enumerate() {
        let Some(field) = focus else {
            return Err(MobileError::Failed(format!(
                "no field has the keyboard focus for {}; {}",
                text::quoted(&character.to_string()),
                held(&fields)
            )));
        };
        let landed = land(device, &field, character, index + 1 < count, &fields).await?;
        focus = took_in(&mut fields, field, &landed, character);
    }
    Ok(Typed {
        checked: true,
        ..Typed::unchecked(fields)
    })
}

/// Records in `fields` what `field` holds once it took `character`, as
/// `landed` read it; answers the field that has the focus next.
fn took_in(fields: &mut Vec<Field>, field: Field, landed: &Landed, character: char) -> Option<Field> {
    let now = match &landed.now {
        Some(now) if now.shown() != field.shown() => now.clone(),
        _ => {
            let mut text = field.text.clone();
            text.push(character);
            Field { text, hint: None, ..field.clone() }
        }
    };
    match fields.iter_mut().find(|took| took.same(&field)) {
        Some(took) => *took = now,
        None => fields.push(now),
    }
    landed.next.clone()
}

/// `field` once it holds `head` typed into `before`, or its text stopped
/// changing between two reads, at most [`LANDING`]: keys still on their
/// way show a read later. None once the field is gone: the app acted on
/// the text, as by leaving the screen.
async fn settled(
    device: &Arc<dyn Device>,
    field: &Field,
    before: &Field,
    head: &str,
) -> Result<Option<Field>, MobileError> {
    let started = Instant::now();
    let mut last: Option<String> = None;
    loop {
        let read = Instant::now();
        let Some(now) = field.in_tree(&device.tree().await?) else {
            return Ok(None);
        };
        let done = took(before, head, &now).is_some()
            || last.as_deref() == Some(now.shown())
            || started.elapsed() >= LANDING;
        if done {
            return Ok(Some(now));
        }
        last = Some(now.shown().to_owned());
        tokio::time::sleep_until(read + READ_GAP).await;
    }
}

/// The text `now` holds when it is `typed` typed into `before` at its
/// cursor, which may have selected some of its text to replace.
fn took<'a>(before: &Field, typed: &str, now: &'a Field) -> Option<&'a str> {
    now.readings()
        .find(|now| before.readings().any(|before| replaced(before, typed, now)))
}

/// Whether `now` is `before` with `typed` in place of a stretch of it,
/// empty where the cursor was.
fn replaced(before: &str, typed: &str, now: &str) -> bool {
    now.match_indices(typed).any(|(at, _)| {
        let (prefix, suffix) = (&now[..at], &now[at + typed.len()..]);
        prefix.len() + suffix.len() <= before.len() && before.starts_with(prefix) && before.ends_with(suffix)
    })
}

/// Whether the characters of `wanted` are in `now`, in order.
fn within(wanted: &str, now: &str) -> bool {
    let mut now = now.chars();
    wanted.chars().all(|wanted| now.any(|character| character == wanted))
}
