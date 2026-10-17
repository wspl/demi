//! What `demi audio` does (`audio.md`): reads the call's text or prompt;
//! asks the Providers service for the audio; wraps raw PCM as WAV; checks
//! that what came back is audio this reads, before a byte reaches stdout;
//! writes it, and says what came back, with a warning for each option the
//! entry's API has no field for, a format the vendor does not make, and a
//! length that differs from the one asked. It also lists a speech model's
//! voices and the audio models. A failure is told in the design's words.

use demi_host_interface::RpcInvocation;
use demi_plugin_interface::{
    AudioAsk, AudioFileFormat, AudioOption, MadeAudio, PluginError, PluginPort, VoiceInfo,
};
use demi_plugin_media::{decode_args, models, prompt, refused, required_model};
use demi_shared_types::audio::{self, AudioFacts, AudioFormat, Pcm};
use demi_shared_types::output::{Cell, Table, note};
use demi_shared_types::{AudioKind, MediaKind};

use crate::commands::{EffectArgs, ModelsArgs, MusicArgs, SpeechArgs, VoicesArgs};

/// How far a length may be from the one asked before it is a different
/// length (`audio.md` § Commands).
const DURATION_SLACK: f64 = 1.0;

/// Runs the leaf `invocation` names.
pub(crate) async fn run(invocation: RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    match invocation.path.last().map(String::as_str) {
        Some("speech") => {
            let args: SpeechArgs = decode_args(&invocation)?;
            let ask = AudioAsk {
                kind: AudioKind::Speech,
                text: prompt(port, args.text).await?,
                voice: args.voice,
                instructions: args.instructions,
                speed: args.speed,
                duration: None,
                instrumental: false,
                seed: None,
                looped: false,
                format: args.format.unwrap_or_default(),
                model: required_model(args.model, MediaKind::Audio)?,
                provider: args.provider,
            };
            make(&invocation, port, ask).await
        }
        Some("music") => {
            let args: MusicArgs = decode_args(&invocation)?;
            let ask = AudioAsk {
                kind: AudioKind::Music,
                text: prompt(port, args.prompt).await?,
                voice: None,
                instructions: None,
                speed: None,
                duration: args.duration,
                instrumental: args.instrumental.unwrap_or(false),
                seed: args.seed,
                looped: false,
                format: args.format.unwrap_or_default().into(),
                model: required_model(args.model, MediaKind::Audio)?,
                provider: args.provider,
            };
            make(&invocation, port, ask).await
        }
        Some("effect") => {
            let args: EffectArgs = decode_args(&invocation)?;
            let ask = AudioAsk {
                kind: AudioKind::Effect,
                text: prompt(port, args.prompt).await?,
                voice: None,
                instructions: None,
                speed: None,
                duration: args.duration,
                instrumental: false,
                seed: None,
                looped: args.looped.unwrap_or(false),
                format: args.format.unwrap_or_default().into(),
                model: required_model(args.model, MediaKind::Audio)?,
                provider: args.provider,
            };
            make(&invocation, port, ask).await
        }
        Some("voices") => {
            let args: VoicesArgs = decode_args(&invocation)?;
            voices(&invocation, port, args).await
        }
        Some("models") => {
            let args: ModelsArgs = decode_args(&invocation)?;
            models(&invocation, port, MediaKind::Audio, args.model).await
        }
        _ => Err(PluginError::failed(format!("no command \"{}\"", invocation.path.join(" ")))),
    }
}

/// Asks for the audio, checks the file and writes it to stdout, and says
/// what came back.
async fn make(invocation: &RpcInvocation, port: &PluginPort, ask: AudioAsk) -> Result<u8, PluginError> {
    let command = format!("demi {}", invocation.path.join(" "));
    let wanted = Wanted { kind: ask.kind, format: ask.format, duration: ask.duration, voice: ask.voice.clone() };
    let made = port
        .make_audio(ask)
        .await
        .map_err(|failure| refused(MediaKind::Audio, failure, None, invocation))?;
    let bytes = made.bytes.clone().into_bytes();
    // Raw PCM is written after the WAV header of its stated shape, which
    // is checked again here, as a file this made; any other answer is a
    // file whose header is read before a byte reaches stdout: a file that
    // does not read writes nothing (`audio.md` § Commands).
    let (header, facts) = match made.pcm {
        Some(shape) => {
            let header = Pcm::new(shape.sample_rate, shape.channels, shape.bits)
                .and_then(|pcm| Ok((pcm.wav_header(bytes.len())?, pcm)))
                .map_err(|bad| PluginError::failed(format!("{} returned PCM this cannot wrap: {bad}", made.vendor)))?;
            let facts = AudioFacts { format: AudioFormat::Wav, seconds: header.1.seconds(bytes.len()) };
            (Some(header.0), facts)
        }
        None => {
            let facts = audio::read(&bytes).map_err(|error| {
                PluginError::failed(format!("{} returned a file that is not audio this reads: {error}", made.vendor))
            })?;
            (None, facts)
        }
    };
    let said = summary(&command, &made, &facts, &wanted);
    let rpc = port.rpc();
    if let Some(header) = header {
        rpc.stdout(header.to_vec()).await?;
    }
    rpc.stdout(bytes).await?;
    rpc.stderr(said).await?;
    Ok(0)
}

/// What the call asked of the result, which it is checked against.
struct Wanted {
    kind: AudioKind,
    format: AudioFileFormat,
    duration: Option<f64>,
    voice: Option<String>,
}

/// `0:04`, `1:30`: a length in minutes and seconds, to the nearest second.
fn length(seconds: f64) -> String {
    let whole = seconds.round().max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// The entry and account a line names, as Settings names the entry.
fn entry(made: &MadeAudio) -> String {
    match &made.account {
        Some(account) => format!("{}, {account}", made.provider),
        None => made.provider.clone(),
    }
}

/// Whether the file is in the format asked for.
fn is(format: AudioFormat, asked: AudioFileFormat) -> bool {
    matches!(
        (format, asked),
        (AudioFormat::Mp3, AudioFileFormat::Mp3)
            | (AudioFormat::Wav, AudioFileFormat::Wav)
            | (AudioFormat::Opus, AudioFileFormat::Opus)
            | (AudioFormat::Flac, AudioFileFormat::Flac)
    )
}

/// The line that says what came back, and a warning for each option the
/// shape had no field for, a format the vendor did not make and a length
/// that differs from the one asked (`audio.md` § Commands).
fn summary(command: &str, made: &MadeAudio, facts: &AudioFacts, wanted: &Wanted) -> String {
    let what = match wanted.kind {
        AudioKind::Speech => "speech",
        AudioKind::Music => "music",
        AudioKind::Effect => "sound effect",
    };
    let mut said = format!(
        "{} {} {what} from {} ({})",
        length(facts.seconds),
        facts.format.name(),
        made.model,
        entry(made)
    );
    if let Some(voice) = &wanted.voice {
        said.push_str(&format!(", voice {voice}"));
    }
    if let Some(cost) = made.cost {
        said.push_str(&format!(", {cost}"));
    }
    said.push('\n');
    let mut warn = |text: String| said.push_str(&format!("{command}: warning: {text}\n"));
    for option in &made.unsent {
        let (flag, hint) = match option {
            AudioOption::Instructions => ("--instructions", ""),
            AudioOption::Speed => ("--speed", ""),
            AudioOption::Duration => ("--duration", "; say the length in the prompt"),
            AudioOption::Instrumental => ("--instrumental", "; say so in the prompt"),
            AudioOption::Seed => ("--seed", ""),
            AudioOption::Loop => ("--loop", ""),
        };
        warn(format!("{} takes no {flag}{hint}", made.shape));
    }
    if !is(facts.format, wanted.format) {
        warn(format!("{} makes no {}; wrote {}", made.shape, wanted.format.name(), facts.format.name()));
    }
    let duration_sent = !made.unsent.contains(&AudioOption::Duration);
    if let Some(duration) = wanted.duration
        && duration_sent
        && (facts.seconds - duration).abs() > DURATION_SLACK
    {
        warn(format!("asked for {}, got {}", length(duration), length(facts.seconds)));
    }
    said
}

/// `demi audio voices --model <id> [<search>]`: the voices as a table, or
/// as JSON with `--json`; an entry that lists none says so on stderr.
async fn voices(invocation: &RpcInvocation, port: &PluginPort, args: VoicesArgs) -> Result<u8, PluginError> {
    let command = format!("demi {}", invocation.path.join(" "));
    let model = required_model(args.model, MediaKind::Audio)?;
    let listing = port
        .audio_voices(model, args.search)
        .await
        .map_err(|failure| refused(MediaKind::Audio, failure, None, invocation))?;
    let rpc = port.rpc();
    let Some(voices) = listing.voices else {
        if invocation.json {
            rpc.stdout("[]\n").await?;
        }
        rpc.stderr(note(&command, &format!("{} lists no voices; --voice is sent as given", listing.provider)))
            .await?;
        return Ok(0);
    };
    let text = if invocation.json {
        let text = serde_json::to_string::<Vec<VoiceInfo>>(&voices).expect("voices encode as JSON");
        format!("{text}\n")
    } else {
        let mut table = Table::new(["id", "name", "kind", "languages", "description"]);
        for voice in &voices {
            let kind = voice.own.map(|own| if own { "own" } else { "built-in" });
            let languages = (!voice.languages.is_empty()).then(|| voice.languages.join(", "));
            table.row::<Cell>([
                voice.id.as_str().into(),
                voice.name.as_deref().into(),
                kind.into(),
                languages.into(),
                voice.description.as_deref().into(),
            ]);
        }
        table.render()
    };
    rpc.stdout(text).await?;
    if voices.is_empty() {
        rpc.stderr(note(&command, &format!("{} lists no voice that matches", listing.provider))).await?;
    }
    Ok(0)
}
