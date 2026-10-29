//! A widget's frame: the size its page declares in its head,
//! `<meta name="demi-frame" content="...">`, in CSS's own words and meaning.
//!
//! Only `height`, `aspect-ratio`, `min-height` and `max-height` count,
//! lengths in `px`. The frame has that size before the page loads and keeps
//! it, so nothing in the reply moves; a page that declares nothing takes its
//! content's height once it loads.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The tallest frame a declaration may ask for: a page taller than a window
/// is read by scrolling inside it or full screen.
pub(crate) const MAX_HEIGHT: u32 = 4000;

/// A widget's frame, as its page declares it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FrameSize {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Width over height.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<AspectRatio>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_height: Option<u32>,
}

/// A ratio of width to height, such as 16 / 9.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AspectRatio {
    pub width: u32,
    pub height: u32,
}

impl FrameSize {
    /// Whether the page declared a size the frame keeps from the start.
    pub(crate) fn declared(&self) -> bool {
        self.height.is_some() || self.aspect_ratio.is_some()
    }

    /// The frame's height at `width`, none for a page that declared no
    /// size, as CSS lays out `height`, `aspect-ratio` and their bounds.
    pub(crate) fn height_at(&self, width: u32) -> Option<u32> {
        let height = match (self.height, self.aspect_ratio) {
            (Some(height), _) => height,
            (None, Some(ratio)) => {
                (u64::from(width) * u64::from(ratio.height)).div_ceil(u64::from(ratio.width)) as u32
            }
            (None, None) => return None,
        };
        let height = self.max_height.map_or(height, |max| height.min(max));
        Some(self.min_height.map_or(height, |min| height.max(min)))
    }

    /// The declaration as `demi widget create` prints it, such as
    /// `height 330px`, or `no frame declared` for one that declares none.
    pub(crate) fn describe(&self) -> String {
        let mut parts = Vec::new();
        if let Some(height) = self.height {
            parts.push(format!("height {height}px"));
        }
        if let Some(ratio) = self.aspect_ratio {
            parts.push(format!("aspect-ratio {} / {}", ratio.width, ratio.height));
        }
        if let Some(min) = self.min_height {
            parts.push(format!("min-height {min}px"));
        }
        if let Some(max) = self.max_height {
            parts.push(format!("max-height {max}px"));
        }
        if parts.is_empty() {
            "no frame declared: it takes its content's height once loaded".into()
        } else {
            parts.join(", ")
        }
    }
}

/// The frame `page` declares; why, for a declaration that cannot be read.
pub(crate) fn declared(page: &str) -> Result<FrameSize, String> {
    let Some(content) = meta_content(page) else {
        return Ok(FrameSize::default());
    };
    let mut frame = FrameSize::default();
    for declaration in content.split(';').map(str::trim).filter(|part| !part.is_empty()) {
        let (property, value) = declaration
            .split_once(':')
            .ok_or_else(|| format!("demi-frame: \"{declaration}\" is not a property and its value"))?;
        let (property, value) = (property.trim(), value.trim());
        match property {
            "height" => frame.height = Some(length(property, value)?),
            "min-height" => frame.min_height = Some(length(property, value)?),
            "max-height" => frame.max_height = Some(length(property, value)?),
            "aspect-ratio" => frame.aspect_ratio = Some(ratio(value)?),
            _ => {
                return Err(format!(
                    "demi-frame: {property} is not one of height, aspect-ratio, min-height and max-height"
                ));
            }
        }
    }
    if let (Some(min), Some(max)) = (frame.min_height, frame.max_height)
        && min > max
    {
        return Err(format!("demi-frame: min-height {min}px is more than max-height {max}px"));
    }
    if !frame.declared() && (frame.min_height.is_some() || frame.max_height.is_some()) {
        return Err("demi-frame: min-height and max-height bound a height or an aspect-ratio, which it lacks".into());
    }
    Ok(frame)
}

/// A length in whole CSS pixels, such as `360px`.
fn length(property: &str, value: &str) -> Result<u32, String> {
    let pixels = value
        .strip_suffix("px")
        .and_then(|number| number.trim().parse::<f64>().ok())
        .filter(|pixels| pixels.is_finite() && *pixels >= 1.0)
        .ok_or_else(|| format!("demi-frame: {property} {value} is not a length in px, such as 360px"))?;
    if pixels > f64::from(MAX_HEIGHT) {
        return Err(format!("demi-frame: {property} {value} is over {MAX_HEIGHT}px"));
    }
    Ok(pixels.round() as u32)
}

/// A ratio such as `16 / 9`, or one number, such as `1.5`, over 1.
fn ratio(value: &str) -> Result<AspectRatio, String> {
    let unreadable = || format!("demi-frame: aspect-ratio {value} is not a ratio, such as 16 / 9");
    let number = |text: &str| {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|number| number.is_finite() && *number > 0.0)
    };
    let (width, height) = match value.split_once('/') {
        Some((width, height)) => (number(width).ok_or_else(unreadable)?, number(height).ok_or_else(unreadable)?),
        None => (number(value).ok_or_else(unreadable)?, 1.0),
    };
    // Hundredths keep a ratio such as 1.85 / 1 exact.
    let ratio = AspectRatio {
        width: (width * 100.0).round() as u32,
        height: (height * 100.0).round() as u32,
    };
    if ratio.width == 0 || ratio.height == 0 {
        return Err(unreadable());
    }
    Ok(ratio)
}

/// The content of the page's first `demi-frame` meta tag outside comments
/// and scripts.
fn meta_content(page: &str) -> Option<String> {
    let scanned = crate::markup::scan(page);
    let mut from = 0;
    while let Some((start, end)) = crate::markup::find_tag(&scanned[from..], "meta").map(|(start, end)| (from + start, from + end)) {
        let tag = &page[start..end];
        if crate::markup::attribute(tag, "name").is_some_and(|name| name.eq_ignore_ascii_case("demi-frame")) {
            return crate::markup::attribute(tag, "content");
        }
        from = end;
    }
    None
}

