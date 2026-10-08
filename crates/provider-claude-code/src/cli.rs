//! How a CLI process starts (`claude-code.md` § Requests over stream-json,
//! § Accounts and sign-in): no tools, session persistence, slash commands or
//! permission prompts of its own; the request's model, system prompt and
//! effort; stream-json in and out with partial messages; its own
//! configuration directory; the account's access token on a descriptor,
//! never in its environment; and the CLI's updater and automatic compaction
//! off. A sign-in runs
//! the CLI's own login in a directory of its own.

use std::collections::BTreeMap;

use bytes::Bytes;
use demi_host_interface::{Descriptor, SpawnEnv, SpawnRequest};
use demi_provider_common::openai_request::reasoning_effort;
use demi_provider_common::{InferenceRequest, Secret};

use crate::placement::CliSite;

/// The descriptor the access token reaches the CLI on.
const TOKEN_FD: u32 = 3;

/// The request that starts a CLI process at `site` for `request`, with the
/// account's access `token` on a descriptor. It is retained: the process is
/// kept between turns and is not activity of its machine.
pub(crate) fn spawn_request(
    site: &CliSite,
    request: &InferenceRequest,
    token: &Secret,
) -> SpawnRequest {
    SpawnRequest {
        command: site.executable.clone(),
        args: args(request),
        cwd: Some(site.run_dir.clone()),
        env: SpawnEnv::Overlay(environment(site)),
        descriptors: vec![Descriptor {
            fd: TOKEN_FD,
            bytes: Bytes::copy_from_slice(token.expose().as_bytes()),
        }],
        retained: true,
    }
}

/// The request that starts the CLI's own sign-in at `site`: it prints the
/// link to sign in at, reads the code the page shows from its input, and
/// writes the tokens into its configuration directory. No web browser
/// opens on the machine. It is work of the machine while it runs.
pub(crate) fn login_request(site: &CliSite) -> SpawnRequest {
    let set = |value: &str| Some(value.to_owned());
    SpawnRequest {
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
    }
}

/// The CLI's arguments for `request`'s model, system prompt and effort.
pub(crate) fn args(request: &InferenceRequest) -> Vec<String> {
    let mut args: Vec<String> = [
        "--print",
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--include-partial-messages",
        "--no-session-persistence",
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
    args
}

/// The changes to the machine's environment: the descriptor the access
/// token is on, and a token in the machine's own environment removed; the
/// process's own configuration directory;
/// the CLI's updater off, since Demi alone changes its version; its
/// automatic compaction off, since Demi compacts the conversation itself;
/// an MCP tool-output limit of one million tokens, so the CLI never cuts a
/// result Demi sends; and `CLAUDECODE` removed, so the CLI never takes
/// itself for a session inside another Claude Code.
fn environment(site: &CliSite) -> BTreeMap<String, Option<String>> {
    let set = |value: &str| Some(value.to_owned());
    BTreeMap::from([
        (
            "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR".to_owned(),
            Some(TOKEN_FD.to_string()),
        ),
        ("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), None),
        ("CLAUDE_CONFIG_DIR".to_owned(), set(&site.config_dir)),
        ("DISABLE_AUTOUPDATER".to_owned(), set("1")),
        ("DISABLE_AUTO_COMPACT".to_owned(), set("1")),
        ("MAX_MCP_OUTPUT_TOKENS".to_owned(), set("1000000")),
        ("CLAUDECODE".to_owned(), None),
    ])
}
