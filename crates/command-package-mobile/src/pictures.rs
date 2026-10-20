//! The pictures a device's encoder makes, as the live view needs them:
//! each one shown as soon as it arrives.
//!
//! A simulator's screen that does not change sends no frames after its key
//! frame. An H.264 decoder that does not know a stream has no reordering
//! holds pictures back until later ones arrive: for an SPS without
//! `bitstream_restriction`, Chromium's waits for as many as the level allows
//! (about 14 at an iPhone's size), and a still screen never paints.
//! VideoToolbox, which says no reordering only by sending none, writes no
//! VUI at all, so each SPS of a key frame gains one that says so. No crate
//! the workspace has writes an SPS, and the ones that read one do not tell
//! where its VUI begins; this is the little of H.264 that takes.

use bytes::{BufMut, Bytes, BytesMut};

/// The NAL unit type of a sequence parameter set.
const SPS: u8 = 7;
/// The `profile_idc`s whose SPS carries the chroma and bit depth fields.
const HIGH_PROFILES: [u32; 12] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134];

/// `frame`, an Annex B access unit, with each SPS saying the stream needs no
/// reordering. A frame without an SPS, or with one that already has a VUI or
/// that this does not read, comes back as it was.
pub(crate) fn without_reordering(frame: Bytes) -> Bytes {
    let units = units(&frame);
    if !units.iter().any(|unit| unit.first().is_some_and(|header| header & 0x1f == SPS)) {
        return frame;
    }
    let mut rewritten = BytesMut::with_capacity(frame.len() + 8);
    for unit in units {
        rewritten.put_slice(&[0, 0, 0, 1]);
        match unit.first() {
            Some(header) if header & 0x1f == SPS => match sps_without_reordering(unit) {
                Some(sps) => rewritten.put_slice(&sps),
                None => rewritten.put_slice(unit),
            },
            _ => rewritten.put_slice(unit),
        }
    }
    rewritten.freeze()
}

/// The NAL units of an Annex B stream, without their start codes.
fn units(stream: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut at = 0;
    while at + 3 <= stream.len() {
        if stream[at] == 0 && stream[at + 1] == 0 && stream[at + 2] == 1 {
            starts.push((at, at + 3));
            at += 3;
        } else {
            at += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (index, &(_, begin)) in starts.iter().enumerate() {
        let mut end = starts.get(index + 1).map_or(stream.len(), |&(next, _)| next);
        // A four-byte start code's leading zero, and trailing zeros, are no unit's.
        while end > begin && stream[end - 1] == 0 {
            end -= 1;
        }
        units.push(&stream[begin..end]);
    }
    units
}

/// The SPS `unit` with a VUI holding only `bitstream_restriction`: no
/// reordering, and a decoded picture buffer of its reference frames.
fn sps_without_reordering(unit: &[u8]) -> Option<Vec<u8>> {
    let (&header, payload) = unit.split_first()?;
    let rbsp = unescape(payload);
    let mut reader = BitReader::new(&rbsp);
    let profile = reader.bits(8)?;
    reader.bits(16)?; // constraint flags, level_idc
    reader.ue()?; // seq_parameter_set_id
    if HIGH_PROFILES.contains(&profile) {
        if reader.ue()? == 3 {
            reader.bits(1)?; // separate_colour_plane_flag
        }
        reader.ue()?; // bit_depth_luma_minus8
        reader.ue()?; // bit_depth_chroma_minus8
        reader.bits(1)?; // qpprime_y_zero_transform_bypass_flag
        if reader.bits(1)? == 1 {
            // Scaling matrices, which no device encoder here writes.
            return None;
        }
    }
    reader.ue()?; // log2_max_frame_num_minus4
    match reader.ue()? {
        0 => {
            reader.ue()?; // log2_max_pic_order_cnt_lsb_minus4
        }
        1 => {
            reader.bits(1)?; // delta_pic_order_always_zero_flag
            reader.se()?; // offset_for_non_ref_pic
            reader.se()?; // offset_for_top_to_bottom_field
            for _ in 0..reader.ue()? {
                reader.se()?; // offset_for_ref_frame
            }
        }
        _ => {}
    }
    let references = reader.ue()?; // max_num_ref_frames
    reader.bits(1)?; // gaps_in_frame_num_value_allowed_flag
    reader.ue()?; // pic_width_in_mbs_minus1
    reader.ue()?; // pic_height_in_map_units_minus1
    if reader.bits(1)? == 0 {
        reader.bits(1)?; // mb_adaptive_frame_field_flag
    }
    reader.bits(1)?; // direct_8x8_inference_flag
    if reader.bits(1)? == 1 {
        for _ in 0..4 {
            reader.ue()?; // frame_crop_*_offset
        }
    }
    let vui = reader.position;
    if reader.bits(1)? == 1 {
        return None;
    }

    let mut writer = BitWriter::default();
    for index in 0..vui {
        writer.bit(BitReader::new(&rbsp).at(index)?);
    }
    writer.bit(1); // vui_parameters_present_flag
    // aspect_ratio_info, overscan_info, video_signal_type, chroma_loc_info,
    // timing_info, nal_hrd, vcl_hrd, pic_struct: none.
    for _ in 0..8 {
        writer.bit(0);
    }
    writer.bit(1); // bitstream_restriction_flag
    writer.bit(1); // motion_vectors_over_pic_boundaries_flag
    writer.ue(2); // max_bytes_per_pic_denom, its default
    writer.ue(1); // max_bits_per_mb_denom, its default
    writer.ue(16); // log2_max_mv_length_horizontal, its default
    writer.ue(16); // log2_max_mv_length_vertical, its default
    writer.ue(0); // max_num_reorder_frames
    writer.ue(references.max(1)); // max_dec_frame_buffering
    writer.bit(1); // rbsp_stop_one_bit
    let mut sps = vec![header];
    escape(&writer.finish(), &mut sps);
    Some(sps)
}

/// A NAL unit's payload without its emulation prevention bytes.
fn unescape(payload: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(payload.len());
    let mut zeros = 0;
    for &byte in payload {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        rbsp.push(byte);
    }
    rbsp
}

/// `rbsp` with the emulation prevention bytes a NAL unit needs.
fn escape(rbsp: &[u8], unit: &mut Vec<u8>) {
    let mut zeros = 0;
    for &byte in rbsp {
        if zeros >= 2 && byte <= 3 {
            unit.push(3);
            zeros = 0;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        unit.push(byte);
    }
}

struct BitReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn at(&self, position: usize) -> Option<u8> {
        let byte = self.bytes.get(position / 8)?;
        Some((byte >> (7 - position % 8)) & 1)
    }

    fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0;
        for _ in 0..count {
            value = value << 1 | u32::from(self.at(self.position)?);
            self.position += 1;
        }
        Some(value)
    }

    /// An unsigned Exp-Golomb value.
    fn ue(&mut self) -> Option<u32> {
        let mut zeros = 0;
        while self.bits(1)? == 0 {
            zeros += 1;
            if zeros > 31 {
                return None;
            }
        }
        Some((1u32 << zeros) - 1 + self.bits(zeros)?)
    }

    /// A signed Exp-Golomb value, whose value this never needs.
    fn se(&mut self) -> Option<()> {
        self.ue().map(drop)
    }
}

#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    bits: usize,
}

impl BitWriter {
    fn bit(&mut self, bit: u8) {
        if self.bits % 8 == 0 {
            self.bytes.push(0);
        }
        if bit == 1 {
            *self.bytes.last_mut().expect("a byte was pushed") |= 1 << (7 - self.bits % 8);
        }
        self.bits += 1;
    }

    fn ue(&mut self, value: u32) {
        let coded = u64::from(value) + 1;
        let length = 64 - coded.leading_zeros();
        for _ in 1..length {
            self.bit(0);
        }
        for index in (0..length).rev() {
            self.bit(((coded >> index) & 1) as u8);
        }
    }

    /// The bytes, the last one padded with zeros.
    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}
