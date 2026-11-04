//! The video role of an xAI key: the xAI Videos shape under the entry's base
//! URL, with the key as a bearer token. It lists the video models xAI lists
//! for the key.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::{
    Secret, endpoint_url,
    media::{Ask, Holding, MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor},
    media_catalog::Catalog,
    videos::{self, JobRef, JobState, JobStep, PolledVideos, VideoMaker, VideoRequest, xai},
};
use demi_shared_types::Clock;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderMap},
};

use crate::images::{key_call, xai_listing};

/// The video role of an xAI key.
pub(crate) struct KeyVideos {
    key: Secret,
    base: Url,
    clock: Arc<dyn Clock>,
}

impl KeyVideos {
    /// The role of an xAI `key` under the API `base`.
    pub(crate) fn xai(key: Secret, base: &Url, clock: Arc<dyn Clock>) -> Self {
        Self { key, base: base.clone(), clock }
    }

    fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, self.key.bearer());
        headers
    }

    fn url(&self, path: &str) -> Url {
        endpoint_url(&self.base, path)
    }

    /// Sends `ask` with the key's headers and answers the vendor's success.
    async fn send(&self, http: &demi_provider_common::ProviderHttp, ask: Ask) -> MediaAnswer<Bytes> {
        let headers = self.headers();
        key_call(true, &*self.clock)
            .send(http, ask, Holding::Json, |_| std::future::ready(Ok((headers.clone(), None))))
            .await
            .and_then(|answered| Ok(answered.body))
    }
}

impl MediaMaker for KeyVideos {
    fn name(&self) -> &str {
        MediaVendor::Xai.name()
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Xai)
    }

    /// xAI's listing of the key's video models, read when asked.
    fn models<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(async move {
            let url = self.url("/video-generation-models");
            let text = key_call(true, &*self.clock).get(http, &url, self.headers()).await?;
            xai_listing(&text)
        })
    }
}

impl VideoMaker for KeyVideos {
    fn run<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        catalog: &'a Catalog,
        request: VideoRequest,
        every: std::time::Duration,
    ) -> BoxStream<'a, MediaAnswer<JobStep>> {
        videos::polled(self, http, catalog, request, every)
    }

    /// xAI's address is temporary and takes no header.
    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, url: &'a Url) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        Box::pin(videos::download("xAI", &*self.clock, http, url, &self.base, HeaderMap::new()))
    }
}

impl PolledVideos for KeyVideos {
    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, _: &'a Catalog, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let url = self.url("/videos/generations");
            self.send(http, Ask::post_json(&url, xai::encode(&request))).await.and_then(|body| xai::read_asked(&body))
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState>> {
        Box::pin(async move {
            let url = self.url(&format!("/videos/{}", job.id));
            self.send(http, Ask::get(&url)).await.and_then(|body| xai::read_state(&body))
        })
    }
}
