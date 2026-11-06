//! A media call's input: its arguments, its model, its prompt, its images
//! and its shape.

use demi_device_interface::RpcInvocation;
use demi_plugin_interface::{ImageAspect, ImageInput, ImageKind, PluginError, PluginPort, PortFailure, PortRefusal};
use demi_device_interface::{DeviceFile, DeviceRead};
use demi_shared_types::{B64Bytes, MediaKind, mp4};
use image::ImageFormat;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The largest image a call sends: it is held whole in memory and sent to a
/// vendor, whose APIs refuse an image over a few tens of megabytes.
pub const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

/// The largest video an edit or an extension sends: the call holds it
/// whole in memory, and a source a vendor takes, of at most 10 seconds as
/// Gemini Omni's, is a few MiB.
pub const MAX_VIDEO_BYTES: u64 = 64 * 1024 * 1024;

/// The largest prompt read from stdin, which is held whole in memory.
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

/// The invocation's arguments, as the leaf declared them.
pub fn decode_args<T: DeserializeOwned>(invocation: &RpcInvocation) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(invocation.args.clone()))
        .map_err(|error| PluginError::failed(format!("the arguments do not decode as declared: {error}")))
}

/// The model the agent named: it always names one, so no media come from
/// a model nobody chose.
pub fn required_model(model: Option<String>, kind: MediaKind) -> Result<String, PluginError> {
    model
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| PluginError::failed(format!("--model is required; demi {kind} models lists them")))
}

/// The call's prompt: the argument, or the whole of stdin for `-`.
pub async fn prompt(port: &PluginPort, prompt: String) -> Result<String, PluginError> {
    let prompt = if prompt == "-" { stdin_text(port).await? } else { prompt };
    if prompt.trim().is_empty() {
        return Err(PluginError::failed("the prompt is empty"));
    }
    Ok(prompt)
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

/// The shape `text` names as `W:H`, each a positive whole number; a call
/// that cannot read it fails naming `--aspect`.
pub fn parse_aspect(text: &str) -> Result<ImageAspect, PluginError> {
    let parsed = text.split_once(':').and_then(|(width, height)| {
        let width: u32 = width.trim().parse().ok()?;
        let height: u32 = height.trim().parse().ok()?;
        (width > 0 && height > 0).then_some(ImageAspect { width, height })
    });
    parsed.ok_or_else(|| PluginError::failed(format!("--aspect {text}: a shape is W:H, such as 16:9")))
}

/// The images at `paths`, read together from the device the command runs
/// on, each relative to the shell's directory or absolute, as that device
/// resolves it: a PNG, JPEG or WebP file of at most 16 MiB each, else the
/// call fails naming it.
pub async fn read_images(port: &PluginPort, paths: &[String]) -> Result<Vec<ImageInput>, PluginError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let reads = paths.iter().map(|path| DeviceRead::content(path.clone(), MAX_IMAGE_BYTES)).collect();
    let files = match port.read_device_files(reads).await {
        Ok(files) => files,
        Err(PortFailure::Refused(PortRefusal::NotRunning)) => {
            return Err(PluginError::failed("the device is not running"));
        }
        Err(failure) => return Err(failure.into()),
    };
    paths
        .iter()
        .zip(files)
        .map(|(path, file)| {
            let named = |reason: &str| PluginError::failed(format!("{path}: {reason}"));
            let bytes = match file {
                DeviceFile::File { size, .. } if size > MAX_IMAGE_BYTES => {
                    return Err(named(&format!("the file is {size} bytes; an image is at most 16 MiB")));
                }
                DeviceFile::File { bytes, .. } => bytes,
                DeviceFile::Missing => return Err(named("no such file")),
                DeviceFile::Directory { .. } => return Err(named("is a directory")),
                DeviceFile::Unreadable { message } => return Err(named(&message)),
                DeviceFile::Other => return Err(named("not a file")),
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

/// The video at `path`, read from the device the command runs on as
/// [`read_images`] reads an image: an MP4 file of at most 64 MiB, else the
/// call fails naming it.
pub async fn read_video(port: &PluginPort, path: &str) -> Result<B64Bytes, PluginError> {
    let named = |reason: &str| PluginError::failed(format!("{path}: {reason}"));
    let read = DeviceRead::content(path.to_owned(), MAX_VIDEO_BYTES);
    let file = match port.read_device_files(vec![read]).await {
        Ok(files) => files.into_iter().next(),
        Err(PortFailure::Refused(PortRefusal::NotRunning)) => {
            return Err(PluginError::failed("the device is not running"));
        }
        Err(failure) => return Err(failure.into()),
    };
    let bytes = match file {
        Some(DeviceFile::File { size, .. }) if size > MAX_VIDEO_BYTES => {
            return Err(named(&format!("the file is {size} bytes; a video is at most 64 MiB")));
        }
        Some(DeviceFile::File { bytes, .. }) => bytes,
        Some(DeviceFile::Missing) | None => return Err(named("no such file")),
        Some(DeviceFile::Directory { .. }) => return Err(named("is a directory")),
        Some(DeviceFile::Unreadable { message }) => return Err(named(&message)),
        Some(DeviceFile::Other) => return Err(named("not a file")),
    };
    if mp4::read(bytes.as_bytes()).is_err() {
        return Err(named("not an MP4 video"));
    }
    Ok(bytes)
}

/// Whether an image of `width`×`height` pixels has the shape `aspect`,
/// within 2%: a vendor rounds its sides to its own grid.
pub fn fits(aspect: ImageAspect, width: u32, height: u32) -> bool {
    if width == 0 || height == 0 {
        return false;
    }
    let asked = f64::from(aspect.width) / f64::from(aspect.height);
    let got = f64::from(width) / f64::from(height);
    (got / asked - 1.0).abs() <= 0.02
}
