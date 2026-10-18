//! The words of a media refusal (`images.md` § Accounts and failures,
//! `videos.md` § Accounts and failures), for the plugin's kind.

use demi_host_interface::RpcInvocation;
use demi_plugin_interface::{ImageAspect, MediaRefusal, PluginError, PortFailure, PortRefusal, Unable};
use demi_shared_types::{MediaKind, Timestamp};
use jiff::tz::TimeZone;

/// `failure` as the call fails with it: a refusal of the Providers
/// service in the kind's words, anything else as the port tells it.
pub fn refused(kind: MediaKind, failure: PortFailure, aspect: Option<ImageAspect>, invocation: &RpcInvocation) -> PluginError {
    match failure {
        PortFailure::Refused(PortRefusal::Media { refusal }) => {
            PluginError::failed(refusal_words(kind, &refusal, aspect, invocation))
        }
        failure => failure.into(),
    }
}

/// Where the user turns a kind on, as the words of no entry name it.
fn sign_in(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "sign in to ChatGPT or Grok, or add an OpenAI, xAI or Google key",
        MediaKind::Video => "sign in to Grok, or add an xAI or Google key or a compatible API with a video shape",
        MediaKind::Audio => "add an OpenAI, Google or ElevenLabs key or a compatible API with an audio shape",
    }
}

/// The words of `refusal` for `kind`; a refused shape names `aspect`, the
/// one asked for.
pub fn refusal_words(kind: MediaKind, refusal: &MediaRefusal, aspect: Option<ImageAspect>, invocation: &RpcInvocation) -> String {
    let generation = match kind {
        MediaKind::Image => "image generation",
        MediaKind::Video => "video generation",
        MediaKind::Audio => "audio generation",
    };
    let zone = || TimeZone::get(&invocation.context.locale.time_zone).unwrap_or(TimeZone::UTC);
    match refusal {
        MediaRefusal::NoProvider => {
            format!("no provider makes {}; {}, in Settings › Providers", kind.plural(), sign_in(kind))
        }
        MediaRefusal::UnknownModel { model } => {
            format!("no provider makes \"{model}\"; demi {kind} models lists them")
        }
        MediaRefusal::AllOff { providers } => {
            format!("{generation} is off for {}; turn it on in Settings › Providers", listed(providers))
        }
        MediaRefusal::Off { model, provider } => {
            format!("\"{model}\" is {provider}'s, whose {generation} is off in Settings › Providers")
        }
        MediaRefusal::UsedUp { vendor, resets_at: Some(moment) } => {
            format!("{vendor}'s usage limit is reached, resetting at {}", clock_time(*moment, &zone()))
        }
        MediaRefusal::UsedUp { vendor, resets_at: None } => format!("{vendor}'s usage limit is reached"),
        MediaRefusal::KeyRefused { vendor, manages: true } => {
            format!("{vendor} refused the API key; check it in Settings › Providers")
        }
        MediaRefusal::KeyRefused { vendor, manages: false } => {
            format!("{vendor} refused the API key; ask your administrator to check it")
        }
        MediaRefusal::Refused { vendor, message } => format!("{vendor} refused the request: {message}"),
        MediaRefusal::Shape { vendor, takes } => {
            let asked = aspect.map_or_else(|| "shape asked for".to_owned(), |aspect| format!("{}:{}", aspect.width, aspect.height));
            format!("{vendor} takes no {asked} (it takes {})", takes.join(", "))
        }
        MediaRefusal::Expired { vendor, account: Some(account) } => {
            format!("the {vendor} sign-in for {account} has expired; sign in again in Settings › Providers")
        }
        MediaRefusal::Expired { vendor, account: None } => {
            format!("the {vendor} sign-in has expired; ask your administrator to sign in again")
        }
        MediaRefusal::Unanswered { vendor, cause } => format!("{vendor} did not answer: {cause}"),
        MediaRefusal::NeedsVoice { vendor, model } => {
            format!("{vendor} needs --voice; demi audio voices --model {model} lists them")
        }
        MediaRefusal::Kind { model, asked, makes } => {
            let made: Vec<String> = makes.iter().map(|kind| kind.words().to_owned()).collect();
            format!("\"{model}\" makes no {}; it makes {}", asked.words(), listed(&made))
        }
        MediaRefusal::JobFailed { vendor, message } => format!("{vendor} failed the video: {message}"),
        MediaRefusal::JobExpired { vendor } => format!("the job expired at {vendor} before it ran"),
        MediaRefusal::StillRunning { vendor, job, seconds } => {
            let minutes = seconds.div_ceil(60);
            format!("the job {job} is still running at {vendor} after {minutes} minutes; it runs to its end there")
        }
        MediaRefusal::Cannot { provider, model, why, can } => {
            let why = match why {
                Unable::NotOffered => format!("it does not make \"{model}\""),
                Unable::UsedUp { until: Some(moment) } => {
                    format!("its usage limit is reached, resetting at {}", clock_time(*moment, &zone()))
                }
                Unable::UsedUp { until: None } => "its usage limit is reached".to_owned(),
                Unable::Off => format!("its {generation} is off in Settings › Providers"),
                Unable::NotSignedIn => "it is not signed in".to_owned(),
                Unable::TurnedOff => "it is turned off in Settings › Providers".to_owned(),
            };
            let others = match can.as_slice() {
                [] => "no other provider can make it now".to_owned(),
                [one] => format!("{one} can"),
                several => format!("{} can", listed(several)),
            };
            format!("{provider} cannot make \"{model}\": {why}; {others}")
        }
        MediaRefusal::Failed { message } => message.clone(),
    }
}

/// `names` as a sentence lists them: `ChatGPT`, `ChatGPT and Grok`,
/// `A, B and C`.
pub fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `moment` as a clock shows it in `zone`: `9:00 PM` today, `Oct 18, 9:00
/// PM` on another day.
fn clock_time(moment: Timestamp, zone: &TimeZone) -> String {
    let at = moment.to_jiff().to_zoned(zone.clone());
    let today = jiff::Timestamp::now().to_zoned(zone.clone()).date();
    if at.date() == today {
        at.strftime("%-I:%M %p").to_string()
    } else {
        at.strftime("%b %-d, %-I:%M %p").to_string()
    }
}
