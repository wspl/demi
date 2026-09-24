//! How a CLI process starts (`claude-code.md` § Requests over stream-json):
//! no tools, session persistence, slash commands or permission prompts of its
//! own; the request's model, system prompt and effort; stream-json in and out
//! with partial messages; and an environment with the account's token, Demi's
//! configuration home and the CLI's updater and automatic compaction off.

use std::collections::BTreeMap;

use demi_provider::openai_request::reasoning_effort;
use demi_provider::{InferenceRequest, Secret};
use demi_shell::{SpawnEnv, SpawnRequest};

use crate::placement::CliSite;

/// The request that starts a CLI process at `site` for `request`, with the
/// account's `token`. It is retained: the process is kept between turns and
/// is not activity of its machine.
pub(crate) fn spawn_request(
    site: &CliSite,
    request: &InferenceRequest,
    token: &Secret,
) -> SpawnRequest {
    SpawnRequest {
        command: site.executable.clone(),
        args: args(request),
        cwd: Some(site.run_dir.clone()),
        env: SpawnEnv::Overlay(environment(site, token)),
        retained: true,
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

/// The changes to the machine's environment: the account's token and
/// Demi's configuration home; the CLI's updater off, since Demi alone
/// changes its version; its automatic compaction off, since Demi compacts
/// the conversation itself; an MCP tool-output limit of one million tokens,
/// so the CLI never cuts a result Demi sends; and `CLAUDECODE` removed, so
/// the CLI never takes itself for a session inside another Claude Code.
fn environment(site: &CliSite, token: &Secret) -> BTreeMap<String, Option<String>> {
    let set = |value: &str| Some(value.to_owned());
    BTreeMap::from([
        ("CLAUDE_CODE_OAUTH_TOKEN".to_owned(), set(token.expose())),
        ("CLAUDE_CONFIG_DIR".to_owned(), set(&site.config_dir)),
        ("DISABLE_AUTOUPDATER".to_owned(), set("1")),
        ("DISABLE_AUTO_COMPACT".to_owned(), set("1")),
        ("MAX_MCP_OUTPUT_TOKENS".to_owned(), set("1000000")),
        ("CLAUDECODE".to_owned(), None),
    ])
}
