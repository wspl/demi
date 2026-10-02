// This program calls the unchanged Rust reference seal implementation.
extern crate self as demi_web_api_protocol;
pub mod ids {
    pub type ProviderId = String;
    pub type CredentialId = String;
}
#[path = "../../../../../../crates/backend-providers/src/vault/seal.rs"]
mod seal;

fn main() {
    let key = seal::VaultKey::new([7; 32]);
    let provider = "entry-1".to_owned();
    let account = "cred-1".to_owned();
    for (name, row, plain) in [
        ("config", seal::Row::Config(&provider), br#"{"apiKey":"sk-test-123"}"#.as_slice()),
        ("secret", seal::Row::Secret(&provider, &account), br#"{"refresh":"rust-token"}"#.as_slice()),
    ] {
        let sealed = key.seal(row, plain);
        assert_eq!(key.open(row, &sealed).unwrap(), plain);
        let hex: String = sealed.iter().map(|byte| format!("{byte:02x}")).collect();
        println!("{name} {hex}");
    }
}
