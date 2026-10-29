//! The plugin page registry: for each
//! app that shows plugin pages, the static list of the page packages it
//! depends on, in the order the backend registers their plugins. A page
//! package is the one its plugin's manifest names, and default-exports its
//! page.
//!
//! Adding a plugin's page to the product is adding the dependency: no import is
//! written by hand and none is computed. A page package's boundary is its page
//! object, the default export of its entry, which the registry imports because
//! its plugin's manifest names the package, and its source-path exports
//! (`./*`), which only the gallery's specimens import.

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
    let all = pages();
    let registered: Vec<(String, String)> = all
        .iter()
        .map(|(manifest, page)| (manifest.id.as_str().to_owned(), page.package.clone()))
        .collect();
    // The pages that draw reply blocks, which a shared conversation's page
    // shows without the product's others.
    let drawing: Vec<(String, String)> = all
        .iter()
        .filter(|(_, page)| !page.blocks.is_empty())
        .map(|(manifest, page)| (manifest.id.as_str().to_owned(), page.package.clone()))
        .collect();
    APPS.iter()
        .map(|(app, directory)| {
            let pages = pages_of(repository, app, &registered)?;
            let mut files = vec![("pages.ts", module(header, &pages, "PLUGIN_PAGES", "The plugin pages this app shows"))];
            if *app == "packages/web" {
                let blocks = pages_of(repository, app, &drawing)?;
                files.push(("blocks.ts", module(header, &blocks, "BLOCK_PAGES", "The plugin pages that draw blocks in replies, which a shared conversation's page shows")));
            }
            if *app == "packages/web-gallery" {
                files.push(("widgets.ts", widgets(header)));
            }
            Ok(((*directory).to_owned(), files))
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
fn module(header: &str, pages: &[(String, String)], name: &str, what: &str) -> String {
    let mut source = String::from(header);
    source.push_str("import type { AnyPluginPage } from \"@demicodes/plugin-sdk\"\n");
    for (plugin, package) in pages {
        writeln!(source, "import {} from \"{package}\"", binding(plugin))
            .expect("writing to a string");
    }
    writeln!(source, "\n/** {what}, in the order the backend registers their plugins. */").expect("writing to a string");
    writeln!(source, "export const {name}: readonly AnyPluginPage[] = [").expect("writing to a string");
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

/// The gallery's widgets: the guide's pages as a reply draws them, by the IDs
/// its replies name them by (`demi_plugin_widget::specimens`).
fn widgets(header: &str) -> String {
    let mut module = String::from(header);
    module.push_str("import type { FrameSize } from \"@demicodes/plugin-widget/generated/plugin\"\n\n");
    module.push_str("/** The guide's pages as a reply draws them, by the IDs the gallery's replies name them by. */\n");
    module.push_str("export const GALLERY_WIDGETS: Record<string, { frame: FrameSize; name: string; document: string }> = {\n");
    for (id, widget) in demi_plugin_widget::specimens() {
        let _ = writeln!(module, "  {id}: {widget},");
    }
    module.push_str("}\n");
    module
}
