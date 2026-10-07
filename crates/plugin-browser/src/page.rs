//! The conversation browser's tab list and tab methods (`live-view.md` § The
//! tab methods): each runs the operation the agent's `demi browser` command
//! runs, as a package call for the conversation's user, and holds no
//! browser logic. The tab list is the plugin's conversation state, which
//! follows the conversation's jobs, since the agent's commands open and
//! close tabs; listing never wakes a stopped Cloud and is no activity.
//! Closing, moving and stopping a tab operate the browser without waking it; opening
//! a tab is work the user starts. Each method that did its work marks the
//! tab list changed. Opening and closing browser tabs for panel tabs is the
//! panel's work ([`crate::panel`]), through the same operations.
//! For a `user` caller the operations answer once their work started,
//! without waiting for the page to load.

use demi_command_declarations::NativeOperation;
use demi_command_package_browser_protocol::PACKAGE;
use demi_command_package_browser_protocol::release::{ARTIFACT, BrowserRelease};
use demi_command_package_browser_protocol::browser::{
    BackInput, BrowserCreatedBy, BrowserErrorCode, BrowserOperation, BrowserTab, CloseInput,
    FailureDocument, ForwardInput, GotoInput, NavigationResult, OpenInput, OpenResult, PREFIX,
    ReloadInput, StopInput, TabId, TabMoved, TabsInput, TabsResult,
};
use demi_plugin_interface::{
    CallKind, Method, Page, PluginError, PluginPort, PortFailure, PortRefusal, Scope, State, Topic,
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::ConversationId;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::panel::{KIND, Work};

/// The most characters of a URL a method takes.
pub const URL_MAX: usize = 4096;

/// The conversation state: the conversation browser's tabs, none while it
/// does not run, and the browser a tab needs on the Host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserTabs {
    pub tabs: Vec<BrowserTab>,
    pub browser: NeededBrowser,
}

/// The pinned Chrome for Testing as the Host's installed artifacts name it,
/// which the page looks for among them (`live-view.md` § A browser tab in
/// the panel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NeededBrowser {
    pub name: String,
    pub version: String,
}

/// `bind { panelTab }`: a browser tab for the panel tab, which Retry and
/// Reload ask for.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindTab {
    pub panel_tab: String,
}

/// What `bind` answers: the browser tab the panel tab shows now, so the page
/// knows its data is up to date once it names that tab; none when the panel
/// no longer has the tab or no browser tab could be opened for it, which its
/// data says.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabBound {
    pub tab: Option<String>,
}

/// `sync {}`: the panel's tabs updated from the browser's.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncTabs {}

/// `navigate { tab, url }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NavigateTab {
    pub tab: String,
    #[schemars(length(min = 1, max = URL_MAX))]
    pub url: String,
}

/// Where `history` moves a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAction {
    Back,
    Forward,
    Reload,
}

/// `history { tab, action }`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabHistory {
    pub tab: String,
    pub action: HistoryAction,
}

/// `stop { tab }`: the user's Stop.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StopTab {
    pub tab: String,
}

/// The operation `operation` names, in the browser's package.
fn operation(name: &str) -> NativeOperation {
    NativeOperation {
        package: PACKAGE.into(),
        operation: format!("{PREFIX}{name}"),
    }
}

/// The page: its tab list, its methods, each declared with the operations
/// it calls, and its panel kind, whose tabs the plugin brings up to date
/// after each job.
pub(crate) fn page() -> Page {
    Page::new("@demicodes/plugin-browser")
        .conversation_state(
            State::new::<BrowserTabs>()
                .follows(Topic::Jobs)
                .calls(operation("tabs")),
        )
        .panel_kind(KIND)
        .told(Topic::Jobs)
        .method(
            Method::new::<BindTab, TabBound>("bind", Scope::Conversation)
                .calls(operation("open"))
                .calls(operation("goto"))
                .calls(operation("close")),
        )
        .method(Method::new::<SyncTabs, ()>("sync", Scope::Conversation).calls(operation("tabs")))
        .method(
            Method::new::<NavigateTab, TabMoved>("navigate", Scope::Conversation)
                .calls(operation("goto")),
        )
        .method(
            Method::new::<TabHistory, TabMoved>("history", Scope::Conversation)
                .calls(operation("back"))
                .calls(operation("forward"))
                .calls(operation("reload")),
        )
        .method(
            Method::new::<StopTab, TabMoved>("stop", Scope::Conversation)
                .calls(operation("stop")),
        )
}

/// The browser's tabs, and the latest ones closed on purpose; none while it
/// does not run, a stopped Cloud's included, whose tabs it lost.
pub(crate) async fn list(port: &PluginPort) -> Result<Listing, PluginError> {
    let input = TabsInput {
        offset: None,
        limit: None,
        timeout: None,
    };
    match run::<TabsResult, _>(port, BrowserOperation::Tabs, input, CallKind::Looks).await {
        Ok(listed) => Ok(Listing {
            tabs: listed.tabs,
            closed: listed.closed.unwrap_or_default(),
        }),
        // A stopped Cloud runs no browser.
        Err(failure) if stopped(&failure) => Ok(Listing::default()),
        Err(failure) => Err(refused(failure)),
    }
}

/// The browser's tabs as the plugin reads them.
#[derive(Debug, Default)]
pub(crate) struct Listing {
    pub tabs: Vec<BrowserTab>,
    /// The latest tabs closed on purpose, by a command or by their own page.
    pub closed: Vec<TabId>,
}

/// The conversation state: the browser's tabs, and the browser they need.
pub(crate) async fn tabs(port: &PluginPort) -> Result<Value, PluginError> {
    let pinned = BrowserRelease::pinned().map_err(PluginError::failed)?;
    to_value(BrowserTabs {
        browser: NeededBrowser {
            name: ARTIFACT.to_owned(),
            version: pinned.version,
        },
        tabs: list(port).await?.tabs,
    })
}

/// A new tab on `url`, starting the environment when needed: work the
/// user starts, which wakes a stopped Cloud.
pub(crate) async fn open(port: &PluginPort, url: &str) -> Result<BrowserTab, PluginError> {
    let input = OpenInput {
        url: url.to_owned(),
        load: None,
        show: None,
        timeout: None,
    };
    let opened = run::<OpenResult, _>(port, BrowserOperation::Open, input, CallKind::Starts)
        .await
        .map_err(refused)?;
    Ok(BrowserTab {
        id: opened.tab,
        title: opened.title.unwrap_or_default(),
        url: opened.url,
        created_by: BrowserCreatedBy::User {},
        loading: false,
        can_go_back: false,
        can_go_forward: false,
        shows: 0,
    })
}

/// Closes the browser's tab `tab`. A tab the browser does not have, or a
/// stopped Cloud's, is closed already.
pub(crate) async fn close(port: &PluginPort, tab: &str) -> Result<(), PluginError> {
    let Ok(tab) = TabId::try_from(tab.to_owned()) else {
        return Ok(());
    };
    let input = CloseInput { tab, timeout: None };
    match run::<Value, _>(port, BrowserOperation::Close, input, CallKind::Operates).await {
        Ok(_) => Ok(()),
        Err(failure) if stopped(&failure) || tab_missing(&failure) => Ok(()),
        Err(failure) => Err(refused(failure)),
    }
}

/// Starts loading `url` in the browser's tab `tab`.
pub(crate) async fn navigate(
    port: &PluginPort,
    tab: &str,
    url: String,
) -> Result<TabMoved, PluginError> {
    let input = GotoInput {
        tab: tab_id(tab.to_owned())?,
        url,
        load: None,
        timeout: None,
    };
    moved(run(port, BrowserOperation::Goto, input, CallKind::Operates).await)
}

/// Answers the page call `method` with `params`, which its schema checked,
/// for `conversation`, and marks the tab list changed once the method did
/// its work: every method opens, closes or moves a tab, and one the browser
/// refused changed none.
pub(crate) async fn call(
    method: &str,
    params: Map<String, Value>,
    port: &PluginPort,
    work: &Work,
    conversation: &ConversationId,
) -> Result<Value, PluginError> {
    let result = run_method(method, params, port, work, conversation).await?;
    port.changed(Scope::Conversation).await?;
    Ok(result)
}

async fn run_method(
    method: &str,
    params: Map<String, Value>,
    port: &PluginPort,
    work: &Work,
    conversation: &ConversationId,
) -> Result<Value, PluginError> {
    match method {
        "bind" => {
            let BindTab { panel_tab } = decode(params)?;
            let tab = work.bind(conversation, port, &panel_tab).await?;
            to_value(TabBound { tab })
        }
        "sync" => {
            let SyncTabs {} = decode(params)?;
            work.sync(conversation, port).await?;
            Ok(Value::Null)
        }
        "navigate" => {
            let NavigateTab { tab, url } = decode(params)?;
            to_value(navigate(port, &tab, url).await?)
        }
        "history" => {
            let TabHistory { tab, action } = decode(params)?;
            let tab = tab_id(tab)?;
            let navigated = match action {
                HistoryAction::Back => {
                    let input = BackInput {
                        tab,
                        load: None,
                        timeout: None,
                    };
                    run(port, BrowserOperation::Back, input, CallKind::Operates).await
                }
                HistoryAction::Forward => {
                    let input = ForwardInput {
                        tab,
                        load: None,
                        timeout: None,
                    };
                    run(port, BrowserOperation::Forward, input, CallKind::Operates).await
                }
                HistoryAction::Reload => {
                    let input = ReloadInput {
                        tab,
                        load: None,
                        timeout: None,
                    };
                    run(port, BrowserOperation::Reload, input, CallKind::Operates).await
                }
            };
            to_value(moved(navigated)?)
        }
        "stop" => {
            let StopTab { tab } = decode(params)?;
            let input = StopInput {
                tab: tab_id(tab)?,
                timeout: None,
            };
            let stopped = run::<TabMoved, _>(port, BrowserOperation::Stop, input, CallKind::Operates)
                .await
                .map_err(refused)?;
            to_value(stopped)
        }
        method => Err(PluginError::failed(format!(
            "the browser has no method {method}"
        ))),
    }
}

/// Runs the browser's operation `variant` with `input` and decodes its
/// answer as `T`.
async fn run<T: DeserializeOwned, I: Serialize>(
    port: &PluginPort,
    variant: fn(I) -> BrowserOperation,
    input: I,
    kind: CallKind,
) -> Result<T, PortFailure> {
    let Ok(Value::Object(args)) = serde_json::to_value(&input) else {
        unreachable!("a browser operation's input serializes to an object")
    };
    let name = variant(input).name();
    let result = port.package_call(operation(name), args, kind).await?;
    serde_json::from_value(result).map_err(|error| {
        PortFailure::Refused(PortRefusal::Operation {
            stderr: format!("the browser answered what the plugin cannot read: {error}"),
        })
    })
}

/// What a user's navigation of one tab answers: the number of the last tab
/// list before it started, or its refusal; a stopped Cloud is refused as
/// `host_stopped`, never woken.
fn moved(navigated: Result<NavigationResult, PortFailure>) -> Result<TabMoved, PluginError> {
    let navigated = navigated.map_err(refused)?;
    let list = navigated.list.ok_or_else(|| {
        PluginError::failed("the browser answered a user's navigation without its tab list")
    })?;
    Ok(TabMoved { list })
}

/// The tab a call names; one that is no tab's id is a tab the browser does
/// not have.
fn tab_id(tab: String) -> Result<TabId, PluginError> {
    TabId::try_from(tab).map_err(|_| PluginError::refused("tab_not_found", "No such tab"))
}

fn stopped(failure: &PortFailure) -> bool {
    matches!(
        failure,
        PortFailure::Refused(PortRefusal::Host {
            code: ErrorCode::HostStopped,
            ..
        })
    )
}

fn tab_missing(failure: &PortFailure) -> bool {
    browser_failure(failure)
        .is_some_and(|document| document.error.code == BrowserErrorCode::TabNotFound)
}

/// The browser's own failure, when the operation failed with one: what it
/// wrote to its standard error.
fn browser_failure(failure: &PortFailure) -> Option<FailureDocument> {
    let PortFailure::Refused(PortRefusal::Operation { stderr }) = failure else {
        return None;
    };
    serde_json::from_str(stderr.trim()).ok()
}

/// A failed call as the page sees it: the browser's refusal under its own
/// code, or the host access's, which passes on.
fn refused(failure: PortFailure) -> PluginError {
    match browser_failure(&failure) {
        Some(document) => {
            PluginError::refused(document.error.code.to_string(), document.error.message)
        }
        None => failure.into(),
    }
}

fn decode<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(params)).map_err(|error| PluginError::Usage {
        message: error.to_string(),
    })
}

fn to_value(value: impl Serialize) -> Result<Value, PluginError> {
    serde_json::to_value(value).map_err(PluginError::failed)
}
