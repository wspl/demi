//! The audio plugin through the JSON loopback over an in-memory Demi, which
//! answers its audio and its voices: the file checked before a byte
//! reaches stdout, raw PCM wrapped as WAV, the summary and its warnings,
//! the voices table, and the words a failure is told in. The whole path
//! through the backend is the backend's `audio` scenario's.

mod support;

use demi_plugin_interface::{
    AudioFileFormat, AudioOption, AudioVoices, MadeAudio, MediaRefusal, PcmShape, PortRefusal, VoiceInfo,
};
use demi_shared_types::{AudioKind, Cost, Currency};

use crate::support::{Plugged, local};

/// Two seconds of a 440 Hz tone, 16 kHz mono, with a Xing header.
const SPEECH_MP3: &[u8] = include_bytes!("fixtures/speech.mp3");
/// One second, 8 kHz mono.
const CLIP_FLAC: &[u8] = include_bytes!("fixtures/clip.flac");
/// Three seconds of Opus in Ogg.
const CLIP_OPUS: &[u8] = include_bytes!("fixtures/clip.opus");

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
        cost: None,
    }
}

/// The file goes to stdout as it came, and a line says its length and
/// format, read from the file, with the model, the entry, the voice and
/// the cost; the call's options reach the Providers service.
/// Planted defects this catches: the length or format taken from the
/// format asked rather than the file; the voice or cost left out of the
/// line; an option dropped on its way to the service.
#[test]
fn the_audio_goes_to_stdout_with_a_line_that_says_what_came_back() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| {
            Ok(MadeAudio {
                model: "gpt-4o-mini-tts".into(),
                provider: "OpenAI".into(),
                vendor: "OpenAI".into(),
                account: Some("Work".into()),
                cost: Some(Cost { amount: 0.01, currency: Currency::Usd }),
                ..made(SPEECH_MP3, "OpenAI Speech", Vec::new())
            })
        });
        let line = [
            "audio", "speech", "Welcome aboard.", "--model", "gpt-4o-mini-tts", "--voice", "coral", "--instructions",
            "Cheerful", "--speed", "1.25",
        ];
        let (code, stdout, stderr) = plugged.run(&line).await;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(stdout, SPEECH_MP3);
        assert_eq!(stderr, "0:02 MP3 speech from gpt-4o-mini-tts (OpenAI, Work), voice coral, $0.01\n");
        let ask = plugged.demi.audio_asked.borrow()[0].clone();
        assert_eq!(
            (ask.kind, ask.text.as_str(), ask.voice.as_deref(), ask.instructions.as_deref(), ask.speed, ask.format),
            (AudioKind::Speech, "Welcome aboard.", Some("coral"), Some("Cheerful"), Some(1.25), AudioFileFormat::Mp3)
        );

        plugged.answer(|_| Ok(made(CLIP_FLAC, "OpenAI Speech", Vec::new())));
        let (code, stdout, stderr) =
            plugged.run(&["audio", "speech", "Hi.", "--model", "m", "--format", "flac"]).await;
        assert_eq!((code, stdout.as_slice()), (0, CLIP_FLAC), "{stderr}");
        assert_eq!(stderr, "0:01 FLAC speech from eleven_v3 (ElevenLabs)\n");
    });
}

/// Raw PCM is written as a WAV file of its shape, whose length the line
/// reads from it; asking MP3 of a vendor that makes only WAV is a warning,
/// not a failure.
/// Planted defects this catches: PCM written without a header, which no
/// player opens and the check refuses; the header's rate or size wrong,
/// which the length shows; stereo written as mono; PCM that ends inside a
/// sample written on; the format mismatch told as a failure, which throws
/// away a paid file.
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

/// Each option the shape took no field for is a warning, with where to say
/// it instead; a length off by more than a second from the one sent is a
/// warning, and none is told for a length that was not sent.
/// Planted defects this catches: an unsent option left untold, so the
/// agent believes the piece is instrumental; a length warning for a length
/// the vendor never received; a length mismatch left untold.
#[test]
fn what_the_shape_does_not_take_and_a_different_length_are_warnings() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| {
            Ok(made(SPEECH_MP3, "Lyria", vec![AudioOption::Duration, AudioOption::Instrumental, AudioOption::Seed]))
        });
        let line = ["audio", "music", "A calm piano", "--model", "lyria-3-clip-preview", "--duration", "30", "--instrumental", "--seed", "3"];
        let (code, _, stderr) = plugged.run(&line).await;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(
            stderr,
            "0:02 MP3 music from eleven_v3 (ElevenLabs)\n\
             demi audio music: warning: Lyria takes no --duration; say the length in the prompt\n\
             demi audio music: warning: Lyria takes no --instrumental; say so in the prompt\n\
             demi audio music: warning: Lyria takes no --seed\n"
        );
        let ask = plugged.demi.audio_asked.borrow()[0].clone();
        assert_eq!((ask.kind, ask.duration, ask.instrumental, ask.seed), (AudioKind::Music, Some(30.0), true, Some(3)));

        plugged.answer(|_| Ok(made(CLIP_OPUS, "ElevenLabs", Vec::new())));
        let line = ["audio", "effect", "Door creak", "--model", "m", "--duration", "5", "--loop", "--format", "wav"];
        let (code, _, stderr) = plugged.run(&line).await;
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(
            stderr,
            "0:03 Opus sound effect from eleven_v3 (ElevenLabs)\n\
             demi audio effect: warning: ElevenLabs makes no WAV; wrote Opus\n\
             demi audio effect: warning: asked for 0:05, got 0:03\n"
        );
        assert!(plugged.demi.audio_asked.borrow()[1].looped);
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

/// The voices are a table with whose each is; an entry that lists none, and
/// a search that matches none, say so on stderr; the model and the search
/// reach the service.
/// Planted defects this catches: the account's voices not told from the
/// vendor's; an entry that lists none shown as an empty table without a
/// word, which reads as an account without voices.
#[test]
fn the_voices_are_listed_and_an_entry_that_lists_none_says_so() {
    local(async {
        let plugged = Plugged::new();
        plugged.voices(|model, search| {
            assert_eq!((model, search), ("eleven_v3", Some("george")));
            Ok(AudioVoices {
                model: model.into(),
                provider: "ElevenLabs".into(),
                vendor: "ElevenLabs".into(),
                voices: Some(vec![
                    VoiceInfo {
                        id: "JBFqnCBsd6RMkjVDRZzb".into(),
                        name: Some("George".into()),
                        languages: vec!["en".into(), "fr".into()],
                        description: Some("Warm".into()),
                        own: Some(false),
                    },
                    VoiceInfo { id: "mine".into(), name: Some("George Clone".into()), languages: Vec::new(), description: None, own: Some(true) },
                ]),
            })
        });
        let (code, stdout, stderr) = plugged.run(&["audio", "voices", "george", "--model", "eleven_v3"]).await;
        assert_eq!((code, stderr.as_str()), (0, ""));
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            "ID                    NAME          KIND      LANGUAGES  DESCRIPTION\n\
             JBFqnCBsd6RMkjVDRZzb  George        built-in  en, fr     Warm\n\
             mine                  George Clone  own       -          -\n"
        );

        plugged.voices(|model, _| {
            Ok(AudioVoices { model: model.into(), provider: "Local".into(), vendor: "Local".into(), voices: None })
        });
        let (code, stdout, stderr) = plugged.run(&["audio", "voices", "--model", "qwen3-tts"]).await;
        assert_eq!(
            (code, stdout.len(), stderr.as_str()),
            (0, 0, "demi audio voices: note: Local lists no voices; --voice is sent as given\n")
        );
    });
}

/// ElevenLabs speech without a voice, and a model of another kind, are
/// told in the design's words, naming what to do.
#[test]
fn a_missing_voice_and_a_model_of_another_kind_are_told_in_the_designs_words() {
    local(async {
        let plugged = Plugged::new();
        plugged.answer(|_| {
            Err(PortRefusal::Media { refusal: MediaRefusal::NeedsVoice { vendor: "ElevenLabs".into(), model: "eleven_v3".into() } })
        });
        let (code, _, stderr) = plugged.run(&["audio", "speech", "Hi.", "--model", "eleven_v3"]).await;
        assert_eq!(
            (code, stderr.as_str()),
            (1, "demi audio speech: ElevenLabs needs --voice; demi audio voices --model eleven_v3 lists them\n")
        );
        plugged.answer(|_| {
            Err(PortRefusal::Media {
                refusal: MediaRefusal::Kind { model: "music_v1".into(), asked: AudioKind::Speech, makes: vec![AudioKind::Music] },
            })
        });
        let (code, _, stderr) = plugged.run(&["audio", "speech", "Hi.", "--model", "music_v1"]).await;
        assert_eq!((code, stderr.as_str()), (1, "demi audio speech: \"music_v1\" makes no speech; it makes music\n"));
    });
}
