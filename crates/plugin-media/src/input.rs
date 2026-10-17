//! A media call's input: its arguments, its model, its prompt, its images
//! and its shape.

use demi_host_interface::RpcInvocation;
use demi_plugin_interface::{
    HostFile, HostRead, ImageAspect, ImageInput, ImageKind, PluginError, PluginPort, PortFailure, PortRefusal,
};
use demi_shared_types::MediaKind;
use image::ImageFormat;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// The largest image a call sends.
pub const MAX_IMAGE_BYTES: u64 = 16 * 1024 * 1024;

/// The largest prompt read from stdin.
const MAX_PROMPT_BYTES: usize = 1024 * 1024;

/// The invocation's arguments, as the leaf declared them.
pub fn decode_args<T: DeserializeOwned>(invocation: &RpcInvocation) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(invocation.args.clone()))
        .map_err(|error| PluginError::failed(format!("the arguments do not decode as declared: {error}")))
}

/// The model the agent named: it always names one, so no media come from
/// a model nobody chose (`media.md` § Which entry makes a call).
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

/// The images at `paths`, read together from the Host the command runs
/// on, each relative to the shell's directory or absolute, as that Host
/// resolves it: a PNG, JPEG or WebP file of at most 16 MiB each, else the
/// call fails naming it.
pub async fn read_images(port: &PluginPort, paths: &[String]) -> Result<Vec<ImageInput>, PluginError> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
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
