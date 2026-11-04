//! Each option a video model's catalog entry does not exclude reaches its
//! vendor. For every video model of the catalog, the maker of its vendor is
//! asked twice for each option, with two values of it, and the two asks must
//! differ: a maker that drops the option sends the same ask for both.
//!
//! The vendor is a local server that records each request and refuses it,
//! so a maker sends its first requests and gives up. It takes an upload as
//! Gemini's Files API does, since Omni uploads each image before its ask,
//! and a refused first image would hide the next. The Grok subscription
//! sends the body the xAI key sends (`videos::xai::encode`), which the xAI
//! models check.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use demi_provider_bytedance::{Ark, ArkConfig, ArkProvider};
use demi_provider_common::{
    ModelWires, Provider, ProviderHttp, Secret,
    images::{Aspect, InputImage},
    media_catalog::{Catalog, MediaKind, VideoCapabilities},
    videos::{VideoRequest, untaken},
};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_provider_minimax::{MinimaxConfig, MinimaxProvider, Region};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_shared_types::{Clock, SystemClock, VideoResolution, WireApi};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
use url::Url;

const CATALOG: &str = include_str!("../../../../catalog/media/v1/models.json");

#[tokio::test]
async fn each_option_a_video_model_takes_reaches_its_vendor() {
    let catalog: Catalog = serde_json::from_str(CATALOG).expect("the media catalog reads");
    let vendor = Vendor::start().await;
    let http = ProviderHttp::new(reqwest::Client::new());
    let mut dropped = Vec::new();
    let mut checked = 0;
    for model in catalog.models.iter().filter(|model| model.kind == MediaKind::Video) {
        let Some(takes) = catalog.video(model) else {
            continue;
        };
        let provider = maker(&model.vendor, &vendor.origin)
            .unwrap_or_else(|| panic!("no maker makes {}'s videos, which the catalog lists", model.vendor));
        let base = VideoRequest {
            model: model.id.clone(),
            prompt: "A paper boat drifts across a pond.".to_owned(),
            ..VideoRequest::default()
        };
        for (option, one, other) in pairs(takes, &base) {
            let one = vendor.asked(&*provider, &http, &catalog, one).await;
            let other = vendor.asked(&*provider, &http, &catalog, other).await;
            if one.is_empty() || one == other {
                dropped.push(format!("{} {}: {option}", model.vendor, model.id));
            }
            checked += 1;
        }
    }
    assert!(checked > 0, "the catalog lists no video model");
    assert!(dropped.is_empty(), "options the vendor never gets: {dropped:#?}");
}

/// The provider whose maker makes `vendor`'s videos, its endpoint at `origin`.
fn maker(vendor: &str, origin: &Url) -> Option<Arc<dyn Provider>> {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let key = || Some(Secret::try_from("example-key".to_owned()).expect("a key"));
    let at = |path: &str| Some(origin.join(path).expect("an endpoint"));
    let ark = |ark| -> Arc<dyn Provider> {
        Arc::new(ArkProvider::new(
            ArkConfig { ark, api_key: key(), base_url: at("api/v3"), makes_images: false, makes_videos: true, text: None },
            clock.clone(),
        ))
    };
    let openai = |vendor: &str, makes_videos| -> Arc<dyn Provider> {
        Arc::new(OpenAiProvider::new(
            OpenAiConfig {
                api_key: key(),
                base_url: at("v1"),
                wire: WireApi::Responses,
                models: ModelWires::default(),
                vendor_id: Some(vendor.to_owned()),
                entry_id: vendor.to_owned(),
                label: vendor.to_owned(),
                makes_images: false,
                makes_videos,
                makes_audio: false,
                cache_ttl: None,
                lists_models: false,
            },
            clock.clone(),
        ))
    };
    Some(match vendor {
        "google" => Arc::new(GoogleProvider::new(
            GoogleConfig { api_key: key(), base_url: at("v1beta"), makes_images: false, makes_videos: true, makes_audio: false },
            clock,
        )),
        "xai" => openai(vendor, true),
        "byteplus" => ark(Ark::Byteplus),
        "volcengine" => ark(Ark::Volcengine),
        _ => {
            let region = Region::of(vendor)?;
            Arc::new(MinimaxProvider::new(
                MinimaxConfig { region, api_key: key(), base_url: at("v1"), makes_images: false, makes_videos: true, makes_audio: false },
                openai(vendor, false),
                clock,
            ))
        }
    })
}

/// Each option `takes` does not exclude, as two asks that differ in it alone.
fn pairs(takes: &VideoCapabilities, base: &VideoRequest) -> Vec<(&'static str, VideoRequest, VideoRequest)> {
    let with = |change: &dyn Fn(&mut VideoRequest)| {
        let mut request = base.clone();
        change(&mut request);
        request
    };
    let image = || InputImage { media_type: "image/png", bytes: Bytes::from_static(PNG) };
    let mut pairs = Vec::new();
    let longest = match &takes.durations {
        Some(durations) => durations.iter().max().copied(),
        None => takes.duration_range.and_then(|range| range.max).map(|max| max.floor() as u32),
    };
    if let (Some(shortest), Some(longest)) = (takes.shortest(), longest)
        && shortest != longest
    {
        pairs.push(("duration", with(&|r| r.duration = Some(shortest)), with(&|r| r.duration = Some(longest))));
    }
    let aspects: Vec<Aspect> = takes.aspect_ratios.iter().flatten().filter_map(|text| Aspect::parse(text)).collect();
    if let [first, .., last] = aspects.as_slice() {
        pairs.push(("aspect", with(&|r| r.aspect = Some(*first)), with(&|r| r.aspect = Some(*last))));
    }
    let resolutions: Vec<VideoResolution> =
        takes.resolutions.iter().flatten().filter_map(|text| VideoResolution::try_from(text.clone()).ok()).collect();
    if let [first, .., last] = resolutions.as_slice() {
        pairs.push(("resolution", with(&|r| r.resolution = Some(*first)), with(&|r| r.resolution = Some(*last))));
    }
    // Sound is checked where a model can be asked both ways: one that makes
    // sound always, or never, honors the one it takes without a field.
    pairs.push(("audio", with(&|r| r.audio = Some(true)), with(&|r| r.audio = Some(false))));
    pairs.push(("seed", with(&|r| r.seed = Some(1)), with(&|r| r.seed = Some(2))));
    pairs.push(("first frame", base.clone(), with(&|r| r.image = Some(image()))));
    pairs.push((
        "last frame",
        with(&|r| r.image = Some(image())),
        with(&|r| {
            r.image = Some(image());
            r.last_frame = Some(image());
        }),
    ));
    pairs.push(("reference", base.clone(), with(&|r| r.references = vec![image()])));
    pairs.retain(|(_, one, other)| untaken(takes, one).is_none() && untaken(takes, other).is_none());
    pairs
}

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01,
    0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41,
    0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The local vendor: it records each request, its line and its body, and
/// refuses it, except an upload.
struct Vendor {
    origin: Url,
    seen: Seen,
}

/// The requests of one ask, each its line and its body.
type Seen = Arc<Mutex<Vec<String>>>;

impl Vendor {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let origin = Url::parse(&format!("http://{}/", listener.local_addr().expect("its address"))).expect("an origin");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = seen.clone();
        let at = origin.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(answer(stream, at.clone(), recorded.clone()));
            }
        });
        Self { origin, seen }
    }

    /// The requests `provider`'s maker sends for `request` before it gives
    /// up on the refusals.
    async fn asked(&self, provider: &dyn Provider, http: &ProviderHttp, catalog: &Catalog, request: VideoRequest) -> Vec<String> {
        self.seen.lock().expect("the record").clear();
        let videos = provider.videos().expect("the provider makes videos");
        // The deadline only guards against a maker that never gives up.
        tokio::time::timeout(Duration::from_secs(10), videos.ask(http, catalog, request))
            .await
            .expect("the maker gives up on a refused ask");
        self.seen.lock().expect("the record").clone()
    }
}

/// Records the request on `stream` and answers it: an upload as Gemini's
/// Files API does, any other request 400.
async fn answer(mut stream: TcpStream, origin: Url, seen: Seen) {
    let mut read = Vec::new();
    let mut buffer = vec![0; 64 * 1024];
    let body_at = loop {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(count) => read.extend_from_slice(&buffer[..count]),
        }
        if let Some(at) = read.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&read[..body_at]).to_ascii_lowercase();
    let length = head.lines().find_map(|line| line.strip_prefix("content-length:")).and_then(|value| value.trim().parse().ok());
    let chunked = head.contains("transfer-encoding: chunked");
    loop {
        let body = &read[body_at..];
        let whole = match length {
            Some(length) => body.len() >= length,
            None => !chunked || body.ends_with(b"0\r\n\r\n"),
        };
        if whole {
            break;
        }
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(count) => read.extend_from_slice(&buffer[..count]),
        }
    }
    let line = head.lines().next().unwrap_or_default().to_owned();
    // The uploads before this request name its file; the record is one ask's.
    let file = {
        let mut seen = seen.lock().expect("the record");
        seen.push(format!("{line}\n{}", String::from_utf8_lossy(&read[body_at..])));
        seen.iter().filter(|request| request.contains("/uploaded/")).count()
    };
    let (status, headers, body) = if head.contains("x-goog-upload-command: start") {
        ("200 OK", format!("x-goog-upload-url: {origin}uploaded/{file}\r\n"), "{}".to_owned())
    } else if line.contains("/uploaded/") {
        let uri = format!("{origin}v1beta/files/{file}");
        ("200 OK", String::new(), format!(r#"{{"file":{{"name":"files/{file}","uri":"{uri}","state":"ACTIVE"}}}}"#))
    } else {
        let refusal = r#"{"error":{"code":400,"message":"Refused.","status":"INVALID_ARGUMENT"}}"#;
        ("400 Bad Request", String::new(), refusal.to_owned())
    };
    let answer = format!(
        "HTTP/1.1 {status}\r\n{headers}content-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(answer.as_bytes()).await;
}
