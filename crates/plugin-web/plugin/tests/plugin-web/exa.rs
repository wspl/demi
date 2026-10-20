//! A fake Exa: its REST search, and its MCP server over streamable HTTP,
//! which answers as JSON or as an event stream, keeps sessions by
//! `Mcp-Session-Id` and can forget them, as the plugin's requests reach it
//! through the port.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use demi_plugin_interface::{WebHeader, WebRequest, WebResponse};
use serde_json::{Value, json};

/// A page Exa knows: title, URL, publication date and lines.
#[derive(Clone)]
pub struct Known {
    pub title: &'static str,
    pub url: &'static str,
    pub published: Option<&'static str>,
    pub highlights: Vec<String>,
}

/// What the fake answers.
#[derive(Default)]
pub struct Exa {
    /// MCP answers as an event stream rather than JSON.
    pub sse: bool,
    /// The key the REST search accepts.
    pub key: Option<&'static str>,
    /// The token the MCP server accepts.
    pub token: Option<&'static str>,
    /// The MCP server also lists `web_search_advanced_exa`, which takes the
    /// filters.
    pub advanced: bool,
    /// A status, with its headers, every search answers instead.
    pub refuse: Option<(u16, Vec<(&'static str, &'static str)>)>,
    /// The pages each query finds, in order.
    pub pages: BTreeMap<&'static str, Vec<Known>>,
    /// The sessions the server knows.
    pub sessions: BTreeSet<String>,
    /// How many times each method was asked, `initialize` and `tools/list`
    /// included.
    pub asked: BTreeMap<String, u32>,
    /// Each search's arguments: a REST body, or a `tools/call`'s name and
    /// arguments.
    pub searched: Vec<Value>,
    /// The headers of each MCP request, by name in lower case.
    pub mcp_headers: Vec<BTreeMap<String, String>>,
}

pub struct FakeExa(pub RefCell<Exa>);

impl FakeExa {
    pub fn new(exa: Exa) -> Self {
        Self(RefCell::new(exa))
    }

    pub fn asked(&self, method: &str) -> u32 {
        self.0.borrow().asked.get(method).copied().unwrap_or(0)
    }

    pub fn answer(&self, request: &WebRequest) -> WebResponse {
        let headers: BTreeMap<String, String> = request
            .headers
            .iter()
            .map(|header| (header.name.to_ascii_lowercase(), header.value.clone()))
            .collect();
        let body: Value = serde_json::from_slice(request.body.as_ref().unwrap().as_bytes()).unwrap();
        match request.url.as_str() {
            "https://api.exa.ai/search" => self.rest(&headers, body),
            "https://mcp.exa.ai/mcp" => self.mcp(headers, body),
            other => panic!("the plugin reached {other}"),
        }
    }

    fn rest(&self, headers: &BTreeMap<String, String>, body: Value) -> WebResponse {
        let mut exa = self.0.borrow_mut();
        if headers.get("x-api-key").map(String::as_str) != exa.key {
            return response(401, &[], br#"{"error":"Invalid API key"}"#.to_vec());
        }
        exa.searched.push(body.clone());
        if let Some((status, headers)) = exa.refuse.clone() {
            return response(status, &headers, b"{}".to_vec());
        }
        let found = exa.found(body["query"].as_str().unwrap(), body["numResults"].as_u64().unwrap());
        let results: Vec<Value> = found
            .iter()
            .map(|page| {
                json!({
                    "title": page.title,
                    "url": page.url,
                    "publishedDate": page.published,
                    "highlights": page.highlights,
                })
            })
            .collect();
        response(200, &[("content-type", "application/json")], json!({ "results": results }).to_string().into_bytes())
    }

    fn mcp(&self, headers: BTreeMap<String, String>, body: Value) -> WebResponse {
        let mut exa = self.0.borrow_mut();
        exa.mcp_headers.push(headers.clone());
        let method = body["method"].as_str().unwrap().to_owned();
        *exa.asked.entry(method.clone()).or_default() += 1;
        if let Some(authorization) = headers.get("authorization")
            && Some(authorization.as_str()) != exa.token.map(|token| format!("Bearer {token}")).as_deref()
        {
            return response(401, &[], br#"{"error":"invalid_token"}"#.to_vec());
        }
        if method == "initialize" {
            let session = format!("session-{}", exa.asked["initialize"]);
            exa.sessions.insert(session.clone());
            let result = json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "exa", "version": "1" },
            });
            return exa.rpc(&body, result, &[("mcp-session-id", &session)]);
        }
        let known = headers
            .get("mcp-session-id")
            .is_some_and(|session| exa.sessions.contains(session));
        if !known {
            return response(404, &[], br#"{"error":"Session not found"}"#.to_vec());
        }
        match method.as_str() {
            "notifications/initialized" => response(202, &[], Vec::new()),
            "tools/list" => {
                let mut tools = vec![json!({
                    "name": "web_search_exa",
                    "inputSchema": { "type": "object", "properties": {
                        "query": { "type": "string" },
                        "numResults": { "type": "number" },
                        "objective": { "type": "string" },
                    }, "required": ["query", "objective"] },
                })];
                if exa.advanced {
                    tools.push(json!({
                        "name": "web_search_advanced_exa",
                        "inputSchema": { "type": "object", "properties": {
                            "query": { "type": "string" },
                            "numResults": { "type": "number" },
                            "includeDomains": { "type": "array" },
                            "excludeDomains": { "type": "array" },
                        } },
                    }));
                }
                exa.rpc(&body, json!({ "tools": tools }), &[])
            }
            "tools/call" => {
                exa.searched.push(body["params"].clone());
                if let Some((status, headers)) = exa.refuse.clone() {
                    return response(status, &headers, b"{}".to_vec());
                }
                let arguments = &body["params"]["arguments"];
                let found = exa.found(
                    arguments["query"].as_str().unwrap(),
                    arguments["numResults"].as_u64().unwrap_or(10),
                );
                let text = found
                    .iter()
                    .map(|page| {
                        format!(
                            "Title: {}\nURL: {}\nPublished: {}\nAuthor: N/A\nHighlights:\n{}",
                            page.title,
                            page.url,
                            page.published.unwrap_or("N/A"),
                            page.highlights.join("\n...\n"),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n\n---\n\n");
                exa.rpc(&body, json!({ "content": [{ "type": "text", "text": text }] }), &[])
            }
            other => panic!("the plugin asked for {other}"),
        }
    }
}

impl Exa {
    /// The first `count` pages `query` finds.
    fn found(&self, query: &str, count: u64) -> Vec<Known> {
        self.pages
            .get(query)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .take(count as usize)
            .collect()
    }

    /// The JSON-RPC answer to `request`, as JSON or as an event stream.
    fn rpc(&self, request: &Value, result: Value, headers: &[(&str, &str)]) -> WebResponse {
        let message = json!({ "jsonrpc": "2.0", "id": request["id"], "result": result }).to_string();
        let mut headers = headers.to_vec();
        if self.sse {
            headers.push(("content-type", "text/event-stream"));
            // A comment and a notification before the answer, as a server
            // may send.
            let body = format!(
                ": ping\n\nevent: message\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{{}}}}\n\nevent: message\ndata: {message}\n\n"
            );
            response(200, &headers, body.into_bytes())
        } else {
            headers.push(("content-type", "application/json"));
            response(200, &headers, message.into_bytes())
        }
    }
}

fn response(status: u16, headers: &[(&str, &str)], body: Vec<u8>) -> WebResponse {
    WebResponse {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| WebHeader {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect(),
        body: body.into(),
    }
}

/// The pages the tests search: two queries that share a page.
pub fn pages() -> BTreeMap<&'static str, Vec<Known>> {
    let vite7 = Known {
        title: "Announcing Vite 7",
        url: "https://www.vite.dev/blog/announcing-vite7",
        published: Some("2026-06-24T00:00:00.000Z"),
        highlights: vec![
            "Vite 7 makes Rolldown the\ndefault bundler for build.".into(),
            format!("{} end", "word ".repeat(80)),
            "Third line.".into(),
            "Fourth line, never shown.".into(),
        ],
    };
    let rolldown = Known {
        title: "Announcing Rolldown 1.0",
        url: "https://voidzero.dev/posts/announcing-rolldown-1-0",
        published: None,
        highlights: vec!["Rolldown reached 1.0.".into()],
    };
    let release = Known {
        title: "v1.0.1",
        url: "https://github.com/rolldown/rolldown/releases/tag/v1.0.1",
        published: Some("2026-05-08T00:00:00.000Z"),
        highlights: vec![],
    };
    BTreeMap::from([
        ("vite rolldown", vec![vite7, rolldown.clone()]),
        ("rolldown 1.0", vec![rolldown, release]),
    ])
}
