//! The tab registry through the service (`browser.md` § One tab registry).

use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::families::{with_browser_fixture, with_numbered_browser_fixture};

/// A command holding one tab does not hold up commands on another tab, and a
/// second command on the held tab reports it busy (`browser.md` § Required
/// checks 5).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn commands_on_another_tab_run_while_one_tab_is_held() {
    with_browser_fixture(|fixture| async move {
        let held = fixture.open("fixture.html").await;
        let other = fixture.open("fixture.html").await;
        let cancel = CancellationToken::new();
        let waiting = fixture.result(
            "browser.wait",
            json!({"tab": held, "url": "**/never", "timeout": 30000}),
            cancel.clone(),
        );
        let alongside = async {
            fixture.wait_until_busy(&held).await;
            let info = fixture.call("browser.info", json!({"tab": other})).await;
            assert_eq!(info["tab"], other);
            fixture.call("browser.inspect", json!({"tab": other})).await;
            let tabs = fixture.call("browser.tabs", json!({})).await;
            let ids: Vec<_> = tabs["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tab| tab["id"].clone())
                .collect();
            assert_eq!(ids, [held.clone(), other.clone()]);
            // The wait still holds its tab: the commands above ran beside it.
            let (_, busy) = fixture
                .result(
                    "browser.info",
                    json!({"tab": held}),
                    CancellationToken::new(),
                )
                .await;
            assert_eq!(busy["error"]["code"], "tab_busy", "{busy}");
            cancel.cancel();
        };
        let ((code, _), ()) = tokio::join!(waiting, alongside);
        assert_eq!(code, 130);
        fixture
    })
    .await;
}

/// A click that opens a tab names it once the registry holds it, and a click
/// that opens none names nothing (`browser.md` § One tab registry).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn an_action_names_the_tabs_it_opened() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("popups.html").await;
        let plain = fixture
            .call("browser.click", json!({"tab": tab, "css": "#stay"}))
            .await;
        assert!(plain.get("openedTabs").is_none(), "{plain}");
        let clicked = fixture
            .call("browser.click", json!({"tab": tab, "css": "#open"}))
            .await;
        let opened = clicked["openedTabs"]
            .as_array()
            .unwrap_or_else(|| panic!("the click names the tab it opened: {clicked}"))
            .clone();
        assert_eq!(opened.len(), 1, "{clicked}");
        // The named tab is registered: it can be operated at once, and the
        // list says who opened it.
        let info = fixture
            .call("browser.info", json!({"tab": opened[0]}))
            .await;
        assert_eq!(info["url"], "about:blank");
        let tabs = fixture.call("browser.tabs", json!({})).await;
        let popup = tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|listed| listed["id"] == opened[0])
            .unwrap_or_else(|| panic!("the list holds the popup: {tabs}"));
        assert_eq!(popup["createdBy"], json!({"kind": "page", "opener": tab}));
        // A later action does not name the tab again; the popup stays in front.
        let again = fixture
            .call("browser.click", json!({"tab": tab, "css": "#stay"}))
            .await;
        assert!(again.get("openedTabs").is_none(), "{again}");
        fixture
    })
    .await;
}

/// A tab number the backend does not give fails only the step that needed
/// it: the `open` answers why, a popup is closed, and the environment keeps
/// its tabs; the next step draws again (`browser.md` § One tab registry).
/// Here the backend gives the first draw's eight numbers, then fails twice,
/// then gives numbers from 101. About 12 s on a small Linux VM, as long as
/// `an_action_names_the_tabs_it_opened` there: most of it starts Chrome.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_failed_number_draw_fails_only_the_step_that_needs_a_number() {
    let (numbers, mut draws) = demi_command_sdk::Numbers::channel();
    let unreachable = "the backend is unreachable";
    let answering = tokio::spawn(async move {
        let mut drawn = Vec::new();
        for answer in [Ok(1), Err(unreachable), Err(unreachable), Ok(101)] {
            let Some(draw) = draws.recv().await else {
                break;
            };
            drawn.push(draw.request.count);
            // A step that stopped waiting needs no answer.
            let _left = draw.answer.send(answer.map_err(str::to_owned));
        }
        drawn
    });
    with_numbered_browser_fixture(numbers, |fixture| async move {
        let page = url::Url::from_file_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser/popups.html"),
        )
        .unwrap();
        let mut opened = Vec::new();
        let refused = loop {
            let (code, result) = fixture
                .result(
                    "browser.open",
                    json!({"url": page.as_str()}),
                    CancellationToken::new(),
                )
                .await;
            if code != 0 {
                break result;
            }
            opened.push(result["tab"].as_str().unwrap().to_owned());
        };
        assert_eq!(
            opened,
            (1..=8)
                .map(|number| format!("t{number}"))
                .collect::<Vec<_>>()
        );
        assert_eq!(refused["error"]["code"], "browser_unavailable", "{refused}");
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(message.contains(unreachable), "{refused}");
        // A popup that gets no number is closed; its opener stays.
        let clicked = fixture
            .call("browser.click", json!({"tab": "t1", "css": "#keep"}))
            .await;
        assert!(clicked.get("openedTabs").is_none(), "{clicked}");
        // Chrome closes it after the click answered; read-only evaluation
        // cannot wait in the page, so the opener asks until it sees it.
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let closed = fixture
                    .call(
                        "browser.eval",
                        json!({"tab": "t1", "expression": "window.opened.closed"}),
                    )
                    .await;
                if closed["value"] == true {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Chrome closes the popup");
        let tabs = fixture.call("browser.tabs", json!({})).await;
        let listed: Vec<_> = tabs["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab["id"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(listed, opened);
        // The next step draws again.
        let next = fixture
            .call("browser.open", json!({"url": page.as_str()}))
            .await;
        assert_eq!(next["tab"], "t101");
        fixture
    })
    .await;
    assert_eq!(answering.await.unwrap(), [8, 8, 8, 8]);
}

/// An action works in a tab that is not the front tab of its window: its
/// hidden document runs no animation frames (`browser.md` § Actionability
/// and coordinates).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn an_action_works_in_the_older_of_two_open_tabs() {
    with_browser_fixture(|fixture| async move {
        let older = fixture.open("popups.html").await;
        let newer = fixture.open("popups.html").await;
        let visibility = |tab: &str| {
            fixture.call(
                "browser.eval",
                json!({"tab": tab, "expression": "document.visibilityState"}),
            )
        };
        assert_eq!(visibility(&older).await["value"], "hidden");
        assert_eq!(visibility(&newer).await["value"], "visible");
        fixture
            .call("browser.click", json!({"tab": older, "css": "#stay"}))
            .await;
        let clicks = fixture
            .call(
                "browser.eval",
                json!({"tab": older, "expression": "window.stays"}),
            )
            .await;
        assert_eq!(clicks["value"], 1);
        // Neither tab came to the front.
        assert_eq!(visibility(&older).await["value"], "hidden");
        fixture
    })
    .await;
}

/// The tab list shows the title each page has now, as `info` does, although
/// Chrome's target events carry only the address that stood in for it when
/// the page committed (`browser.md` § Tabs and navigation).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn the_tab_list_shows_the_title_a_page_has_now() {
    with_browser_fixture(|fixture| async move {
        let path = fixture.root.path().join("titled.html");
        std::fs::write(
            &path,
            "<!doctype html><title>First title</title><script>document.title = 'Listed title'</script>",
        )
        .unwrap();
        let url = url::Url::from_file_path(path).unwrap();
        let opened = fixture.call("browser.open", json!({"url": url.as_str()})).await;
        let info = fixture.call("browser.info", json!({"tab": opened["tab"]})).await;
        assert_eq!(info["title"], "Listed title");
        let tabs = fixture.call("browser.tabs", json!({})).await;
        assert_eq!(tabs["tabs"][0]["title"], "Listed title", "{tabs}");
        fixture
    })
    .await;
}

/// `show` and `open --show` count the times the agent showed a tab, which
/// the tab list carries to the user's work panel (`live-view.md` § Showing
/// a tab); a tab never shown counts 0.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn showing_a_tab_raises_its_count_in_the_tab_list() {
    with_browser_fixture(|fixture| async move {
        let first = fixture.open("fixture.html").await;
        let shows = async || {
            let tabs = fixture.call("browser.tabs", json!({})).await;
            tabs["tabs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|tab| (tab["id"].as_str().unwrap().to_owned(), tab["shows"].as_u64().unwrap()))
                .collect::<Vec<_>>()
        };
        assert_eq!(shows().await, [(first.clone(), 0)]);
        let shown = fixture.call("browser.show", json!({"tab": first})).await;
        assert_eq!(shown, json!({"tab": first}));
        fixture.call("browser.show", json!({"tab": first})).await;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser/fixture.html");
        let url = url::Url::from_file_path(path).unwrap();
        let second = fixture
            .call("browser.open", json!({"url": url.as_str(), "show": true}))
            .await["tab"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(shows().await, [(first, 2), (second, 1)]);
        fixture
    })
    .await;
}

/// A command answers readable text unless the agent asks for JSON, as its
/// shell's `--json` does, and an action's or a wait's text names the element
/// its target resolved to (`browser.md` § Default text).
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn commands_answer_readable_text_unless_json_is_asked() {
    with_browser_fixture(|fixture| async move {
        let mut readable = fixture.clone();
        readable.json = false;
        let path = fixture.root.path().join("readable.html");
        std::fs::write(
            &path,
            "<!doctype html><title>Readable page</title><h1>Welcome back</h1>\
            <button>Sign in</button>\
            <select aria-label=\"Country\"><option value=\"JP\">Japan</option>\
            <option value=\"SG\">Singapore</option></select>",
        )
        .unwrap();
        let url = url::Url::from_file_path(path).unwrap();
        let text = async |operation: &str, args: serde_json::Value| {
            let (code, answer) = readable
                .result(operation, args, CancellationToken::new())
                .await;
            assert_eq!(code, 0, "{operation}: {answer}");
            answer["diagnostic"].as_str().unwrap().to_owned()
        };
        let opened = text("browser.open", json!({"url": url.as_str()})).await;
        let tab = opened
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("Tab: "))
            .unwrap();
        assert!(opened.contains("\nTitle: Readable page\n"), "{opened}");
        let listed = text("browser.tabs", json!({})).await;
        let row = listed.lines().nth(1).unwrap();
        assert!(
            row.starts_with(tab) && row.contains("Readable page"),
            "{listed}"
        );
        let info = fixture.call("browser.info", json!({"tab": tab})).await;
        assert_eq!(info["title"], "Readable page");
        let reference = regex::Regex::new(r"\[ref=e[0-9]+\]").unwrap();
        let clicked = text(
            "browser.click",
            json!({"tab": tab, "role": "button", "name": "Sign in"}),
        )
        .await;
        // A click also reports the URL it waited for.
        assert_eq!(
            reference.replace(clicked.lines().next().unwrap(), "[ref]"),
            "Clicked button \"Sign in\" [ref]."
        );
        let selected = text(
            "browser.select",
            json!({"tab": tab, "role": "combobox", "name": "Country", "option-label": ["Singapore"]}),
        )
        .await;
        assert_eq!(selected, "Selected: Singapore (SG).\n");
        let matched = text(
            "browser.wait",
            json!({"tab": tab, "role": "heading", "name": "Welcome back", "state": "visible"}),
        )
        .await;
        assert_eq!(
            reference.replace(&matched, "[ref]"),
            "Matched heading \"Welcome back\" [ref]. State: visible.\n"
        );
        fixture
    })
    .await;
}
