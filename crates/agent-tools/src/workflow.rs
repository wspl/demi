//! A project's workflows (`subagents.md` § Workflows of a project): the
//! format of `.demi/workflows/<id>.yaml`, the one parser that reads it, and
//! the help that teaches the format, kept together so they never disagree.

use garde::Validate;
use serde::Deserialize;

/// Where a project keeps its workflows, under the nearest directory from
/// the working directory up to the repository's root that has it.
pub const WORKFLOWS_DIRECTORY: [&str; 2] = [".demi", "workflows"];

/// The extension of a workflow file; the file's name without it is the
/// workflow's id.
pub const WORKFLOW_EXTENSION: &str = ".yaml";

/// The longest workflow file read, in bytes: two texts of the longest
/// instructions and a margin, so a file that holds what the format allows
/// always reads.
pub const WORKFLOW_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// The most characters a workflow's name has.
const NAME_MAX: usize = 60;
/// The most characters of instructions, a workflow's or a profile's.
const INSTRUCTIONS_MAX: usize = 65_536;
/// The most characters of a profile's description.
const DESCRIPTION_MAX: usize = 500;

/// The help of `demi agent workflows`: where workflow files live, each
/// field, and a whole example (`subagents.md` § Workflows of a project).
pub const WORKFLOWS_HELP: &str = r#"A workflow is a group of subagent profiles with instructions on how to split the work between them, kept in the project's repository so a team shares it. The user picks one for a conversation in the composer's Workflow menu; nothing applies until they do. With a workflow picked, its profiles replace the user's for that conversation, and its instructions reach the agent leading the conversation with the project's instructions.

Where: .demi/workflows/<id>.yaml in the nearest directory, from the working directory up to the git repository's root, that has a .demi/workflows/ directory. The file's name without .yaml is the workflow's id.

Fields:
  name          What the menu shows: 1 to 60 characters. Required.
  instructions  For the agent leading the conversation: when to use which profile and how to split the work. Up to 65,536 characters. Optional.
  profiles      The workflow's profiles, a list. Required. Each has:
    id            The --profile value of demi agent spawn: 1 to 40 lowercase letters, digits and hyphens, starting with a letter; not default; unique in the list. Required.
    description   When an agent should use it: one line of 1 to 500 characters, which demi agent profiles list shows. Required.
    instructions  Added to the child's system prompt after Demi's own: what this kind of subagent is for and how it works. Up to 65,536 characters. Optional.
    spawning      true lets its children spawn children of their own. Optional, false by default.
    model         A model id demi providers list lists; the first of the user's provider entries that offers it serves it. Optional: without it, the child runs on its parent's model.
    effort        An effort the model lists, or off for no thinking. Optional, the model's first effort by default; needs model.
    tier          A service tier the model offers, such as fast. Optional, the vendor's default tier by default; needs model.

A workflow sets no limits: how many subagents run at once and how deep they go stay the user's Settings > Subagents. Check a file you wrote or changed with demi agent workflows check <file> before you report it.

Example, .demi/workflows/review.yaml:

name: Review Team
instructions: |
  Have reviewer check every change before you report it.
  Use explore for any search across more than one package.
profiles:
  - id: explore
    description: Use for finding code and call sites; it reports and changes nothing.
    model: claude-haiku-4-5
    effort: low
    instructions: |
      Read and report where things are. Never change a file.
  - id: reviewer
    description: Use to review a finished change before reporting it.
    instructions: |
      Review the diff for bugs and missed cases; change nothing."#;

/// A workflow file, as the format allows it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    /// What the Workflow menu shows.
    #[garde(length(chars, min = 1, max = NAME_MAX), custom(one_line))]
    pub name: String,
    /// The root's instructions while the workflow is chosen.
    #[serde(default)]
    #[garde(inner(length(chars, min = 1, max = INSTRUCTIONS_MAX)))]
    pub instructions: Option<String>,
    #[garde(dive, custom(unique_ids))]
    pub profiles: Vec<WorkflowProfile>,
}

/// One profile of a workflow: the fields of a user's profile, its model by
/// its id alone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct WorkflowProfile {
    /// The `--profile` value.
    #[garde(custom(profile_id))]
    pub id: String,
    #[garde(length(chars, min = 1, max = DESCRIPTION_MAX), custom(one_line))]
    pub description: String,
    /// Added to the child's system prompt after the product's.
    #[serde(default)]
    #[garde(inner(length(chars, min = 1, max = INSTRUCTIONS_MAX)))]
    pub instructions: Option<String>,
    /// Whether its children may spawn children of their own.
    #[serde(default)]
    #[garde(skip)]
    pub spawning: bool,
    /// A model id, looked for in the user's provider entries in their
    /// order; none for the parent's model.
    #[serde(default)]
    #[garde(inner(length(chars, min = 1)))]
    pub model: Option<String>,
    /// An effort the model lists, or `off`; none for the model's first.
    #[serde(default)]
    #[garde(inner(length(chars, min = 1)), custom(needs_model(&self.model)))]
    pub effort: Option<String>,
    /// A tier the model offers; none for the vendor's default.
    #[serde(default)]
    #[garde(inner(length(chars, min = 1)), custom(needs_model(&self.model)))]
    pub tier: Option<String>,
}

/// Why a workflow file does not read: the line and the field that are
/// wrong, with the lines around it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct WorkflowError(String);

impl WorkflowError {
    /// The first line alone, such as `line 3 column 9: … for
    /// \`profiles[0].id\``, for a list that shows one line per file.
    pub fn reason(&self) -> &str {
        let first = self.0.lines().next().unwrap_or_default();
        first.strip_prefix("error: ").unwrap_or(first)
    }
}

/// The workflow `text` describes, or what is wrong with it.
pub fn parse_workflow(text: &str) -> Result<Workflow, WorkflowError> {
    let options = serde_saphyr::options! {
        // `yes` is a string, as a description or an effort may be.
        strict_booleans: true,
    };
    serde_saphyr::from_str_with_options_valid(text, options)
        .map_err(|error| WorkflowError(error.to_string()))
}

/// The workflow id of a file named `name` in the workflows directory: its
/// name without `.yaml`; none for any other file.
pub fn workflow_id(name: &str) -> Option<&str> {
    name.strip_suffix(WORKFLOW_EXTENSION)
        .filter(|id| !id.is_empty() && !id.starts_with('.'))
}

fn one_line(value: &str, _: &()) -> garde::Result {
    if value.contains(['\n', '\r']) {
        return Err(garde::Error::new("is more than one line"));
    }
    Ok(())
}

fn profile_id(value: &str, _: &()) -> garde::Result {
    let mut chars = value.chars();
    let well_formed = chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|char| char.is_ascii_lowercase() || char.is_ascii_digit() || char == '-')
        && value.len() <= 40;
    if !well_formed {
        return Err(garde::Error::new(
            "is not 1 to 40 lowercase letters, digits and hyphens starting with a letter",
        ));
    }
    if value == "default" {
        return Err(garde::Error::new(
            "is default, which names the parent's own profile",
        ));
    }
    Ok(())
}

fn unique_ids(profiles: &[WorkflowProfile], _: &()) -> garde::Result {
    for (index, profile) in profiles.iter().enumerate() {
        if profiles[..index].iter().any(|earlier| earlier.id == profile.id) {
            return Err(garde::Error::new(format!(
                "has the id {} twice",
                profile.id
            )));
        }
    }
    Ok(())
}

fn needs_model(model: &Option<String>) -> impl FnOnce(&Option<String>, &()) -> garde::Result + '_ {
    move |value, _| {
        if value.is_some() && model.is_none() {
            return Err(garde::Error::new("needs model"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The help's own example is a workflow the parser reads whole, so the
    /// help never teaches a file that fails `check`.
    #[test]
    fn the_helps_example_reads_with_every_field() {
        let example = WORKFLOWS_HELP
            .split_once("Example, .demi/workflows/review.yaml:\n\n")
            .expect("the help ends with its example")
            .1;
        let workflow = parse_workflow(example).unwrap();
        assert_eq!(workflow.name, "Review Team");
        assert_eq!(
            workflow.instructions.as_deref(),
            Some("Have reviewer check every change before you report it.\nUse explore for any search across more than one package.\n"),
        );
        assert_eq!(
            workflow.profiles[0],
            WorkflowProfile {
                id: "explore".into(),
                description: "Use for finding code and call sites; it reports and changes nothing.".into(),
                instructions: Some("Read and report where things are. Never change a file.\n".into()),
                spawning: false,
                model: Some("claude-haiku-4-5".into()),
                effort: Some("low".into()),
                tier: None,
            },
        );
        assert_eq!(workflow.profiles[1].id, "reviewer");
        assert_eq!(workflow.profiles[1].model, None);
    }

    /// Each wrong file names the line and the field that are wrong.
    #[test]
    fn a_wrong_file_names_its_line_and_field() {
        let cases = [
            ("name: Review\n", "line 1 column 1: missing field `profiles`"),
            (
                "name: Review\nprofile: []\n",
                "line 2 column 1: unknown field `profile`, expected one of name, instructions, profiles",
            ),
            (
                "name: Review\nprofiles:\n  - id: Explore\n    description: Finds code.\n",
                "line 3 column 9: validation error: is not 1 to 40 lowercase letters, digits and hyphens starting with a letter for `profiles[0].id`",
            ),
            (
                "name: Review\nprofiles:\n  - id: default\n    description: Finds code.\n",
                "line 3 column 9: validation error: is default, which names the parent's own profile for `profiles[0].id`",
            ),
            (
                "name: Review\nprofiles:\n  - id: explore\n    description: Finds code.\n    effort: low\n",
                "line 5 column 13: validation error: needs model for `profiles[0].effort`",
            ),
            (
                "name: Review\nprofiles:\n  - id: explore\n    description: Finds code.\n  - id: explore\n    description: Finds more.\n",
                "line 3 column 3: validation error: has the id explore twice for `profiles`",
            ),
            (
                "name: Review\nprofiles:\n  - id: explore\n    description: |\n      Finds\n      code.\n",
                "line 5 column 7: validation error: is more than one line for `profiles[0].description`",
            ),
            (
                &format!("name: {}\nprofiles: []\n", "n".repeat(61)),
                "line 1 column 7: validation error: length is greater than 60 for `name`",
            ),
            ("name: [\n", "line 1 column 7: expected string scalar"),
        ];
        for (text, reason) in cases {
            let error = parse_workflow(text).unwrap_err();
            assert_eq!(error.reason(), reason, "{text}");
        }
    }
}
