//! `tinyjsc <bundle.mjs> --bin <bare tinyjs> --out <file>`: the packer.
//! Built from the same crate as tinyjs, so the bytecode it writes is the
//! interpreter's own; the bare binary may be any platform's build of this
//! release (`docs/demi-next/tinyjs.md`, "Entry modes").

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = || -> ! {
        eprintln!("usage: tinyjsc <bundle.mjs> --bin <bare tinyjs> --out <file>");
        std::process::exit(2);
    };
    let Some(bundle) = args.get(1) else { usage() };
    if bundle.starts_with("--") {
        usage();
    }
    let mut bin = None;
    let mut out = None;
    let mut rest = args[2..].iter();
    while let Some(flag) = rest.next() {
        match (flag.as_str(), rest.next()) {
            ("--bin", Some(v)) => bin = Some(v.as_str()),
            ("--out", Some(v)) => out = Some(v.as_str()),
            _ => usage(),
        }
    }
    let (Some(bin), Some(out)) = (bin, out) else { usage() };
    if let Err(e) = tinyjs::pack::pack(bundle, bin, out) {
        eprintln!("tinyjsc: {e}");
        std::process::exit(1);
    }
}
