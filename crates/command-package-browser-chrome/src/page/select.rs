//! Native select matching and retry share the browser's element conditions.

use serde::Deserialize;
use serde_json::{Value, json};

use crate::driver::operation::{BrowserError, Operation, Result};
use crate::tabs::{session::References, tab::BrowserTab};

use crate::page::{element, protocol::BrowserTarget};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectedOptions {
    status: SelectionStatus,
    values: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SelectionStatus {
    Ready,
    Disabled,
    Missing,
}

/// Select browser options in document order within the command's shared deadline.
pub(crate) async fn select_options(
    tab: &BrowserTab,
    target: &BrowserTarget,
    references: &mut References,
    candidates: [Option<Value>; 3],
    operation: &Operation<'_>,
) -> Result<Vec<String>> {
    if candidates.iter().flatten().count() != 1
        || candidates
            .iter()
            .flatten()
            .any(|values| values.as_array().is_none_or(Vec::is_empty))
    {
        return Err(BrowserError::Configuration(
            "select requires one nonempty value, option-label, or option-index list".into(),
        ));
    }
    let args: Vec<_> = candidates
        .into_iter()
        .map(|candidate| candidate.unwrap_or(Value::Null))
        .collect();
    let mut last_failure = None;
    let result: Result<Vec<String>> = async {
        loop {
            let (element, _) = crate::page::element::ready_with_failure(
                tab,
                target,
                references,
                element::SELECT,
                operation,
                &mut last_failure,
            )
            .await?;
            let mut probe = args.clone();
            probe.push(json!(false));
            // Chromiumoxide exposes no select-options action. The page algorithm
            // uses the shared element-state routine for control and option readiness.
            let mut selected: SelectedOptions = operation
                .run(element::call_with_states(
                    tab.page(),
                    &element,
                    include_str!("select-options.js"),
                    probe,
                ))
                .await?;
            if matches!(selected.status, SelectionStatus::Ready) {
                let mut apply = args.clone();
                apply.push(json!(true));
                operation.begin_input();
                selected = operation
                    .run(element::call_with_states(
                        tab.page(),
                        &element,
                        include_str!("select-options.js"),
                        apply,
                    ))
                    .await?;
                if matches!(selected.status, SelectionStatus::Ready) {
                    operation.complete_input();
                    return Ok(selected.values);
                }
                // The algorithm reports missing/disabled before changing selection.
                operation.input_not_delivered();
            }
            last_failure = Some(match selected.status {
                SelectionStatus::Disabled => BrowserError::NotActionable {
                    condition: "enabled select option or control".into(),
                    interceptor: None,
                },
                SelectionStatus::Missing => BrowserError::TargetNotFound,
                SelectionStatus::Ready => unreachable!("ready selections returned above"),
            });
            operation
                .run(async {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    Ok(())
                })
                .await?;
        }
    }
    .await;
    match result {
        Err(error) if error.is_deadline() => Err(match last_failure {
            Some(cause) => error.with_deadline_cause(cause),
            None => error,
        }),
        result => result,
    }
}
