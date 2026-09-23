//! Browser point inspection and screenshot annotation use one observed candidate list.
use super::{
    BrowserError, BrowserTab, Result, element,
    observation::{Observation, References},
    operation::Operation,
    protocol::{Bounds, DEFAULT_NODES, ProbeInput, ProbeResult},
};

impl BrowserTab {
    pub(super) async fn probe(
        &self,
        input: &ProbeInput,
        refs: &mut References,
        operation: &Operation<'_>,
    ) -> Result<ProbeResult> {
        let point = self.coordinates(&input.xy).await?;
        let observation = Observation::capture(&self.page, refs).await?;
        let elements = observation
            .probe_targets(
                point.x,
                point.y,
                input.include_non_interactable == Some(true),
            )
            .await?;
        let mut nodes = observation.describe_elements(&elements, refs, DEFAULT_NODES)?;
        for (node, element) in nodes.iter_mut().zip(&elements) {
            let mut bounds: [f64; 4] = operation.run(element::call(&element.page, element, "function() { const r = this.getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; }", vec![])).await?;
            for (page, backend) in &element.frame_chain {
                let frame = operation
                    .run(element::TargetElement::resolve(page, *backend))
                    .await?;
                let offset = operation.run(element::frame_offset(&frame)).await?;
                bounds[0] += offset[0];
                bounds[1] += offset[1];
            }
            let [x, y, width, height] = bounds;
            node.bounds = Some(Bounds {
                x,
                y,
                width,
                height,
            });
        }
        Ok(ProbeResult {
            matches: nodes,
            viewport: self.viewport(),
            path: None,
            truncated: elements.len() > DEFAULT_NODES,
        })
    }
}

/// Outline browser probe candidates in the captured PNG without changing the page DOM.
pub(super) fn annotate(bytes: Vec<u8>, result: &ProbeResult) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or(BrowserError::ResultTooLarge)?
    ];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        _ => {
            return Err(BrowserError::InvalidResult(
                "Chrome screenshot must be RGB or RGBA".into(),
            ));
        }
    };
    let scale_x = f64::from(info.width) / f64::from(result.viewport.width);
    let scale_y = f64::from(info.height) / f64::from(result.viewport.height);
    for node in &result.matches {
        let Bounds {
            x,
            y,
            width,
            height,
        } = node
            .bounds
            .ok_or_else(|| BrowserError::InvalidResult("probe bounds are missing".into()))?;
        let left = (x * scale_x).max(0.0).min(f64::from(info.width)) as u32;
        let top = (y * scale_y).max(0.0).min(f64::from(info.height)) as u32;
        let right = ((x + width) * scale_x).max(0.0).min(f64::from(info.width)) as u32;
        let bottom = ((y + height) * scale_y)
            .max(0.0)
            .min(f64::from(info.height)) as u32;
        for (rows, columns) in [
            (top..top.saturating_add(2).min(bottom), left..right),
            (bottom.saturating_sub(2).max(top)..bottom, left..right),
            (top..bottom, left..left.saturating_add(2).min(right)),
            (top..bottom, right.saturating_sub(2).max(left)..right),
        ] {
            for row in rows {
                for column in columns.clone() {
                    let index = (row as usize * info.width as usize + column as usize) * channels;
                    pixels[index..index + 3].copy_from_slice(&[255, 0, 80]);
                }
            }
        }
    }
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut output, info.width, info.height);
        encoder.set_color(info.color_type);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?
            .write_image_data(&pixels[..info.buffer_size()])
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
    }
    Ok(output)
}
