mod browser_families;

use browser_families::with_browser_fixture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires pinned real Chrome for Testing"]
async fn upload_attaches_files_and_cleans_chooser_observation() {
    with_browser_fixture(|fixture| async move {
        let tab = fixture.open("upload.html").await;
        for name in ["one.txt", "two.txt"] {
            tokio::fs::write(fixture.root.path().join(name), name)
                .await
                .unwrap();
        }
        for (css, files, names) in [
            ("#single", vec!["one.txt"], "one.txt"),
            ("#multiple", vec!["one.txt", "two.txt"], "one.txt,two.txt"),
            ("#choose", vec!["two.txt", "one.txt"], "two.txt,one.txt"),
        ] {
            let result = fixture
                .call(
                    "browser.upload",
                    json!({"tab": tab, "css": css, "file": files}),
                )
                .await;
            assert_eq!(result["attached"], files.len());
            let names_result = fixture
                .call(
                    "browser.read",
                    json!({"tab": tab, "css": "#names", "property": "text"}),
                )
                .await;
            assert_eq!(names_result["value"], names);
        }
        for (css, files, expected, timeout) in [
            ("#single", vec!["one.txt", "two.txt"], "invalid_input", 2000),
            ("#choose", vec!["one.txt", "missing.txt"], "io_error", 2000),
            ("#choose", vec!["."], "invalid_input", 2000),
            ("#disabled", vec!["one.txt"], "not_actionable", 200),
            ("#no-chooser", vec!["one.txt"], "timeout", 300),
        ] {
            let (code, result) = fixture
                .result(
                    "browser.upload",
                    json!({"tab": tab, "css": css, "file": files, "timeout": timeout}),
                    CancellationToken::new(),
                )
                .await;
            assert_eq!(
                code,
                if expected == "invalid_input" { 2 } else { 1 },
                "{result}"
            );
            assert_eq!(result["error"]["code"], expected, "{result}");
        }
        assert_eq!(
            fixture
                .call(
                    "browser.eval",
                    json!({"tab": tab, "expression": "triggers"})
                )
                .await["value"],
            2
        );
        let cancel = CancellationToken::new();
        let cancelled = cancel.clone();
        let (result, ()) = tokio::join!(
            fixture.result(
                "browser.upload",
                json!({"tab": tab, "css": "#no-chooser", "file": ["one.txt"]}),
                cancel
            ),
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                cancelled.cancel();
            }
        );
        assert!(matches!(result.0, 1 | 130), "{result:?}");
        fixture
            .call(
                "browser.upload",
                json!({"tab": tab, "css": "#choose", "file": ["one.txt"]}),
            )
            .await;
        assert_eq!(
            fixture
                .call(
                    "browser.read",
                    json!({"tab": tab, "css": "#names", "property": "text"})
                )
                .await["value"],
            "one.txt"
        );
        fixture
    })
    .await;
}
