//! Compresses each vendored library with gzip, so the backend answers a
//! browser that accepts gzip with those bytes (`src/page.rs` § `Library`):
//! a widget's sandboxed frame keeps nothing in Chrome's HTTP cache, so every
//! frame that shows a widget fetches its libraries again.

use std::fs;
use std::io::Write as _;
use std::path::Path;

use flate2::Compression;
use flate2::write::GzEncoder;

fn main() {
    let vendor = Path::new("vendor");
    println!("cargo::rerun-if-changed=vendor");
    let out = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    for entry in fs::read_dir(vendor).expect("the vendored libraries") {
        let path = entry.expect("a vendored file").path();
        if path.extension().is_none_or(|extension| extension != "js") {
            continue;
        }
        let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
        encoder
            .write_all(&fs::read(&path).expect("a vendored library reads"))
            .expect("gzip writes to memory");
        let name = path.file_name().expect("a file name").to_string_lossy();
        fs::write(Path::new(&out).join(format!("{name}.gz")), encoder.finish().expect("gzip ends"))
            .expect("the compressed copy writes");
    }
}
