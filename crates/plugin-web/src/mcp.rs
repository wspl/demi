//! The smallest client of Exa's MCP server, for a search signed in or
//! anonymous: it `initialize`s once per token, keeps the server's
//! `Mcp-Session-Id`, reads the tools the server lists once per session,
//! calls `web_search_exa`, or `web_search_advanced_exa` when an option needs
//! it and the server lists it, reads the answer as JSON or as an event
//! stream, initializes again when the server forgot the session, and sends a
//! request the server refused with 401 once more with the token the plugin
//! host renewed (`plugins.md` § OAuth sign-in). It takes on nothing else of
//! MCP: Exa signs a third-party app in only for its MCP server, whose
//! resource the tokens are issued for, while its REST API accepts API keys
//! alone, so a signed-in search has to speak to that server, but `demi web
//! search` stays its own command.

use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::rc::Rc;
use std::time::Instant;

use demi_plugin_interface::{
    PluginPort, PortFailure, PortRefusal, WebMethod, WebRequest, WebResponse,
};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use futures_util::future::join_all;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::search::{
    Failure, Kind, Query, SearchError, SearchedPage, page, remaining, status_failure,
};

/// The server, which is also the resource the sign-in's tokens are for.
pub(crate) const ENDPOINT: &str = "https://mcp.exa.ai/mcp";
/// The protocol version the client asks for.
const PROTOCOL_VERSION: &str = "2025-06-18";
const SEARCH_TOOL: &str = "web_search_exa";
const ADVANCED_TOOL: &str = "web_search_advanced_exa";
/// The most bytes of an answer.
const LIMIT: u64 = 4 * 1024 * 1024;

/// The sessions of one user's searches: one anonymous, one for the token
/// of the user's sign-in, which a new token replaces.
#[derive(Default)]
pub(crate) struct Mcp {
    anonymous: RefCell<Option<Rc<Session>>>,
    account: RefCell<Option<(String, Rc<Session>)>>,
    /// The id of the next request.
    next_id: Cell<u64>,
}

/// The access token of a signed-in search, which a renewal replaces for
/// the requests of the search after it.
pub(crate) struct Bearer(RefCell<String>);

impl Bearer {
    pub(crate) fn new(token: String) -> Self {
        Self(RefCell::new(token))
    }

    fn get(&self) -> String {
        self.0.borrow().clone()
    }
}

/// What `initialize` and `tools/list` gave.
struct Session {
    /// The server's `Mcp-Session-Id`; none for a server that keeps none.
    id: Option<String>,
    /// The protocol version the server chose.
    version: String,
    tools: Vec<Tool>,
}

#[derive(Deserialize)]
struct Tool {
    name: String,
    #[serde(rename = "inputSchema", default)]
    input: ToolInput,
}

#[derive(Default, Deserialize)]
struct ToolInput {
    #[serde(default)]
    properties: Map<String, Value>,
}

/// Why a tool call did not answer.
enum CallError {
    /// The server forgot the session.
    Forgotten,
    Search(SearchError),
}

impl<E: Into<SearchError>> From<E> for CallError {
    fn from(error: E) -> Self {
        Self::Search(error.into())
    }
}

impl Mcp {
    /// The pages of each of `queries`, run at once, with `bearer`'s token or
    /// anonymously.
    pub(crate) async fn search(
        &self,
        port: &PluginPort,
        bearer: Option<&Bearer>,
        queries: &[Query<'_>],
        deadline: Instant,
    ) -> Result<Vec<Result<Vec<SearchedPage>, SearchError>>, SearchError> {
        let session = self.session(port, bearer, None, deadline).await?;
        let searches = queries
            .iter()
            .map(|query| self.search_one(port, bearer, session.clone(), query, deadline));
        Ok(join_all(searches).await)
    }

    async fn search_one(
        &self,
        port: &PluginPort,
        bearer: Option<&Bearer>,
        session: Rc<Session>,
        query: &Query<'_>,
        deadline: Instant,
    ) -> Result<Vec<SearchedPage>, SearchError> {
        match self.call(port, bearer, &session, query, deadline).await {
            Ok(pages) => Ok(pages),
            Err(CallError::Search(error)) => Err(error),
            Err(CallError::Forgotten) => {
                let session = self.session(port, bearer, Some(&session), deadline).await?;
                match self.call(port, bearer, &session, query, deadline).await {
                    Ok(pages) => Ok(pages),
                    Err(CallError::Search(error)) => Err(error),
                    Err(CallError::Forgotten) => Err(Failure::Vendor(
                        "the server forgot the session it had just begun".into(),
                    )
                    .into()),
                }
            }
        }
    }

    /// The session of `bearer`'s token: the one kept, unless it is `stale`,
    /// or a new one.
    async fn session(
        &self,
        port: &PluginPort,
        bearer: Option<&Bearer>,
        stale: Option<&Rc<Session>>,
        deadline: Instant,
    ) -> Result<Rc<Session>, SearchError> {
        let kept = match bearer {
            None => self.anonymous.borrow().clone(),
            Some(bearer) => self
                .account
                .borrow()
                .as_ref()
                .filter(|(kept, _)| *kept == bearer.get())
                .map(|(_, session)| session.clone()),
        };
        if let Some(kept) = kept
            && stale.is_none_or(|stale| !Rc::ptr_eq(stale, &kept))
        {
            return Ok(kept);
        }
        let session = Rc::new(self.initialize(port, bearer, deadline).await?);
        // Kept for the token the search holds now, renewed or not.
        match bearer {
            None => *self.anonymous.borrow_mut() = Some(session.clone()),
            Some(bearer) => *self.account.borrow_mut() = Some((bearer.get(), session.clone())),
        }
        Ok(session)
    }

    /// `initialize`, `notifications/initialized` and `tools/list`.
    async fn initialize(
        &self,
        port: &PluginPort,
        bearer: Option<&Bearer>,
        deadline: Instant,
    ) -> Result<Session, SearchError> {
        let kind = kind(bearer);
        let id = self.id();
        let initialize = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "demi", "version": env!("CARGO_PKG_VERSION") },
            },
        });
        let response = post(port, bearer, None, None, &initialize, deadline).await?;
        if !success(&response) {
            return Err(status_failure(&response, kind));
        }
        let session = response.header("mcp-session-id").map(str::to_owned);
        let result = answer(&response, id).await?;
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(PROTOCOL_VERSION)
            .to_owned();
        let initialized = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let response = post(port, bearer, session.as_deref(), Some(&version), &initialized, deadline).await?;
        if !success(&response) {
            return Err(status_failure(&response, kind));
        }
        let id = self.id();
        let list = json!({ "jsonrpc": "2.0", "id": id, "method": "tools/list" });
        let response = post(port, bearer, session.as_deref(), Some(&version), &list, deadline).await?;
        if !success(&response) {
            return Err(status_failure(&response, kind));
        }
        let listed = answer(&response, id).await?;
        let tools = serde_json::from_value(listed.get("tools").cloned().unwrap_or_default())
            .map_err(|error| Failure::Vendor(format!("a tool list that could not be read: {error}")))?;
        Ok(Session {
            id: session,
            version,
            tools,
        })
    }

    /// `tools/call` of the search tool `query` needs.
    async fn call(
        &self,
        port: &PluginPort,
        bearer: Option<&Bearer>,
        session: &Session,
        query: &Query<'_>,
        deadline: Instant,
    ) -> Result<Vec<SearchedPage>, CallError> {
        let kind = kind(bearer);
        let (tool, arguments) = tool_call(&session.tools, query, kind)?;
        let id = self.id();
        let call = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": tool, "arguments": arguments },
        });
        let response = post(
            port,
            bearer,
            session.id.as_deref(),
            Some(&session.version),
            &call,
            deadline,
        )
        .await?;
        // A server answers 404 to a session it no longer knows.
        if response.status == 404 && session.id.is_some() {
            return Err(CallError::Forgotten);
        }
        if !success(&response) {
            return Err(CallError::Search(status_failure(&response, kind)));
        }
        let result = answer(&response, id).await?;
        let text: String = result
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|content| content.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|content| content.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if result.get("isError").and_then(Value::as_bool) == Some(true) {
            return Err(Failure::Vendor(text.split_whitespace().collect::<Vec<_>>().join(" ")).into());
        }
        Ok(pages(&text))
    }

    fn id(&self) -> u64 {
        let id = self.next_id.get() + 1;
        self.next_id.set(id);
        id
    }
}

fn kind(bearer: Option<&Bearer>) -> Kind {
    if bearer.is_some() {
        Kind::Account
    } else {
        Kind::Anonymous
    }
}

fn success(response: &WebResponse) -> bool {
    (200..300).contains(&response.status)
}

/// The tool for `query` and its arguments: `web_search_exa`, or the first
/// listed tool that takes every filter given; [`Failure::NotOffered`] names
/// a filter no listed tool takes.
fn tool_call(tools: &[Tool], query: &Query<'_>, kind: Kind) -> Result<(String, Map<String, Value>), Failure> {
    let listed = |name: &str| tools.iter().find(|tool| tool.name == name);
    let given = query.filters.given();
    let takes = |tool: &Tool, name: &str| tool.input.properties.contains_key(name);
    let tool = if given.is_empty() {
        listed(SEARCH_TOOL)
    } else {
        [ADVANCED_TOOL, SEARCH_TOOL]
            .into_iter()
            .filter_map(listed)
            .find(|tool| given.iter().all(|(_, name, _)| takes(tool, name)))
    };
    let Some(tool) = tool else {
        let candidates: Vec<&Tool> = [ADVANCED_TOOL, SEARCH_TOOL].into_iter().filter_map(listed).collect();
        return Err(match given
            .iter()
            .find(|(_, name, _)| !candidates.iter().any(|tool| takes(tool, name)))
        {
            Some((option, _, _)) => Failure::NotOffered { option, kind },
            None => Failure::Vendor(format!("the server lists no {SEARCH_TOOL} tool")),
        });
    };
    let mut arguments = Map::new();
    arguments.insert("query".into(), query.text.into());
    if takes(tool, "numResults") {
        arguments.insert("numResults".into(), query.count.into());
    }
    // What the search is for, which Exa's search tool asks besides the
    // query: the query says it.
    if takes(tool, "objective") {
        arguments.insert("objective".into(), query.text.into());
    }
    for (_, name, value) in given {
        arguments.insert(name.into(), value);
    }
    Ok((tool.name.clone(), arguments))
}

/// Posts `message` to the server. When it refuses `bearer`'s token with
/// 401, the plugin host renews the token, or answers the one another
/// request renewed, and the message goes once more with it; whatever the
/// server answers then is its answer. [`SearchError::SignedOut`] when the
/// host could not renew the sign-in and forgot it.
async fn post(
    port: &PluginPort,
    bearer: Option<&Bearer>,
    session: Option<&str>,
    version: Option<&str>,
    message: &Value,
    deadline: Instant,
) -> Result<WebResponse, SearchError> {
    let sent = bearer.map(Bearer::get);
    let response = port
        .send(request(sent.as_deref(), session, version, message, deadline))
        .await?;
    let (Some(bearer), Some(sent)) = (bearer, sent) else {
        return Ok(response);
    };
    if response.status != 401 {
        return Ok(response);
    }
    // The token this request sent, which another request may have renewed
    // already: the host then answers the current one without renewing.
    let renewed = match port.oauth_renew(&sent).await {
        Ok(token) => token,
        Err(PortFailure::Refused(PortRefusal::SignedOut)) => return Err(SearchError::SignedOut),
        Err(failure) => return Err(failure.into()),
    };
    bearer.0.replace(renewed.clone());
    Ok(port
        .send(request(Some(&renewed), session, version, message, deadline))
        .await?)
}

/// The request that posts `message` to the server.
fn request(
    token: Option<&str>,
    session: Option<&str>,
    version: Option<&str>,
    message: &Value,
    deadline: Instant,
) -> WebRequest {
    let mut request = WebRequest::new(
        WebMethod::Post,
        ENDPOINT.parse().expect("a valid URL"),
        remaining(deadline),
        LIMIT,
    )
    .header("Content-Type", "application/json")
    .header("Accept", "application/json, text/event-stream")
    .body(serde_json::to_vec(message).expect("a message encodes as JSON"));
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    if let Some(session) = session {
        request = request.header("Mcp-Session-Id", session);
    }
    if let Some(version) = version {
        request = request.header("MCP-Protocol-Version", version);
    }
    request
}

/// The result of the request `id` in `response`, a JSON message or an
/// event stream of them; a JSON-RPC error is Exa's failure.
async fn answer(response: &WebResponse, id: u64) -> Result<Value, SearchError> {
    let streamed = response
        .header("content-type")
        .is_some_and(|value| value.contains("text/event-stream"));
    let messages: Vec<Value> = if streamed {
        let body = response.body.as_bytes().to_vec();
        futures_util::stream::iter([Ok::<_, Infallible>(body)])
            .eventsource()
            .filter_map(|event| async move {
                event.ok().and_then(|event| serde_json::from_str(&event.data).ok())
            })
            .collect()
            .await
    } else {
        serde_json::from_slice(response.body.as_bytes())
            .map(|message| vec![message])
            .map_err(|error| Failure::Vendor(format!("an answer that could not be read: {error}")))?
    };
    let Some(message) = messages
        .into_iter()
        .find(|message| message.get("id").and_then(Value::as_u64) == Some(id))
    else {
        return Err(Failure::Vendor("no answer to the request".into()).into());
    };
    if let Some(error) = message.get("error") {
        let words = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("an error without a message");
        return Err(Failure::Vendor(words.to_owned()).into());
    }
    Ok(message.get("result").cloned().unwrap_or_default())
}

/// The pages of a search tool's text: blocks parted by `---` lines, each
/// with `Title:`, `URL:` and `Published:` lines, then `Highlights:` and its
/// lines, one parted from the next by a `...` line.
fn pages(text: &str) -> Vec<SearchedPage> {
    let mut pages = Vec::new();
    for block in text.split("\n---\n") {
        let (mut title, mut url, mut published) = (None, None, None);
        let mut highlights: Vec<String> = Vec::new();
        let mut lines = block.lines();
        for line in lines.by_ref() {
            if let Some(value) = line.strip_prefix("Title:") {
                title = Some(value.trim());
            } else if let Some(value) = line.strip_prefix("URL:") {
                url = Some(value.trim());
            } else if let Some(value) = line.strip_prefix("Published:") {
                published = Some(value.trim());
            } else if line.starts_with("Highlights:") {
                break;
            }
        }
        let mut current = String::new();
        for line in lines {
            if line.trim() == "..." {
                highlights.push(std::mem::take(&mut current));
            } else {
                current.push_str(line);
                current.push('\n');
            }
        }
        highlights.push(current);
        if let Some(url) = url {
            pages.push(page(title.unwrap_or_default(), url, published, highlights));
        }
    }
    pages
}
