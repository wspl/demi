//! The pictures a device's encoder makes, as the live view needs them: each one
//! shown as soon as it arrives.
//!
//! A simulator's screen that does not change sends no frames after its key
//! frame. An H.264 decoder that does not know a stream has no reordering holds
//! pictures back until later ones arrive: for an SPS without
//! `bitstream_restriction`, Chromium's waits for as many as the level allows
//! (about 14 at an iPhone's size), and a still screen never paints.
//!
//! VideoToolbox, which says no reordering only by sending none, writes no VUI
//! at all, so each SPS of a key frame gains one that says so. No crate the
//! workspace has writes an SPS, and the ones that read one do not tell where
//! its VUI begins; this is the little of H.264 that takes.

use bytes::Bytes;
use demi_shared_h264::bitstream::{BitReader, BitWriter, SPS, annex_b, escape, kind, unescape, units};

/// The `profile_idc`s whose SPS carries the chroma and bit depth fields.
const HIGH_PROFILES: [u32; 12] = [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134];

/// `frame`, an Annex B access unit, with each SPS saying the stream needs no
/// reordering. A frame without an SPS, or with one that already has a VUI or
/// that this does not read, comes back as it was.
pub(crate) fn without_reordering(frame: Bytes) -> Bytes {
    let units = units(&frame);
    if !units.iter().any(|unit| kind(unit) == Some(SPS)) {
        return frame;
    }
    let rewritten: Vec<Vec<u8>> = units
        .iter()
        .map(|unit| match kind(unit) {
            Some(SPS) => sps_without_reordering(unit).unwrap_or_else(|| unit.to_vec()),
            _ => unit.to_vec(),
        })
        .collect();
    Bytes::from(annex_b(&rewritten))
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
    let vui = reader.position();
    if reader.bits(1)? == 1 {
        return None;
    }

    let mut writer = BitWriter::default();
    writer.copy(&rbsp, 0..vui);
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
