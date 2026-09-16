//! Attach validated Host files to a page input or an observed file chooser.

use chromiumoxide::cdp::browser_protocol::{
    dom::SetFileInputFilesParams,
    page::{EventFileChooserOpened, FileChooserOpenedMode, SetInterceptFileChooserDialogParams},
};
use demi_command_service::InvocationContext;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    BrowserEnvironment, BrowserError, BrowserTab, Result, element,
    operation::{CONTROL_TIMEOUT, Operation, after_cleanup},
    protocol::BrowserCommand,
};

/// Attach Host files only after all paths have passed validation.
pub(super) async fn execute(
    context: &InvocationContext,
    _environment: &BrowserEnvironment,
    tab: Option<&BrowserTab>,
    command: &BrowserCommand,
    cancel: &CancellationToken,
    deadline: tokio::time::Instant,
) -> Result<Value> {
    let tab = tab.ok_or(BrowserError::TabNotFound)?;
    let BrowserCommand::Upload(input) = command else {
        unreachable!("upload dispatch accepts only upload");
    };
    let operation = Operation::until(&tab.ended, cancel, deadline);
    let mut references = tab
        .state
        .operations
        .try_lock()
        .map_err(|_| operation.failure(BrowserError::Busy, &tab.id(), None))?;
    let work = async {
        let mut files = Vec::with_capacity(input.file.len());
        for file in &input.file {
            let path = crate::files::resolve_path(&context.request.cwd, file)
                .map_err(BrowserError::Configuration)?;
            let path = operation
                .run(async {
                    let path = tokio::fs::canonicalize(path).await?;
                    if !tokio::fs::metadata(&path).await?.is_file() {
                        return Err(BrowserError::Configuration(
                            "upload paths must name readable regular files".into(),
                        ));
                    }
                    let _file = tokio::fs::File::open(&path).await?;
                    path.into_os_string().into_string().map_err(|_| {
                        BrowserError::Configuration("upload paths must be UTF-8".into())
                    })
                })
                .await?;
            files.push(path);
        }
        let target = command
            .target()
            .ok_or_else(|| BrowserError::Configuration("upload requires a target".into()))?;
        let (control, _) = tab
            .ready_element(&target, &mut references, &["enabled"], &operation)
            .await?;
        let is_input: bool = operation
            .run(element::call(
                &tab.page,
                &control,
                "function() { return this.localName === 'input' && this.type === 'file'; }",
                vec![],
            ))
            .await?;
        if is_input {
            attach(tab, &control, &files, &operation).await?;
        } else {
            // Subscribe before interception and before the page can open a chooser.
            let mut choosers = operation
                .run(async {
                    Ok(control
                        .page
                        .event_listener::<EventFileChooserOpened>()
                        .await?)
                })
                .await?;
            let chooser_work = async {
                operation
                    .run(async {
                        control
                            .page
                            .execute(SetInterceptFileChooserDialogParams::new(true))
                            .await?;
                        Ok(())
                    })
                    .await?;
                tab.input(&operation, async {
                    operation.begin_input();
                    let _: Value = element::call_with_user_gesture(
                        &control.page,
                        &control,
                        "function() { this.click(); return null; }",
                        vec![],
                    )
                    .await?;
                    operation.complete_input();
                    Ok(())
                })
                .await?;
                let chooser = operation
                    .run(async {
                        choosers
                            .next()
                            .await
                            .ok_or(BrowserError::Closed)?
                            .map_err(BrowserError::from)
                    })
                    .await?;
                if files.len() > 1 && chooser.mode != FileChooserOpenedMode::SelectMultiple {
                    return Err(BrowserError::Configuration(
                        "multiple files require a multiple file input".into(),
                    ));
                }
                let node = chooser.backend_node_id.ok_or_else(|| {
                    BrowserError::UnsupportedCapability(
                        "the chooser does not expose an attachable file input".into(),
                    )
                })?;
                let input = operation
                    .run(element::TargetElement::resolve(&control.page, node))
                    .await?;
                attach(tab, &input, &files, &operation).await
            }
            .await;
            // Always disable interception, including a cancelled enable request whose
            // acknowledgement was lost. Dropping the stream removes the listener.
            drop(choosers);
            let cleanup = tokio::time::timeout(
                CONTROL_TIMEOUT,
                control
                    .page
                    .execute(SetInterceptFileChooserDialogParams::new(false)),
            )
            .await
            .map_err(|_| BrowserError::Timeout)
            .and_then(|result| result.map(|_| ()).map_err(BrowserError::from));
            let cleanup = match cleanup {
                // A destroyed target/connection cannot retain chooser interception.
                Err(
                    BrowserError::Closed | BrowserError::Connection(_) | BrowserError::TabNotFound,
                ) => Ok(()),
                result => result,
            };
            after_cleanup(chooser_work, cleanup)?;
        }
        Ok(json!({"files": files, "attached": files.len()}))
    }
    .await;
    // The shared object-group cleanup also handles dialogs and destroyed targets.
    let result = after_cleanup(work, tab.release_objects().await);
    result.map_err(|error| operation.failure(error, &tab.id(), None))
}

/// Set and verify the file list on an enabled browser file input.
async fn attach(
    tab: &BrowserTab,
    input: &element::TargetElement,
    files: &[String],
    operation: &Operation<'_>,
) -> Result<()> {
    let state = operation
        .run(element::state(&tab.page, input, &["enabled"], false))
        .await?;
    if state.failed.is_some() {
        return Err(state.failure());
    }
    let multiple: bool = operation
        .run(element::call(
            &tab.page,
            input,
            "function() { return this.multiple === true; }",
            vec![],
        ))
        .await?;
    if files.len() > 1 && !multiple {
        return Err(BrowserError::Configuration(
            "multiple files require a multiple file input".into(),
        ));
    }
    operation
        .run(async {
            operation.begin_input();
            input
                .page
                .execute(
                    SetFileInputFilesParams::builder()
                        .backend_node_id(input.backend_node_id)
                        .files(files.iter().cloned())
                        .build()
                        .map_err(BrowserError::Configuration)?,
                )
                .await?;
            operation.complete_input();
            let attached: usize = element::call(
                &tab.page,
                input,
                "function() { return this.files.length; }",
                vec![],
            )
            .await?;
            if attached != files.len() {
                return Err(BrowserError::NotActionable {
                    condition: "the page did not retain the attached files".into(),
                    interceptor: None,
                });
            }
            Ok(())
        })
        .await
}
