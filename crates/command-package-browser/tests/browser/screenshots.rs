//! A screenshot the command would return as a medium, and the bounds of a
//! capture (`browser.md` § Images and large outputs).

use demi_command_protocol::StdoutTarget;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::families::{BrowserFixture, with_browser_fixture};

/// The exit code and diagnostic of a full-page screenshot of `tab` that a
/// job's command returns as a medium.
async fn full_page_medium(fixture: &mut BrowserFixture, tab: &str) -> (u8, String) {
    fixture.stdout = Some(StdoutTarget::Job);
    fixture.json = false;
    let (code, failure) = fixture
        .result(
            "browser.screenshot",
            json!({"tab": tab, "full-page": true}),
            CancellationToken::new(),
        )
        .await;
    fixture.stdout = None;
    fixture.json = true;
    (code, failure["diagnostic"].as_str().unwrap_or_default().into())
}

/// The size a diagnostic matching `pattern` names in its first group.
fn named_size(pattern: &str, text: &str) -> u64 {
    regex::Regex::new(pattern)
        .unwrap()
        .captures(text)
        .unwrap_or_else(|| panic!("{text}"))[1]
        .parse()
        .unwrap()
}

/// About 2 s here: Chrome starts, and a full page of noise is captured
/// twice, over 16 MiB each time.
///
/// Planted defect this catches: a screenshot handed to the writer whatever
/// its size, which refuses it as a failed invocation without telling the
/// model what to do instead.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_screenshot_over_16_mib_fails_and_names_its_size_and_the_output_file_that_keeps_it() {
    with_browser_fixture(|mut fixture| async move {
        let tab = fixture.open("noise.html").await;

        // To the job's output, the screenshot would be a medium: the command
        // fails, and says how large it is and what keeps it.
        let (code, text) = full_page_medium(&mut fixture, &tab).await;
        assert_eq!(code, 1, "{text}");
        let size = named_size(
            r"^Error: result_too_large\nthe screenshot is (\d+) bytes, more than the 16777216 a command returns as an image; save it to a file with --output <file>\n",
            &text,
        );
        assert!(size > 16 * 1024 * 1024, "{size}");

        // The file it names keeps the whole page.
        let saved = fixture
            .call(
                "browser.screenshot",
                json!({"tab": tab, "full-page": true, "output": "page.png"}),
            )
            .await;
        assert_eq!((saved["width"].clone(), saved["height"].clone()), (json!(1280), json!(6000)));
        let file = std::fs::metadata(fixture.root.path().join("page.png")).unwrap();
        assert_eq!(file.len(), size);
        fixture
    })
    .await;
}

/// About 2.5 s here: Chrome starts, draws 18 million noisy pixels, and
/// sends a capture of them over 64 MiB once.
///
/// Planted defect this catches: a CDP message over the connection's limit
/// rejected from its frame header, which loses the connection and with it
/// the browser and every tab.
#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn a_capture_chrome_cannot_send_fails_alone_and_the_browser_goes_on() {
    with_browser_fixture(|mut fixture| async move {
        let mut url = url::Url::from_file_path(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/browser/noise.html"),
        )
        .unwrap();
        url.set_query(Some("height=14000"));
        let opened = fixture
            .call("browser.open", json!({"url": url.as_str(), "timeout": 120000}))
            .await;
        let tab = opened["tab"].as_str().unwrap().to_owned();

        // About 54 MB of PNG is over 64 MiB as Chrome sends it, in base64:
        // even a file cannot keep it, and the command says what can.
        let (code, text) = full_page_medium(&mut fixture, &tab).await;
        assert_eq!(code, 1, "{text}");
        let size = named_size(
            r"^Error: result_too_large\nthe screenshot is too large for Chrome to send: the CDP message is (\d+) bytes, more than the 67108864 bytes a message may have; capture a part of the page with --clip x,y,width,height\n",
            &text,
        );
        assert!(size > 64 * 1024 * 1024, "{size}");

        // The same browser and tab answer the next call.
        let shown: Value = fixture
            .call(
                "browser.screenshot",
                json!({"tab": tab, "clip": "0,0,1280,100", "output": "part.png"}),
            )
            .await;
        assert_eq!((shown["width"].clone(), shown["height"].clone()), (json!(1280), json!(100)));
        fixture
    })
    .await;
}
