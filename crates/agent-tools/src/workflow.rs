//! A project's workflows: the format of `.demi/workflows/<id>.yaml`, the one
//! parser that reads it, and the help that teaches the format.
//!
//! They are kept together so they never disagree. The help is the only place
//! the format is written, since no skill or prompt teaches it: a model asked to
//! write a workflow reads `demi agent workflows --help` and checks its file
//! with `demi agent workflows check`, which uses this parser.
//!
//! A workflow's name is a menu item's in the composer's Workflow menu, so it
//! names the way of working and never the task at hand. `check` warns of one
//! that is not, and the workflow works whatever its name.

use demi_shared_types::ContextChoice;
use garde::Validate;
use serde::Deserialize;

/// Where a project keeps its workflows, under the nearest directory from
/// the working directory up to the repository's root that has it.
pub const WORKFLOWS_DIRECTORY: [&str; 2] = [".demi", "workflows"];

/// The extension of a workflow file; the file's name without it is the
/// workflow's id.
pub const WORKFLOW_EXTENSION: &str = ".yaml";

/// The longest workflow file read, in bytes: a file is read whole into memory
/// from a Host, whose files have no bound of their own.
pub const WORKFLOW_FILE_MAX_BYTES: u64 = 1024 * 1024;

/// What a menu item's name does not hold: colons, plus signs and brackets,
/// ASCII and full-width, which mark a task's title rather than the name of
/// a way of working; `check` warns of them.
const MENU_NAME_MARKS: &[char] = &[
    ':', '：', '+', '＋', '(', ')', '[', ']', '{', '}', '<', '>', '（', '）', '［', '］',
    '｛', '｝', '＜', '＞', '【', '】', '「', '」', '『', '』', '《', '》', '〈', '〉', '〔', '〕',
];

/// The help of `demi agent workflows`: where workflow files live, each
/// field, and a whole example (`crates/backend-user-shard/src/conversation/workflows.rs`).
pub const WORKFLOWS_HELP: &str = r#"A workflow is a group of subagent profiles with instructions on how to split the work between them, kept in the project's repository so a team shares it. The user picks one for a conversation in the composer's Workflow menu, or lets an agent pick one with demi agent workflows use; nothing applies until one is picked. With a workflow picked, its profiles replace the user's for that conversation, and its instructions reach the agent leading the conversation with the project's instructions.

Where: .demi/workflows/<id>.yaml in the nearest directory, from the working directory up to the git repository's root, that has a .demi/workflows/ directory. The file's name without .yaml is the workflow's id.

Fields:
  name          What the Workflow menu shows, so name it as a menu item: the way of working in a few words, never the task at hand, in English Title Case or as the user's language writes a menu item. With no colon, plus sign or brackets; check warns of a name that has. Good: Review Team, Parallel Development, 并行开发. Bad: Payments refactor: lead + 2 workers, Review (strict), 支付重构：主管 + 两个 worker. Required.
  instructions  For the agent leading the conversation: when to use which profile and how to split the work. Optional.
  subagent_instructions  For every subagent of every profile, added to its system prompt after Demi's own and before its profile's instructions: what all of them share, such as when to message the lead. Optional.
  profiles      The workflow's profiles, a list. Required. Each has:
    id            The --profile value of demi agent spawn: 1 to 40 lowercase letters, digits and hyphens, starting with a letter; not default; unique in the list. Required.
    description   When an agent should use it: one line, which demi agent profiles list shows. Required.
    instructions  Added to the child's system prompt after Demi's own: what this kind of subagent is for and how it works. Optional.
    spawning      true lets its children spawn children of their own. Optional, false by default.
    model         A model id of an entry demi providers show shows; the first of the user's provider entries that offers it serves it. Optional: without it, the child runs on its parent's model.
    effort        An effort the model lists, or off for no thinking. Optional, the model's first effort by default; needs model.
    tier          A service tier the model offers, such as fast. Optional, the vendor's default tier by default; needs model.
    context       The context window its children use: full, 500K, 300K or 200K, as the model offers them, or default for the user's context limit on the model. Optional, default by default; any but default needs model.

A workflow sets no limits: how many subagents run at once and how deep they go stay the user's Settings > Subagents. Check a file you wrote or changed with demi agent workflows check <file> before you report it.

Example, .demi/workflows/review.yaml:

name: Review Team
instructions: |
  Have reviewer check every change before you report it.
  Use explore for any search across more than one package.
profiles:
  - id: explore
    description: Use for finding code and call sites; it reports and changes nothing.
    model: claude-sonnet-5-5
    effort: low
    context: 300K
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
    #[garde(length(chars, min = 1), custom(one_line))]
    pub name: String,
    /// The root's instructions while the workflow is chosen.
    #[serde(default)]
    #[garde(inner(length(chars, min = 1)))]
    pub instructions: Option<String>,
    /// What every child of every profile is told, before its profile's
    /// instructions.
    #[serde(default)]
    #[garde(inner(length(chars, min = 1)))]
    pub subagent_instructions: Option<String>,
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
    #[garde(length(chars, min = 1), custom(one_line))]
    pub description: String,
    /// Added to the child's system prompt after the product's.
    #[serde(default)]
    #[garde(inner(length(chars, min = 1)))]
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
    /// The Context its children use; none, as `default`, for the user's
    /// limit on the model.
    #[serde(default)]
    #[garde(custom(context_needs_model(&self.model)))]
    pub context: Option<ContextChoice>,
}

impl Workflow {
    /// Why the name does not read as a menu item, for `check` to warn of:
    /// holding a colon, a plus sign or a bracket. The workflow works whatever
    /// its name.
    pub fn name_warning(&self) -> Option<String> {
        let mut faults = Vec::new();
        if self.name.contains(MENU_NAME_MARKS) {
            faults.push("has a colon, a plus sign or brackets".to_owned());
        }
        if faults.is_empty() {
            return None;
        }
        Some(format!(
            "the name {} {}; the Workflow menu shows it as a menu item, the way of working in a few words, such as Review Team or Parallel Development.",
            self.name,
            faults.join(" and "),
        ))
    }
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

/// The same rule as a user profile's name (`crates/backend-accounts/src/subagents.rs`),
/// whose limit of 40 characters that crate owns.
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

fn needs_model<T>(model: &Option<String>) -> impl FnOnce(&Option<T>, &()) -> garde::Result + '_ {
    move |value, _| {
        if value.is_some() && model.is_none() {
            return Err(garde::Error::new("needs model"));
        }
        Ok(())
    }
}

/// A Context of its own needs a model of its own; `default`, the user's
/// limit, needs none.
fn context_needs_model(
    model: &Option<String>,
) -> impl FnOnce(&Option<ContextChoice>, &()) -> garde::Result + '_ {
    move |value, context| {
        let own = value.filter(|choice| *choice != ContextChoice::Default);
        needs_model(model)(&own, context)
    }
}
