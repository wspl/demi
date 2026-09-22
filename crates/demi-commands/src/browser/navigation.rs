//! Per-command main-document observation; subscriptions are dropped with the command.

use super::{BrowserError, BrowserTab, Result, operation::Operation, protocol::BrowserCommand};
use chromiumoxide::{
    Page,
    cdp::browser_protocol::{
        network::{EventLoadingFailed, EventRequestWillBeSent, LoaderId, RequestId, ResourceType},
        page::{
            EventFrameNavigated, EventFrameStartedLoading, EventLifecycleEvent,
            EventNavigatedWithinDocument, FrameId, GetFrameTreeParams, GetNavigationHistoryParams,
            NavigateParams, NavigateToHistoryEntryParams, NavigationEntry, ReloadParams,
        },
    },
    listeners::EventStream,
};
use futures_util::{FutureExt, StreamExt};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

/// How long a navigation nobody waits for may load.
const DETACHED_LOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// A page and its child frames can emit a burst while the baseline is read.
const NAVIGATION_EVENT_CAPACITY: usize = 256;

pub(super) enum Navigation {
    Url(String),
    Reload,
    History(i64),
}

pub(super) struct NavigationObservation {
    requests: EventStream<EventRequestWillBeSent>,
    failures: EventStream<EventLoadingFailed>,
    frames: EventStream<EventFrameNavigated>,
    same_document: EventStream<EventNavigatedWithinDocument>,
    lifecycle: EventStream<EventLifecycleEvent>,
    loading: EventStream<EventFrameStartedLoading>,
    frame: FrameId,
    previous_loader: LoaderId,
    expected_loader: Option<LoaderId>,
    documents: HashMap<RequestId, LoaderId>,
    states: HashMap<LoaderId, HashSet<String>>,
    committed: Option<LoaderId>,
    pub url: String,
}

enum Observed {
    Other,
    Started,
    SameDocument,
    Commit,
    Lifecycle,
    Failed(String),
}

impl NavigationObservation {
    pub fn document_changed(&self) -> bool {
        self.committed.is_some()
    }

    pub async fn subscribe(page: &Page) -> Result<Self> {
        let requests = page
            .event_listener_with_capacity(NAVIGATION_EVENT_CAPACITY)
            .await?;
        let failures = page
            .event_listener_with_capacity(NAVIGATION_EVENT_CAPACITY)
            .await?;
        let frames = page
            .event_listener_with_capacity(NAVIGATION_EVENT_CAPACITY)
            .await?;
        let same_document = page
            .event_listener_with_capacity(NAVIGATION_EVENT_CAPACITY)
            .await?;
        let lifecycle = page
            .event_listener_with_capacity(NAVIGATION_EVENT_CAPACITY)
            .await?;
        let loading = page
            .event_listener_with_capacity(NAVIGATION_EVENT_CAPACITY)
            .await?;
        let frame = page
            .execute(GetFrameTreeParams {})
            .await?
            .result
            .frame_tree
            .frame;
        let mut observation = Self {
            requests,
            failures,
            frames,
            same_document,
            lifecycle,
            loading,
            frame: frame.id,
            previous_loader: frame.loader_id,
            expected_loader: None,
            documents: HashMap::new(),
            states: HashMap::new(),
            committed: None,
            url: format!(
                "{}{}",
                frame.url,
                frame.url_fragment.as_deref().unwrap_or_default()
            ),
        };
        // Discard events preceding the baseline read, before the caller sends input.
        while observation.next().now_or_never().transpose()?.is_some() {}
        observation.expected_loader = None;
        observation.documents.clear();
        observation.states.clear();
        observation.committed = None;
        Ok(observation)
    }

    async fn next(&mut self) -> Result<Observed> {
        tokio::select! {
            biased;
            event = self.requests.next() => {
                let event = event.ok_or(BrowserError::TabNotFound)??;
                if event.frame_id.as_ref() == Some(&self.frame) && event.r#type == Some(ResourceType::Document) {
                    self.documents.insert(event.request_id.clone(), event.loader_id.clone());
                    if event.loader_id != self.previous_loader {
                        self.expected_loader.get_or_insert_with(|| event.loader_id.clone());
                    }
                    Ok(Observed::Started)
                } else { Ok(Observed::Other) }
            }
            event = self.failures.next() => {
                let event = event.ok_or(BrowserError::TabNotFound)??;
                if self.documents.get(&event.request_id).is_some_and(|loader| self.expected_loader.as_ref().is_none_or(|expected| loader == expected)) {
                    Ok(Observed::Failed(event.error_text.clone()))
                } else { Ok(Observed::Other) }
            }
            event = self.same_document.next() => {
                let event = event.ok_or(BrowserError::TabNotFound)??;
                if event.frame_id == self.frame {
                    self.url.clone_from(&event.url);
                    Ok(Observed::SameDocument)
                } else { Ok(Observed::Other) }
            }
            event = self.frames.next() => {
                let event = event.ok_or(BrowserError::TabNotFound)??;
                if event.frame.id == self.frame {
                    self.url = format!("{}{}", event.frame.url, event.frame.url_fragment.as_deref().unwrap_or_default());
                    if self.expected_loader.as_ref().is_none_or(|expected| *expected == event.frame.loader_id) && event.frame.loader_id != self.previous_loader {
                        self.committed = Some(event.frame.loader_id.clone());
                        if event.r#type == chromiumoxide::cdp::browser_protocol::page::NavigationType::BackForwardCacheRestore {
                            self.states.entry(event.frame.loader_id.clone()).or_default().extend(["DOMContentLoaded".into(), "load".into()]);
                        }
                    }
                    Ok(Observed::Commit)
                } else { Ok(Observed::Other) }
            }
            event = self.lifecycle.next() => {
                let event = event.ok_or(BrowserError::TabNotFound)??;
                if event.frame_id == self.frame {
                    self.states.entry(event.loader_id.clone()).or_default().insert(event.name.clone());
                    Ok(Observed::Lifecycle)
                } else { Ok(Observed::Other) }
            }
            event = self.loading.next() => {
                let event = event.ok_or(BrowserError::TabNotFound)??;
                Ok(if event.frame_id == self.frame { Observed::Started } else { Observed::Other })
            }
        }
    }

    pub async fn wait_load(
        &mut self,
        load: &str,
        ordinary_click: bool,
        operation: &Operation<'_>,
    ) -> Result<()> {
        // Only classifying a click's load start is bounded separately. A started
        // navigation shares the command deadline with every other action phase.
        let classification = tokio::time::Instant::now() + Duration::from_millis(250);
        let mut started = !ordinary_click;
        loop {
            let event = if started {
                self.next().await?
            } else {
                match tokio::time::timeout_at(classification, self.next()).await {
                    Ok(event) => event?,
                    Err(_) => return Ok(()),
                }
            };
            if matches!(
                event,
                Observed::Started | Observed::Commit | Observed::SameDocument
            ) {
                operation.complete_input();
            }
            match event {
                Observed::SameDocument if self.expected_loader.is_none() => return Ok(()),
                Observed::Failed(reason) if !ordinary_click => {
                    return Err(BrowserError::NavigationFailed(reason));
                }
                Observed::Failed(_) => return Ok(()),
                Observed::Started | Observed::Commit => started = true,
                _ => {}
            }
            if let Some(loader) = &self.committed
                && (load == "commit"
                    || self.states.get(loader).is_some_and(|states| {
                        states.contains(if load == "load" {
                            "load"
                        } else {
                            "DOMContentLoaded"
                        })
                    }))
            {
                return Ok(());
            }
        }
    }

    /// Observe buffered URL changes first, then the current URL after delivery.
    pub async fn wait_url(&mut self, page: &Page, pattern: &str) -> Result<()> {
        let matcher = url_pattern(pattern)?;
        while let Some(event) = self.next().now_or_never() {
            if matches!(event?, Observed::SameDocument | Observed::Commit)
                && matcher.is_match(&self.url)
            {
                return Ok(());
            }
        }
        let frame = page
            .execute(GetFrameTreeParams {})
            .await?
            .result
            .frame_tree
            .frame;
        self.url = format!(
            "{}{}",
            frame.url,
            frame.url_fragment.as_deref().unwrap_or_default()
        );
        if matcher.is_match(&self.url) {
            return Ok(());
        }
        loop {
            if matches!(
                self.next().await?,
                Observed::SameDocument | Observed::Commit
            ) && matcher.is_match(&self.url)
            {
                return Ok(());
            }
        }
    }
}

impl BrowserTab {
    pub(super) async fn navigate(
        &self,
        navigation: Navigation,
        load: &str,
        operation: &Operation<'_>,
        references: &mut super::observation::References,
    ) -> Result<String> {
        if let Navigation::Url(url) = &navigation {
            validate_url(url)?;
        }
        let mut observation = operation
            .run(NavigationObservation::subscribe(&self.page))
            .await?;
        let result = operation
            .run(async {
                operation.begin_input();
                let request = async {
                    let failure = match navigation {
                        Navigation::Url(url) => {
                            self.page
                                .execute(NavigateParams::new(url))
                                .await?
                                .result
                                .error_text
                        }
                        Navigation::Reload => {
                            self.page.execute(ReloadParams::default()).await?;
                            None
                        }
                        Navigation::History(id) => {
                            self.page
                                .execute(NavigateToHistoryEntryParams::new(id))
                                .await?;
                            None
                        }
                    };
                    operation.complete_input();
                    if let Some(error) = failure {
                        return Err(BrowserError::NavigationFailed(error));
                    }
                    Ok(())
                };
                tokio::pin!(request);
                // Chromiumoxide holds Page.navigate's response until its own load
                // waiter completes. Our subscribed events establish the requested
                // state independently. Dropping that response receiver does not
                // cancel input; the driver's existing bounded request owns its expiry.
                tokio::select! {
                    biased;
                    result = &mut request => {
                        result?;
                        observation.wait_load(load, false, operation).await?;
                    }
                    result = observation.wait_load(load, false, operation) => { result?; }
                }
                Ok(observation.url.clone())
            })
            .await;
        if observation.document_changed() {
            references.invalidate();
        }
        match result {
            Ok(url) => Ok(url),
            Err(error) => {
                // A failed diagnostic must preserve the navigation's original cause.
                if let Ok(Ok(Some(url))) =
                    tokio::time::timeout(super::operation::CONTROL_TIMEOUT, self.page.url()).await
                {
                    observation.url = url;
                }
                Err(operation.failure(error, &self.id(), Some(&observation.url)))
            }
        }
    }
}

/// Wait for the document current at subscription, never for a future navigation.
pub(super) async fn wait_current_load(page: &Page, load: &str) -> Result<()> {
    let mut lifecycle = page
        .event_listener_with_capacity::<EventLifecycleEvent>(NAVIGATION_EVENT_CAPACITY)
        .await?;
    let mut navigated = page
        .event_listener_with_capacity::<EventFrameNavigated>(NAVIGATION_EVENT_CAPACITY)
        .await?;
    let frame = page
        .execute(GetFrameTreeParams {})
        .await?
        .result
        .frame_tree
        .frame;
    let ready: String = page
        .evaluate_expression("document.readyState")
        .await?
        .into_value()
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    let after = page
        .execute(GetFrameTreeParams {})
        .await?
        .result
        .frame_tree
        .frame;
    if after.loader_id != frame.loader_id {
        return Err(BrowserError::NavigationFailed(
            "current document was replaced before its load wait completed".into(),
        ));
    }
    if load == "commit"
        || ready == "complete"
        || (load == "domcontentloaded" && ready == "interactive")
    {
        return Ok(());
    }
    loop {
        let event = tokio::select! {
            event = lifecycle.next() => event.ok_or(BrowserError::Closed)??,
            event = navigated.next() => {
                let event = event.ok_or(BrowserError::Closed)??;
                if event.frame.id == frame.id && event.frame.loader_id != frame.loader_id {
                    return Err(BrowserError::NavigationFailed("current document was replaced before its load wait completed".into()));
                }
                continue;
            }
        };
        if event.frame_id == frame.id
            && event.loader_id == frame.loader_id
            && (event.name == "load"
                || (load == "domcontentloaded" && event.name == "DOMContentLoaded"))
        {
            return Ok(());
        }
    }
}

/// The history entry one step back or forward from the tab's current one.
pub(super) async fn history_step(
    tab: &BrowserTab,
    back: bool,
    operation: &Operation<'_>,
) -> Result<NavigationEntry> {
    let history = operation
        .run(async {
            Ok(tab
                .page
                .execute(GetNavigationHistoryParams {})
                .await?
                .result)
        })
        .await?;
    let index = history.current_index + if back { -1 } else { 1 };
    usize::try_from(index)
        .ok()
        .and_then(|index| history.entries.into_iter().nth(index))
        .ok_or(BrowserError::HistoryBoundary)
}

/// Starts loading `url` in `tab` as an address bar does. The user sees the
/// page load, or Chrome's own error page, so nothing waits for it.
/// Loads the page again without waiting for it.
pub(super) fn reload(tab: &BrowserTab) {
    detach(tab, ReloadParams::default());
}

pub(super) fn visit(tab: &BrowserTab, url: &str) {
    detach(tab, NavigateParams::new(url));
}

/// Sends a navigation without waiting for it: chromiumoxide answers one
/// only once the page loaded.
fn detach<C>(tab: &BrowserTab, command: C)
where
    C: chromiumoxide::types::Command + Send + 'static,
    C::Response: Send,
{
    let page = tab.page.clone();
    let ended = tab.ended.clone();
    tokio::spawn(async move {
        // A closing tab must not receive a late address-bar navigation. The user
        // sees load failures in Chrome; there is no command waiting for an answer.
        tokio::select! {
            biased;
            _ = ended.cancelled() => {},
            _ = tokio::time::timeout(DETACHED_LOAD_TIMEOUT, page.execute(command)) => {},
        }
    });
}

/// The user's `goto`, `reload`, `back` or `forward` (`web-api.md` §
/// Conversation browser tabs): the work panel shows the page loading, so the
/// navigation starts and the answer does not wait for it.
pub(super) async fn steer(
    tab: &BrowserTab,
    command: &BrowserCommand,
    operation: &Operation<'_>,
) -> Result<serde_json::Value> {
    let url = match command {
        BrowserCommand::Goto(input) => {
            validate_url(&input.url)?;
            visit(tab, &input.url);
            Some(input.url.clone())
        }
        BrowserCommand::Reload(_) => {
            let url = operation.run(async { Ok(tab.page.url().await?) }).await?;
            detach(tab, ReloadParams::default());
            url
        }
        BrowserCommand::Back(_) | BrowserCommand::Forward(_) => {
            let back = matches!(command, BrowserCommand::Back(_));
            let entry = history_step(tab, back, operation).await?;
            detach(tab, NavigateToHistoryEntryParams::new(entry.id));
            Some(entry.url)
        }
        _ => {
            return Err(BrowserError::Configuration(
                "not a navigation command".into(),
            ));
        }
    };
    let mut result = serde_json::json!({ "tab": tab.id() });
    if let Some(url) = url {
        result["url"] = serde_json::json!(url);
    }
    Ok(result)
}

/// Compile the browser URL glob; regex escaping owns every literal character.
fn url_pattern(pattern: &str) -> Result<regex::Regex> {
    let mut expression = String::from("\\A");
    let mut characters = pattern.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '*' {
            if characters.next_if_eq(&'*').is_some() {
                expression.push_str("(?s:.*)");
            } else {
                expression.push_str("[^/]*");
            }
        } else {
            expression.push_str(&regex::escape(&character.to_string()));
        }
    }
    expression.push_str("\\z");
    regex::Regex::new(&expression).map_err(|error| BrowserError::Configuration(error.to_string()))
}

/// Validate every requested browser URL before a batch can create or navigate tabs.
pub(super) fn validate_url(url: &str) -> Result<()> {
    let parsed =
        url::Url::parse(url).map_err(|error| BrowserError::Configuration(error.to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https" | "file") && parsed.as_str() != "about:blank" {
        return Err(BrowserError::Configuration(
            "navigation accepts http:, https:, file:, or about:blank".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::url_pattern;
    #[test]
    fn literal_browser_url_globs() {
        for (pattern, url, matches) in [
            ("/a/*", "/a/b/c", false),
            ("/a/**", "/a/b/c", true),
            ("/a/?", "/a/b", false),
            ("/a/?", "/a/?", true),
            ("/[a]{b}", "/[a]{b}", true),
            ("/[a]{b}", "/ab", false),
        ] {
            assert_eq!(url_pattern(pattern).unwrap().is_match(url), matches);
        }
    }
}
