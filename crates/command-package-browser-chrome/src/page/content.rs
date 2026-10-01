//! Content reads and temporary fetch tabs share one extraction path.
use crate::driver::operation::{BrowserError, Result};
use crate::tabs::{session::References, tab::BrowserTab};

use crate::page::{observation::Observation, protocol::ContentFormat};

pub(crate) async fn read(
    tab: &BrowserTab,
    format: ContentFormat,
    references: &mut References,
) -> Result<String> {
    match format {
        ContentFormat::Html => Ok(tab
            .page()
            .find_element("html")
            .await?
            .outer_html()
            .await?
            .unwrap_or_default()),
        ContentFormat::Text => Ok(tab
            .page()
            .find_element("body")
            .await?
            .inner_text()
            .await?
            .unwrap_or_default()),
        ContentFormat::Dom => {
            let observation = Observation::capture(tab.page(), references).await?;
            serde_json::to_string(&observation.dom_tree(references, usize::MAX)?.0)
                .map_err(|error| BrowserError::InvalidResult(error.to_string()))
        }
    }
}
