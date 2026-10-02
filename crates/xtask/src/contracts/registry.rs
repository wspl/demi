//! The plugin page registry (`plugin-pages.md` § Registration): for each
//! app that shows plugin pages, the static list of the page packages it
//! depends on, in the order the backend registers their plugins. A page
//! package is the one its plugin's manifest names, and default-exports its
//! page.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;

use super::pages::pages;
use super::{Directory, Error};

/// Each app that shows plugin pages, and the directory its registry goes to.
const APPS: [(&str, &str); 2] = [
    ("packages/web", "packages/web/src/plugins/generated"),
    ("packages/web-gallery", "packages/web-gallery/src/generated"),
];

/// The part of an app's `package.json` the registry reads.
#[derive(Deserialize)]
struct PackageManifest {
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
}

/// Each app's registry directory and its one module, `pages.ts`.
pub fn modules(repository: &Path, header: &str) -> Result<Vec<Directory>, Error> {
    // Each plugin with a page, as (plugin id, page package), in the
    // backend's order of registration.
    let registered: Vec<(String, String)> = pages()
        .into_iter()
        .map(|(manifest, page)| (manifest.id.as_str().to_owned(), page.package))
        .collect();
    APPS.iter()
        .map(|(app, directory)| {
            let pages = pages_of(repository, app, &registered)?;
            Ok((
                (*directory).to_owned(),
                vec![("pages.ts", module(header, &pages))],
            ))
        })
        .collect()
}

/// The pages of `registered` whose package `app` depends on.
fn pages_of(
    repository: &Path,
    app: &str,
    registered: &[(String, String)],
) -> Result<Vec<(String, String)>, Error> {
    let path = repository.join(app).join("package.json");
    let text = std::fs::read_to_string(&path).map_err(|source| Error::Read {
        path: path.clone(),
        source,
    })?;
    let manifest: PackageManifest = serde_json::from_str(&text)
        .map_err(|error| Error::Registry(format!("{}: {error}", path.display())))?;
    Ok(registered
        .iter()
        .filter(|(_, package)| manifest.dependencies.contains_key(package))
        .cloned()
        .collect())
}

/// The module: one import per page, and the list in order.
fn module(header: &str, pages: &[(String, String)]) -> String {
    let mut source = String::from(header);
    source.push_str("import type { AnyPluginPage } from \"@demicodes/plugin-sdk\"\n");
    for (plugin, package) in pages {
        writeln!(source, "import {} from \"{package}\"", binding(plugin))
            .expect("writing to a string");
    }
    source.push_str(
        "\n/** The plugin pages this app shows, in the order the backend registers their plugins (`plugin-pages.md` § Registration). */\n",
    );
    source.push_str("export const PLUGIN_PAGES: readonly AnyPluginPage[] = [\n");
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

    // Cost: a temporary directory of one small file; well under a second.

    fn pair(plugin: &str) -> (String, String) {
        (plugin.to_owned(), format!("@demicodes/plugin-{plugin}"))
    }

    #[test]
    fn an_app_shows_the_pages_it_depends_on_in_the_backends_order() {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("packages/app");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(
            app.join("package.json"),
            r#"{ "dependencies": { "@demicodes/utils": "workspace:^", "@demicodes/plugin-skills": "workspace:^", "@demicodes/plugin-browser": "workspace:^" } }"#,
        )
        .unwrap();
        let registered = [pair("browser"), pair("expose"), pair("skills")];
        let pages = pages_of(root.path(), "packages/app", &registered).unwrap();
        assert_eq!(pages, [pair("browser"), pair("skills")]);
        let module = module("", &pages);
        assert!(module.contains("import browserPage from \"@demicodes/plugin-browser\""));
        assert!(module.contains("[\n  browserPage,\n  skillsPage,\n]"));
    }

    #[test]
    fn a_hyphenated_plugin_id_names_a_camel_case_binding() {
        assert_eq!(binding("file-browser"), "fileBrowserPage");
    }
}
