//! The screen's rounded corners as Android draws them into its pictures
//! (`crates/plugin-mobile/command/src/live.rs`).
//!
//! SystemUI paints a drawable over each corner of the panel:
//! `rounded_corner_top` at the top two and `rounded_corner_bottom` at the
//! bottom two, each drawn for the top left and mirrored into its place, and
//! none at all where the display reports no corner radius.
//!
//! The shape is read from the drawable, a vector path, in the APK the device
//! resolves it to. When it cannot be, it is the circle of the reported radius,
//! and where that is unknown too, a square corner.

use std::{collections::BTreeMap, f64::consts::FRAC_PI_2, time::Duration};

use kurbo::{Affine, BezPath, Shape as _};

use demi_command_package_mobile_protocol::live::{LiveCorner, LiveCorners, LiveCornerPoint};

use super::{
    axml::{self, Attribute, Event, Value},
    guest::Guest,
};
use crate::driver::{DriverError, Result};

/// How long reading the corners may take; what is not known by then falls
/// back as a failed read does.
pub const LIMIT: Duration = Duration::from_secs(20);
/// The most path elements a corner's drawable may hold: a Pixel's holds a
/// handful of curves.
const PATH_ELEMENTS: usize = 4096;

/// How many points of its edge a corner's shape is carried as.
const POINTS: usize = 48;
/// The system UI's package, whose drawables paint the corners.
const SYSTEM_UI: &str = "com.android.systemui";

/// One corner of the panel, as the top left one, in its pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum Corner {
    /// A quarter circle of this radius; a square corner at 0.
    Round(f64),
    /// The edge across the corner, from the upright edge to the level one,
    /// each point across from the upright edge and down from the level one.
    Curve(Vec<(f64, f64)>),
}

/// The panel's corners, which turn with the picture.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelCorners {
    pub top: Corner,
    pub bottom: Corner,
}

impl PanelCorners {
    /// Square corners, as where the display's radius is unknown.
    pub fn square() -> Self {
        Self {
            top: Corner::Round(0.0),
            bottom: Corner::Round(0.0),
        }
    }

    /// The corners as the picture shows the panel turned `turns` quarter
    /// turns counterclockwise, at `scale` of the units given per pixel.
    pub fn turned(&self, turns: u8, scale: f64) -> LiveCorners {
        // The panel's corners clockwise from the top left; the picture's
        // top left is the panel's corner `turns` after it.
        let panel = [&self.top, &self.top, &self.bottom, &self.bottom];
        let corner = |index: usize| live(panel[(index + turns as usize) % 4], turns % 2 == 1, scale);
        LiveCorners {
            top_left: corner(0),
            top_right: corner(1),
            bottom_right: corner(2),
            bottom_left: corner(3),
        }
    }
}

/// `corner` at `scale`; `across` when a quarter turn lays its upright edge
/// level, which swaps its two distances and so the order of its points.
fn live(corner: &Corner, across: bool, scale: f64) -> LiveCorner {
    match corner {
        Corner::Round(radius) => LiveCorner::Round { radius: radius * scale },
        Corner::Curve(points) => {
            let point = |&(x, y): &(f64, f64)| match across {
                false => LiveCornerPoint { x: x * scale, y: y * scale },
                true => LiveCornerPoint { x: y * scale, y: x * scale },
            };
            let mut points: Vec<LiveCornerPoint> = points.iter().map(point).collect();
            if across {
                points.reverse();
            }
            LiveCorner::Curve { points }
        }
    }
}

/// The panel's corners as the device's system UI draws them, at `density`
/// pixels per point, read within `limit`: its drawables' shapes, the
/// circles of the radii the display reports where a drawable does not
/// read, and square corners where the radius does not.
pub async fn read(guest: &dyn Guest, density: f64, limit: Duration) -> PanelCorners {
    let deadline = tokio::time::Instant::now() + limit;
    let (top, bottom) = match tokio::time::timeout_at(deadline, radii(guest, density)).await {
        Ok(Ok(radii)) => radii,
        Ok(Err(error)) => {
            tracing::warn!("the screen's corner radius does not read, so its corners are square: {error}");
            return PanelCorners::square();
        }
        Err(_) => {
            tracing::warn!("the screen's corner radius took too long to read, so its corners are square");
            return PanelCorners::square();
        }
    };
    let [top_shape, bottom_shape] = match tokio::time::timeout_at(deadline, drawables(guest, density, [top, bottom])).await {
        Ok(shapes) => shapes,
        Err(_) => {
            let late = || Err("it took too long to read".to_owned());
            [late(), late()]
        }
    };
    PanelCorners {
        top: corner(top, top_shape),
        bottom: corner(bottom, bottom_shape),
    }
}

/// A corner of `radius` whose drawable read as `shape`.
fn corner(radius: f64, shape: std::result::Result<Vec<(f64, f64)>, String>) -> Corner {
    if radius <= 0.0 {
        // SystemUI paints no corner where the display reports none.
        return Corner::Round(0.0);
    }
    match shape {
        Ok(points) => Corner::Curve(points),
        Err(error) => {
            tracing::warn!("the screen's corner drawable does not read, so its corner is a circle: {error}");
            Corner::Round(radius)
        }
    }
}

/// The radii the display reports for the top and the bottom corners.
async fn radii(guest: &dyn Guest, density: f64) -> Result<(f64, f64)> {
    let radius = |name: &'static str| async move {
        let value = guest.resource("android", &format!("android:dimen/{name}")).await?.value;
        dimension(&value, density).ok_or_else(|| DriverError::Failed(format!("{name} is not a size: {value}")))
    };
    let shared = radius("rounded_corner_radius").await?;
    // A top or bottom radius of its own stands in for the shared one, as
    // Android's `RoundedCorners` takes it; a device without one has none.
    let own = |radius: Result<f64>| radius.ok().filter(|radius| *radius > 0.0).unwrap_or(shared);
    Ok((own(radius("rounded_corner_radius_top").await), own(radius("rounded_corner_radius_bottom").await)))
}

/// The shapes of the top and the bottom corner drawables SystemUI uses
/// for corners of `radii`, none where the radius is none: each from the
/// APK the device resolves it to, and each APK read once.
async fn drawables(guest: &dyn Guest, density: f64, radii: [f64; 2]) -> [std::result::Result<Vec<(f64, f64)>, String>; 2] {
    let mut shapes: [std::result::Result<Vec<(f64, f64)>, String>; 2] = [Err("no corner".into()), Err("no corner".into())];
    // The drawables' paths by the APK each resolves to.
    let mut apks: BTreeMap<String, Vec<(usize, String)>> = BTreeMap::new();
    for (index, drawable) in ["rounded_corner_top", "rounded_corner_bottom"].into_iter().enumerate() {
        if radii[index] <= 0.0 {
            continue;
        }
        match guest.resource(SYSTEM_UI, &format!("{SYSTEM_UI}:drawable/{drawable}")).await {
            Ok(resolved) => match resolved.apk {
                Some(apk) => apks.entry(apk).or_default().push((index, resolved.value)),
                None => shapes[index] = Err(format!("the device does not say which APK holds {drawable}")),
            },
            Err(error) => shapes[index] = Err(error.to_string()),
        }
    }
    for (apk, wanted) in apks {
        let paths: Vec<String> = wanted.iter().map(|(_, path)| path.clone()).collect();
        let files = match guest.apk_files(&apk, &paths).await {
            Ok(files) => files,
            Err(error) => {
                for (index, _) in &wanted {
                    shapes[*index] = Err(format!("{apk} does not read: {error}"));
                }
                continue;
            }
        };
        for ((index, path), file) in wanted.into_iter().zip(files) {
            shapes[index] = match file {
                Some(xml) => tokio::task::spawn_blocking(move || shape(&xml, density))
                    .await
                    .unwrap_or_else(|error| Err(format!("reading the drawable stopped: {error}"))),
                None => Err(format!("{apk} holds no {path}")),
            };
        }
    }
    shapes
}

/// A size as Android prints a dimension, such as `132.0px` or `24.0dp`, in
/// pixels at `density` of them per point.
pub fn dimension(text: &str, density: f64) -> Option<f64> {
    let text = text.trim();
    let (number, scale) = if let Some(number) = text.strip_suffix("px") {
        (number, 1.0)
    } else if let Some(number) = text.strip_suffix("dip").or_else(|| text.strip_suffix("dp")) {
        (number, density)
    } else {
        return None;
    };
    number.parse::<f64>().ok().filter(|size| size.is_finite()).map(|size| size * scale)
}

/// The resource ids of the vector drawable's attributes.
const WIDTH: u32 = 0x0101_0159;
const HEIGHT: u32 = 0x0101_0155;
const VIEWPORT_WIDTH: u32 = 0x0101_0402;
const VIEWPORT_HEIGHT: u32 = 0x0101_0403;
const FILL_COLOR: u32 = 0x0101_0404;
const PATH_DATA: u32 = 0x0101_0405;

/// The attribute of resource `id`, or named `name` where the drawable keeps
/// no id for it.
fn attribute<'a>(attributes: &'a [Attribute], id: Option<u32>, name: &str) -> Option<&'a Value> {
    attributes
        .iter()
        .find(|attribute| (id.is_some() && attribute.id == id) || &*attribute.name == name)
        .map(|attribute| &attribute.value)
}

/// The screen's edge across a corner whose compiled vector drawable is
/// `xml`, at `density` pixels per point: where the screen begins along
/// each of [`POINTS`] rays from the far corner of the drawable's box,
/// whose filled paths cover the screen's corner.
pub fn shape(xml: &[u8], density: f64) -> std::result::Result<Vec<(f64, f64)>, String> {
    let events = axml::events(xml).map_err(|error| error.to_string())?;
    let Some(Event::Start { name, attributes }) = events.first() else {
        return Err("the drawable is empty".into());
    };
    if &**name != "vector" {
        return Err(format!("the drawable is a {name}, not a vector"));
    }
    let size = |id, name| match attribute(attributes, Some(id), name) {
        Some(Value::Dimension(size, 0)) => Ok(f64::from(*size)),
        Some(Value::Dimension(size, 1)) => Ok(f64::from(*size) * density),
        other => Err(format!("the vector's {name} is {other:?}")),
    };
    let viewport = |id, name| match attribute(attributes, Some(id), name) {
        Some(Value::Float(size)) if *size > 0.0 => Ok(f64::from(*size)),
        other => Err(format!("the vector's {name} is {other:?}")),
    };
    let (width, height) = (size(WIDTH, "width")?, size(HEIGHT, "height")?);
    if !(width > 0.0 && height > 0.0) {
        return Err(format!("the vector is {width} × {height} pixels"));
    }
    let to_pixels = Affine::scale_non_uniform(
        width / viewport(VIEWPORT_WIDTH, "viewportWidth")?,
        height / viewport(VIEWPORT_HEIGHT, "viewportHeight")?,
    );
    let mut fills: Vec<(BezPath, bool)> = Vec::new();
    let mut elements = 0;
    for event in &events[1..] {
        let Event::Start { name, attributes } = event else { continue };
        match &**name {
            // A group only names its paths; one that moves them is not
            // followed here.
            "group" if attributes.iter().all(|attribute| &*attribute.name == "name") => {}
            "path" => {
                let filled = match attribute(attributes, Some(FILL_COLOR), "fillColor") {
                    Some(Value::Color(color)) => color >> 24 != 0,
                    // A colour resource, which paints.
                    Some(Value::Reference(_)) => true,
                    _ => false,
                };
                let transparent = matches!(attribute(attributes, None, "fillAlpha"), Some(Value::Float(alpha)) if *alpha <= 0.0);
                if !filled || transparent {
                    continue;
                }
                let Some(Value::Text(data)) = attribute(attributes, Some(PATH_DATA), "pathData") else {
                    return Err("a path has no path data".into());
                };
                let path = BezPath::from_svg(data).map_err(|error| format!("a path does not read: {error}"))?;
                elements += path.elements().len();
                if elements > PATH_ELEMENTS {
                    return Err(format!("the drawable holds more than {PATH_ELEMENTS} path elements"));
                }
                let even_odd = matches!(attribute(attributes, None, "fillType"), Some(Value::Integer(1)));
                fills.push((to_pixels * path, even_odd));
            }
            other => return Err(format!("the vector holds a {other} this does not draw")),
        }
    }
    let covered = |point: kurbo::Point| {
        fills.iter().any(|(path, even_odd)| {
            let winding = path.winding(point);
            if *even_odd { winding % 2 != 0 } else { winding != 0 }
        })
    };
    let far = kurbo::Point::new(width, height);
    if covered(far) {
        return Err("the drawable covers its whole box".into());
    }
    let mut points: Vec<(f64, f64)> = Vec::with_capacity(POINTS + 1);
    for index in 0..=POINTS {
        // From the upright edge to the level one.
        let angle = FRAC_PI_2 * index as f64 / POINTS as f64;
        let direction = kurbo::Vec2::new(-angle.cos(), -angle.sin());
        // As far as the box's upright or level edge.
        let reach = (width / angle.cos().max(1e-9)).min(height / angle.sin().max(1e-9));
        let at = |distance: f64| far + direction * distance;
        let distance = if covered(at(reach)) || covered(at(reach * 0.999)) {
            let (mut clear, mut inside) = (0.0, reach);
            for _ in 0..40 {
                let middle = (clear + inside) / 2.0;
                if covered(at(middle)) { inside = middle } else { clear = middle }
            }
            clear
        } else {
            reach
        };
        let point = at(distance);
        let point = (point.x.max(0.0), point.y.max(0.0));
        if points.last() != Some(&point) {
            points.push(point);
        }
    }
    if points.len() < 2 {
        return Err("the drawable leaves no edge".into());
    }
    Ok(points)
}
