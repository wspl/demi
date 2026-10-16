//! Passkeys (`web-api-settings.md` § Account API): signing in with one, and the
//! account's list of them, which Settings › Account adds to, renames and
//! removes from.
//!
//! WebAuthn's options and the browser's answers are JSON whose shape the
//! WebAuthn standard owns: the backend decodes and encodes them as
//! `webauthn-rs` types, and their schemas, which the web app's types come
//! from, mirror the standard's JSON dictionaries, so the page hands them to
//! `PublicKeyCredential.parseRequestOptionsFromJSON` and takes them from
//! `PublicKeyCredential.toJSON()` as they are.

use demi_shared_types::Timestamp;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use webauthn_rs_proto::{
    PublicKeyCredential, PublicKeyCredentialCreationOptions, PublicKeyCredentialRequestOptions,
    RegisterPublicKeyCredential,
};

use crate::text::Trimmed;

demi_shared_types::id!(
    /// A passkey's id: its credential id in base64url without padding, as
    /// the browser names it.
    PasskeyId
);

demi_shared_types::id!(
    /// A passkey sign-in's or addition's challenge, which the backend keeps
    /// for five minutes and uses once.
    PasskeyChallengeId
);

/// A passkey of the account, as Settings › Account lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyDto {
    pub id: PasskeyId,
    /// The password manager's name, the browser and system that added it,
    /// or the name the user gave it.
    pub name: String,
    /// Whether it is backed up, as a password manager that syncs it reports;
    /// otherwise it stays on one device.
    pub synced: bool,
    pub created_at: Timestamp,
    /// When it last signed in; none until it has.
    pub last_used_at: Option<Timestamp>,
}

/// `GET /auth/passkeys`: the account's passkeys, oldest first, and the
/// account's WebAuthn user handle, which the page passes to the browser's
/// signal methods with the relying party's id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyList {
    pub passkeys: Vec<PasskeyDto>,
    /// The user handle in base64url without padding.
    pub user_handle: String,
}

/// The 201 answer of `POST /auth/passkeys`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PasskeyAnswer {
    pub passkey: PasskeyDto,
}

/// `POST /auth/passkey/options`'s answer: a challenge and the request
/// options to sign it with.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignInChallenge {
    pub challenge_id: PasskeyChallengeId,
    #[schemars(with = "RequestOptionsJson")]
    pub options: PublicKeyCredentialRequestOptions,
}

/// `POST /auth/passkeys/options`'s answer: a challenge and the creation
/// options of the new passkey, which exclude the account's passkeys.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreationChallenge {
    pub challenge_id: PasskeyChallengeId,
    #[schemars(with = "CreationOptionsJson")]
    pub options: PublicKeyCredentialCreationOptions,
}

/// `POST /auth/passkey`: the challenge and the browser's signed answer.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PasskeySignIn {
    #[garde(skip)]
    pub challenge_id: PasskeyChallengeId,
    /// `webauthn-rs` checks it whole.
    #[garde(skip)]
    #[schemars(with = "AuthenticationResponseJson")]
    pub credential: PublicKeyCredential,
}

/// `POST /auth/passkeys`: the challenge and the browser's new credential.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewPasskey {
    #[garde(skip)]
    pub challenge_id: PasskeyChallengeId,
    /// `webauthn-rs` checks it whole.
    #[garde(skip)]
    #[schemars(with = "RegistrationResponseJson")]
    pub credential: RegisterPublicKeyCredential,
}

/// `PATCH /auth/passkeys/:id`.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PasskeyPatch {
    #[garde(dive)]
    pub name: PasskeyName,
}

/// A passkey's name as a rename sets it: trimmed, then 1 to 80 characters.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(transparent)]
#[schemars(inline)]
#[garde(transparent)]
pub struct PasskeyName(#[garde(length(chars, min = 1, max = 80))] Trimmed);

impl PasskeyName {
    pub fn into_string(self) -> String {
        self.0.into_string()
    }
}

// The schemas below only describe WebAuthn's JSON, which `webauthn-rs`'s
// types encode and decode; nothing builds them. A member the standard makes
// optional is one with a default here, which the schema leaves out of its
// required members without making it nullable, as the browser's types have
// it. They derive `Default` and `Serialize` only because a schema names a
// member's default.

/// `PublicKeyCredentialRequestOptionsJSON`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct RequestOptionsJson {
    challenge: String,
    #[serde(default)]
    timeout: u32,
    #[serde(default)]
    rp_id: String,
    #[serde(default)]
    allow_credentials: Vec<DescriptorJson>,
    #[serde(default)]
    user_verification: String,
    #[serde(default)]
    hints: Vec<String>,
    #[serde(default)]
    extensions: ExtensionInputsJson,
}

/// `PublicKeyCredentialCreationOptionsJSON`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CreationOptionsJson {
    rp: RelyingPartyJson,
    user: UserJson,
    challenge: String,
    pub_key_cred_params: Vec<CredentialParametersJson>,
    #[serde(default)]
    timeout: u32,
    #[serde(default)]
    exclude_credentials: Vec<DescriptorJson>,
    #[serde(default)]
    authenticator_selection: SelectionJson,
    #[serde(default)]
    hints: Vec<String>,
    #[serde(default)]
    attestation: String,
    #[serde(default)]
    extensions: ExtensionInputsJson,
}

/// `PublicKeyCredentialRpEntity`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
struct RelyingPartyJson {
    #[serde(default)]
    id: String,
    name: String,
}

/// `PublicKeyCredentialUserEntityJSON`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UserJson {
    id: String,
    name: String,
    display_name: String,
}

/// `PublicKeyCredentialParameters`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
struct CredentialParametersJson {
    #[serde(rename = "type")]
    kind: PublicKeyType,
    alg: i32,
}

/// `PublicKeyCredentialType`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
enum PublicKeyType {
    #[default]
    #[serde(rename = "public-key")]
    PublicKey,
}

/// `PublicKeyCredentialDescriptorJSON`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
struct DescriptorJson {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    transports: Vec<String>,
}

/// `AuthenticatorSelectionCriteria`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SelectionJson {
    #[serde(default)]
    authenticator_attachment: Attachment,
    #[serde(default)]
    resident_key: ResidentKey,
    #[serde(default)]
    require_resident_key: bool,
    #[serde(default)]
    user_verification: Verification,
}

/// `AuthenticatorAttachment`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
enum Attachment {
    #[default]
    Platform,
    CrossPlatform,
}

/// `ResidentKeyRequirement`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ResidentKey {
    #[default]
    Discouraged,
    Preferred,
    Required,
}

/// `UserVerificationRequirement`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Verification {
    #[default]
    Required,
    Preferred,
    Discouraged,
}

/// `AuthenticationExtensionsClientInputsJSON`: the extensions `webauthn-rs`
/// asks for, which the browser passes to the authenticator: credProtect's
/// policy, the user verification method and the credential's properties.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ExtensionInputsJson {
    #[serde(default)]
    credential_protection_policy: String,
    #[serde(default)]
    enforce_credential_protection_policy: bool,
    #[serde(default)]
    uvm: bool,
    #[serde(default)]
    cred_props: bool,
}

/// `RegistrationResponseJSON`, which `PublicKeyCredential.toJSON()` gives
/// for a new credential.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistrationResponseJson {
    id: String,
    raw_id: String,
    #[serde(rename = "type")]
    kind: String,
    response: AttestationResponseJson,
    client_extension_results: ExtensionOutputsJson,
    #[serde(default)]
    authenticator_attachment: String,
}

/// `AuthenticatorAttestationResponseJSON`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AttestationResponseJson {
    #[serde(rename = "clientDataJSON")]
    client_data_json: String,
    attestation_object: String,
    authenticator_data: String,
    transports: Vec<String>,
    #[serde(default)]
    public_key: String,
    public_key_algorithm: i32,
}

/// `AuthenticationResponseJSON`, which `PublicKeyCredential.toJSON()` gives
/// for a signed challenge.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthenticationResponseJson {
    id: String,
    raw_id: String,
    #[serde(rename = "type")]
    kind: String,
    response: AssertionResponseJson,
    client_extension_results: ExtensionOutputsJson,
    #[serde(default)]
    authenticator_attachment: String,
}

/// `AuthenticatorAssertionResponseJSON`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AssertionResponseJson {
    #[serde(rename = "clientDataJSON")]
    client_data_json: String,
    authenticator_data: String,
    signature: String,
    #[serde(default)]
    user_handle: String,
}

/// `AuthenticationExtensionsClientOutputsJSON`, of the extensions the
/// backend asks for.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExtensionOutputsJson {
    #[serde(default)]
    appid: bool,
    #[serde(default)]
    cred_props: CredentialPropertiesJson,
}

/// `CredentialPropertiesOutput`.
#[allow(dead_code)]
#[derive(Default, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CredentialPropertiesJson {
    #[serde(default)]
    rk: bool,
}
