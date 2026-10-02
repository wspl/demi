use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString},
};
use icu_locale::{Locale, LocaleCanonicalizer};
use icu_time::zone::iana::IanaParserExtended;
use std::{env, fs};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.get(1).map(String::as_str) == Some("verify") {
        let hash = PasswordHash::new(&args[3]).unwrap();
        Argon2::default()
            .verify_password(args[2].as_bytes(), &hash)
            .unwrap();
        println!("Rust verified Go PHC");
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let go_fixture = fs::read_to_string(root.join("go-password.tsv")).unwrap();
    for line in go_fixture.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        let hash = PasswordHash::new(fields[2]).unwrap();
        Argon2::default()
            .verify_password(fields[0].as_bytes(), &hash)
            .unwrap();
    }
    println!("Rust verified Go PHC fixture");
    let mut hashes = String::new();
    for (password, salt) in [
        ("correct horse battery staple", "0123456789abcdef"),
        ("密碼🔑", "fedcba9876543210"),
        ("", "abcdefghijklmnop"),
    ] {
        let salt = SaltString::encode_b64(salt.as_bytes()).unwrap();
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .unwrap();
        hashes.push_str(&format!("{password}\t{salt}\t{hash}\n"));
    }
    fs::write(root.join("passwords.tsv"), hashes).unwrap();
    let parser = IanaParserExtended::new();
    let mut zones: Vec<_> = parser
        .iter_all()
        .filter(|z| !z.time_zone.is_unknown())
        .map(|z| z.normalized)
        .collect();
    zones.sort();
    zones.dedup();
    fs::write(
        root.parent().unwrap().join("iana_names.txt"),
        zones.join("\n") + "\n",
    )
    .unwrap();
    let canonicalizer = LocaleCanonicalizer::new_extended();
    let mut locales = String::new();
    for tag in [
        "zh-cn",
        "EN",
        "zh-CN",
        "iw",
        "in",
        "ji",
        "sh",
        "mo",
        "en-us",
        "sr-latn-rs",
        "und",
        "abc",
        "zz",
        "en-u-ca-gregory",
        "en_US",
        "x-private",
        "en--US",
        "en-a-foo-a-bar",
        "i-klingon",
        "de-DE-1901",
        "en-u-nu-latn-ca-gregory",
        "cmn",
        "en-Latn",
        "en-Zzzz",
        "en-ZZ",
        "en-XX",
        "en-Qaaa",
        "de-foobar",
        "de-1901-1901",
        "de-1901-1996",
        "de-1996-1901",
        "en-u-ca-islamicc",
        "en-u-ca-gregorian",
        "en-u-kn-true",
        "en-u-kn",
        "en-t-en-us",
        "en-x-private",
        "en-u-ca-gregory-ca-buddhist",
        "nb",
        "nn",
        "no",
        "arb",
        "yue",
        "zh-hakka",
        "en-x-u-kn-true",
        "zz-foobar-x-u-true",
        "de-foobar-foobar",
        "eK",
        "en-Kaaa",
    ] {
        let canonical = match Locale::try_from_str(tag) {
            Ok(mut locale) => {
                canonicalizer.canonicalize(&mut locale);
                locale.to_string()
            }
            Err(_) => "ERROR".into(),
        };
        locales.push_str(&format!("{tag}\t{canonical}\n"));
    }
    fs::write(root.join("locales.tsv"), locales).unwrap();
}
