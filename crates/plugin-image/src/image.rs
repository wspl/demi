//! What `demi image` does (`images.md`): reads the call's prompt and, for
//! an edit, its images from the Host the command runs on; asks the
//! Providers service for one image; decodes it whole and writes it to
//! stdout; and says on stderr what came back, with a warning for each
//! option the result did not honour. A failure is told in the design's
//! words.

use std::io::Cursor;

use demi_host_interface::RpcInvocation;
use demi_host_interface::text::table;
use demi_plugin_interface::{
    HostFile, HostRead, ImageAsk, ImageAspect, ImageInput, ImageKind, ImageRefusal, PluginError,
    PluginPort, PortFailure, PortRefusal,
};
use demi_shared_types::Timestamp;
use image::{GenericImageView as _, ImageFormat, ImageReader, Limits};
use jiff::tz::TimeZone;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::commands::{EditArgs, GenerateArgs, ListedModel, Listing, ModelsArgs};

/// The largest image an edit sends.
const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

/// The most memory decoding the image made may take.
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;

/// The largest prompt read from stdin.
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

const NO_PROVIDER: &str = "no provider makes images; sign in to ChatGPT or Grok in Settings › Providers";

/// Runs the leaf `invocation` names.
pub(crate) async fn run(invocation: RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    match invocation.path.last().map(String::as_str) {
        Some("generate") => {
            let args: GenerateArgs = decode(&invocation)?;
            let ask = ask(port, args.prompt, args.model, args.aspect, args.transparent).await?;
            make(&invocation, port, ask).await
        }
        Some("edit") => {
            let args: EditArgs = decode(&invocation)?;
            let mut ask = ask(port, args.prompt, args.model, args.aspect, args.transparent).await?;
            ask.images = read_images(port, &args.image).await?;
            make(&invocation, port, ask).await
        }
        Some("models") => {
            let _: ModelsArgs = decode(&invocation)?;
            models(&invocation, port).await
        }
        _ => Err(PluginError::failed(format!("no command \"{}\"", invocation.path.join(" ")))),
    }
}

fn decode<T: DeserializeOwned>(invocation: &RpcInvocation) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(invocation.args.clone()))
        .map_err(|error| PluginError::failed(format!("the arguments do not decode as declared: {error}")))
}

/// The call's ask: its prompt, from stdin for `-`, and its options.
async fn ask(
    port: &PluginPort,
    prompt: String,
    model: Option<String>,
    aspect: Option<String>,
    transparent: Option<bool>,
) -> Result<ImageAsk, PluginError> {
    let prompt = if prompt == "-" { stdin_text(port).await? } else { prompt };
    if prompt.trim().is_empty() {
        return Err(PluginError::failed("the prompt is empty"));
    }
    let aspect = aspect
        .map(|text| {
            parse_aspect(&text)
                .ok_or_else(|| PluginError::failed(format!("--aspect {text}: a shape is W:H, such as 16:9")))
        })
        .transpose()?;
    Ok(ImageAsk {
        prompt,
        images: Vec::new(),
        aspect,
        transparent: transparent == Some(true),
        model,
    })
}

/// The shape `text` names as `W:H`, each a positive whole number.
fn parse_aspect(text: &str) -> Option<ImageAspect> {
    let (width, height) = text.split_once(':')?;
    let width: u32 = width.trim().parse().ok()?;
    let height: u32 = height.trim().parse().ok()?;
    (width > 0 && height > 0).then_some(ImageAspect { width, height })
}

/// Whether an image of `width`×`height` pixels has the shape `aspect`,
/// within 2%: a vendor rounds its sides to its own grid.
fn fits(aspect: ImageAspect, width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let asked = f64::from(aspect.width) / f64::from(aspect.height);
    let got = f64::from(width) / f64::from(height);
    (got / asked - 1.0).abs() <= 0.02
}

/// The whole of the call's stdin, as text.
async fn stdin_text(port: &PluginPort) -> Result<String, PluginError> {
    let rpc = port.rpc();
    let mut bytes = Vec::new();
    while let Some(chunk) = rpc.read_stdin().await? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_PROMPT_BYTES {
            return Err(PluginError::failed("the prompt on stdin is over 1 MiB"));
        }
    }
    String::from_utf8(bytes).map_err(|_| PluginError::failed("the prompt on stdin is not UTF-8 text"))
}

/// The images of an edit, read together from the Host the command runs
/// on, each relative to the shell's directory or absolute, as that Host
/// resolves it.
async fn read_images(port: &PluginPort, paths: &[String]) -> Result<Vec<ImageInput>, PluginError> {
    let reads = paths.iter().map(|path| HostRead::new(path.clone(), MAX_IMAGE_BYTES)).collect();
    let files = match port.read_host_files(reads).await {
        Ok(files) => files,
        Err(PortFailure::Refused(PortRefusal::NotRunning)) => {
            return Err(PluginError::failed("the Host is not running"));
        }
        Err(failure) => return Err(failure.into()),
    };
    paths
        .iter()
        .zip(files)
        .map(|(path, file)| {
            let named = |reason: &str| PluginError::failed(format!("{path}: {reason}"));
            let bytes = match file {
                HostFile::File { size, .. } if size > MAX_IMAGE_BYTES => {
                    return Err(named(&format!("the file is {size} bytes; an image is at most 16 MiB")));
                }
                HostFile::File { bytes, .. } => bytes,
                HostFile::Missing => return Err(named("no such file")),
                HostFile::Directory { .. } => return Err(named("is a directory")),
                HostFile::Unreadable { message } => return Err(named(&message)),
                HostFile::Other | HostFile::Link => return Err(named("not a file")),
            };
            let kind = match image::guess_format(&bytes) {
                Ok(ImageFormat::Png) => ImageKind::Png,
                Ok(ImageFormat::Jpeg) => ImageKind::Jpeg,
                Ok(ImageFormat::WebP) => ImageKind::Webp,
                _ => return Err(named("not a PNG, JPEG or WebP image")),
            };
            Ok(ImageInput { kind, bytes })
        })
        .collect()
}

/// What the decoded image is: its format's name, its size, and whether a
/// pixel of it is transparent, which an alpha channel alone does not say.
struct Decoded {
    name: &'static str,
    width: u32,
    height: u32,
    transparent: bool,
}

/// `bytes` decoded whole as a PNG, JPEG or WebP image within
/// [`MAX_DECODE_BYTES`]; none for anything else, or one cut or corrupt.
fn decode_image(bytes: &[u8]) -> Option<Decoded> {
    let (format, name) = match image::guess_format(bytes).ok()? {
        ImageFormat::Png => (ImageFormat::Png, "PNG"),
        ImageFormat::Jpeg => (ImageFormat::Jpeg, "JPEG"),
        ImageFormat::WebP => (ImageFormat::WebP, "WebP"),
        _ => return None,
    };
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let transparent = image.color().has_alpha() && image.pixels().any(|(_, _, pixel)| pixel.0[3] < u8::MAX);
    Some(Decoded {
        name,
        width: image.width(),
        height: image.height(),
        transparent,
    })
}

/// Asks for the image, writes it to stdout, and says what came back.
async fn make(invocation: &RpcInvocation, port: &PluginPort, ask: ImageAsk) -> Result<u8, PluginError> {
    let aspect = ask.aspect;
    let transparent = ask.transparent;
    let made = match port.make_image(ask).await {
        Ok(made) => made,
        Err(PortFailure::Refused(PortRefusal::Image { refusal })) => {
            return Err(PluginError::failed(words(&refusal, invocation)));
        }
        Err(failure) => return Err(failure.into()),
    };
    let unreadable = || PluginError::failed(format!("{} returned an image that could not be read", made.vendor));
    let bytes = made.bytes.clone().into_bytes();
    // Decoding takes the thread for a while; the shard's thread never
    // blocks (`plugins.md` § Rules for a plugin in process).
    let decoded = tokio::task::spawn_blocking(move || decode_image(&bytes))
        .await
        .map_err(|error| PluginError::failed(format!("the image's decoding stopped: {error}")))?;
    let Some(Decoded { name, width, height, transparent: has_transparency }) = decoded else {
        return Err(unreadable());
    };
    let rpc = port.rpc();
    rpc.stdout(made.bytes.into_bytes()).await?;
    let mut said = format!("{width}×{height} {name} from {} ({}", made.model, made.vendor);
    if let Some(account) = &made.account {
        said.push_str(&format!(", {account}"));
    }
    said.push_str(")\n");
    let command = format!("demi {}", invocation.path.join(" "));
    if let Some(aspect) = aspect
        && !fits(aspect, width, height)
    {
        said.push_str(&format!(
            "{command}: warning: asked for {}:{}, got {width}×{height}\n",
            aspect.width, aspect.height
        ));
    }
    if transparent && !has_transparency {
        said.push_str(&format!(
            "{command}: warning: asked for a transparent background, got an opaque image\n"
        ));
    }
    rpc.stderr(said).await?;
    Ok(0)
}

async fn models(invocation: &RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    let models = match port.image_models().await {
        Ok(models) => models,
        Err(PortFailure::Refused(PortRefusal::Image { refusal })) => {
            return Err(PluginError::failed(words(&refusal, invocation)));
        }
        Err(failure) => return Err(failure.into()),
    };
    let listed: Vec<ListedModel> = models
        .into_iter()
        .enumerate()
        .map(|(index, model)| ListedModel {
            id: model.id,
            provider: model.provider,
            provider_id: model.provider_id,
            default: index == 0,
        })
        .collect();
    let rpc = port.rpc();
    if invocation.json {
        let text = serde_json::to_string(&Listing { models: listed }).expect("a listing encodes as JSON");
        rpc.stdout(format!("{text}\n")).await?;
        return Ok(0);
    }
    if listed.is_empty() {
        return Err(PluginError::failed(NO_PROVIDER));
    }
    let rows: Vec<[String; 2]> = listed.into_iter().map(|model| [model.id, model.provider]).collect();
    rpc.stdout(table(&rows)).await?;
    Ok(0)
}

/// The words of a refusal (`images.md` § Accounts and failures).
fn words(refusal: &ImageRefusal, invocation: &RpcInvocation) -> String {
    match refusal {
        ImageRefusal::NoProvider => NO_PROVIDER.to_owned(),
        ImageRefusal::UnknownModel { model } => {
            format!("no provider makes \"{model}\"; demi image models lists them")
        }
        ImageRefusal::AllOff { providers } => {
            format!("image generation is off for {}; turn it on in Settings › Providers", listed(providers))
        }
        ImageRefusal::Off { model, provider } => {
            format!("\"{model}\" is {provider}'s, whose image generation is off in Settings › Providers")
        }
        ImageRefusal::UsedUp { vendor, resets_at: Some(moment) } => {
            let zone = TimeZone::get(&invocation.context.locale.time_zone).unwrap_or(TimeZone::UTC);
            format!("{vendor}'s usage limit is reached, resetting at {}", clock_time(*moment, &zone))
        }
        ImageRefusal::UsedUp { vendor, resets_at: None } => format!("{vendor}'s usage limit is reached"),
        ImageRefusal::Refused { vendor, message } => format!("{vendor} refused the request: {message}"),
        ImageRefusal::Expired { vendor, account: Some(account) } => {
            format!("the {vendor} sign-in for {account} has expired; sign in again in Settings › Providers")
        }
        ImageRefusal::Expired { vendor, account: None } => {
            format!("the {vendor} sign-in has expired; ask your administrator to sign in again")
        }
        ImageRefusal::Unanswered { vendor, cause } => format!("{vendor} did not answer: {cause}"),
        ImageRefusal::Failed { message } => message.clone(),
    }
}

/// `names` as a sentence lists them: `ChatGPT`, `ChatGPT and Grok`,
/// `A, B and C`.
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `moment` as a clock shows it in `zone`: `9:00 PM` today, `Oct 18, 9:00
/// PM` on another day.
fn clock_time(moment: Timestamp, zone: &TimeZone) -> String {
    let at = moment.to_jiff().to_zoned(zone.clone());
    let today = jiff::Timestamp::now().to_zoned(zone.clone()).date();
    if at.date() == today {
        at.strftime("%-I:%M %p").to_string()
    } else {
        at.strftime("%b %-d, %-I:%M %p").to_string()
    }
}
