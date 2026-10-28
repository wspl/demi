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
    body: Option<fn(&mut SchemaGenerator) -> Schema>,
    /// The status of a success and its body; none for an empty one.
    answer: (u16, Option<fn(&mut SchemaGenerator) -> Schema>),
}

fn of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

/// Every route of the integration API, in the order the document lists them.
fn routes() -> Vec<Route> {
    vec![
        Route { method: "get", path: "/integration", summary: "The integration", body: None, answer: (200, Some(of::<Integration>)) },
        Route { method: "patch", path: "/integration", summary: "Change the integration's capabilities", body: Some(of::<IntegrationPatch>), answer: (200, Some(of::<Integration>)) },
        Route { method: "get", path: "/models", summary: "The models a conversation may run on", body: None, answer: (200, Some(of::<ModelList>)) },
        Route { method: "get", path: "/clouds", summary: "The integration's Clouds", body: None, answer: (200, Some(of::<CloudList>)) },
        Route { method: "post", path: "/clouds", summary: "Make a Cloud, or find the one of a key", body: Some(of::<CreateCloud>), answer: (201, Some(of::<Cloud>)) },
        Route { method: "get", path: "/clouds/{cloud}", summary: "A Cloud", body: None, answer: (200, Some(of::<Cloud>)) },
        Route { method: "post", path: "/clouds/{cloud}/reset", summary: "Reset a Cloud's system; /home stays", body: None, answer: (200, Some(of::<Cloud>)) },
        Route { method: "get", path: "/conversations", summary: "The integration's conversations", body: None, answer: (200, Some(of::<ConversationList>)) },
        Route { method: "post", path: "/conversations", summary: "Make a conversation, or find the one of a key", body: Some(of::<CreateConversation>), answer: (201, Some(of::<Conversation>)) },
        Route { method: "get", path: "/conversations/{conversation}", summary: "A conversation with its agents", body: None, answer: (200, Some(of::<ConversationDetail>)) },
        Route { method: "patch", path: "/conversations/{conversation}", summary: "Move a conversation to another Cloud, or rename it", body: Some(of::<ConversationPatch>), answer: (200, Some(of::<Conversation>)) },
        Route { method: "delete", path: "/conversations/{conversation}", summary: "Delete a conversation", body: None, answer: (204, None) },
        Route { method: "post", path: "/conversations/{conversation}/messages", summary: "Send a message", body: Some(of::<SendMessage>), answer: (200, Some(of::<MessageAnswer>)) },
        Route { method: "post", path: "/conversations/{conversation}/stop", summary: "Stop every agent of a conversation", body: None, answer: (204, None) },
        Route { method: "get", path: "/conversations/{conversation}/agents/{agent}/transcript", summary: "An agent's latest transcript", body: None, answer: (200, Some(of::<Transcript>)) },
    ]
}

/// The document, its routes under `server` and [`PREFIX`].
pub fn openapi(server: &str) -> Value {
    let mut generator = SchemaSettings::draft2020_12()
        .with(|settings| settings.definitions_path = "/components/schemas".into())
        .into_generator();
    let error = of::<ErrorBody>(&mut generator);
    let event = of::<Event>(&mut generator);
    let mut paths = Map::new();
    for route in routes() {
        let parameters: Vec<Value> = route
            .path
            .split('/')
            .filter_map(|part| part.strip_prefix('{').and_then(|part| part.strip_suffix('}')))
            .map(|name| json!({ "name": name, "in": "path", "required": true, "schema": { "type": "string" } }))
            .collect();
        let mut operation = json!({
            "summary": route.summary,
            "parameters": parameters,
            "responses": {
                route.answer.0.to_string(): match route.answer.1 {
                    Some(schema) => json!({ "description": "Done", "content": { "application/json": { "schema": schema(&mut generator) } } }),
                    None => json!({ "description": "Done" }),
                },
                "default": { "description": "Refused", "content": { "application/json": { "schema": error.clone() } } },
            },
        });
        if let Some(body) = route.body {
            operation["requestBody"] =
                json!({ "required": true, "content": { "application/json": { "schema": body(&mut generator) } } });
        }
        let path = paths.entry(route.path.to_owned()).or_insert_with(|| json!({}));
        path[route.method] = operation;
    }
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Demi integration API",
            "version": "1",
            "description": "Programs that build on Demi make Clouds and conversations, send messages and read transcripts. Every request carries the integration's token as a bearer token. GET /socket opens a WebSocket that sends every change of the integration's conversations, one Event per text message.",
        },
        "servers": [{ "url": format!("{server}{PREFIX}") }],
        "security": [{ "token": [] }],
        "paths": paths,
        "components": {
            "securitySchemes": { "token": { "type": "http", "scheme": "bearer" } },
            "schemas": generator.take_definitions(true),
            "x-events": event,
        },
    })
}
