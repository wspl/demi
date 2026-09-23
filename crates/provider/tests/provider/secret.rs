//! Credentials as providers hold them.

use demi_provider::{Secret, SecretError};

#[test]
fn a_secret_is_one_line_of_text_and_is_never_printed() {
    assert_eq!(Secret::try_from(String::new()), Err(SecretError::Empty));
    assert_eq!(Secret::try_from("sk-1\nx".to_owned()), Err(SecretError::Control));
    let secret = Secret::try_from("sk-ant-123".to_owned()).unwrap();
    assert_eq!(format!("{secret:?}"), "Secret(..)");
    assert_eq!(secret.expose(), "sk-ant-123");
    assert!(secret.header_value().is_sensitive());
    let decoded: Secret = serde_json::from_str(r#""sk-ant-123""#).unwrap();
    assert_eq!(decoded, secret);
    assert!(serde_json::from_str::<Secret>(r#""""#).is_err());
}
