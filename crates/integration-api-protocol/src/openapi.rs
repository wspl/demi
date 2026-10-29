//! The integration API's OpenAPI document (`GET /v1/openapi.json`), made from
//! the types of this crate and the routes below, so the document and the
//! routes' bodies are one source.

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde_json::{Map, Value, json};

use crate::*;

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
}

impl Route {
    fn refuses(mut self, status: u16, when: &'static str) -> Self {
        self.refusals.push((status, when));
        self
    }
}

type Make = fn(&mut SchemaGenerator) -> Schema;

/// A request's or an answer's body.
enum Content {
    Json(Make),
    /// A file's bytes, of any media type.
    Bytes,
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
        json("delete", "/clouds/{cloud}", "Delete a Cloud no conversation works on", None, (204, None)).refuses(409, "A conversation works on the Cloud, and the message names each; or the Cloud is being deleted, or its machine refused (`integration_refused`)."),
        json("post", "/clouds/{cloud}/reset", "Reset a Cloud's system; /home stays", None, (200, Some(of::<Cloud>))).refuses(409, "The Cloud's machine refused the reset, as the message says (`integration_refused`)."),
        json("get", "/conversations", "The integration's conversations", None, (200, Some(of::<ConversationList>))),
        json("post", "/conversations", "Make a conversation, or find the one of a key", Some(of::<CreateConversation>), (201, Some(of::<Conversation>))).refuses(400, "`providerId` and `modelId` name no model of `GET /models` (`integration_refused`).").refuses(404, "The integration has no Cloud of the body's `cloud` (`not_found`).").refuses(409, "The integration or the Cloud is being deleted, or the conversation could not be made, as the message says (`integration_refused`)."),
        json("get", "/conversations/{conversation}", "A conversation with its agents", None, (200, Some(of::<ConversationDetail>))).refuses(409, "Its agents could not be read, as the message says (`integration_refused`)."),
        json("patch", "/conversations/{conversation}", "Move a conversation to another Cloud, or rename it", Some(of::<ConversationPatch>), (200, Some(of::<Conversation>))).refuses(404, "The integration has no Cloud of the body's `cloud` (`not_found`).").refuses(409, "The Cloud it moves to is being deleted, or another move of it came first (`integration_refused`)."),
        json("delete", "/conversations/{conversation}", "Delete a conversation", None, (204, None)).refuses(409, "The conversation could not be deleted, as the message says (`integration_refused`)."),
        json("post", "/conversations/{conversation}/messages", "Send a message", Some(of::<SendMessage>), (200, Some(of::<MessageAnswer>))).refuses(409, "The conversation did not take the message, as the message says (`integration_refused`)."),
        json("post", "/conversations/{conversation}/stop", "Stop every agent of a conversation", None, (204, None)).refuses(409, "The conversation did not take the stop, as the message says (`integration_refused`)."),
        json("get", "/conversations/{conversation}/agents/{agent}/transcript", "An agent's latest transcript", None, (200, Some(of::<Transcript>))),
        json("get", "/conversations/{conversation}/attachments/{attachment}", "An attachment a conversation holds, such as an agent's `a3`", None, (200, Some(of::<ConversationAttachment>))),
        Route {
            method: "post",
            path: "/attachments",
            summary: "Upload a file a message carries, at most 25 MiB, sent with its media type",
            query: Some(of::<UploadQuery>),
            body: Some(Content::Bytes),
            answer: (201, Some(Content::Json(of::<Attachment>))),
            refusals: vec![(400, "The body is empty, or `Content-Type` is missing, invalid or multipart (`invalid_body`).")],
            owned_ids: true,
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
            operation["requestBody"] = json!({ "required": true, "content": content(body, &mut generator) });
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
