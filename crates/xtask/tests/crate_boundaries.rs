//! The crate boundary check (`crates-and-packages.md` § Boundary checks): the
//! Cargo workspace's members and the first-party dependencies their manifests
//! declare, against the document's Rust crate graph, which the check reads
//! rather than a copy of it.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use serde::Deserialize;

/// The document that holds the graph, relative to the workspace root.
const DOCUMENT: &str = "docs/architecture/crates-and-packages.md";
/// The heading of the section whose `text` block is the Rust crate graph.
const HEADING: &str = "\n### Rust crates\n";

/// What the check reads of `cargo metadata --format-version 1 --no-deps`,
/// which lists the workspace's members only; serde skips the fields it does
/// not name. It is not the `cargo_metadata` crate's type: every version of
/// that crate turns on serde_json's `unbounded_depth` feature, and the one
/// Cargo selection would then rebuild every crate that depends on serde_json,
/// for one test.
#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_root: PathBuf,
}

#[derive(Deserialize)]
struct Package {
    manifest_path: PathBuf,
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    /// A path dependency's directory.
    path: Option<PathBuf>,
    /// Null for a normal dependency.
    kind: Option<DependencyKind>,
}

#[derive(Deserialize)]
enum DependencyKind {
    #[serde(rename = "dev")]
    Development,
    #[serde(rename = "build")]
    Build,
}

/// A member's dependencies on members, each named by its directory.
#[derive(Default)]
struct Dependencies {
    /// Normal and build dependencies: what the member's line names.
    production: BTreeSet<String>,
    /// Dev-dependencies, the member itself among them when its tests turn on
    /// its own `testing` feature.
    development: BTreeSet<String>,
}

struct Workspace {
    /// The graph's lines in the document's order, duplicates kept.
    lines: Vec<(String, BTreeSet<String>)>,
    /// The workspace members by directory name.
    members: BTreeMap<String, Dependencies>,
}

impl Workspace {
    /// The graph as name -> dependencies; of two lines for one name, the last.
    fn graph(&self) -> BTreeMap<&str, &BTreeSet<String>> {
        self.lines
            .iter()
            .map(|(name, dependencies)| (name.as_str(), dependencies))
            .collect()
    }

    /// Whether `from` depends on `to` through the manifests' normal and build
    /// dependencies, directly or through other members.
    fn depends_on(&self, from: &str, to: &str) -> bool {
        let mut pending = vec![from];
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name) {
                continue;
            }
            let Some(member) = self.members.get(name) else {
                continue;
            };
            if member.production.contains(to) {
                return true;
            }
            pending.extend(member.production.iter().map(String::as_str));
        }
        false
    }
}

static WORKSPACE: LazyLock<Workspace> = LazyLock::new(|| {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps", "--manifest-path"])
        .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml"))
        .output()
        .expect("cargo starts");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).expect("cargo metadata's output decodes");
    // A member is named by its directory, and a dependency is first-party
    // when its path is a member's directory.
    let mut directories = BTreeMap::<&Path, String>::new();
    for package in &metadata.packages {
        let directory = package
            .manifest_path
            .parent()
            .expect("a manifest lies in its member's directory");
        let name = directory
            .file_name()
            .and_then(OsStr::to_str)
            .expect("a member's directory has a name");
        assert!(
            !directories.values().any(|known| known == name),
            "two members lie in directories named {name}"
        );
        directories.insert(directory, name.to_owned());
    }
    let mut members = BTreeMap::new();
    for package in &metadata.packages {
        let mut dependencies = Dependencies::default();
        for dependency in &package.dependencies {
            let Some(name) = dependency
                .path
                .as_deref()
                .and_then(|path| directories.get(path))
            else {
                continue;
            };
            let set = match dependency.kind {
                None | Some(DependencyKind::Build) => &mut dependencies.production,
                Some(DependencyKind::Development) => &mut dependencies.development,
            };
            set.insert(name.clone());
        }
        let directory = package.manifest_path.parent().expect("checked above");
        members.insert(directories[directory].clone(), dependencies);
    }
    let document = std::fs::read_to_string(metadata.workspace_root.join(DOCUMENT))
        .expect("the document is readable");
    Workspace {
        lines: graph_lines(&document),
        members,
    }
});

/// The lines of the `text` block in the document's Rust crate graph section,
/// each `name -> dependency, dependency` or `name -> none`.
fn graph_lines(document: &str) -> Vec<(String, BTreeSet<String>)> {
    let start = document
        .find(HEADING)
        .expect("the document has a Rust crate graph section")
        + HEADING.len();
    let section = &document[start..];
    let section = &section[..section.find("\n#").unwrap_or(section.len())];
    let (_, block) = section
        .split_once("```text\n")
        .expect("the section has a text block");
    let (block, _) = block
        .split_once("\n```")
        .expect("the section's text block ends");
    block
        .lines()
        .map(|line| {
            let (name, dependencies) = line
                .split_once(" -> ")
                .unwrap_or_else(|| panic!("not a graph line: {line:?}"));
            let dependencies = match dependencies {
                "none" => BTreeSet::new(),
                list => list.split(", ").map(str::to_owned).collect(),
            };
            (name.to_owned(), dependencies)
        })
        .collect()
}

fn assert_none(violations: Vec<String>) {
    assert!(violations.is_empty(), "\n{}", violations.join("\n"));
}

fn listed(names: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    let names: Vec<_> = names
        .into_iter()
        .map(|name| name.as_ref().to_owned())
        .collect();
    format!("[{}]", names.join(", "))
}

#[test]
fn every_member_has_exactly_one_line_and_every_line_names_members() {
    let workspace = &*WORKSPACE;
    let mut violations = Vec::new();
    let mut named = BTreeSet::new();
    for (name, dependencies) in &workspace.lines {
        if !named.insert(name.as_str()) {
            violations.push(format!("{name} has two lines"));
        }
        if !workspace.members.contains_key(name) {
            violations.push(format!("{name} has a line but is not a workspace member"));
        }
        for dependency in dependencies {
            if !workspace.members.contains_key(dependency) {
                violations.push(format!(
                    "{name}'s line names {dependency}, which is not a workspace member"
                ));
            }
        }
    }
    for name in workspace.members.keys() {
        if !named.contains(name.as_str()) {
            violations.push(format!("{name} is a workspace member without a line"));
        }
    }
    assert_none(violations);
}

#[test]
fn each_members_normal_and_build_dependencies_equal_its_line() {
    let workspace = &*WORKSPACE;
    let graph = workspace.graph();
    let mut violations = Vec::new();
    for (name, dependencies) in &workspace.members {
        // A member without a line fails the check of the lines.
        let Some(line) = graph.get(name.as_str()) else {
            continue;
        };
        let undeclared: Vec<_> = line.difference(&dependencies.production).collect();
        let unlisted: Vec<_> = dependencies.production.difference(line).collect();
        if !undeclared.is_empty() {
            violations.push(format!(
                "{name}'s line names {}, which its manifest does not depend on",
                listed(undeclared)
            ));
        }
        if !unlisted.is_empty() {
            violations.push(format!(
                "{name}'s manifest depends on {}, which its line does not name",
                listed(unlisted)
            ));
        }
    }
    assert_none(violations);
}

#[test]
fn no_member_has_a_dev_dependency_on_a_member_that_depends_on_it() {
    let workspace = &*WORKSPACE;
    let mut violations = Vec::new();
    for (name, dependencies) in &workspace.members {
        // A member's dev-dependency on itself turns on its own `testing`
        // feature for its tests; it depends on nothing above the member.
        for dependency in dependencies.development.iter().filter(|dependency| *dependency != name) {
            if workspace.depends_on(dependency, name) {
                violations.push(format!(
                    "{name} has a dev-dependency on {dependency}, which depends on {name}"
                ));
            }
        }
    }
    assert_none(violations);
}

#[test]
fn the_graph_is_acyclic() {
    let workspace = &*WORKSPACE;
    let graph = workspace.graph();
    // Takes out, round by round, the lines that depend on no line left and
    // the lines no line left depends on; the lines that stay lie on a cycle,
    // or between two. A name without a line fails the check of the lines,
    // not this one.
    let mut remaining: BTreeMap<&str, BTreeSet<&str>> = graph
        .iter()
        .map(|(name, dependencies)| {
            let dependencies = dependencies
                .iter()
                .map(String::as_str)
                .filter(|dependency| graph.contains_key(dependency))
                .collect();
            (*name, dependencies)
        })
        .collect();
    loop {
        let depended_on: BTreeSet<&str> = remaining.values().flatten().copied().collect();
        let loose: Vec<&str> = remaining
            .iter()
            .filter(|(name, dependencies)| {
                dependencies.is_empty() || !depended_on.contains(*name)
            })
            .map(|(name, _)| *name)
            .collect();
        if loose.is_empty() {
            break;
        }
        for name in &loose {
            remaining.remove(name);
        }
        for dependencies in remaining.values_mut() {
            for name in &loose {
                dependencies.remove(name);
            }
        }
    }
    assert!(
        remaining.is_empty(),
        "the graph has a cycle through {}",
        listed(remaining.keys())
    );
}
