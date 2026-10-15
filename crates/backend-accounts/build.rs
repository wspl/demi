// `rust_i18n::i18n!` reads the catalogues under `locales/` as it expands,
// which Cargo does not track: a changed catalogue rebuilds the crate.
fn main() {
    println!("cargo::rerun-if-changed=locales");
}
