//! The integration API's OpenAPI document (`GET /v1/openapi.json`), made from
//! the types of this crate and the routes below, so the document and the
//! routes' bodies are one source.

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde_json::{Map, Value, json};

use crate::*;
use demi_web_api_protocol::files;

/// A route as the document describes it.
struct Route {
    method: &'static str,
    path: &'static str,
    summary: &'static str,
    /// The object whose fields are its query's parameters.
    query: Option<Make>,
    body: Option<Content>,
    /// The status of a success and its body; none for an empty one.
    answer: (u16, Option<Content>),
    /// The refusals of its own, beyond those its body, query and path give
    /// it: each status, and when it comes with which error code.
    refusals: Vec<(u16, &'static str)>,
    /// Whether the ids in its path name things of the integration's own,
    /// which another integration's answer 404 for.
    owned_ids: bool,
    /// Whether a request may leave its body out, which then reads as the
    /// body's defaults.
    body_optional: bool,
}

impl Route {
    fn refuses(mut self, status: u16, when: &'static str) -> Self {
        self.refusals.push((status, when));
        self
    }

    /// The route, its query's parameters the fields of `make`'s object.
    fn query(mut self, make: Make) -> Self {
        self.query = Some(make);
        self
    }

    /// The route, with the HTTP method `method`.
    fn method(mut self, method: &'static str) -> Self {
        self.method = method;
        self
    }

    /// The route, whose body a request may leave out.
    fn body_optional(mut self) -> Self {
        self.body_optional = true;
        self
    }

    /// The route, answering a file's bytes or a stream with `status`.
    fn bytes(mut self, status: u16) -> Self {
        self.answer = (status, Some(Content::Bytes));
        self
    }
}

/// A route that stops, resumes or compacts, answering 204 once the
/// conversation took it.
fn act(path: &'static str, summary: &'static str) -> Route {
    json("post", path, summary, None, (204, None))
        .refuses(409, "The conversation did not take it, as the message says (`integration_refused`).")
}

type Make = fn(&mut SchemaGenerator) -> Schema;

/// A request's or an answer's body.
enum Content {
    Json(Make),
    /// A file's bytes, of any media type.
    Bytes,
}

/// A route that reaches a Cloud's files, which wakes the Cloud for it.
fn cloud_files(route: Route) -> Route {
    route
        .refuses(409, "The Cloud is being reset (`cloud_resetting`).")
        .refuses(503, "The Cloud did not start, as the message says (`cloud_unavailable`, `cloud_capacity`, `cloud_crash_loop`).")
}

fn of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

/// A route that takes and answers JSON or nothing.
fn json(method: &'static str, path: &'static str, summary: &'static str, body: Option<Make>, answer: (u16, Option<Make>)) -> Route {
    Route {
        method,
        path,
        summary,
        query: None,
        body: body.map(Content::Json),
        answer: (answer.0, answer.1.map(Content::Json)),
        refusals: Vec::new(),
        owned_ids: true,
        body_optional: false,
    }
}

/// Every route of the integration API, in the order the document lists them.
fn routes() -> Vec<Route> {
    vec![
        json("get", "/integration", "The integration", None, (200, Some(of::<Integration>))),
        json("patch", "/integration", "Change the integration's settings", Some(of::<IntegrationPatch>), (200, Some(of::<Integration>))).refuses(400, "`sharedPath` has an empty, `.` or `..` part, is neither under `~` nor a top-level directory, or covers the system's own (`integration_refused`)."),
        json("get", "/models", "The models a conversation may run on", None, (200, Some(of::<ModelList>))),
        json("get", "/clouds", "The integration's Clouds", None, (200, Some(of::<CloudList>))),
        json("post", "/clouds", "Make a Cloud, or find the one of a key", Some(of::<CreateCloud>), (201, Some(of::<Cloud>))).refuses(409, "The integration is being deleted (`integration_refused`)."),
        json("get", "/clouds/{cloud}", "A Cloud", None, (200, Some(of::<Cloud>))),
        json("patch", "/clouds/{cloud}", "Make a Cloud always on, or not", Some(of::<CloudPatch>), (200, Some(of::<Cloud>))),
        json("delete", "/clouds/{cloud}", "Delete a Cloud, with its files; a conversation that worked on it has no Cloud until it moves to one", None, (204, None)).refuses(409, "The Cloud is being deleted, or its machine refused (`integration_refused`)."),
        json("post", "/clouds/{cloud}/reset", "Reset a Cloud's system; /home stays", None, (200, Some(of::<Cloud>))).refuses(409, "The Cloud's machine refused the reset, as the message says (`integration_refused`)."),
        json("post", "/clouds/{cloud}/commands", "Run a script on a Cloud as `demi`, outside any conversation, once its init script ends; it answers at once", Some(of::<StartCloudCommand>), (201, Some(of::<CloudCommand>))),
        json("get", "/clouds/{cloud}/commands/{command}", "A command on a Cloud, with its output from `after`; an hour after it ends, or once the server restarts, it is gone", None, (200, Some(of::<CloudCommand>)))
            .query(of::<OutputQuery>),
        json("post", "/clouds/{cloud}/commands/{command}/stop", "Stop a command on a Cloud; one that ended stays as it is", None, (204, None)),
        cloud_files(json("get", "/clouds/{cloud}/fs", "A directory's entries on a Cloud, its home without `path`", None, (200, Some(of::<files::Directory>)))
            .query(of::<files::DirectoryQuery>)),
        cloud_files(json("post", "/clouds/{cloud}/fs", "Make a directory on a Cloud, with its parents", Some(of::<files::CreateDirectory>), (201, Some(of::<files::CreatedDirectory>)))),
        cloud_files(json("delete", "/clouds/{cloud}/fs", "Delete a file on a Cloud, or a directory with what it holds", None, (204, None))
            .query(of::<files::RemoveQuery>)
            .refuses(409, "The path is the root or the home directory, or holds one (`protected_path`).")),
        cloud_files(json("get", "/clouds/{cloud}/fs/raw", "A file's bytes on a Cloud: 206 with the part a `Range` names, 304 when `If-None-Match` names its ETag", None, (200, None))
            .query(of::<files::RawFileQuery>)
            .bytes(200)
            .refuses(404, "Nothing, or no file, is at the path (`fs_error`, `not_found`).")),
        cloud_files(Route {
            method: "put",
            path: "/clouds/{cloud}/fs/raw",
            summary: "Write a file's bytes on a Cloud, whole or not at all",
            query: Some(of::<files::UploadQuery>),
            body: Some(Content::Bytes),
            answer: (204, None),
            refusals: vec![(409, "A directory is at the path (`is_directory`), or a file and `replace` is false (`file_exists`).")],
            owned_ids: true,
            body_optional: false,
        }),
        cloud_files(json("post", "/clouds/{cloud}/fs/search", "Search the names and the text of the files under a directory of a Cloud, its home without `root`: one JSON object per line", Some(of::<files::SearchRequest>), (200, None)).bytes(200)),
        json("get", "/conversations", "A page of the integration's conversations, newest first; or with `key`, the one of that key, `limit` and `after` aside", None, (200, Some(of::<ConversationList>)))
            .query(of::<ConversationQuery>)
            .refuses(400, "`after` is no page's `next` (`integration_refused`)."),
        json("post", "/conversations", "Make a conversation, or find the one of a key", Some(of::<CreateConversation>), (201, Some(of::<Conversation>))).refuses(400, "`providerId` and `modelId` name no model of `GET /models`, or `workflow` is no workflow file (`integration_refused`).").refuses(404, "The integration has no Cloud of the body's `cloud` (`not_found`).").refuses(409, "The integration or the Cloud is being deleted, or the conversation could not be made, as the message says (`integration_refused`)."),
        json("get", "/conversations/{conversation}", "A conversation with its agents", None, (200, Some(of::<ConversationDetail>))).refuses(409, "Its agents could not be read, as the message says (`integration_refused`)."),
        json("patch", "/conversations/{conversation}", "Move a conversation to another Cloud or directory, rename it, or change its model, its workflow, its identity or its commands", Some(of::<ConversationPatch>), (200, Some(of::<Conversation>))).refuses(400, "`model` names no model of `GET /models`, or an effort or a service tier it does not offer; or `workflow` is no workflow file (`integration_refused`).").refuses(404, "The integration has no Cloud of the body's `cloud`, or it names only a `directory` while the conversation's Cloud is deleted (`not_found`).").refuses(409, "The Cloud it moves to is being deleted, or another move of it came first (`integration_refused`)."),
        json("delete", "/conversations/{conversation}", "Delete a conversation", None, (204, None)).refuses(409, "The conversation could not be deleted, as the message says (`integration_refused`)."),
        json("post", "/conversations/{conversation}/messages", "Send a message", Some(of::<SendMessage>), (200, Some(of::<MessageAnswer>))).refuses(404, "The integration has no Cloud of the body's `cloud` (`not_found`).").refuses(409, "A steer names another `cloud` than the running turn's (`cloud_differs`); or the conversation did not take the message, as the message says (`integration_refused`)."),
        act("/conversations/{conversation}/messages", "Empty the queue").method("delete"),
        act("/conversations/{conversation}/messages/{message}", "Withdraw a queued message or a steer the turn has not taken").method("delete"),
        act("/conversations/{conversation}/messages/{message}/send-now", "Send a waiting message now: a queued one ends the running turn and runs next, a steer cuts the current step short"),
        json("post", "/conversations/{conversation}/stop", "Stop a conversation's agents: every one, or those `scope` names", Some(of::<StopRequest>), (204, None)).body_optional().refuses(409, "The conversation did not take the stop, as the message says (`integration_refused`)."),
        act("/conversations/{conversation}/resume", "Resume a turn that stopped or failed"),
        act("/conversations/{conversation}/retry", "Retry the last failed request"),
        act("/conversations/{conversation}/compact", "Compact the main agent's context"),
        json("post", "/conversations/{conversation}/fork", "A new conversation with the transcript up to a block, or the one of the body's key", Some(of::<ForkRequest>), (201, Some(of::<Conversation>)))
            .refuses(400, "`blockId` is no completed reply of the main agent's (`integration_refused`).")
            .refuses(409, "The fork failed, as the message says (`integration_refused`)."),
        act("/conversations/{conversation}/commands/{command}/stop", "Stop a running command"),
        act("/conversations/{conversation}/agents/{agent}/stop", "Stop one of the main agent's subagents; one that already ended has nothing to stop")
            .refuses(409, "The agent is a subagent's subagent, which this route does not stop, as the page's Stop does not (`integration_refused`)."),
        act("/conversations/{conversation}/agents/{agent}/compact", "Compact a subagent's context"),
        json("post", "/conversations/{conversation}/agents/{agent}/messages", "Send a subagent a message; a closed one resumes with it", Some(of::<SubagentMessage>), (204, None)).refuses(409, "The subagent did not take the message, as the message says (`integration_refused`)."),
        json("get", "/conversations/{conversation}/agents/{agent}/transcript", "A page of an agent's transcript, the latest without a query", None, (200, Some(of::<Transcript>)))
            .query(of::<TranscriptQuery>)
            .refuses(400, "The query names more than one of `before`, `after` and `around`, `before` or `after` without `edge`, or an index outside the transcript (`invalid_query`).")
            .refuses(409, "The page moved since: `edge` is no longer the block at `before` or `after` (`transcript_changed`)."),
        json("get", "/conversations/{conversation}/agents/{agent}/blocks/{block}", "A block whole, where the transcript's is light", None, (200, Some(of::<demi_web_api_protocol::conversations::WholeBlock>))),
        json("get", "/conversations/{conversation}/commands/{command}/output", "What an agent's command printed, from `after`, as `demi shell output` reads it: while it runs and after it ended", None, (200, Some(of::<CommandOutput>)))
            .query(of::<OutputQuery>)
            .refuses(409, "The command's Host did not answer while it runs, or its output was not stored when it ended, as the message says (`integration_refused`)."),
        json("post", "/conversations/{conversation}/browser/tabs/{tab}/views", "A link to one tab of the conversation's browser, live, which any page may frame; only this answer carries its `url`", Some(of::<CreateBrowserView>), (201, Some(of::<BrowserView>)))
            .refuses(400, "`tab` is no tab's id, such as `t3`, or `expiresIn` is beyond the times Demi keeps (`integration_refused`).")
            .refuses(409, "The integration's browser capability is off (`integration_refused`)."),
        json("get", "/conversations/{conversation}/browser/views", "The links to the conversation's browser tabs that have not expired", None, (200, Some(of::<BrowserViewList>))),
        json("delete", "/conversations/{conversation}/browser/views/{view}", "Remove a link; each page open on it ends", None, (204, None)),
        json("get", "/conversations/{conversation}/fs", "A directory's entries on the conversation's Cloud, its working directory without `path`", None, (200, Some(of::<files::Directory>)))
            .query(of::<files::DirectoryQuery>),
        json("post", "/conversations/{conversation}/fs", "Make a directory, with its parents", Some(of::<files::CreateDirectory>), (201, Some(of::<files::CreatedDirectory>))),
        json("delete", "/conversations/{conversation}/fs", "Delete a file, or a directory with what it holds", None, (204, None))
            .query(of::<files::RemoveQuery>)
            .refuses(409, "The path is the root, the home directory or the working directory, or holds one (`protected_path`)."),
        json("get", "/conversations/{conversation}/fs/raw", "A file's bytes: 206 with the part a `Range` names, 304 when `If-None-Match` names its ETag", None, (200, None))
            .query(of::<files::RawFileQuery>)
            .bytes(200)
            .refuses(404, "Nothing, or no file, is at the path (`fs_error`, `not_found`)."),
        Route {
            method: "put",
            path: "/conversations/{conversation}/fs/raw",
            summary: "Write a file's bytes, whole or not at all",
            query: Some(of::<files::UploadQuery>),
            body: Some(Content::Bytes),
            answer: (204, None),
            refusals: vec![(409, "A directory is at the path (`is_directory`), or a file and `replace` is false (`file_exists`).")],
            owned_ids: true,
            body_optional: false,
        },
        json("post", "/conversations/{conversation}/fs/search", "Search the names and the text of the files under a directory: one JSON object per line", Some(of::<files::SearchRequest>), (200, None)).bytes(200),
        json("get", "/conversations/{conversation}/hosts/{device}/fs", "A directory's entries on another Host the conversation reaches", None, (200, Some(of::<files::Directory>)))
            .query(of::<files::DirectoryQuery>),
        json("post", "/conversations/{conversation}/hosts/{device}/fs", "Make a directory on another Host the conversation reaches", Some(of::<files::CreateDirectory>), (201, Some(of::<files::CreatedDirectory>))),
        json("get", "/conversations/{conversation}/attachments/{attachment}", "An attachment a conversation holds, such as an agent's `a3`", None, (200, Some(of::<ConversationAttachment>))),
        Route {
            method: "post",
            path: "/attachments",
            summary: "Upload a file a message carries, at most 25 MiB, sent with its media type",
            query: Some(of::<AttachmentQuery>),
            body: Some(Content::Bytes),
            answer: (201, Some(Content::Json(of::<Attachment>))),
            refusals: vec![(400, "The body is empty, or `Content-Type` is missing, invalid or multipart (`invalid_body`).")],
            owned_ids: true,
            body_optional: false,
        },
        Route {
            method: "get",
            path: "/blobs/{sha256}",
            summary: "A file's bytes, such as one an agent attached to its reply",
            query: None,
            body: None,
            answer: (200, Some(Content::Bytes)),
            refusals: vec![(
                404,
                "No file of the integration's user has that SHA-256 (`not_found`); every integration of the user reads the files any of them uploaded or was sent.",
            )],
            owned_ids: false,
            body_optional: false,
        },
        Route {
            method: "get",
            path: "/usage",
            summary: "The integration's usage in a period, grouped",
            query: Some(of::<UsageQuery>),
            body: None,
            answer: (200, Some(Content::Json(of::<Usage>))),
            refusals: Vec::new(),
            owned_ids: true,
            body_optional: false,
        },
    ]
}

/// A body's content in the document.
fn content(body: &Content, generator: &mut SchemaGenerator) -> Value {
    match body {
        Content::Json(make) => json!({ "application/json": { "schema": make(generator) } }),
        Content::Bytes => json!({ "*/*": { "schema": { "type": "string", "contentMediaType": "application/octet-stream" } } }),
    }
}

/// The query parameters of the object `make` describes, each field one.
fn query_parameters(make: Make, generator: &mut SchemaGenerator) -> Vec<Value> {
    let reference = make(generator);
    let name = reference
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|path| path.rsplit('/').next())
        .map(str::to_owned);
    let definitions = generator.definitions();
    let object = name.and_then(|name| definitions.get(&name)).cloned().unwrap_or(Value::Null);
    let required: Vec<&str> = object
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    object
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| {
            properties
                .iter()
                .map(|(name, schema)| {
                    json!({ "name": name, "in": "query", "required": required.contains(&name.as_str()), "schema": schema })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Each status `route` answers with an error body, and when, its error
/// codes named: those every route shares, those of its body, its query and
/// its path's ids, and its own.
fn refusals(route: &Route) -> Vec<(u16, String)> {
    let mut refusals: Vec<(u16, &str)> = Vec::new();
    if matches!(route.body, Some(Content::Json(_))) {
        refusals.push((400, "The body does not read as its type (`invalid_body`)."));
    }
    if route.query.is_some() {
        refusals.push((400, "The query does not read as its type (`invalid_query`)."));
    }
    refusals.push((401, "The token is missing, wrong or replaced (`unauthenticated`)."));
    if route.owned_ids && route.path.contains('{') {
        refusals.push((
            404,
            "The integration has nothing of an id in the path (`not_found`); another integration's ids answer this too.",
        ));
    }
    if route.body.is_some() {
        refusals.push((408, "The body stopped arriving (`transfer_stalled`)."));
        refusals.push((413, "The body is over its limit, which `params.limitBytes` gives where known (`too_large`)."));
    }
    refusals.push((500, "The server failed; `code` says how, such as `internal_error`."));
    refusals.push((503, "The server is starting, updating or stopping; retry later (`backend_starting`, `updating`, `backend_closing`)."));
    refusals.extend(route.refusals.iter().copied());
    let mut merged: std::collections::BTreeMap<u16, Vec<&str>> = std::collections::BTreeMap::new();
    for (status, when) in refusals {
        merged.entry(status).or_default().push(when);
    }
    merged.into_iter().map(|(status, whens)| (status, whens.join(" "))).collect()
}

/// The document, its routes under `server` and [`PREFIX`].
pub fn openapi(server: &str) -> Value {
    let mut generator = SchemaSettings::draft2020_12()
        .with(|settings| settings.definitions_path = "/components/schemas".into())
        .into_generator();
    let error = of::<ErrorBody>(&mut generator);
    // The socket's messages, which no route's body names, are schemas of
    // their own, as an API whose WebSocket OpenAPI cannot describe lists
    // its events.
    of::<Event>(&mut generator);
    of::<ClientMessage>(&mut generator);
    let mut paths = Map::new();
    for route in routes() {
        let mut parameters: Vec<Value> = route
            .path
            .split('/')
            .filter_map(|part| part.strip_prefix('{').and_then(|part| part.strip_suffix('}')))
            .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        if let Some(query) = route.query {
            parameters.extend(query_parameters(query, &mut generator));
        }
        let mut operation = json!({
            "summary": route.summary,
            "parameters": parameters,
            "responses": {
                route.answer.0.to_string(): match &route.answer.1 {
                    Some(body) => json!({ "description": "Done", "content": content(body, &mut generator) }),
                    None => json!({ "description": "Done" }),
                },
            },
        });
        for (status, when) in refusals(&route) {
            operation["responses"][status.to_string()] =
                json!({ "description": when, "content": { "application/json": { "schema": error.clone() } } });
        }
        if let Some(body) = &route.body {
            operation["requestBody"] = json!({ "required": !route.body_optional, "content": content(body, &mut generator) });
        }
        let path = paths.entry(route.path.to_owned()).or_insert_with(|| json!({}));
        path[route.method] = operation;
    }
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Demi integration API",
            "version": "1",
            "description": "Programs that build on Demi make Clouds and conversations, send messages and read transcripts. Every request carries the integration's token as a bearer token. GET /socket opens a WebSocket that sends every change of the integration's conversations, one Event per text message, and serves its plugin: it takes ClientMessages.",
        },
        "servers": [{ "url": format!("{server}{PREFIX}") }],
        "security": [{ "token": [] }],
        "paths": paths,
        "components": {
            "securitySchemes": { "token": { "type": "http", "scheme": "bearer" } },
            "schemas": generator.take_definitions(true),
        },
    })
}
