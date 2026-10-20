//! Files the backend offers a device's runner for download
//! (`crates/shared-artifacts/src/download.rs`): an attachment `demi attachment
//! download` writes on a Host is one. Unlike a pipe's bytes, an offer's stay whole while its call lasts, so
//! the runner's download resumes from any byte after a break or a stall; the
//! offer goes when the call that made it ends.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use bytes::Bytes;
use demi_runner_protocol::wire::FileOffer;

/// The offers of one user's devices, by id.
#[derive(Clone, Default)]
pub struct FileOffers(Rc<RefCell<HashMap<String, Offered>>>);

struct Offered {
    device: String,
    bytes: Bytes,
}

/// An offer, withdrawn when dropped.
pub struct Offer {
    id: String,
    offers: FileOffers,
    wire: FileOffer,
}

impl FileOffers {
    /// Offers `bytes`, whose SHA-256 is `sha256`, to `device`'s runner.
    pub fn offer(&self, device: &str, bytes: Bytes, sha256: String) -> Offer {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let wire = FileOffer {
            url: format!("/api/offers/{id}"),
            size: bytes.len() as u64,
            sha256,
        };
        self.0.borrow_mut().insert(
            id.clone(),
            Offered {
                device: device.into(),
                bytes,
            },
        );
        Offer {
            id,
            offers: self.clone(),
            wire,
        }
    }

    /// The bytes of offer `id` for `device`'s `GET`: none for an offer
    /// withdrawn, unknown, or made to another device.
    pub fn bytes(&self, id: &str, device: &str) -> Option<Bytes> {
        let offers = self.0.borrow();
        offers
            .get(id)
            .filter(|offered| offered.device == device)
            .map(|offered| offered.bytes.clone())
    }
}

impl Offer {
    /// What the runner's message names.
    pub fn wire(&self) -> FileOffer {
        self.wire.clone()
    }
}

impl Drop for Offer {
    fn drop(&mut self) {
        self.offers.0.borrow_mut().remove(&self.id);
    }
}

/// The answer to a runner's `GET` of an offer's `bytes` from byte `from` on:
/// the whole for 0, the rest as 206 for a download that resumes, and 416
/// with the size for a byte past the end, as the downloader expects
/// (`crates/shared-artifacts/src/download.rs`).
#[cfg(feature = "edge")]
pub fn offer_response(bytes: Bytes, from: u64) -> axum::response::Response {
    use axum::http::{StatusCode, header};
    use axum::response::IntoResponse as _;
    let size = bytes.len() as u64;
    let octets = (header::CONTENT_TYPE, "application/octet-stream");
    let fresh = (header::CACHE_CONTROL, "no-store");
    if from == 0 {
        return ([octets, fresh], bytes).into_response();
    }
    if from >= size {
        return (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [(header::CONTENT_RANGE, format!("bytes */{size}"))],
        )
            .into_response();
    }
    let rest = bytes.slice(from as usize..);
    (
        StatusCode::PARTIAL_CONTENT,
        [
            octets,
            fresh,
            (header::CONTENT_RANGE, &*format!("bytes {from}-{}/{size}", size - 1)),
        ],
        rest,
    )
        .into_response()
}
