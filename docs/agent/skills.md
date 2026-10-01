# Skills

A skill is a packaged workflow the agent can follow: a directory with a
`SKILL.md` that says when the skill applies and what to do, and whatever
scripts or references that file names. Skills follow the
[Agent Skills](https://agentskills.io) format, so a skill written for another
agent works in Demi unchanged. They come from two places: git repositories a
user adds as sources and turns on skill by skill, and the repository a
conversation works in. The agent sees the skills that are available, reads one
when a task matches it, and runs its scripts with its ordinary shell tools.
Skills are a [plugin](../architecture/plugins.md), `plugin-skills`; nothing
else in Demi knows them.

For example, a user adds the source `vercel-labs/agent-skills` and turns on
`web-design-guidelines`. Their repository `~/app` holds a skill of its own in
`.agents/skills/release/`. In a conversation on their laptop in `~/app`, they
ask the agent to review a page's markup:

1. Before the turn's first request, the model receives a context block with
   the catalog of both skills and the path of each `SKILL.md`: the source's
   skill in its installed directory, the repository's skill where it is.
2. The model sees that `web-design-guidelines` matches and runs
   `cat ~/.demi/plugins/skills/web-design-guidelines-41ab07c2d9e5/SKILL.md`.
3. Before that job starts, host access finds that the laptop does not hold the
   skill's directory yet and installs it
   ([Host directories](../architecture/plugins.md#host-directories)).
4. The model follows the file, which may tell it to run a script from the same
   directory.

## A skill

A skill is a directory that holds a file named `SKILL.md`. The file starts
with YAML front matter, between two `---` lines. Demi reads three of its
fields:

| Field | Rule |
| --- | --- |
| `name` | 1 to 64 characters: lowercase letters, digits and hyphens, neither first nor last a hyphen and no two in a row, and the same as the skill's directory name |
| `description` | 1 to 1,024 characters: what the skill does and when to use it |
| `disable-model-invocation` | `true` keeps the skill out of the catalog |

Validation is lenient, as the format's guide for clients asks:

- A `name` that breaks its rule, or differs from the directory's name, is a
  warning, and the skill loads under the name its front matter gives. A
  missing `name` takes the directory's name.
- A `description` longer than 1,024 characters is a warning, and the skill
  loads.
- A missing or empty `description`, front matter that does not parse as YAML,
  or a `SKILL.md` larger than 256 KiB is not a skill: it is skipped, with its
  path and the reason.

Other front matter fields are ignored. A skill that sets
`disable-model-invocation` is meant to be invoked by the user alone, which
Demi does not offer, so it is never listed to the model.

The skill's files are every regular file under its directory, except the
files of a directory below it that is a skill of its own. Symbolic links and
submodules are not followed. A file keeps its executable bit, so a skill's
scripts run as the repository has them.

## User skills

A source is one git repository, added by one user. It is written as
`owner/repo`, for a repository on GitHub, or as an `https` URL of any git
repository. Only public repositories can be added: Demi sends no credential
when it fetches. Every directory of the repository that holds a `SKILL.md` is
a skill.

Each source is pinned to one commit:

- **Adding** a source fetches the repository's default branch, pins its newest
  commit and lists the skills that commit holds. Every skill starts off.
- **Updating** fetches again and pins the new newest commit. A skill that was
  on stays on if the new commit still has a skill of that name; a skill the
  commit no longer has is gone.
- **Removing** forgets the source and its skills.

A source never changes by itself. Its skills change only when the user adds,
updates, removes or turns on or off.

A fetch is shallow: it reads one commit, without history. It runs on the
blocking pool and stops as soon as it has received 64 MiB. One source has at
most 100 skills, whose files hold at most 16 MiB together. Every user skill
that is on is installed on each Host its user's conversations run jobs on, so
these bounds bound what a Host keeps. A fetch that exceeds one of them fails.

A fetch that fails, because the repository does not answer, does not exist,
is too large or holds no skill, leaves the source as it was and records the
failure, which the page shows until the next fetch of that source succeeds.
A source whose first fetch failed has no commit and no skills, and the user
can update or remove it. One fetch of a source runs at a time; updating a
source that is being fetched changes nothing. A fetch cut off by the backend's
shutdown records nothing, so the source shows what it showed before.

Two user skills that are on never have the same name. Turning on a skill
whose name another user skill that is on already has is refused, and the
refusal names the other skill's source. An update cannot break this: a skill
stays on only under the name it was on with, and a new skill starts off.

### What the plugin keeps

The plugin keeps one [value](../architecture/plugins.md#the-contract) per
source, keyed by the source's id:

| Field | Holds |
| --- | --- |
| `origin` | The repository, as the user wrote it |
| `commit` | The pinned commit; absent until the first fetch succeeds |
| `fetchedAt` | When the pinned commit was fetched |
| `skills` | Each skill's name, description, directory in the repository, files (path, mode and the SHA-256 of its bytes), warnings, whether it sets `disable-model-invocation`, and whether it is on |
| `skipped` | Each `SKILL.md` that is not a skill, with its path and reason |
| `failure` | The last fetch's failure, with its time and message; absent once a fetch succeeds |

The files' bytes are blobs in the user's namespace, which the value names, so
they stay as long as the source does. Whether a fetch is running is not
stored: the plugin's instance holds it in memory.

Whenever the user skills that are on change, the plugin sets its user's
[Host directories](../architecture/plugins.md#host-directories) to one
directory per skill that is on, named after the skill, and marks its part of
the product state as changed. Such a skill's path in the catalog is its
directory on a Host, `~/.demi/plugins/skills/<name>-<short digest>/SKILL.md`,
the same on every Host, so a target switch changes no path: the new Host
receives the directory when its first job needs it.

## Project skills

A repository can carry its own skills, in the directories other agents read
too. For a node working in `~/app/web`, inside the git repository `~/app`, the
plugin looks in:

```text
~/app/web/.agents/skills/    ~/app/web/.claude/skills/
~/app/.agents/skills/        ~/app/.claude/skills/
```

that is, `.agents/skills` and `.claude/skills` in the node's working
directory and in each directory above it up to the root of its git
repository, the nearest directory that holds `.git`. Outside a git
repository, only the working directory is searched. Every directory under
these that holds a `SKILL.md` is a skill, and the search does not descend into
a skill's directory. It reads at most 2,000 directories, to a depth of six
below each skills directory, and finds at most 100 skills.

Project skills are always available: they come with the repository, and
changing the repository is how they change. Their files stay where they are,
and the catalog names the `SKILL.md` at its own path on the Host. Demi does
not check them first: the user chose the working directory, and the agent
runs its code in any case.

The plugin reads them through its port's
[Host files](../architecture/plugins.md#reading-a-conversations-files), which
never wakes a Host:

- It searches at the first request of each input turn of a node, and again at
  the next request of that turn when the Host was not running, until a search
  succeeds. For example, on a stopped Cloud the first request lists no
  project skill; the model's first command wakes the Cloud, and the request
  after it lists them.
- It keeps the result in memory, for the conversation's Host and the node's
  working directory, and lists it while a search cannot run. An edit to the
  repository's skills reaches the catalog at the next input turn.
- A project skill that is not a skill, or has warnings, is logged. The page
  shows only user skills.

When two skills that are available have the same name, one of them is listed:
a project skill over a user skill, as the format's guide asks; of two project
skills, the one in the nearer directory; and in one directory, `.agents/skills`
over `.claude/skills`. The other is logged as shadowed.

## What the model sees

The plugin is a context source
([Prompt text and context](../architecture/plugins.md#prompt-text-and-context)).
Its block holds the catalog of every skill that is available to the node: the
user skills that are on and the project skills it found, without the skills
that set `disable-model-invocation`. The catalog has the format other agents
and the Agent Skills guide use, sorted by name, with every value XML-escaped:

```text
The following skills provide specialized instructions for specific tasks.
When a task matches a skill's description, read its SKILL.md at the listed
location before you start, and resolve relative paths in it against the
skill's directory.

<available_skills>
  <skill>
    <name>release</name>
    <description>Cut a release: bump, tag and publish the changelog.</description>
    <location>/home/me/app/.agents/skills/release/SKILL.md</location>
  </skill>
  <skill>
    <name>web-design-guidelines</name>
    <description>Review UI against Vercel's web interface guidelines.</description>
    <location>~/.demi/plugins/skills/web-design-guidelines-41ab07c2d9e5/SKILL.md</location>
  </skill>
</available_skills>
```

The block holds at most 8,000 characters. When the catalog is longer, every
description is shortened to the same length, cut at a word and ended with
`…`, the longest that fits. When the entries do not fit even without
descriptions, the last entries are left out, and the block ends with the line
`<n> more skills are not listed.` 8,000 characters are about 2,000 tokens,
about 1% of a 200,000-token context window, the budget other agents give their
catalogs.

Before each request of a node, the plugin compares the block it would write
now with the newest of its blocks the model receives, and writes a new one
only when they differ. When no skill is available and the model has been told
of some, the block says `No skills are available now.`; when no skill is
available and the model was never told of one, the plugin writes nothing.

So a subagent receives the catalog before its first request as its parent
did, for its own working directory; a conversation whose user turns a skill on
mid-conversation learns of it before its next request; and after a compaction
the model is told the catalog again.

## The page

The settings section is `@demicodes/plugin-skills`, which composes `web-ui`'s
Skills page ([The page](../architecture/plugins.md#the-page)). It shows user
skills; project skills belong to their repositories.

The plugin's state for the user's pages holds every source, in the order they
were added: its id, its origin, its commit and when it was fetched, whether a
fetch is running, its failure, its skills with their names, descriptions,
warnings, whether each is on, and whether it sets `disable-model-invocation`,
which the page shows as never offered to the agent, and its skipped files.

| Method | Parameters | Result |
| --- | --- | --- |
| `add_source` | `origin` | The new source's id, once the source is recorded; its fetch continues after the call |
| `update_source` | `source` | Nothing; the fetch continues after the call |
| `remove_source` | `source` | Nothing |
| `set_enabled` | `source`, `skill`, `enabled` | Nothing |
| `set_source_enabled` | `source`, `enabled` | Nothing; turns every skill of the source on or off, and refuses as `set_enabled` would |

An origin that is neither `owner/repo` nor an `https` URL, or that names a
source the user has already added, is refused. A call that names a source or
a skill the user does not have is refused. Every change, and the end of every
fetch, sends the new state to each of the user's pages.

## Acceptance

- Adding a source lists its skills, all off, with their warnings, and its
  skipped files with their reasons; a skill whose name breaks its rule loads
  with a warning; a source too large or without a skill shows its failure and
  keeps no skill.
- A user skill turned on reaches the next request of every node of every
  conversation of its user, in the catalog, and a model that reads its listed
  `SKILL.md` on the Host reads the skill's file; a script of the skill keeps
  its executable bit.
- A conversation that runs no job installs nothing on its Host; one that runs
  a job installs each directory once per runner connection, and a directory
  no longer on is removed from the Host at its next installation.
- A repository's `.agents/skills` and `.claude/skills` skills, in the working
  directory and up to the repository's root, are in the catalog at their own
  paths; on a stopped Cloud they appear at the request after the Cloud woke,
  and no search wakes it; a project skill shadows a user skill of the same
  name.
- A skill that sets `disable-model-invocation` is never in the catalog.
- A catalog over 8,000 characters is shortened, then cut with the count of
  the skills left out.
- After a compaction, the next request carries the catalog again.
- An update keeps the skills that are on by name and drops the ones the new
  commit lacks; a failed update leaves the source as it was and shows the
  failure.
- Turning on a second user skill of a taken name is refused with the other
  skill's source.
- A shutdown during a fetch leaves the source as it was before the fetch.
