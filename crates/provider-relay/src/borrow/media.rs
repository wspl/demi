//! A borrowed entry's makers of media: each a [`Remote`] whose calls the relay
//! carries to the lender's maker of its kind (`crate::wire::media`), and words
//! as the borrower reads them, since a failure crosses without its words.

use std::sync::Arc;

use demi_provider_common::media::channel::{Call, Channel, NO_REASON, Reply};
use demi_provider_common::media::{MediaFailure, transport, unreadable};
use demi_provider_common::{ErrorCode, ProviderHttp};
use demi_shared_types::{MediaKind, VendorTrace};
use futures_util::future::BoxFuture;
use futures_util::stream::StreamExt as _;
use http::Method;

use super::{Config, Refusal};
use crate::wire::decoded;
use crate::wire::media::{Frames, MAKES_NONE, MEDIA_ROUTE, frame};

pub(super) use demi_provider_common::media::channel::Remote;

/// The relay as the channel of a borrowed entry's media.
pub(super) struct Relay(pub(super) Arc<Config>);

impl Relay {
    /// Why the lender answered `status`, as the borrower reads it.
    fn refused(&self, kind: MediaKind, status: http::StatusCode) -> MediaFailure {
        let lender = &self.0.lender;
        if status.as_u16() == MAKES_NONE {
            let cause = format!("{lender} makes no {} with it now", kind.plural());
            return MediaFailure::Unanswered { cause, code: None, trace: VendorTrace::default() };
        }
        let refusal = Refusal::of(status);
        MediaFailure::Unanswered { cause: refusal.message(&self.0), code: refusal.code(), trace: VendorTrace::default() }
    }
}

/// The path of `kind`'s media under [`MEDIA_ROUTE`].
pub fn kind_route(kind: MediaKind) -> String {
    let name = match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Audio => "audio",
    };
    format!("{MEDIA_ROUTE}/{name}")
}

impl Channel for Relay {
    fn name(&self, vendor: Option<&str>) -> String {
        match vendor {
            Some(vendor) => format!("{vendor} on {}", self.0.lender),
            None => self.0.lender.clone(),
        }
    }

    fn call<'a>(&'a self, http: &'a ProviderHttp, kind: MediaKind, call: Call) -> BoxFuture<'a, Result<Reply, MediaFailure>> {
        Box::pin(async move {
            let url = self.0.route(&kind_route(kind));
            let body = frame(&call).ok_or_else(|| unreadable("its call could not be sent".to_owned()))?;
            let request = self
                .0
                .authorized(http.request(Method::POST, url))
                .header(http::header::CONTENT_TYPE, "application/octet-stream")
                .body(body);
            let response = request.send().await.map_err(|error| transport(error.without_url()))?;
            let status = response.status();
            if !status.is_success() {
                return Err(self.refused(kind, status));
            }
            let mut frames = Frames::default();
            let mut body = response.bytes_stream();
            loop {
                if let Some(message) = frames.next().map_err(|error| unreadable(error.to_string()))? {
                    return decoded::<Reply>(&message)
                        .ok_or_else(|| unreadable(format!("{} answered what no lender answers", self.0.lender)));
                }
                match body.next().await {
                    Some(Ok(chunk)) => frames.push(&chunk),
                    Some(Err(error)) => return Err(transport(error.without_url())),
                    None if frames.is_partial() => return Err(unreadable("its answer ended within a message".to_owned())),
                    None => {
                        // A revoked key ends its calls: the call says so
                        // rather than that its answer ended.
                        return Err(match self.0.lending().await {
                            Err(refusal) => MediaFailure::Unanswered {
                                cause: refusal.message(&self.0),
                                code: refusal.code(),
                                trace: VendorTrace::default(),
                            },
                            Ok(_) => unreadable("its answer ended before its reply".to_owned()),
                        });
                    }
                }
            }
        })
    }

    fn worded(&self, _: MediaKind, failure: MediaFailure) -> MediaFailure {
        let lender = &self.0.lender;
        let trace = VendorTrace::default();
        match failure {
            MediaFailure::UsageLimit { .. } => MediaFailure::UsageLimit { retry_at: None, trace },
            MediaFailure::SignIn(_) | MediaFailure::KeyRefused { .. } => {
                MediaFailure::Unanswered { cause: format!("its account on {lender} could not be used"), code: None, trace }
            }
            MediaFailure::Refused { code, .. } => MediaFailure::Refused { reason: Some(NO_REASON.to_owned()), code, trace },
            MediaFailure::Shape { takes, code, .. } => MediaFailure::Shape { takes, code, trace },
            MediaFailure::Unanswered { code, .. } => {
                let cause = match code {
                    Some(ErrorCode::Overloaded | ErrorCode::RateLimit | ErrorCode::DeviceOffline) => {
                        "it is not available right now".to_owned()
                    }
                    _ => NO_REASON.to_owned(),
                };
                MediaFailure::Unanswered { cause, code, trace }
            }
            MediaFailure::SourceGone { .. } => MediaFailure::SourceGone { trace },
        }
    }
}
