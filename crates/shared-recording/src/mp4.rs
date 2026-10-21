//! The recording's video as an MP4 file: the kept frames' H.264 as samples,
//! each lasting until the next frame was drawn, so a still screen holds the
//! picture and plays at the pace it was recorded.
//!
//! A frame of another size starts a sample entry of its own, which the format
//! allows and players follow. The movie box goes before the data, so the file
//! plays while it downloads.

use std::io::Write;

use crate::Error;

/// One coded picture, as the video holds it, and when it shows.
pub(crate) struct Sample {
    /// The picture's Annex B bytes.
    pub data: Vec<u8>,
    pub key: bool,
    /// How long it shows, in microseconds.
    pub duration_us: u64,
    pub width: u32,
    pub height: u32,
}

/// The parameter sets and size one sample entry names.
#[derive(PartialEq)]
struct Entry {
    sps: Vec<u8>,
    pps: Vec<u8>,
    width: u32,
    height: u32,
}

/// What the movie box needs of a sample.
struct Prepared {
    /// How many bytes its slices take, each after its length.
    size: usize,
    entry: usize,
    key: bool,
    duration_us: u64,
}

/// The sample's slices, each after its length, as the movie holds them.
fn slices(data: &[u8]) -> Vec<u8> {
    let mut slices = Vec::with_capacity(data.len());
    for unit in nal_units(data) {
        if matches!(unit.first().map(|header| header & 0x1f), Some(1 | 5)) {
            slices.extend_from_slice(&(unit.len() as u32).to_be_bytes());
            slices.extend_from_slice(unit);
        }
    }
    slices
}

/// The NAL units of Annex B `data`, without their start codes.
fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut at = 0;
    while at + 3 <= data.len() {
        if data[at..at + 3] == [0, 0, 1] {
            starts.push(at + 3);
            at += 3;
        } else {
            at += 1;
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let mut end = starts.get(index + 1).map_or(data.len(), |next| next - 3);
            // A four-byte start code leaves a zero before the next unit.
            while end > *start && data[end - 1] == 0 {
                end -= 1;
            }
            &data[*start..end]
        })
        .collect()
}

fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + payload.len());
    bytes.extend_from_slice(&(8 + payload.len() as u32).to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(payload);
    bytes
}

fn full(kind: &[u8; 4], flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = flags.to_be_bytes().to_vec();
    body.extend_from_slice(payload);
    boxed(kind, &body)
}

fn u32s(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_be_bytes()).collect()
}

const MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];
const TIMESCALE: u32 = 1_000_000;

fn avc1(entry: &Entry) -> Vec<u8> {
    let mut body = vec![0; 6];
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&[0; 16]);
    body.extend_from_slice(&(entry.width as u16).to_be_bytes());
    body.extend_from_slice(&(entry.height as u16).to_be_bytes());
    body.extend_from_slice(&u32s(&[0x0048_0000, 0x0048_0000, 0]));
    body.extend_from_slice(&1u16.to_be_bytes());
    body.extend_from_slice(&[0; 32]);
    body.extend_from_slice(&0x0018u16.to_be_bytes());
    body.extend_from_slice(&(-1i16).to_be_bytes());
    let mut config = vec![1, entry.sps[1], entry.sps[2], entry.sps[3], 0xFF, 0xE1];
    config.extend_from_slice(&(entry.sps.len() as u16).to_be_bytes());
    config.extend_from_slice(&entry.sps);
    config.push(1);
    config.extend_from_slice(&(entry.pps.len() as u16).to_be_bytes());
    config.extend_from_slice(&entry.pps);
    if matches!(entry.sps[1], 100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135) {
        // 4:2:0, 8 bits, no extensions.
        config.extend_from_slice(&[0xFD, 0xF8, 0xF8, 0]);
    }
    body.extend_from_slice(&boxed(b"avcC", &config));
    boxed(b"avc1", &body)
}

fn moov(entries: &[Entry], samples: &[Prepared], data_start: u64) -> Vec<u8> {
    let duration: u64 = samples.iter().map(|sample| sample.duration_us).sum();
    let duration = duration.min(u64::from(u32::MAX)) as u32;
    let mvhd = {
        let mut body = u32s(&[0, 0, TIMESCALE, duration, 0x0001_0000]);
        body.extend_from_slice(&0x0100u16.to_be_bytes());
        body.extend_from_slice(&[0; 10]);
        body.extend_from_slice(&u32s(&MATRIX));
        body.extend_from_slice(&[0; 24]);
        body.extend_from_slice(&2u32.to_be_bytes());
        full(b"mvhd", 0, &body)
    };
    let tkhd = {
        let mut body = u32s(&[0, 0, 1, 0, duration]);
        body.extend_from_slice(&[0; 8]);
        body.extend_from_slice(&[0; 8]);
        body.extend_from_slice(&u32s(&MATRIX));
        body.extend_from_slice(&u32s(&[entries[0].width << 16, entries[0].height << 16]));
        full(b"tkhd", 3, &body)
    };
    let mdhd = {
        let mut body = u32s(&[0, 0, TIMESCALE, duration]);
        body.extend_from_slice(&0x55c4u16.to_be_bytes());
        body.extend_from_slice(&[0; 2]);
        full(b"mdhd", 0, &body)
    };
    let hdlr = {
        let mut body = vec![0; 4];
        body.extend_from_slice(b"vide");
        body.extend_from_slice(&[0; 12]);
        body.extend_from_slice(b"VideoHandler\0");
        full(b"hdlr", 0, &body)
    };
    let vmhd = full(b"vmhd", 1, &[0; 8]);
    let dinf = boxed(b"dinf", &full(b"dref", 0, &[&1u32.to_be_bytes()[..], &full(b"url ", 1, &[])].concat()));
    let stsd = {
        let mut body = (entries.len() as u32).to_be_bytes().to_vec();
        for entry in entries {
            body.extend_from_slice(&avc1(entry));
        }
        full(b"stsd", 0, &body)
    };
    // Durations in runs, the key samples, the chunks (one for each run of
    // samples of one entry) and each sample's size.
    let mut stts: Vec<(u32, u32)> = Vec::new();
    for sample in samples {
        let delta = sample.duration_us.min(u64::from(u32::MAX)) as u32;
        match stts.last_mut() {
            Some((count, last)) if *last == delta => *count += 1,
            _ => stts.push((1, delta)),
        }
    }
    let stts = full(
        b"stts",
        0,
        &[(stts.len() as u32).to_be_bytes().to_vec(), stts.iter().flat_map(|(count, delta)| u32s(&[*count, *delta])).collect()].concat(),
    );
    let keys: Vec<u32> = samples
        .iter()
        .enumerate()
        .filter(|(_, sample)| sample.key)
        .map(|(index, _)| index as u32 + 1)
        .collect();
    let stss = full(b"stss", 0, &[(keys.len() as u32).to_be_bytes().to_vec(), u32s(&keys)].concat());
    let mut chunks: Vec<(u32, u32, u64)> = Vec::new();
    let mut offset = data_start;
    for sample in samples {
        match chunks.last_mut() {
            Some((entry, count, _)) if *entry == sample.entry as u32 + 1 => *count += 1,
            _ => chunks.push((sample.entry as u32 + 1, 1, offset)),
        }
        offset += sample.size as u64;
    }
    let stsc = full(
        b"stsc",
        0,
        &[
            (chunks.len() as u32).to_be_bytes().to_vec(),
            chunks
                .iter()
                .enumerate()
                .flat_map(|(index, (entry, count, _))| u32s(&[index as u32 + 1, *count, *entry]))
                .collect(),
        ]
        .concat(),
    );
    let stsz = full(
        b"stsz",
        0,
        &[
            u32s(&[0, samples.len() as u32]),
            samples.iter().flat_map(|sample| (sample.size as u32).to_be_bytes()).collect(),
        ]
        .concat(),
    );
    let stco = full(
        b"stco",
        0,
        &[
            (chunks.len() as u32).to_be_bytes().to_vec(),
            chunks.iter().flat_map(|(_, _, offset)| (*offset as u32).to_be_bytes()).collect(),
        ]
        .concat(),
    );
    let stbl = boxed(b"stbl", &[stsd, stts, stss, stsc, stsz, stco].concat());
    let minf = boxed(b"minf", &[vmhd, dinf, stbl].concat());
    let mdia = boxed(b"mdia", &[mdhd, hdlr, minf].concat());
    let trak = boxed(b"trak", &[tkhd, mdia].concat());
    boxed(b"moov", &[mvhd, trak].concat())
}

/// Writes the MP4 of `count` samples to `out`; `sample` gives each in turn,
/// and is asked for each twice, once for the movie box that goes first and
/// once for the data, so the video is never held whole.
pub(crate) fn write(
    out: &mut impl Write,
    count: usize,
    mut sample: impl FnMut(usize) -> Result<Sample, Error>,
) -> Result<(), Error> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut prepared = Vec::with_capacity(count);
    for index in 0..count {
        let sample = sample(index)?;
        let (mut sps, mut pps) = (None, None);
        for unit in nal_units(&sample.data) {
            match unit.first().map(|header| header & 0x1f) {
                Some(7) => sps = Some(unit.to_vec()),
                Some(8) => pps = Some(unit.to_vec()),
                _ => {}
            }
        }
        let size = slices(&sample.data).len();
        if size == 0 {
            return Err(Error::Corrupt("a frame of the video holds no picture".into()));
        }
        let entry = match (sps, pps) {
            (Some(sps), Some(pps)) if sps.len() >= 4 => {
                let wanted = Entry {
                    sps,
                    pps,
                    width: sample.width,
                    height: sample.height,
                };
                match entries.last() {
                    Some(last) if *last == wanted => entries.len() - 1,
                    _ => {
                        entries.push(wanted);
                        entries.len() - 1
                    }
                }
            }
            _ if entries.is_empty() => {
                return Err(Error::Corrupt("the video does not start with its parameter sets".into()));
            }
            _ => entries.len() - 1,
        };
        prepared.push(Prepared {
            size,
            entry,
            key: sample.key,
            duration_us: sample.duration_us,
        });
    }
    let ftyp = boxed(b"ftyp", &[&b"isom"[..], &0x200u32.to_be_bytes(), b"isomiso2avc1mp41"].concat());
    let probe = moov(&entries, &prepared, 0);
    let data_start = (ftyp.len() + probe.len() + 8) as u64;
    let movie = moov(&entries, &prepared, data_start);
    let size = prepared.iter().map(|sample| sample.size as u64).sum::<u64>() + 8;
    let io = |error: std::io::Error| Error::Failed(format!("the video was not written: {error}"));
    out.write_all(&ftyp).map_err(io)?;
    out.write_all(&movie).map_err(io)?;
    out.write_all(&(size as u32).to_be_bytes()).map_err(io)?;
    out.write_all(b"mdat").map_err(io)?;
    for index in 0..count {
        out.write_all(&slices(&sample(index)?.data)).map_err(io)?;
    }
    Ok(())
}
