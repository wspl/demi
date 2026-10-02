//! The plugin page registry (`plugin-pages.md` § Registration): for each
//! app that shows plugin pages, the static list of the page packages it
//! depends on, in the order the backend registers their plugins. A page
//! package names its plugin in its `package.json` (`"demi": { "plugin" }`)
//! and default-exports its `PluginPage`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

use super::{Directory, Error};

/// Each app that shows plugin pages, and the directory its registry goes to.
const APPS: [(&str, &str); 2] = [
    ("packages/web", "packages/web/src/plugins/generated"),
    ("packages/web-gallery", "packages/web-gallery/src/generated"),
];

/// The workspace's packages share one scope; a package of it lives in
/// `packages/<name>`.
const SCOPE: &str = "@demicodes/";

/// The part of a package's manifest the registry reads.
#[derive(Deserialize)]
struct PackageManifest {
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    demi: Option<PageDeclaration>,
}

/// What a page package says of itself.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageDeclaration {
    plugin: String,
}

/// Each app's registry directory and its one module, `pages.ts`.
pub fn modules(repository: &Path, header: &str) -> Result<Vec<Directory>, Error> {
    // The plugins the backend registers, in their order of registration.
    let registered: Vec<String> = demi_backend::plugins::builtin()
        .iter()
        .map(|factory| factory.manifest().id.as_str().to_owned())
        .collect();
    APPS.iter()
        .map(|(app, directory)| {
            let pages = pages_of(repository, app, &registered)?;
            Ok((*directory, vec![("pages.ts", module(header, &pages))]))
        })
        .collect()
}

/// The page packages `app` depends on, as (plugin id, package name), in the
/// backend's order.
fn pages_of(
    repository: &Path,
    app: &str,
    registered: &[String],
) -> Result<Vec<(String, String)>, Error> {
    let manifest = read_manifest(&repository.join(app).join("package.json"))?;
    let mut pages = Vec::new();
    for name in manifest.dependencies.keys() {
        let Some(directory) = name.strip_prefix(SCOPE) else {
            continue;
        };
        let path = repository
            .join("packages")
            .join(directory)
            .join("package.json");
        let Some(page) = read_manifest(&path)?.demi else {
            continue;
        };
        if !registered.contains(&page.plugin) {
            return Err(Error::Registry(format!(
                "{name} is the page of the plugin \"{}\", which the backend does not register",
                page.plugin
            )));
        }
        pages.push((page.plugin, name.clone()));
    }
    pages.sort_by_key(|(plugin, _)| registered.iter().position(|id| id == plugin));
    Ok(pages)
}

fn read_manifest(path: &Path) -> Result<PackageManifest, Error> {
    let text = std::fs::read_to_string(path).map_err(|source| Error::Read {
        path: path.to_owned(),
        source,
    })?;
    serde_json::from_str(&text)
        .map_err(|error| Error::Registry(format!("{}: {error}", path.display())))
}

/// The module: one import per page, and the list in order.
fn module(header: &str, pages: &[(String, String)]) -> String {
    let mut source = String::from(header);
    source.push_str("import type { PluginPage } from \"@demicodes/plugin-sdk\"\n");
    for (plugin, package) in pages {
        writeln!(source, "import {} from \"{package}\"", binding(plugin))
            .expect("writing to a string");
    }
    source.push_str(
        "\n/** The plugin pages this app shows, in the order the backend registers their plugins (`plugin-pages.md` § Registration). */\n",
    );
    source.push_str("export const PLUGIN_PAGES: readonly PluginPage[] = [\n");
    for (plugin, _) in pages {
        writeln!(source, "  {},", binding(plugin)).expect("writing to a string");
    }
    source.push_str("]\n");
    source
}

/// A plugin id as a TypeScript name: `file-browser` is `fileBrowserPage`.
fn binding(plugin: &str) -> String {
    let mut name = String::new();
    let mut upper = false;
    for character in plugin.chars() {
        if character == '-' {
            upper = true;
        } else if upper {
            name.extend(character.to_uppercase());
            upper = false;
        } else {
            name.push(character);
        }
    }
    name.push_str("Page");
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cost: a temporary directory of four small files; well under a second.

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// An app that depends on a utility package and on the pages of `plugins`.
    fn workspace(plugins: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let mut dependencies = vec![r#""@demicodes/utils": "workspace:^""#.to_owned()];
        write(
            root.path(),
            "packages/utils/package.json",
            r#"{ "name": "@demicodes/utils" }"#,
        );
        for plugin in plugins {
            dependencies.push(format!(r#""@demicodes/plugin-{plugin}": "workspace:^""#));
            write(
                root.path(),
                &format!("packages/plugin-{plugin}/package.json"),
                &format!(
                    r#"{{ "name": "@demicodes/plugin-{plugin}", "demi": {{ "plugin": "{plugin}" }} }}"#
                ),
            );
        }
        write(
            root.path(),
            "packages/app/package.json",
            &format!(r#"{{ "dependencies": {{ {} }} }}"#, dependencies.join(", ")),
        );
        root
    }

    #[test]
    fn an_app_shows_the_pages_it_depends_on_in_the_backends_order() {
        let root = workspace(&["skills", "browser"]);
        let registered = [
            "browser".to_owned(),
            "expose".to_owned(),
            "skills".to_owned(),
        ];
        let pages = pages_of(root.path(), "packages/app", &registered).unwrap();
        assert_eq!(
            pages,
            [
                ("browser".to_owned(), "@demicodes/plugin-browser".to_owned()),
                ("skills".to_owned(), "@demicodes/plugin-skills".to_owned()),
            ]
        );
        let module = module("", &pages);
        assert!(module.contains("import browserPage from \"@demicodes/plugin-browser\""));
        assert!(module.contains("[\n  browserPage,\n  skillsPage,\n]"));
    }

    #[test]
    fn a_page_of_a_plugin_the_backend_does_not_register_fails_generation() {
        let root = workspace(&["browser", "nope"]);
        let error = pages_of(root.path(), "packages/app", &["browser".to_owned()]).unwrap_err();
        assert!(error.to_string().contains("\"nope\""), "{error}");
    }

    #[test]
    fn a_hyphenated_plugin_id_names_a_camel_case_binding() {
        assert_eq!(binding("file-browser"), "fileBrowserPage");
    }
}
