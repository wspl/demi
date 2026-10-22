//! The audio plugin through the JSON loopback over an in-memory Demi, which
//! answers its audio: raw PCM wrapped as WAV, and the file checked before a
//! byte reaches stdout.

mod support;

use demi_plugin_interface::{
    AudioOption, MadeAudio, PcmShape,
};

use crate::support::{Plugged, local};

fn made(bytes: &[u8], shape: &str, unsent: Vec<AudioOption>) -> MadeAudio {
    MadeAudio {
        bytes: bytes.to_vec().into(),
        pcm: None,
        model: "eleven_v3".into(),
        provider: "ElevenLabs".into(),
        vendor: "ElevenLabs".into(),
        account: None,
        shape: shape.into(),
        unsent,
        speed: None,
        cost: None,
    }
}

/// Raw PCM is written as a WAV file of its shape, whose length the line reads
/// from it. Asking MP3 of a vendor that makes only WAV is a warning, not a
/// failure.
///
/// Planted defects this catches:
///
/// - PCM written without a header, which no player opens and the check refuses;
/// - the header's rate or size wrong, which the length shows;
/// - stereo written as mono;
/// - PCM that ends inside a sample written on;
/// - the format mismatch told as a failure, which throws away a paid file.
#[test]
fn raw_pcm_is_wrapped_as_wav() {
    local(async {
        let plugged = Plugged::new();
        // Two seconds at 24 kHz, 16-bit mono.
        let samples = vec![0u8; 96_000];
        let answer = samples.clone();
        plugged.answer(move |_| {
            Ok(MadeAudio {
                pcm: Some(PcmShape { sample_rate: 24_000, channels: 1, bits: 16 }),
                model: "gemini-2.5-flash-preview-tts".into(),
                provider: "Google".into(),
                vendor: "Google".into(),
                ..made(&answer, "Gemini", Vec::new())
            })
        });
        let (code, stdout, stderr) = plugged.run(&["audio", "speech", "Hello.", "--model", "m", "--voice", "Kore"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!((&stdout[..4], &stdout[8..16], stdout.len()), (b"RIFF".as_slice(), b"WAVEfmt ".as_slice(), 96_044));
        assert_eq!(&stdout[44..], samples.as_slice());
        assert_eq!(
            stderr,
            "0:02 WAV speech from gemini-2.5-flash-preview-tts (Google), voice Kore\n\
             demi audio speech: warning: Gemini makes no MP3; wrote WAV\n"
        );

        // Stereo stays stereo: one second of 48 kHz 16-bit stereo.
        plugged.answer(|_| {
            Ok(MadeAudio { pcm: Some(PcmShape { sample_rate: 48_000, channels: 2, bits: 16 }), ..made(&[0u8; 192_000], "Gemini", Vec::new()) })
        });
        let (code, stdout, stderr) = plugged.run(&["audio", "speech", "Hello.", "--model", "m", "--format", "wav"]).await;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!((u16::from_le_bytes([stdout[22], stdout[23]]), u32::from_le_bytes(stdout[24..28].try_into().unwrap())), (2, 48_000));
        assert!(stderr.starts_with("0:01 WAV speech"), "{stderr}");

        // PCM that ends inside a sample is not wrapped, and nothing is written.
        plugged.answer(|_| {
            Ok(MadeAudio { pcm: Some(PcmShape { sample_rate: 48_000, channels: 2, bits: 16 }), ..made(&[0u8; 6], "Gemini", Vec::new()) })
        });
        let (code, stdout, stderr) = plugged.run(&["audio", "speech", "Hello.", "--model", "m"]).await;
        assert_eq!((code, stdout.len()), (1, 0));
        assert_eq!(
            stderr,
            "demi audio speech: ElevenLabs returned PCM this cannot wrap: its PCM of 6 bytes does not end on a whole sample of 4 bytes\n"
        );
    });
}

/// A file that is not audio fails the call and writes nothing.
/// Planted defect this catches: the bytes written before the check, which
/// leaves a broken file behind `> voice.mp3`.
#[test]
fn a_file_that_is_not_audio_fails_and_writes_nothing() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| Ok(made(b"<html>Access denied</html>", "ElevenLabs", Vec::new())));
        let (code, stdout, stderr) = plugged.run(&["audio", "speech", "Hi.", "--model", "m", "--voice", "v"]).await;
        assert_eq!((code, stdout.len()), (1, 0));
        assert!(
            stderr.starts_with("demi audio speech: ElevenLabs returned a file that is not audio this reads: the file is no MP3, WAV, FLAC or Ogg audio"),
            "{stderr}"
        );
    });
}
