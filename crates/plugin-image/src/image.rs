//! What `demi image` does (`images.md`): reads the call's prompt and, for
//! an edit, its images from the Host the command runs on; asks the
//! Providers service for one image; decodes it whole and writes it to
//! stdout in the format asked for, PNG by default; and says on stderr what
//! came back, with a warning for each option the result did not honour. A
//! failure is told in the design's words.

use std::io::Cursor;

use demi_host_interface::RpcInvocation;
use demi_plugin_interface::{
    EntryStatus, HostFile, HostRead, ImageAsk, ImageAspect, ImageInput, ImageKind, ImageRefusal, PluginError,
    PluginPort, PortFailure, PortRefusal, Unable,
};
use demi_shared_types::Timestamp;
use demi_shared_types::output::{Cell, Fields, Table, instant, note, warning};
use demi_shared_types::whole_image;
use image::{DynamicImage, GenericImageView as _, ImageFormat};
use jiff::tz::TimeZone;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::commands::{EditArgs, Format, GenerateArgs, ListedModel, ModelsArgs, ModelsOutput};

/// The largest image an edit sends.
const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

/// The most memory decoding the image made may take.
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;

/// The largest prompt read from stdin.
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

const NO_PROVIDER: &str =
    "no provider makes images; sign in to ChatGPT or Grok, or add an OpenAI, xAI or Google key, in Settings › Providers";

const NO_MODEL: &str = "--model is required; demi image models lists them";

/// Runs the leaf `invocation` names.
pub(crate) async fn run(invocation: RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    match invocation.path.last().map(String::as_str) {
        Some("generate") => {
            let args: GenerateArgs = decode(&invocation)?;
            let mut ask = ask(port, args.prompt, args.model, args.aspect, args.transparent).await?;
            ask.provider = args.provider;
            make(&invocation, port, ask, args.format.unwrap_or_default()).await
        }
        Some("edit") => {
            let args: EditArgs = decode(&invocation)?;
            let mut ask = ask(port, args.prompt, args.model, args.aspect, args.transparent).await?;
            ask.provider = args.provider;
            ask.images = read_images(port, &args.image).await?;
            make(&invocation, port, ask, args.format.unwrap_or_default()).await
        }
        Some("models") => {
            let args: ModelsArgs = decode(&invocation)?;
            models(&invocation, port, args.model).await
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
    // The agent always names the model, so no image comes from a model
    // nobody chose (`images.md` § Commands).
    let model = model.filter(|model| !model.trim().is_empty()).ok_or_else(|| PluginError::failed(NO_MODEL))?;
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
        provider: None,
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

/// The image as written: its bytes when they are not the vendor's, its
/// format's name, its size, and whether a pixel of it is transparent,
/// which an alpha channel alone does not say.
struct Finished {
    encoded: Option<Vec<u8>>,
    name: &'static str,
    width: u32,
    height: u32,
    transparent: bool,
}

/// Why the vendor's image was not written.
enum Unfinished {
    /// Not a PNG, JPEG or WebP image, or one cut or corrupt.
    Unreadable,
    /// It decoded but did not encode in the format asked for.
    Unencoded(image::ImageError),
}

/// `bytes`, decoded whole as a PNG, JPEG or WebP image within
/// [`MAX_DECODE_BYTES`], as an image in `format`: encoded again when the
/// vendor sent another format (`images.md` § Commands). A JPEG has no
/// alpha channel, so one is written opaque.
fn finish(bytes: &[u8], format: Format) -> Result<Finished, Unfinished> {
    let sent = match image::guess_format(bytes) {
        Ok(sent @ (ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP)) => sent,
        _ => return Err(Unfinished::Unreadable),
    };
    // A picture cut short is refused before it is converted, as the JPEG
    // decoder would fill it in.
    let image = whole_image::decode(bytes, sent, MAX_DECODE_BYTES).map_err(|_| Unfinished::Unreadable)?;
    let target = match format {
        Format::Png => ImageFormat::Png,
        Format::Jpeg => ImageFormat::Jpeg,
        Format::Webp => ImageFormat::WebP,
    };
    let name = format_name(format);
    let alpha = image.color().has_alpha() && target != ImageFormat::Jpeg;
    let transparent = alpha && image.pixels().any(|(_, _, pixel)| pixel.0[3] < u8::MAX);
    let (width, height) = image.dimensions();
    let encoded = if sent == target {
        None
    } else {
        // JPEG and WebP take 8-bit samples; PNG takes the image as it is.
        let written = match (target, alpha) {
            (ImageFormat::Png, _) => image,
            (_, true) => DynamicImage::ImageRgba8(image.to_rgba8()),
            (_, false) => DynamicImage::ImageRgb8(image.to_rgb8()),
        };
        let mut out = Cursor::new(Vec::new());
        written.write_to(&mut out, target).map_err(Unfinished::Unencoded)?;
        Some(out.into_inner())
    };
    Ok(Finished {
        encoded,
        name,
        width,
        height,
        transparent,
    })
}

/// Asks for the image, writes it to stdout in `format`, and says what came
/// back.
async fn make(invocation: &RpcInvocation, port: &PluginPort, ask: ImageAsk, format: Format) -> Result<u8, PluginError> {
    let aspect = ask.aspect;
    let transparent = ask.transparent;
    let made = match port.make_image(ask).await {
        Ok(made) => made,
        Err(PortFailure::Refused(PortRefusal::Image { refusal })) => {
            return Err(PluginError::failed(words(&refusal, aspect, invocation)));
        }
        Err(failure) => return Err(failure.into()),
    };
    let bytes = made.bytes.clone().into_bytes();
    // Decoding takes the thread for a while; the shard's thread never
    // blocks (`plugins.md` § Rules for a plugin in process).
    let finished = tokio::task::spawn_blocking(move || finish(&bytes, format))
        .await
        .map_err(|error| PluginError::failed(format!("the image's decoding stopped: {error}")))?;
    let Finished { encoded, name, width, height, transparent: has_transparency } = match finished {
        Ok(finished) => finished,
        Err(Unfinished::Unreadable) => {
            return Err(PluginError::failed(format!("{} returned an image that could not be read", made.vendor)));
        }
        Err(Unfinished::Unencoded(error)) => {
            return Err(PluginError::failed(format!("the image could not be written as {name}: {error}", name = format_name(format))));
        }
    };
    let rpc = port.rpc();
    match encoded {
        Some(encoded) => rpc.stdout(encoded).await?,
        None => rpc.stdout(made.bytes.into_bytes()).await?,
    }
    // A model the vendor does not confirm is the one asked of, not the one
    // that answered (`images.md` § Commands); the entry is named as
    // Settings names it (`media.md` § What the agent is told).
    let mut said = if made.confirmed {
        format!("{width}×{height} {name} from {} ({}", made.model, made.provider)
    } else {
        format!("{width}×{height} {name}, asked of {} ({}", made.model, made.provider)
    };
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

fn format_name(format: Format) -> &'static str {
    match format {
        Format::Png => "PNG",
        Format::Jpeg => "JPEG",
        Format::Webp => "WebP",
    }
}

/// `models`: the list as a table, or `shown` as labelled lines, and the
/// same as JSON with `--json` (`commands.md` § Output). Notes, such as an
/// entry whose models could not be listed, go to stderr.
async fn models(invocation: &RpcInvocation, port: &PluginPort, shown: Option<String>) -> Result<u8, PluginError> {
    let listing = match port.image_models().await {
        Ok(listing) => listing,
        Err(PortFailure::Refused(PortRefusal::Image { refusal })) => {
            return Err(PluginError::failed(words(&refusal, None, invocation)));
        }
        Err(failure) => return Err(failure.into()),
    };
    let command = format!("demi {}", invocation.path.join(" "));
    let mut notes = String::new();
    for unlisted in &listing.unlisted {
        notes.push_str(&warning(
            &command,
            &format!(
                "{}'s models could not be listed: {}",
                unlisted.provider,
                words(&unlisted.refusal, None, invocation)
            ),
        ));
    }
    let listed: Vec<ListedModel> = listing
        .models
        .into_iter()
        .map(|model| ListedModel {
            entry: model
                .entries
                .iter()
                .find(|entry| entry.status == EntryStatus::Now)
                .map(|entry| entry.provider.clone()),
            model,
        })
        .collect();
    let rpc = port.rpc();
    if let Some(id) = shown {
        // An alias names the model it is an alias of, listed once.
        let Some(model) = listed
            .into_iter()
            .find(|model| model.model.id == id || model.model.aliases.iter().any(|alias| *alias == id))
        else {
            return Err(PluginError::failed(format!("no provider makes \"{id}\"; demi image models lists them")));
        };
        unconfirmed(&mut notes, &command, std::slice::from_ref(&model));
        let text = if invocation.json {
            json_line(&ModelsOutput::One(model))
        } else {
            show(&model)
        };
        rpc.stdout(text).await?;
        if !notes.is_empty() {
            rpc.stderr(notes).await?;
        }
        return Ok(0);
    }
    if let Some(none) = &listing.none {
        notes.push_str(&note(&command, &words(none, None, invocation)));
    }
    unconfirmed(&mut notes, &command, &listed);
    let text = if invocation.json {
        json_line(&ModelsOutput::List(listed))
    } else {
        let mut table = Table::new(["id", "vendor", "released", "entry", "name"]);
        for listed in &listed {
            let model = &listed.model;
            table.row::<Cell>([
                model.id.as_str().into(),
                model.vendor.as_deref().into(),
                model.released.as_deref().into(),
                listed.entry.as_deref().into(),
                model.name.as_deref().into(),
            ]);
        }
        table.render()
    };
    rpc.stdout(text).await?;
    if !notes.is_empty() {
        rpc.stderr(notes).await?;
    }
    Ok(0)
}

fn json_line(output: &ModelsOutput) -> String {
    let text = serde_json::to_string(output).expect("a listing encodes as JSON");
    format!("{text}\n")
}

/// A note, once for each entry, that its vendor does not confirm the model
/// that answers a call, which a call's line then says as *asked of*.
fn unconfirmed(notes: &mut String, command: &str, models: &[ListedModel]) {
    let mut named: Vec<&str> = Vec::new();
    for entry in models.iter().flat_map(|model| &model.model.entries) {
        if !entry.confirmed && !named.contains(&entry.provider.as_str()) {
            named.push(&entry.provider);
            notes.push_str(&note(command, &format!("{} does not confirm which model answers a call", entry.provider)));
        }
    }
}

fn joined(values: Option<&[String]>) -> String {
    match values {
        Some(values) if !values.is_empty() => values.join(", "),
        _ => "-".to_owned(),
    }
}

/// One model as labelled lines, its prices and its entries as tables
/// under their keys.
fn show(listed: &ListedModel) -> String {
    let model = &listed.model;
    let image = model.image.clone().unwrap_or_default();
    let custom = image.custom_size.as_ref().map(|rule| {
        let mut said = vec!["any WxH".to_owned()];
        if let Some(multiple) = rule.multiple_of {
            said.push(format!("each edge a multiple of {multiple}"));
        }
        if let Some(edge) = rule.max_edge {
            said.push(format!("each edge at most {edge}"));
        }
        if let Some(ratio) = rule.max_ratio {
            said.push(format!("long:short at most {ratio}:1"));
        }
        match (rule.min_pixels, rule.max_pixels) {
            (Some(min), Some(max)) => said.push(format!("{min} to {max} pixels")),
            (None, Some(max)) => said.push(format!("at most {max} pixels")),
            (Some(min), None) => said.push(format!("at least {min} pixels")),
            (None, None) => {}
        }
        said.join("; ")
    });
    let mut fields = Fields::new();
    fields
        .field("ID", model.id.as_str())
        .field("Vendor", model.vendor.as_deref())
        .field("Released", model.released.as_deref())
        .field("Entry", listed.entry.as_deref())
        .field("Name", model.name.as_deref())
        .field("Alias of", model.alias_of.as_deref())
        .field("Aliases", joined(Some(&model.aliases)))
        .field("Sizes", joined(image.sizes.as_deref()))
        .field("Custom size", custom)
        .field("Aspect ratios", joined(image.aspect_ratios.as_deref()))
        .field("Resolutions", joined(image.resolutions.as_deref()))
        .field("Edits", image.edits)
        .field("Max input images", image.max_input_images)
        .field("Transparent background", image.transparent)
        .field("Price page", model.price.as_ref().and_then(|price| price.page.as_deref()));
    let rates = model.price.as_ref().map(|price| &price.rates[..]).unwrap_or_default();
    if rates.is_empty() {
        fields.field("Prices", None::<&str>);
    } else {
        let mut prices = Table::new(["unit", "modality", "amount", "currency", "size", "quality", "resolution", "duration (s)"]);
        for rate in rates {
            prices.row::<Cell>([
                rate.unit.as_str().into(),
                rate.modality.as_deref().into(),
                rate.amount.to_string().into(),
                rate.currency.as_str().into(),
                rate.size.as_deref().into(),
                rate.quality.as_deref().into(),
                rate.resolution.as_deref().into(),
                rate.duration.into(),
            ]);
        }
        fields.table("Prices", &prices);
    }
    let mut entries = Table::new(["entry", "kind", "account", "price", "status"]);
    for entry in &model.entries {
        let price = match (entry.subscription, rates.is_empty()) {
            (true, _) => Some("included"),
            (false, false) => Some("listed"),
            (false, true) => None,
        };
        entries.row::<Cell>([
            entry.provider.as_str().into(),
            if entry.subscription { "subscription" } else { "api-key" }.into(),
            entry.account.as_deref().into(),
            price.into(),
            status_words(&entry.status).into(),
        ]);
    }
    fields.table("Entries", &entries);
    if let Some(description) = &model.description {
        fields.text("Description", description);
    }
    fields.render()
}

/// Why an entry would or would not make a call now, in the list's words.
fn status_words(status: &EntryStatus) -> String {
    match status {
        EntryStatus::Now => "now".to_owned(),
        EntryStatus::Next => "next".to_owned(),
        EntryStatus::UsedUp { until: Some(until) } => format!("used up until {}", instant(*until)),
        EntryStatus::UsedUp { until: None } => "used up".to_owned(),
        EntryStatus::Off => "Image Generation off".to_owned(),
        EntryStatus::NotSignedIn => "not signed in".to_owned(),
        EntryStatus::TurnedOff => "turned off in Settings › Providers".to_owned(),
    }
}

/// The words of a refusal (`images.md` § Accounts and failures); a refused
/// shape names `aspect`, the one asked for.
fn words(refusal: &ImageRefusal, aspect: Option<ImageAspect>, invocation: &RpcInvocation) -> String {
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
        ImageRefusal::KeyRefused { vendor, manages: true } => {
            format!("{vendor} refused the API key; check it in Settings › Providers")
        }
        ImageRefusal::KeyRefused { vendor, manages: false } => {
            format!("{vendor} refused the API key; ask your administrator to check it")
        }
        ImageRefusal::Refused { vendor, message } => format!("{vendor} refused the request: {message}"),
        ImageRefusal::Shape { vendor, takes } => {
            let asked = aspect.map_or_else(|| "shape asked for".to_owned(), |aspect| format!("{}:{}", aspect.width, aspect.height));
            format!("{vendor} takes no {asked} (it takes {})", takes.join(", "))
        }
        ImageRefusal::Expired { vendor, account: Some(account) } => {
            format!("the {vendor} sign-in for {account} has expired; sign in again in Settings › Providers")
        }
        ImageRefusal::Expired { vendor, account: None } => {
            format!("the {vendor} sign-in has expired; ask your administrator to sign in again")
        }
        ImageRefusal::Unanswered { vendor, cause } => format!("{vendor} did not answer: {cause}"),
        ImageRefusal::Cannot { provider, model, why, can } => {
            let zone = TimeZone::get(&invocation.context.locale.time_zone).unwrap_or(TimeZone::UTC);
            let why = match why {
                Unable::NotOffered => format!("it does not make \"{model}\""),
                Unable::UsedUp { until: Some(moment) } => {
                    format!("its usage limit is reached, resetting at {}", clock_time(*moment, &zone))
                }
                Unable::UsedUp { until: None } => "its usage limit is reached".to_owned(),
                Unable::Off => "its image generation is off in Settings › Providers".to_owned(),
                Unable::NotSignedIn => "it is not signed in".to_owned(),
                Unable::TurnedOff => "it is turned off in Settings › Providers".to_owned(),
            };
            let others = match can.as_slice() {
                [] => "no other provider can make it now".to_owned(),
                [one] => format!("{one} can"),
                several => format!("{} can", listed(several)),
            };
            format!("{provider} cannot make \"{model}\": {why}; {others}")
        }
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
