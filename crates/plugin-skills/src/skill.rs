//! A skill's `SKILL.md` (`skills.md` § A skill): the three fields of its
//! front matter Demi reads, checked leniently, as the format's guide for
//! clients asks.

use serde::Deserialize;

/// The largest `SKILL.md` that is a skill.
pub(crate) const SKILL_MD_MAX_BYTES: usize = 256 * 1024;

/// The longest description without a warning, in characters.
const DESCRIPTION_MAX_CHARS: usize = 1024;

/// What a `SKILL.md` says of its skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub(crate) name: String,
    pub(crate) description: String,
    /// The skill is the user's alone to invoke, so it is never listed.
    pub(crate) disable_model_invocation: bool,
    pub(crate) warnings: Vec<String>,
}

#[derive(Deserialize)]
struct FrontMatter {
    name: Option<String>,
    description: Option<String>,
    #[serde(rename = "disable-model-invocation")]
    disable_model_invocation: Option<bool>,
}

/// The skill `text` describes, in the directory named `directory`; the
/// reason it is not a skill otherwise.
pub(crate) fn parse(directory: &str, text: &[u8]) -> Result<Parsed, String> {
    if text.len() > SKILL_MD_MAX_BYTES {
        return Err("SKILL.md is larger than 256 KiB".into());
    }
    let text = std::str::from_utf8(text).map_err(|_| "SKILL.md is not UTF-8".to_owned())?;
    let yaml = front_matter(text).ok_or_else(|| "SKILL.md has no front matter".to_owned())?;
    let fields: FrontMatter = serde_saphyr::from_str(yaml)
        .map_err(|error| format!("the front matter does not parse: {error}"))?;
    let description = fields
        .description
        .filter(|description| !description.trim().is_empty())
        .ok_or_else(|| "the front matter has no description".to_owned())?;
    let mut warnings = Vec::new();
    let name = match fields.name {
        Some(name) => {
            if !valid_name(&name) {
                warnings.push(format!(
                    "The name \"{name}\" is not 1 to 64 lowercase letters, digits and single hyphens"
                ));
            }
            if name != directory {
                warnings.push(format!(
                    "The name \"{name}\" differs from its directory \"{directory}\""
                ));
            }
            name
        }
        None => directory.to_owned(),
    };
    if description.chars().count() > DESCRIPTION_MAX_CHARS {
        warnings.push("The description is longer than 1,024 characters".into());
    }
    Ok(Parsed {
        name,
        description,
        disable_model_invocation: fields.disable_model_invocation.unwrap_or(false),
        warnings,
    })
}

/// The YAML between a first line `---` and the next line `---`.
fn front_matter(text: &str) -> Option<&str> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let start = text.find('\n')? + 1;
    let mut offset = start;
    for line in lines {
        if line.trim_end() == "---" {
            return Some(&text[start..offset]);
        }
        offset += line.len();
    }
    None
}

/// Whether `name` keeps the format's rule: 1 to 64 lowercase letters,
/// digits and hyphens, neither first nor last a hyphen and no two in a row.
pub(crate) fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
}
