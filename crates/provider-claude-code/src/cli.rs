//! How a CLI process starts (`claude-code.md` § Requests over stream-json,
//! § Accounts and sign-in): no tools, slash commands or permission prompts
//! of its own; the session the provider wrote, resumed and mirrored, with
//! the system prompt and tools of the request rather than those the session
//! recorded; the request's model, system prompt and effort; stream-json in
//! and out with partial messages; its own configuration directory; the
//! account's access token on a descriptor, never in its environment; the
//! CLI's updater and automatic compaction off; and the input a session ends
//! with answered at once. A sign-in runs the CLI's own login in a directory
//! of its own.

use std::collections::BTreeMap;

use bytes::Bytes;
use demi_host_interface::{Descriptor, SpawnEnv, SpawnRequest};
use demi_provider_common::openai_request::reasoning_effort;
use demi_provider_common::{InferenceRequest, Secret};

use crate::input::MCP_SERVER;
use crate::placement::{CliSite, CliStart};
use crate::session::{PROJECT_DIR, SessionFile};

/// The descriptor the access token reaches the CLI on.
const TOKEN_FD: u32 = 3;

/// What starts a CLI process at `site` for `request`: the session `session`
/// in its configuration directory, and the process, with the account's
/// access `token` on a descriptor. It is retained: the process is kept
/// between turns and is not activity of its machine.
pub(crate) fn start(
    site: &CliSite,
    request: &InferenceRequest,
    token: &Secret,
    session: &SessionFile,
) -> CliStart {
    let spawn = SpawnRequest {
        command: site.executable.clone(),
        args: args(request, &session.id.to_string()),
        cwd: Some(site.run_dir.clone()),
        env: SpawnEnv::Overlay(environment(site)),
        descriptors: vec![Descriptor {
            fd: TOKEN_FD,
            bytes: Bytes::copy_from_slice(token.expose().as_bytes()),
        }],
        retained: true,
    };
    CliStart {
        spawn,
        files: vec![(session.path(), session.bytes(&site.system))],
    }
}

/// The request that starts the CLI's own sign-in at `site`: it prints the
/// link to sign in at, reads the code the page shows from its input, and
/// writes the tokens into its configuration directory. No web browser
/// opens on the machine. It is work of the machine while it runs.
pub(crate) fn login_request(site: &CliSite) -> CliStart {
    let set = |value: &str| Some(value.to_owned());
    let spawn = SpawnRequest {
        command: site.executable.clone(),
        args: ["auth", "login", "--claudeai"].map(String::from).to_vec(),
        cwd: Some(site.run_dir.clone()),
        env: SpawnEnv::Overlay(BTreeMap::from([
            ("CLAUDE_CONFIG_DIR".to_owned(), set(&site.config_dir)),
            ("BROWSER".to_owned(), set("true")),
            ("DISABLE_AUTOUPDATER".to_owned(), set("1")),
            ("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), None),
            ("CLAUDECODE".to_owned(), None),
        ])),
        descriptors: Vec::new(),
        retained: false,
    };
    CliStart {
        spawn,
        files: Vec::new(),
    }
}

/// The CLI's arguments for `request`'s model, system prompt and effort, to
/// resume the session `session`.
pub(crate) fn args(request: &InferenceRequest, session: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--include-partial-messages",
        "--resume",
        session,
        "--session-mirror",
        "--system-prompt-snapshot",
        "off",
        "--safe-mode",
        "--disable-slash-commands",
        "--tools",
        "",
        "--permission-mode",
        "bypassPermissions",
        "--allow-dangerously-skip-permissions",
        "--model",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    args.push(request.model_id.clone());
    args.push("--system-prompt".into());
    args.push(request.system_prompt.clone());
    // The CLI levels thinking by effort only; a token budget, thinking turned
    // off, or none leaves its default.
    if let Some(effort) = reasoning_effort(request.thinking.as_ref()) {
        args.push("--effort".into());
        args.push(effort.to_owned());
    }
    // A server the configuration names is one the CLI waits for before its
    // first request; the `initialize` request alone declares it only after
    // the CLI began to answer the input its session ends with, and that
    // request would offer no tools (Claude Code 2.1.294).
    if !request.tools.is_empty() {
        let servers = serde_json::json!({
            "mcpServers": { MCP_SERVER: { "type": "sdk", "name": MCP_SERVER } },
        });
        args.push("--mcp-config".into());
        args.push(servers.to_string());
    }
    args
}

/// The changes to the machine's environment: the descriptor the access
/// token is on, and a token in the machine's own environment removed; the
/// process's own configuration directory, and the directory of its
/// sessions there; the CLI's updater off, since Demi alone changes its
/// version; its automatic compaction off, since Demi compacts the
/// conversation itself; an MCP tool-output limit of one million tokens, so
/// the CLI never cuts a result Demi sends; `CLAUDECODE` removed, so the CLI
/// never takes itself for a session inside another Claude Code; and the
/// input the session ends with answered at once.
fn environment(site: &CliSite) -> BTreeMap<String, Option<String>> {
    let set = |value: &str| Some(value.to_owned());
    BTreeMap::from([
        (
            "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR".to_owned(),
            Some(TOKEN_FD.to_string()),
        ),
        ("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), None),
        ("CLAUDE_CONFIG_DIR".to_owned(), set(&site.config_dir)),
        ("CLAUDE_CODE_PROJECT_DIR_NAME".to_owned(), set(PROJECT_DIR)),
        ("CLAUDE_CODE_RESUME_INTERRUPTED_TURN".to_owned(), set("1")),
        ("DISABLE_AUTOUPDATER".to_owned(), set("1")),
        ("DISABLE_AUTO_COMPACT".to_owned(), set("1")),
        ("MAX_MCP_OUTPUT_TOKENS".to_owned(), set("1000000")),
        ("CLAUDECODE".to_owned(), None),
    ])
}
