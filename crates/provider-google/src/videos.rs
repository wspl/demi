//! The video role of a Gemini API key (`videos.md` § Vendors), Veo's:
//! `POST …/models/<model>:predictLongRunning` with `instances` and
//! `parameters` answers an operation, `GET …/<operation.name>` reads it
//! until it is `done`, and the video at
//! `response.generateVideoResponse.generatedSamples[0].video.uri` is
//! downloaded with the key, following redirects, as Google's guide does
//! ([Veo](https://ai.google.dev/gemini-api/docs/veo)). The models are those
//! Google lists for the key with `predictLongRunning`.

use bytes::Bytes;
use demi_provider_common::{
    endpoint_url,
    images::InputImage,
    json_body,
    media::{MediaAnswer, MediaFailure, MediaMaker, MediaModelInfo, MediaVendor, unreadable},
    media_catalog::Catalog,
    videos::{self, Done, JobState, VideoMaker, VideoRequest},
};
use demi_shared_types::VideoShape;
use futures_util::future::BoxFuture;
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::images::{GeminiKey, LABEL};

/// A Gemini API key's video role.
pub(crate) struct KeyVideos(pub(crate) GeminiKey);

/// The models that list `predictLongRunning`, Veo's method.
fn makes_videos(_: &[String], methods: &[String]) -> bool {
    methods.iter().any(|method| method == "predictLongRunning")
}

impl MediaMaker for KeyVideos {
    fn name(&self) -> &str {
        LABEL
    }

    fn vendor(&self) -> Option<MediaVendor> {
        Some(MediaVendor::Google)
    }

    fn models<'a>(
        &'a self,
        http: &'a demi_provider_common::ProviderHttp,
        _: &'a Catalog,
    ) -> BoxFuture<'a, Result<Vec<MediaModelInfo>, MediaFailure>> {
        Box::pin(self.0.list(http, makes_videos))
    }
}

impl VideoMaker for KeyVideos {
    fn shape(&self) -> VideoShape {
        VideoShape::Veo
    }

    fn ask<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, request: VideoRequest) -> BoxFuture<'a, MediaAnswer<String>> {
        Box::pin(async move {
            let key = &self.0;
            let url = key.models_url(Some(&format!("{}:predictLongRunning", request.model)));
            let headers = key.headers();
            key.call()
                .post_json(http, &url, encode(&request), |_| std::future::ready(Ok((headers.clone(), None))))
                .await
                .and_then(|body| {
                    #[derive(Deserialize)]
                    struct Operation {
                        name: String,
                    }
                    let operation: Operation = serde_json::from_slice(&body)
                        .map_err(|error| unreadable(format!("its answer could not be read: {error}")))?;
                    Ok(operation.name)
                })
        })
    }

    fn state<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, job: &'a str) -> BoxFuture<'a, MediaAnswer<JobState>> {
        Box::pin(async move {
            let key = &self.0;
            let url = endpoint_url(key.base(), &format!("/{job}"));
            let headers = key.headers();
            key.call()
                .send(|headers| http.get(url.clone()).headers(headers), |_| {
                    std::future::ready(Ok((headers.clone(), None)))
                })
                .await
                .and_then(|answered| read_operation(&answered.body))
        })
    }

    fn download<'a>(&'a self, http: &'a demi_provider_common::ProviderHttp, done: &'a Done) -> BoxFuture<'a, MediaAnswer<Bytes>> {
        // The file's address is the key's own endpoint's, and its redirect
        // to storage gets no key.
        Box::pin(videos::download(LABEL, &*self.0.clock, http, &done.url, self.0.base(), self.0.headers()))
    }
}

#[derive(Serialize)]
struct Body<'a> {
    instances: [Instance<'a>; 1],
    #[serde(skip_serializing_if = "Parameters::is_empty")]
    parameters: Parameters,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Instance<'a> {
    prompt: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<Image>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_frame: Option<Image>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reference_images: Vec<Reference>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Image {
    inline_data: InlineData,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InlineData {
    mime_type: &'static str,
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Reference {
    image: Image,
    reference_type: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Parameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

impl Parameters {
    fn is_empty(&self) -> bool {
        self.aspect_ratio.is_none() && self.duration_seconds.is_none() && self.resolution.is_none() && self.seed.is_none()
    }
}

fn image(image: &InputImage) -> Image {
    Image { inline_data: InlineData { mime_type: image.media_type, data: image.base64() } }
}

/// The ask: every option Veo has a field for, sent only when given; Veo
/// always makes sound, so it has none for it.
fn encode(request: &VideoRequest) -> Bytes {
    let body = Body {
        instances: [Instance {
            prompt: &request.prompt,
            image: request.image.as_ref().map(image),
            last_frame: request.last_frame.as_ref().map(image),
            reference_images: request
                .references
                .iter()
                .map(|reference| Reference { image: image(reference), reference_type: "asset" })
                .collect(),
        }],
        parameters: Parameters {
            aspect_ratio: request.aspect.map(|aspect| aspect.to_string()),
            duration_seconds: request.duration,
            resolution: request.resolution.map(|resolution| resolution.as_str()),
            seed: request.seed,
        },
    };
    Bytes::from(json_body(&body))
}

#[derive(Deserialize)]
struct Operation {
    #[serde(default)]
    done: bool,
    #[serde(default)]
    error: Option<OperationError>,
    #[serde(default)]
    response: Option<OperationResponse>,
}

#[derive(Deserialize)]
struct OperationError {
    message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationResponse {
    #[serde(default)]
    generate_video_response: Option<VideoResponse>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoResponse {
    #[serde(default)]
    generated_samples: Vec<Sample>,
    #[serde(default)]
    rai_media_filtered_reasons: Vec<String>,
}

#[derive(Deserialize)]
struct Sample {
    video: SampleVideo,
}

#[derive(Deserialize)]
struct SampleVideo {
    uri: String,
}

/// The operation's state: running until `done`, then failed with its
/// `error` or the reasons Google's filters withheld the video for, or made.
fn read_operation(body: &[u8]) -> Result<JobState, MediaFailure> {
    let operation: Operation = serde_json::from_slice(body)
        .map_err(|error| unreadable(format!("its operation could not be read: {error}")))?;
    if !operation.done {
        return Ok(JobState::Running { progress: None });
    }
    if let Some(error) = operation.error {
        return Ok(JobState::Failed { message: error.message });
    }
    let response = operation.response.and_then(|response| response.generate_video_response);
    let Some(response) = response else {
        return Err(unreadable("its finished operation named no video".to_owned()));
    };
    let Some(sample) = response.generated_samples.into_iter().next() else {
        if response.rai_media_filtered_reasons.is_empty() {
            return Err(unreadable("its finished operation named no video".to_owned()));
        }
        return Ok(JobState::Failed { message: response.rai_media_filtered_reasons.join("; ") });
    };
    let url = Url::parse(&sample.video.uri)
        .map_err(|error| unreadable(format!("its video's address could not be read: {error}")))?;
    Ok(JobState::Done(Done { url, cost: None, usage: None }))
}
