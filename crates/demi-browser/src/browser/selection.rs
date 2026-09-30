//! Rendered text selection shares browser references and visibility checks.
use super::{
    BrowserError, BrowserTab, Result, element,
    observation::References,
    operation::Operation,
    protocol::{ActionResult, BrowserTarget, SelectTextInput},
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    status: String,
    count: usize,
}

impl BrowserTab {
    pub(super) async fn select_text(
        &self,
        target: &BrowserTarget,
        refs: &mut References,
        input: &SelectTextInput,
        operation: &Operation<'_>,
    ) -> Result<ActionResult> {
        if input.text.is_empty() {
            return Err(BrowserError::Configuration(
                "select-text requires nonempty text".into(),
            ));
        }
        let mut last_failure = None;
        loop {
            let attempt = async {
                let (element, _) = self
                    .ready_element_with_failure(
                        target,
                        refs,
                        &["visible"],
                        operation,
                        &mut last_failure,
                    )
                    .await?;
                operation.begin_input();
                let selected: Selection = operation
                    .run(element::call(
                        &self.page,
                        &element,
                        include_str!("select-text.js"),
                        vec![
                            json!(input.text),
                            json!(input.cursor),
                            json!(input.prefix),
                            json!(input.suffix),
                        ],
                    ))
                    .await?;
                if selected.status != "selected" {
                    operation.input_not_delivered();
                }
                match selected.status.as_str() {
                    "selected" => {
                        operation.complete_input();
                        let result = input.cursor.map_or("selected".to_owned(), |cursor| cursor.to_string());
                        Ok(Some(ActionResult::new("select-text", json!(result))))
                    }
                    "missing" => {
                        last_failure = Some(BrowserError::NotActionable {
                            condition: "requested text is rendered".into(),
                            interceptor: None,
                        });
                        operation
                            .run(async {
                                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                                Ok(())
                            })
                            .await?;
                        Ok(None)
                    }
                    "ambiguous" => Err(BrowserError::Ambiguous(selected.count)),
                    "unsupported" => Err(BrowserError::NotActionable {
                        condition: "text selection is unsupported on this input type".into(),
                        interceptor: None,
                    }),
                    _ => Err(BrowserError::InvalidResult(
                        "unknown text selection result".into(),
                    )),
                }
            }
            .await;
            match attempt {
                Ok(Some(result)) => return Ok(result),
                Ok(None) => {}
                Err(error) if error.is_deadline() => {
                    return Err(error.with_deadline_cause(
                        last_failure.take().unwrap_or(BrowserError::TargetNotFound),
                    ));
                }
                Err(error) => return Err(error),
            }
        }
    }
}
