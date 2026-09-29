// Writes rust-vault.json: values sealed by the Rust backend's own vault code
// (crates/backend/src/vault/seal.rs and secret.rs, included with their module
// documentation removed), which the Go vault must open; and the catalog keys
// of two entries, made by the calls crates/backend/src/llm/catalog.rs's
// catalog_key makes. Every value is made up; none is a real credential.
extern crate aes_gcm;
extern crate demi_artifact;
extern crate demi_web_api;
extern crate hex;
extern crate hkdf;
extern crate rand;
extern crate serde_json;
extern crate serde_json_canonicalizer;
extern crate sha2;
extern crate thiserror;
extern crate tokio;

mod vault {
    pub mod seal {
        include!("seal_body.rs");
    }
    pub mod secret {
        include!("secret_body.rs");
    }
}

use demi_web_api::ids::{CredentialId, ProviderId};
use serde_json::json;
use sha2::Digest;
use vault::seal::Row;
use vault::secret::InstanceSecret;

fn main() {
    let path = std::env::args().nth(1).expect("the output path");
    let text = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    let secret: InstanceSecret = text.parse().unwrap();
    let key = secret.vault_key();
    let provider = ProviderId::try_from("3f1c9a52-7d4e-4b8a-9c0f-2e6d5b7a8c91").unwrap();
    let account = CredentialId::try_from("cred-0a1b2c3d4e5f6071").unwrap();
    let config = r#"{"apiKey":"sk-fixture-not-a-key","baseUrl":"https://api.example.test/v1","wireApi":"responses","vendorId":"example","models":[{"id":"gpt-5.5","displayName":"GPT-5.5 \u201cFast\u201d \u2713","contextWindow":272000,"outputLimit":128000,"thinkingEfforts":["low","high"],"acceptedExtensions":["png","pdf"],"fastTier":"priority"}]}"#;
    let document = r#"{"refresh":"fixture-refresh-token"}"#;
    let sealed_config = key.seal(Row::Config(&provider), config.as_bytes());
    // catalog_key's own calls, for an API-key entry and a subscription entry.
    let catalog_key = |identity: serde_json::Value| {
        let canonical = serde_json_canonicalizer::to_string(&identity).unwrap();
        hex::encode(sha2::Sha256::digest(canonical.as_bytes()))
    };
    let config_value: serde_json::Value = serde_json::from_str(config).unwrap();
    let api_key_catalog = catalog_key(json!({ "family": "openai", "config": config_value, "account": null }));
    let subscription_catalog = catalog_key(json!({ "family": "codex", "config": null, "account": account.as_str() }));
    let sealed_secret = key.seal(Row::Secret(&provider, &account), document.as_bytes());
    let fixture = format!(
        concat!(
            "{{\n",
            "  \"instanceSecret\": \"{}\",\n",
            "  \"emailCodeKey\": \"{}\",\n",
            "  \"provider\": \"{}\",\n",
            "  \"account\": \"{}\",\n",
            "  \"config\": {:?},\n",
            "  \"sealedConfig\": \"{}\",\n",
            "  \"secret\": {:?},\n",
            "  \"sealedSecret\": \"{}\",\n",
            "  \"apiKeyCatalogKey\": \"{}\",\n",
            "  \"subscriptionCatalogKey\": \"{}\"\n",
            "}}\n"
        ),
        text,
        hex::encode(secret.email_code_key().as_bytes()),
        provider.as_str(),
        account.as_str(),
        config,
        hex::encode(sealed_config),
        document,
        hex::encode(sealed_secret),
        api_key_catalog,
        subscription_catalog,
    );
    std::fs::write(path, fixture).unwrap();
}
