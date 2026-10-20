//! How a CLI process starts (`crates/provider-claude-code/src/run.rs`,
//! `crates/provider-claude-code/src/account.rs`).
//!
//! The process starts with:
//!
//! - no tools, slash commands or permission prompts of its own, and nothing the
//!   machine or the account adds to them;
//! - the session the provider wrote, resumed and mirrored, with the system
//!   prompt and tools of the request rather than those the session recorded;
//! - the request's model, system prompt and effort;
//! - the thinking display;
//! - stream-json in and out with partial messages, each user message printed
//!   back as the CLI takes it;
//! - the account's directory as its configuration directory, where the CLI
//!   reads and renews the sign-in itself, and a directory of its own there for
//!   its session;
//! - the CLI's updater and automatic compaction off;
//! - the input a session ends with answered at once.
//!
//! A sign-in runs the CLI's own login in a new directory, and a usage probe a
//! process that sends no message.

use std::collections::BTreeMap;

use demi_host_interface::{SpawnEnv, SpawnRequest};
use demi_provider_common::InferenceRequest;
use demi_provider_common::openai_request::reasoning_effort;
use demi_shared_types::ThinkingConfig;

use crate::input::MCP_SERVER;
use crate::placement::{CliSite, CliStart};
use crate::session::SessionFile;

/// What starts a CLI process at `site` with `args`, a request's.
///
/// The session `session` is in the process's own directory, and the process
/// runs in the account's directory. It is retained: the process is kept between
/// turns and is not activity of its machine.
pub(crate) fn start(site: &CliSite, args: Vec<String>, session: &SessionFile) -> CliStart {
    let spawn = SpawnRequest {
        command: site.executable.clone(),
        args,
        cwd: Some(site.run_dir.clone()),
        env: SpawnEnv::Overlay(environment(site)),
        descriptors: Vec::new(),
        retained: true,
    };
    CliStart {
        spawn,
        files: vec![(session.path(), session.bytes(&site.system))],
    }
}

/// What starts the usage probe's process at `site`: stream-json in and out,
/// with no message.
///
/// The CLI renews the sign-in when it is due and answers control requests
/// without inferring (`crates/provider-claude-code/src/account.rs`). Its
/// environment never turns the CLI's nonessential traffic off, which turns its
/// quota reading off with it. It is work of the machine while it runs.
pub(crate) fn probe(site: &CliSite) -> CliStart {
    let mut env = environment(site);
    env.remove("CLAUDE_CODE_RESUME_INTERRUPTED_TURN");
    env.insert("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(), None);
    let spawn = SpawnRequest {
        command: site.executable.clone(),
        args: [
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
        ]
        .map(String::from)
        .to_vec(),
        cwd: Some(site.run_dir.clone()),
        env: SpawnEnv::Overlay(env),
        descriptors: Vec::new(),
        retained: false,
    };
    CliStart {
        spawn,
        files: Vec::new(),
    }
}

/// The request that starts the CLI's own sign-in at `site`.
///
/// It prints the link to sign in at, reads the code the page shows from its
/// input, and writes the sign-in into its configuration directory, a new one.
/// No web browser opens on the machine. It is work of the machine while it
/// runs.
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

/// The CLI's arguments for `request`'s model, system prompt, effort and
/// thinking display, to resume the session `session`.
pub(crate) fn args(request: &InferenceRequest, session: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--include-partial-messages",
        // Each user message is printed back as the CLI takes it into a
        // turn, so a run knows which `result` answers its last one
        // (`crates/provider-claude-code/src/run.rs`).
        "--replay-user-messages",
        "--resume",
        session,
        "--session-mirror",
        "--system-prompt-snapshot",
        "off",
        // Safe mode loads none of the machine's CLAUDE.md, skills, plugins,
        // agents or LSP servers, and none of the claude.ai account's
        // connectors.
        //
        // Without it, a token with the `user:mcp_servers` scope gives the model
        // the account's connector tools beside `main`'s and an "MCP Server
        // Instructions" message (Claude Code 2.1.296, its safe-mode table's
        // `mcpClaudeAi: false`, logged as `[claudeai-mcp] Disabled in safe
        // mode`).
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
    // The vendor's summary of the thinking, the most it gives, whatever the
    // selection's summary says, unless the selection turns thinking off.
    //
    // Without the flag the CLI leaves the display to the vendor, which sends
    // every thinking block empty (Claude Code 2.1.296). A summarized block's
    // text arrives in its `thinking_delta`s, and the block's start ends the
    // page's Requesting at once.
    if !matches!(request.thinking, Some(ThinkingConfig::Disabled {})) {
        args.extend(["--thinking-display".into(), "summarized".into()]);
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

/// The changes to the machine's environment:
///
/// - A token in the machine's own environment is removed, so the CLI reads the
///   directory's sign-in.
/// - The account's directory is the configuration directory, and the process's
///   own directory of sessions is there.
/// - The CLI's updater is off, since Demi alone changes its version.
/// - Its automatic compaction is off, since Demi compacts the conversation
///   itself.
/// - An MCP tool-output limit of one million tokens means the CLI never cuts a
///   result Demi sends.
/// - `CLAUDECODE` is removed, so the CLI never takes itself for a session
///   inside another Claude Code.
/// - The input the session ends with is answered at once.
///
/// The CLI's telemetry stays on. Turning it off (`DISABLE_TELEMETRY=1`) halved
/// the CLI's start in a gVisor sandbox, but CLI 2.1.296 then reports each
/// call's `iterations` empty, and Demi read the turn's sum as the last call's
/// usage and compacted after every response.
fn environment(site: &CliSite) -> BTreeMap<String, Option<String>> {
    let set = |value: &str| Some(value.to_owned());
    BTreeMap::from([
        ("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), None),
        ("CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR".to_owned(), None),
        ("CLAUDE_CONFIG_DIR".to_owned(), set(&site.config_dir)),
        ("CLAUDE_CODE_PROJECT_DIR_NAME".to_owned(), set(&site.process_dir)),
        ("CLAUDE_CODE_RESUME_INTERRUPTED_TURN".to_owned(), set("1")),
        ("DISABLE_AUTOUPDATER".to_owned(), set("1")),
        ("DISABLE_AUTO_COMPACT".to_owned(), set("1")),
        ("MAX_MCP_OUTPUT_TOKENS".to_owned(), set("1000000")),
        ("CLAUDECODE".to_owned(), None),
    ])
}
