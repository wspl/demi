//! Content reads and temporary fetch tabs share one extraction path.
use super::{
    BrowserError, BrowserTab, Result,
    observation::{Observation, References},
};

impl BrowserTab {
    pub(super) async fn content(
        &self,
        format: &str,
        references: &mut References,
    ) -> Result<String> {
        match format {
            "html" => Ok(self
                .page
                .find_element("html")
                .await?
                .outer_html()
                .await?
                .unwrap_or_default()),
            "text" => Ok(self
                .page
                .find_element("body")
                .await?
                .inner_text()
                .await?
                .unwrap_or_default()),
            "dom" => {
                let observation = Observation::capture(&self.page, references).await?;
                serde_json::to_string(&observation.dom_tree(references, usize::MAX)?.0)
                    .map_err(|error| BrowserError::InvalidResult(error.to_string()))
            }
            _ => Err(BrowserError::Configuration(format!(
                "unknown browser content format: {format}"
            ))),
        }
    }
}
