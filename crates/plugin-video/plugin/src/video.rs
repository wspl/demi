//! What `demi video` does:
//!
//! 1. It reads the call's prompt, its frames and references, and the video
//!    an edit or an extension changes from the Host the command runs on.
//! 2. It asks the Providers service for the video's job and names it on stderr
//!    at once.
//! 3. It says each change of the job's state.
//! 4. It checks that what came back is an MP4 and writes it to stdout.
//! 5. It says what came back, with a warning for each option the result did not
//!    honour. Every option is sent: one the model does not take fails the call
//!    before anything is paid.
//!
//! A failure is told in the design's words.

use demi_host_interface::RpcInvocation;
use demi_plugin_interface::{
    AskedVideo, ImageAspect, MadeVideo, PluginError, PluginPort, SentSource, VideoAsk, VideoChange, VideoResolution,
    VideoSourceAsk, VideoStep,
};
use demi_plugin_media::{
    decode_args, fits, models, parse_aspect, prompt, read_images, read_video, refused, required_model,
};
use demi_shared_types::{MediaKind, mp4};

use crate::commands::{ChangeArgs, GenerateArgs, ModelsArgs};

/// How far a video's length may be from the one asked before it is a
/// different length: the vendors cut at a frame, not at the second.
const DURATION_SLACK: f64 = 0.5;

/// Runs the leaf `invocation` names.
pub(crate) async fn run(invocation: RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    match invocation.path.last().map(String::as_str) {
        Some("generate") => {
            let args: GenerateArgs = decode_args(&invocation)?;
            let ask = ask(port, args).await?;
            generate(&invocation, port, ask, None).await
        }
        Some(leaf @ ("edit" | "extend")) => {
            let change = if leaf == "edit" { VideoChange::Edit } else { VideoChange::Extend };
            let args: ChangeArgs = decode_args(&invocation)?;
            let path = args.video.clone();
            let ask = change_ask(port, args, change).await?;
            generate(&invocation, port, ask, Some(&path)).await
        }
        Some("models") => {
            let args: ModelsArgs = decode_args(&invocation)?;
            models(&invocation, port, MediaKind::Video, args.model).await
        }
        _ => Err(PluginError::failed(format!("no command \"{}\"", invocation.path.join(" ")))),
    }
}

/// The call's ask: its prompt, from stdin for `-`, its options, and its
/// frames and references read together from the Host.
async fn ask(port: &PluginPort, args: GenerateArgs) -> Result<VideoAsk, PluginError> {
    let model = required_model(args.model, MediaKind::Video)?;
    if args.last_frame.is_some() && args.image.is_none() {
        return Err(PluginError::failed("--last-frame needs --image, the first frame"));
    }
    let audio = match (args.audio, args.no_audio) {
        (Some(true), Some(true)) => {
            return Err(PluginError::failed("--audio and --no-audio ask for opposite things; give one"));
        }
        (Some(audio), _) => Some(audio),
        (None, Some(silent)) => Some(!silent),
        (None, None) => None,
    };
    let prompt = prompt(port, args.prompt).await?;
    let aspect = args.aspect.as_deref().map(parse_aspect).transpose()?;
    let references = args.reference.unwrap_or_default();
    let mut paths: Vec<String> = args.image.iter().chain(&args.last_frame).cloned().collect();
    paths.extend(references.iter().cloned());
    let mut images = read_images(port, &paths).await?.into_iter();
    let image = args.image.as_ref().and_then(|_| images.next());
    let last_frame = args.last_frame.as_ref().and_then(|_| images.next());
    Ok(VideoAsk {
        prompt,
        image,
        last_frame,
        references: images.collect(),
        duration: args.duration,
        aspect,
        resolution: args.resolution,
        audio,
        seed: args.seed,
        model,
        provider: args.provider,
        source: None,
    })
}

/// An edit's or an extension's ask: its prompt, its options, its
/// references and the video it changes, read together from the Host.
async fn change_ask(port: &PluginPort, args: ChangeArgs, change: VideoChange) -> Result<VideoAsk, PluginError> {
    let model = required_model(args.model, MediaKind::Video)?;
    let prompt = prompt(port, args.prompt).await?;
    let references = read_images(port, &args.reference.unwrap_or_default()).await?;
    let bytes = read_video(port, &args.video).await?;
    Ok(VideoAsk {
        prompt,
        image: None,
        last_frame: None,
        references,
        duration: None,
        aspect: None,
        resolution: args.resolution,
        audio: None,
        seed: None,
        model,
        provider: args.provider,
        source: Some(VideoSourceAsk { change, bytes }),
    })
}

/// Asks for the video, follows its job to its end, checks it and writes
/// it to stdout, and says what came back; `source` is the path of the
/// video an edit or an extension changes.
async fn generate(invocation: &RpcInvocation, port: &PluginPort, ask: VideoAsk, source: Option<&str>) -> Result<u8, PluginError> {
    let command = format!("demi {}", invocation.path.join(" "));
    let wanted = Wanted::of(&ask, source);
    let aspect = ask.aspect;
    let asked = port
        .make_video(ask)
        .await
        .map_err(|failure| refused(MediaKind::Video, failure, aspect, invocation))?;
    let rpc = port.rpc();
    // The job is named at once: from here it runs, and is paid for, at the
    // vendor whatever becomes of the call.
    rpc.stderr(format!("{command}: asked {} ({}), job {}\n", asked.model, entry(&asked), asked.job))
        .await?;
    let video = loop {
        let step = port
            .video_progress()
            .await
            .map_err(|failure| refused(MediaKind::Video, failure, aspect, invocation))?;
        let state = match step {
            VideoStep::Made { video } => break video,
            VideoStep::Queued => "queued".to_owned(),
            VideoStep::Running { progress: None } => "running".to_owned(),
            VideoStep::Running { progress: Some(progress) } => format!("running, {progress}%"),
        };
        rpc.stderr(format!("{command}: {state}\n")).await?;
    };
    let facts = mp4::read(&video.bytes).map_err(|error| {
        PluginError::failed(format!("{} returned a file that is not a video this reads: {error}", asked.vendor))
    })?;
    let said = summary(&command, &asked, &video, &facts, &wanted);
    rpc.stdout(video.bytes.into_bytes()).await?;
    rpc.stderr(said).await?;
    Ok(0)
}

/// What the call asked of the result, which it is checked against.
struct Wanted {
    duration: Option<u32>,
    aspect: Option<ImageAspect>,
    resolution: Option<VideoResolution>,
    audio: Option<bool>,
    /// The change asked of the video at the path, for an edit or an
    /// extension.
    change: Option<(VideoChange, String)>,
}

impl Wanted {
    fn of(ask: &VideoAsk, source: Option<&str>) -> Self {
        Self {
            duration: ask.duration,
            aspect: ask.aspect,
            resolution: ask.resolution,
            audio: ask.audio,
            change: ask.source.as_ref().zip(source).map(|(asked, path)| (asked.change, path.to_owned())),
        }
    }
}

/// The entry and account a line names, as Settings names the entry.
fn entry(asked: &AskedVideo) -> String {
    match &asked.account {
        Some(account) => format!("{}, {account}", asked.provider),
        None => asked.provider.clone(),
    }
}

/// `5 s`, `1.5 s`: seconds to the tenth, as a length is said.
fn seconds(value: f64) -> String {
    let text = format!("{value:.1}");
    format!("{} s", text.strip_suffix(".0").unwrap_or(&text))
}

/// The line that says what came back, and a warning for each option the
/// result did not honour, in the order of the options.
fn summary(command: &str, asked: &AskedVideo, video: &MadeVideo, facts: &mp4::Mp4Facts, wanted: &Wanted) -> String {
    let sound = if facts.sound { "with sound" } else { "without sound" };
    let mut said = format!(
        "{} {}×{} MP4 {sound} from {} ({})",
        seconds(facts.seconds),
        facts.width,
        facts.height,
        asked.model,
        entry(asked)
    );
    if let (Some((change, _)), Some(SentSource::Job { job })) = (&wanted.change, &asked.source) {
        let verb = match change {
            VideoChange::Edit => "editing",
            VideoChange::Extend => "extending",
        };
        said.push_str(&format!(", {verb} its job {job}"));
    }
    if let Some(cost) = video.cost {
        said.push_str(&format!(", {cost}"));
    }
    said.push('\n');
    let mut warn = |text: String| said.push_str(&format!("{command}: warning: {text}\n"));
    if let (Some((_, path)), Some(SentSource::FileInstead { job })) = (&wanted.change, &asked.source) {
        warn(format!("{} no longer keeps job {job}; {path} was sent instead", asked.vendor));
    }
    let got = format!("{}×{}", facts.width, facts.height);
    if let Some(duration) = wanted.duration
        && (facts.seconds - f64::from(duration)).abs() > DURATION_SLACK
    {
        warn(format!("asked for {}, got {}", seconds(f64::from(duration)), seconds(facts.seconds)));
    }
    if let Some(aspect) = wanted.aspect
        && !fits(aspect, facts.width, facts.height)
    {
        warn(format!("asked for {}:{}, got {got}", aspect.width, aspect.height));
    }
    if let Some(resolution) = wanted.resolution
        && !resolution.fits(facts.width, facts.height)
    {
        warn(format!("asked for {}, got {got}", resolution.lowercase()));
    }
    match wanted.audio {
        Some(true) if !facts.sound => warn("asked for sound, got a video without sound".to_owned()),
        Some(false) if facts.sound => warn("asked for no sound, got a video with sound".to_owned()),
        _ => {}
    }
    said
}
