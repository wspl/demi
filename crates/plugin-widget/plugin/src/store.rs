//! The user's widgets, as the plugin's values.
//!
//! Widgets are numbered `w1`, `w2`, ... across the user's conversations, each
//! the value of its number, which names the blob of its page and so keeps it.
//! A reply names its widget by that number alone, so a conversation moved to
//! another workspace shows the widgets its replies name.
//!
//! A widget belongs to the conversations whose replies can name it: the one
//! it was created in, and each forked from one it belongs to. A deleted
//! conversation lets go of its widgets, and a widget that belongs to none is
//! removed, its page's blob with it.

use demi_plugin_interface::{PluginError, PluginPort, PortFailure, PortRefusal, Scope};
use demi_shared_types::{B64Bytes, BlobRef};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::frame::FrameSize;

/// The value that counts the user's widgets.
const COUNT: &str = "count";

/// One widget, as its value keeps it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Widget {
    pub(crate) number: u32,
    /// Its file's name, which a download of it takes.
    pub(crate) name: String,
    pub(crate) frame: FrameSize,
    pub(crate) page: BlobRef,
    /// The conversations it belongs to.
    pub(crate) conversations: Vec<String>,
}

/// The key of widget `number`'s value.
fn key(number: u32) -> String {
    format!("w{number}")
}

/// Keeps `page` as the user's next widget, with its `name` made from its
/// number and its `frame`, and answers it.
pub(crate) async fn add(
    port: &PluginPort,
    conversation: &str,
    page: String,
    name: impl Fn(u32) -> String,
    frame: FrameSize,
) -> Result<Widget, PluginError> {
    let blob = port.put_blob(B64Bytes::new(page.into_bytes())).await?;
    // A number is taken once its count is written; a create that loses the
    // count's race to another takes the next.
    let number = loop {
        let stored = port.value(COUNT).await?;
        let (count, revision) = match &stored {
            Some(stored) => (stored.value.as_u64().unwrap_or(0) as u32, Some(stored.revision)),
            None => (0, None),
        };
        match port.write_value(COUNT, json!(count + 1), revision).await {
            Ok(_) => break count + 1,
            Err(PortFailure::Refused(PortRefusal::Conflict)) => continue,
            Err(error) => return Err(error.into()),
        }
    };
    let widget = Widget {
        number,
        name: name(number),
        frame,
        page: blob.clone(),
        conversations: vec![conversation.to_owned()],
    };
    port.write_value_naming(key(number), to_value(&widget)?, None, vec![blob])
        .await?;
    Ok(widget)
}

/// The widget `id` names, without its page; none for an ID that names no
/// widget of the user's.
pub(crate) async fn read_facts(port: &PluginPort, id: &str) -> Result<Option<Widget>, PluginError> {
    let Some(number) = id.strip_prefix('w').and_then(|number| number.parse::<u32>().ok()) else {
        return Ok(None);
    };
    let Some(stored) = port.value(key(number)).await? else {
        return Ok(None);
    };
    serde_json::from_value(stored.value)
        .map(Some)
        .map_err(|error| PluginError::failed(format!("widget {id} cannot be read: {error}")))
}

/// The widget the reply names `id`, such as `w3`, and its page; none for
/// one the user does not have.
pub(crate) async fn read(port: &PluginPort, id: &str) -> Result<Option<(Widget, String)>, PluginError> {
    let Some(number) = id.strip_prefix('w').and_then(|number| number.parse::<u32>().ok()) else {
        return Ok(None);
    };
    let Some(stored) = port.value(key(number)).await? else {
        return Ok(None);
    };
    let widget: Widget = serde_json::from_value(stored.value)
        .map_err(|error| PluginError::failed(format!("widget {id} cannot be read: {error}")))?;
    let Some(bytes) = port.blob(widget.page.clone()).await? else {
        return Ok(None);
    };
    let page = String::from_utf8(bytes.into_bytes().to_vec())
        .map_err(|_| PluginError::failed(format!("widget {id}'s page is not text")))?;
    Ok(Some((widget, page)))
}

/// `conversation`, forked from `from`, takes each widget `from` has.
pub(crate) async fn forked(port: &PluginPort, from: &str, conversation: &str) -> Result<(), PluginError> {
    for key in keys(port).await? {
        update(port, &key, |widget| {
            if widget.conversations.iter().any(|owner| owner == from)
                && !widget.conversations.iter().any(|owner| owner == conversation)
            {
                widget.conversations.push(conversation.to_owned());
                Change::Write
            } else {
                Change::Keep
            }
        })
        .await?;
    }
    Ok(())
}

/// `conversation`, deleted, lets go of its widgets; one it alone had goes.
pub(crate) async fn deleted(port: &PluginPort, conversation: &str) -> Result<(), PluginError> {
    let mut removed = false;
    for key in keys(port).await? {
        let change = update(port, &key, |widget| {
            let before = widget.conversations.len();
            widget.conversations.retain(|owner| owner != conversation);
            match (widget.conversations.len() == before, widget.conversations.is_empty()) {
                (true, _) => Change::Keep,
                (false, true) => Change::Remove,
                (false, false) => Change::Write,
            }
        })
        .await?;
        removed |= change == Change::Remove;
    }
    if removed {
        port.changed(Scope::User).await?;
    }
    Ok(())
}

/// What a change of a widget's conversations does to its value.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Change {
    Keep,
    Write,
    Remove,
}

/// The keys of the user's widgets.
async fn keys(port: &PluginPort) -> Result<Vec<String>, PluginError> {
    Ok(port.values().await?.into_keys().filter(|key| key != COUNT).collect())
}

/// Applies `change` to widget `key` as it is now; a write that loses to
/// another, such as a second fork's or deletion's that reached it first,
/// reads the widget again and applies it again. A value that is no widget is
/// left as it is.
async fn update(port: &PluginPort, key: &str, change: impl Fn(&mut Widget) -> Change) -> Result<Change, PluginError> {
    loop {
        let Some(stored) = port.value(key).await? else {
            return Ok(Change::Keep);
        };
        let Ok(mut widget) = serde_json::from_value::<Widget>(stored.value) else {
            return Ok(Change::Keep);
        };
        let decided = change(&mut widget);
        let written = match decided {
            Change::Keep => return Ok(Change::Keep),
            Change::Remove => port.remove_value(key, stored.revision).await,
            Change::Write => {
                let blob = widget.page.clone();
                port.write_value_naming(key, to_value(&widget)?, Some(stored.revision), vec![blob])
                    .await
                    .map(drop)
            }
        };
        match written {
            Ok(()) => return Ok(decided),
            Err(PortFailure::Refused(PortRefusal::Conflict)) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

fn to_value(widget: &Widget) -> Result<Value, PluginError> {
    serde_json::to_value(widget).map_err(PluginError::failed)
}
