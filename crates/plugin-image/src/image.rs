//! What `demi image` does: reads the call's prompt and, for an edit, its
//! images from the Host the command runs on; asks the Providers service
//! for one image; decodes it whole and writes it to stdout in the format
//! asked for, PNG by default; and says on stderr what came back, with a
//! warning for each option the result did not honour. A failure is told in
//! the design's words.

use std::io::Cursor;

use demi_host_interface::RpcInvocation;
use demi_plugin_interface::{ImageAsk, PluginError, PluginPort};
use demi_plugin_media::{decode_args, fits, models, parse_aspect, prompt, read_images, refused, required_model};
use demi_shared_types::MediaKind;
use demi_shared_types::whole_image;
use image::{DynamicImage, GenericImageView as _, ImageFormat};

use crate::commands::{EditArgs, Format, GenerateArgs, ModelsArgs};

/// The most memory decoding the image made may take.
const MAX_DECODE_BYTES: u64 = 512 * 1024 * 1024;

/// Runs the leaf `invocation` names.
pub(crate) async fn run(invocation: RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    match invocation.path.last().map(String::as_str) {
        Some("generate") => {
            let args: GenerateArgs = decode_args(&invocation)?;
            let mut ask = ask(port, args.prompt, args.model, args.aspect, args.transparent).await?;
            ask.provider = args.provider;
            make(&invocation, port, ask, args.format.unwrap_or_default()).await
        }
        Some("edit") => {
            let args: EditArgs = decode_args(&invocation)?;
            let mut ask = ask(port, args.prompt, args.model, args.aspect, args.transparent).await?;
            ask.provider = args.provider;
            ask.images = read_images(port, &args.image).await?;
            make(&invocation, port, ask, args.format.unwrap_or_default()).await
        }
        Some("models") => {
            let args: ModelsArgs = decode_args(&invocation)?;
            models(&invocation, port, MediaKind::Image, args.model).await
        }
        _ => Err(PluginError::failed(format!("no command \"{}\"", invocation.path.join(" ")))),
    }
}

/// The call's ask: its prompt, from stdin for `-`, and its options.
async fn ask(
    port: &PluginPort,
    text: String,
    model: Option<String>,
    aspect: Option<String>,
    transparent: Option<bool>,
) -> Result<ImageAsk, PluginError> {
    let model = required_model(model, MediaKind::Image)?;
    let prompt = prompt(port, text).await?;
    let aspect = aspect.as_deref().map(parse_aspect).transpose()?;
    Ok(ImageAsk {
        prompt,
        images: Vec::new(),
        aspect,
        transparent: transparent == Some(true),
        model,
        provider: None,
    })
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
/// vendor sent another format. A JPEG has no alpha channel, so one is
/// written opaque.
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
    let made = port
        .make_image(ask)
        .await
        .map_err(|failure| refused(MediaKind::Image, failure, aspect, invocation))?;
    let bytes = made.bytes.clone().into_bytes();
    // Decoding takes the thread for a while; the shard's thread never
    // blocks (`crates/plugin-interface/src/plugin.rs`).
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
    // that answered; the entry is named as Settings names it.
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
