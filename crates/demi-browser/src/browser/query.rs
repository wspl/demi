//! Declarative browser queries combine the same observed node identities as locators.
use super::{
    BrowserError, Result,
    element::{self, TargetElement},
    observation::{Observation, References, validate_target},
    protocol::{BrowserQuery, BrowserTarget},
};
use chromiumoxide::Page;
use serde_json::json;

/// Use Chrome's ECMAScript engine for both browser name and text patterns.
pub(super) async fn pattern_matches(
    page: &Page,
    pattern: &str,
    values: &[&str],
) -> Result<Vec<bool>> {
    let source = format!(
        "(() => {{ try {{ const pattern = new RegExp({}); return {{matches: {}.map(value => pattern.test(value))}}; }} catch(error) {{ return {{error: error.message}}; }} }})()",
        json!(pattern),
        json!(values)
    );
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum PatternResult {
        Matches { matches: Vec<bool> },
        Error { error: String },
    }
    let result: PatternResult = page
        .evaluate_expression(source)
        .await?
        .into_value()
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    match result {
        PatternResult::Matches { matches } => Ok(matches),
        PatternResult::Error { error } => Err(BrowserError::Configuration(error)),
    }
}

/// Validate the entire browser query before resolving any branch or publishing references.
pub(super) fn parse(body: &str) -> Result<BrowserQuery> {
    let query =
        BrowserQuery::parse(body).map_err(|error| BrowserError::Configuration(error.to_string()))?;
    for branch in query.branches() {
        if let Some(locator) = &branch.r#match {
            validate_target(&BrowserTarget::from(locator.clone()))?;
        }
    }
    Ok(query)
}

impl Observation {
    pub(super) fn query<'a>(
        &'a self,
        page: &'a Page,
        query: &'a BrowserQuery,
        refs: &'a References,
    ) -> futures_util::future::BoxFuture<'a, Result<Vec<TargetElement>>> {
        self.query_in(page, query, refs, None)
    }

    fn query_in<'a>(
        &'a self,
        page: &'a Page,
        query: &'a BrowserQuery,
        refs: &'a References,
        mut scope: Option<super::observation::DomIdentity>,
    ) -> futures_util::future::BoxFuture<'a, Result<Vec<TargetElement>>> {
        Box::pin(async move {
            for (container, frame) in [(query.frame.as_ref(), true), (query.within.as_ref(), false)]
            {
                if let Some(container) = container {
                    let container = element::single(
                        page,
                        self.query_in(page, container, refs, scope.clone()).await?,
                    )
                    .await?
                    .ok_or(BrowserError::TargetNotFound)?;
                    if frame {
                        let is_frame: bool = element::call(page, &container, "function() { return this.localName === 'iframe' || this.localName === 'frame'; }", vec![]).await?;
                        if !is_frame {
                            return Err(BrowserError::Configuration(
                                "query frame scope must resolve to a frame".into(),
                            ));
                        }
                    }
                    scope = Some(container.identity());
                }
            }
            let mut matches = if let Some(locator) = &query.r#match {
                self.resolve(page, &BrowserTarget::from(locator.clone()), refs)
                    .await?
            } else {
                let mut branches = query
                    .and
                    .as_ref()
                    .or(query.or.as_ref())
                    .expect("query base was validated")
                    .iter();
                let mut matches = self
                    .query_in(
                        page,
                        branches.next().expect("nonempty query array"),
                        refs,
                        scope.clone(),
                    )
                    .await?;
                for branch in branches {
                    let next = self.query_in(page, branch, refs, scope.clone()).await?;
                    if query.and.is_some() {
                        let ids: std::collections::HashSet<_> =
                            next.iter().map(|element| element.identity()).collect();
                        matches.retain(|element| ids.contains(&element.identity()));
                    } else {
                        matches.extend(next);
                    }
                }
                matches
            };
            if let Some(scope) = &scope {
                matches.retain(|element| self.descendant(element.identity(), scope.clone(), false));
            }
            for (filter, excluded) in [(query.has.as_ref(), false), (query.has_not.as_ref(), true)]
            {
                if let Some(filter) = filter {
                    let mut filtered = Vec::new();
                    for element in matches {
                        let descendants = self
                            .query_in(page, filter, refs, Some(element.identity()))
                            .await?;
                        if !descendants.is_empty() != excluded {
                            filtered.push(element);
                        }
                    }
                    matches = filtered;
                }
            }
            if query.has_text.is_some() || query.has_not_text.is_some() || query.visible.is_some() {
                let mut filtered = Vec::new();
                for element in matches {
                    if let Some(visible) = query.visible
                        && element::state(page, &element, &[], false).await?.visible != visible
                    {
                        continue;
                    }
                    if query.has_text.is_some() || query.has_not_text.is_some() {
                        let text: String = element::call(
                            page,
                            &element,
                            "function() { return this.innerText || ''; }",
                            vec![],
                        )
                        .await?;
                        if query
                            .has_text
                            .as_ref()
                            .is_some_and(|filter| !text.contains(filter))
                            || query
                                .has_not_text
                                .as_ref()
                                .is_some_and(|filter| text.contains(filter))
                        {
                            continue;
                        }
                    }
                    filtered.push(element);
                }
                matches = filtered;
            }
            self.dom_order(&mut matches);
            if let Some(nth) = query.nth {
                matches = matches.into_iter().nth(nth as usize).into_iter().collect();
            }
            Ok(matches)
        })
    }
}
