//! Grok Build's video role (`videos.md` § Vendors): the chat proxy's video
//! endpoints, which speak the xAI Videos shape, with the account's identity
//! as inference sends it. A finished video's address is downloaded without
//! a header.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    endpoint_url,
    media::{Ask, Holding, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, subscription_models},
    media_catalog::{Catalog, MediaKind, Subscription},
    videos::{self, Done, JobState, VideoMaker, VideoRequest, xai},
};
use demi_shared_types::VideoShape;
use futures_util::future::BoxFuture;
use reqwest::header::HeaderMap;

use crate::Shared;

/// The video role of the provider's account.
pub(crate) struct GrokVideos(pub(crate) Arc<Shared>);

impl MediaMaker for GrokVideos {
    fn name(&self) -> &str {
        "Grok"
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Xai)
    }

    /// The catalog's video models of a Grok subscription, those it
    /// accepted: the proxy lists none.
    fn models<'a>(
        &'a self,
        _: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(std::future::ready(Ok(subscription_models(catalog, Subscription::Grok, MediaKind::Video))))
    }
}

impl VideoMaker for GrokVideos {
    fn shape(&self) -> VideoShape {
        VideoShape::Grok
    }

    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, _: &'a demi_provider_common::media_catalog::Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let shared = &*self.0;
            shared
                .media_call()
                .post_json(http, &shared.video_generations_url, xai::encode(&request), |refused| {
                    shared.media_headers(http, refused)
                })
                .await
                .and_then(|body| xai::read_asked(&body))
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: demi_provider_common::videos::JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>>  {
        let job = job.id;
        Box::pin(async move {
            let shared = &*self.0;
            let url = endpoint_url(&shared.proxy_url, &format!("/videos/{job}"));
            shared
                .media_call()
                .send(http, Ask::get(&url), Holding::Json, |refused| shared.media_headers(http, refused))
                .await
                .and_then(|answered| xai::read_state(&answered.body))
        })
    }

    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, done: &'a Done) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        Box::pin(videos::download(crate::LABEL, &*self.0.clock, http, &done.url, &done.url, HeaderMap::new()))
    }
}
