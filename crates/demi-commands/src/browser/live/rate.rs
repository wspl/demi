//! Fits a viewer's picture to its path (`browser-live-view.md` § Delivery).
//! End-to-end acknowledgement delay is the main signal: queueing anywhere on
//! the path shows as a round trip above the smallest one seen. Under
//! congestion the bit rate falls first, then the frame rate, then the
//! resolution; each comes back once the path has room again.

/// Bits per pixel per frame for sharp scrolling text, calibrated in the Tab
/// Lab's scrolling-text comparison.
const SHARP_BITS: f64 = 0.6;
/// Below this the picture is mush in motion: trade frames, then pixels.
const LOW_BITS: f64 = 0.05;
/// A lowered frame rate or resolution returns once the budget covers it at
/// this many bits, well above `LOW_BITS`, so the two never alternate.
const RECOVERED_BITS: f64 = 0.15;
const MIN_BITRATE: f64 = 100_000.0;
pub(super) const FRAME_RATES: [u32; 3] = [60, 30, 15];
pub(super) const SCALES: [f64; 3] = [1.0, 0.75, 0.5];

/// What a viewer's delivery looked like over the last interval.
#[derive(Debug, Clone, Default)]
pub(super) struct Sample {
    /// Milliseconds on the viewer's clock.
    pub now: f64,
    /// Bytes waiting for the stream's writer.
    pub buffered_bytes: usize,
    /// Age in milliseconds of the oldest frame the page has not acknowledged.
    pub ack_age: f64,
    /// The latest acknowledgement's round trip, or 0 without frames.
    pub round_trip: f64,
    pub decode_queue: u64,
    /// Frames sent in the interval.
    pub active_frames: u32,
    /// Frames dropped for the viewer in the interval.
    pub congested: bool,
    /// Bits per second the encoder produced in the interval.
    pub encoded_bitrate: f64,
    /// Bits per second the page acknowledged in the interval.
    pub delivered_bitrate: f64,
}

/// The budget a sharp picture of `pixels` device pixels needs at `fps`.
pub(super) fn initial_bitrate(pixels: u64, fps: u32) -> u32 {
    (pixels as f64 * f64::from(fps) * SHARP_BITS)
        .max(MIN_BITRATE)
        .round() as u32
}

pub(super) struct Rate {
    minimum_round_trip: f64,
    previous_round_trip: f64,
    backoff_until: f64,
    pixels: f64,
    bitrate: f64,
    rate: usize,
    scale: usize,
    /// Frames that may be in flight to the page.
    pub window: u32,
}

impl Rate {
    /// A picture of `pixels` device pixels at full resolution.
    pub fn new(pixels: u64) -> Self {
        let mut rate = Self {
            minimum_round_trip: f64::INFINITY,
            previous_round_trip: 0.0,
            backoff_until: 0.0,
            pixels: 0.0,
            bitrate: 0.0,
            rate: 0,
            scale: 0,
            window: 4,
        };
        rate.resize(pixels);
        rate
    }

    /// The picture's size changed. Before any congestion the budget follows
    /// it; after, the path's measured capacity stays the budget.
    pub fn resize(&mut self, pixels: u64) {
        self.pixels = pixels as f64;
        if self.backoff_until == 0.0 {
            self.bitrate = f64::from(initial_bitrate(pixels, self.fps()));
        }
    }

    pub fn bitrate(&self) -> u32 {
        self.bitrate.round() as u32
    }

    pub fn fps(&self) -> u32 {
        FRAME_RATES[self.rate]
    }

    /// The fraction of the picture's width and height to capture.
    pub fn scale(&self) -> f64 {
        SCALES[self.scale]
    }

    pub fn acknowledged(&mut self, round_trip: f64) {
        self.minimum_round_trip = self.minimum_round_trip.min(round_trip);
    }

    fn bits(&self, fps: u32, scale: f64) -> f64 {
        self.bitrate / (self.pixels * scale * scale * f64::from(fps)).max(1.0)
    }

    pub fn update(&mut self, sample: &Sample) {
        let delayed = self.minimum_round_trip.is_finite()
            && sample.round_trip > self.minimum_round_trip + 150.0
            && sample.round_trip > self.previous_round_trip + 25.0;
        let previous = self.previous_round_trip;
        if sample.round_trip > 0.0 {
            self.previous_round_trip = sample.round_trip;
        }
        let ack_limit = (self.previous_round_trip + 250.0).max(500.0);
        let pressure = sample.congested
            || sample.buffered_bytes > 256 * 1024
            || sample.ack_age > ack_limit
            || delayed
            || sample.decode_queue > 3;
        let fps = f64::from(self.fps());
        // Per-frame demand, so intermittent scrolling can recover quality too.
        let demand = if sample.active_frames > 0 {
            sample.encoded_bitrate * fps / f64::from(sample.active_frames)
        } else {
            0.0
        };
        if pressure {
            if sample.now >= self.backoff_until {
                let delivered = if sample.delivered_bitrate > 0.0 && demand >= self.bitrate * 0.65 {
                    sample.delivered_bitrate * 0.85
                } else {
                    f64::INFINITY
                };
                self.bitrate = (self.bitrate * 0.7).min(delivered).max(MIN_BITRATE).round();
                self.backoff_until = sample.now + (self.previous_round_trip * 2.0).max(1000.0);
                // Too few bits for sharp motion: fewer frames, then fewer pixels.
                if self.bits(self.fps(), self.scale()) < LOW_BITS {
                    if self.rate + 1 < FRAME_RATES.len() {
                        self.rate += 1;
                    } else if self.scale + 1 < SCALES.len() {
                        self.scale += 1;
                    }
                }
            }
        } else if sample.now >= self.backoff_until {
            if sample.active_frames >= 10 && demand >= self.bitrate * 0.65 {
                let recovering =
                    self.backoff_until > 0.0 && sample.now < self.backoff_until + 10_000.0;
                self.bitrate = (self.bitrate * if recovering { 1.1 } else { 1.5 }).round();
            }
            // Pixels come back before frames, the reverse of their loss.
            if self.scale > 0 && self.bits(self.fps(), SCALES[self.scale - 1]) >= RECOVERED_BITS {
                self.scale -= 1;
            } else if self.scale == 0
                && self.rate > 0
                && self.bits(FRAME_RATES[self.rate - 1], 1.0) >= RECOVERED_BITS
            {
                self.rate -= 1;
            }
        }
        if !pressure && sample.active_frames > 0 && (sample.round_trip - previous).abs() < 25.0 {
            self.window = (fps * (sample.round_trip + 50.0) / 1000.0)
                .ceil()
                .clamp(4.0, 60.0) as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> Sample {
        Sample {
            now: 0.0,
            buffered_bytes: 0,
            ack_age: 0.0,
            round_trip: 5.0,
            decode_queue: 0,
            active_frames: 30,
            congested: false,
            encoded_bitrate: 6_000_000.0,
            delivered_bitrate: 0.0,
        }
    }

    /// The lab's reference: 500 × 1000 pixels with a 6 Mbps budget.
    fn reference() -> Rate {
        let mut rate = Rate::new(500 * 1000);
        rate.bitrate = 6_000_000.0;
        rate
    }

    #[test]
    fn short_scrolling_bursts_raise_quality_without_warming_up() {
        let mut rate = reference();
        rate.acknowledged(5.0);
        rate.update(&healthy());
        assert_eq!(rate.bitrate(), 9_000_000);
        rate.update(&Sample {
            now: 1000.0,
            active_frames: 0,
            encoded_bitrate: 0.0,
            ..healthy()
        });
        assert_eq!(rate.bitrate(), 9_000_000);
        rate.update(&Sample {
            now: 2000.0,
            encoded_bitrate: 9_000_000.0,
            ..healthy()
        });
        assert_eq!(rate.bitrate(), 13_500_000);
    }

    #[test]
    fn recovery_waits_for_feedback_and_probes_conservatively() {
        let mut rate = reference();
        rate.update(&Sample {
            ack_age: 900.0,
            ..healthy()
        });
        assert_eq!(rate.bitrate(), 4_200_000);
        rate.update(&Sample {
            now: 500.0,
            ..healthy()
        });
        assert_eq!(rate.bitrate(), 4_200_000);
        rate.update(&Sample {
            now: 1000.0,
            ..healthy()
        });
        assert_eq!(rate.bitrate(), 4_620_000);
    }

    #[test]
    fn the_first_budget_scales_with_device_pixels_without_a_ceiling() {
        // 766 × 431 CSS pixels at ratio 2, 60 frames per second.
        let mut rate = Rate::new(1532 * 862);
        assert_eq!(rate.bitrate(), 47_541_024);
        rate.resize(2000 * 1400);
        assert_eq!(rate.bitrate(), 100_800_000);
        rate.update(&Sample {
            congested: true,
            now: 1000.0,
            ..healthy()
        });
        let constrained = rate.bitrate();
        rate.resize(2800 * 1800);
        assert_eq!(rate.bitrate(), constrained);
    }

    #[test]
    fn an_idle_picture_is_no_evidence_of_bandwidth() {
        let mut rate = reference();
        rate.update(&Sample {
            congested: true,
            ..healthy()
        });
        for _ in 0..30 {
            rate.update(&Sample {
                active_frames: 0,
                ..healthy()
            });
        }
        assert_eq!(rate.bitrate(), 4_200_000);
    }

    #[test]
    fn a_long_round_trip_alone_keeps_quality_but_a_stalled_ack_does_not() {
        let mut rate = reference();
        rate.acknowledged(600.0);
        rate.update(&Sample {
            round_trip: 600.0,
            ack_age: 600.0,
            encoded_bitrate: 100_000.0,
            ..healthy()
        });
        assert_eq!(rate.bitrate(), 6_000_000);
        rate.update(&Sample {
            ack_age: 900.0,
            ..healthy()
        });
        assert_eq!(rate.bitrate(), 4_200_000);
        for index in 0..30 {
            rate.update(&Sample {
                now: f64::from(index + 1) * 2000.0,
                decode_queue: 5,
                ..healthy()
            });
        }
        assert_eq!(rate.bitrate(), 100_000);
    }

    #[test]
    fn low_encoder_demand_keeps_a_simple_animation_from_raising_the_budget() {
        let mut rate = reference();
        for _ in 0..300 {
            rate.update(&Sample {
                encoded_bitrate: 100_000.0,
                ..healthy()
            });
        }
        assert_eq!(rate.bitrate(), 6_000_000);
    }

    #[test]
    fn intermittent_frames_do_not_hide_demand_while_scrolling() {
        let mut rate = reference();
        rate.update(&Sample {
            active_frames: 12,
            encoded_bitrate: 1_200_000.0,
            ..healthy()
        });
        assert_eq!(rate.bitrate(), 9_000_000);
    }

    #[test]
    fn congestion_lowers_bits_then_frames_then_pixels_and_recovery_reverses_it() {
        // 2880 × 1800 device pixels: a 1440 × 900 panel at ratio 2.
        let mut rate = Rate::new(2880 * 1800);
        assert_eq!((rate.fps(), rate.scale()), (60, 1.0));
        let mut now = 0.0;
        let mut seen = vec![(rate.fps(), rate.scale())];
        while rate.bitrate() > 100_000 {
            now += 2000.0;
            rate.update(&Sample {
                now,
                congested: true,
                ..healthy()
            });
            if seen.last() != Some(&(rate.fps(), rate.scale())) {
                seen.push((rate.fps(), rate.scale()));
            }
        }
        assert_eq!(
            seen,
            [(60, 1.0), (30, 1.0), (15, 1.0), (15, 0.75), (15, 0.5)]
        );
        // The path clears and the encoder uses what it gets.
        seen.clear();
        while rate.fps() != 60 || rate.scale() != 1.0 {
            now += 11_000.0;
            // A second of frames that use the whole budget.
            let (encoded, frames) = (f64::from(rate.bitrate()), rate.fps());
            rate.update(&Sample {
                now,
                encoded_bitrate: encoded,
                active_frames: frames,
                ..healthy()
            });
            if seen.last() != Some(&(rate.fps(), rate.scale())) {
                seen.push((rate.fps(), rate.scale()));
            }
            assert!(now < 1_000_000.0, "never recovered");
        }
        assert_eq!(
            seen,
            [(15, 0.5), (15, 0.75), (15, 1.0), (30, 1.0), (60, 1.0)]
        );
    }
}
