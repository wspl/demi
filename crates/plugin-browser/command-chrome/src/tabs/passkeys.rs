//! Passkeys, which this browser cannot use: a page that asks for one is
//! refused at once, as when the user cancels the request in Chrome.
//!
//! - Headless Chrome has no authenticator to ask and no dialog to show, so a
//!   request would wait past the page's own timeout and leave its sign-in
//!   hanging. The page learns `NotAllowedError` instead and offers its other
//!   ways in.
//! - Each tab holds two virtual authenticators, a platform one and a USB key,
//!   whose user verification always fails: every request ends at once, also
//!   one that names a kind of authenticator, and none creates a credential, so
//!   no account is left with a passkey nobody holds. Pages then see a
//!   platform authenticator, as on a Mac with Touch ID.
//! - A request for autofill (`mediation: "conditional"`) is refused as well,
//!   where Chrome would let it wait.
//! - Chrome keeps the authenticators per tab, across navigations and for its
//!   frames, but drops them whenever any debugging session of the tab detaches
//!   or disables them. The tab sets them up again when one of its debugging
//!   connections closes; an agent that disables them on its own connection to
//!   add its own keeps that until any of the tab's debugging connections
//!   closes.
//! - Nothing observes a request, since the protocol reports none, so the
//!   panel cannot say that a page asked.

use chromiumoxide::{
    Page,
    types::{Command, Method, MethodId},
};
use serde_json::{Value, json};

use crate::driver::operation::Result;

/// Sets up the tab's authenticators afresh, whatever it had before.
pub(crate) async fn refuse(page: &Page) -> Result<()> {
    page.execute(WebAuthn::new("WebAuthn.disable", json!({})))
        .await?;
    page.execute(WebAuthn::new("WebAuthn.enable", json!({"enableUI": false})))
        .await?;
    for transport in ["internal", "usb"] {
        page.execute(WebAuthn::new(
            "WebAuthn.addVirtualAuthenticator",
            json!({"options": {
                "protocol": "ctap2",
                "ctap2Version": "ctap2_1",
                "transport": transport,
                "hasResidentKey": true,
                "hasUserVerification": true,
                "isUserVerified": false,
                "automaticPresenceSimulation": true,
            }}),
        ))
        .await?;
    }
    Ok(())
}

/// A command of the `WebAuthn` domain, which the vendored protocol lacks.
#[derive(Debug, serde::Serialize)]
#[serde(transparent)]
struct WebAuthn {
    #[serde(skip)]
    method: &'static str,
    params: Value,
}

impl WebAuthn {
    fn new(method: &'static str, params: Value) -> Self {
        Self { method, params }
    }
}

impl Method for WebAuthn {
    fn identifier(&self) -> MethodId {
        self.method.into()
    }
}

impl Command for WebAuthn {
    type Response = Value;
}
