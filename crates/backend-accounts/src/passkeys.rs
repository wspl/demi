//! Passkeys (`backend.md` § Authentication and ownership, `web-api.md`
//! § Account API): signing in with one, without an email first, and adding
//! one to an account, each in two requests around the browser's own step.
//! `webauthn-rs` makes every challenge and does every check; this module
//! keeps the challenges in the control store between the two requests,
//! finds the account a passkey names, and names a new passkey.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::passkeys::{
    ChallengeLimits, ChallengeState, PasskeyRecord, credential_id,
};
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::passkeys::{
    CreationChallenge, PasskeyChallengeId, PasskeyId, PasskeyList, SignInChallenge,
};
use jiff::SignedDuration;
use serde::Deserialize;
use url::{Host, Url};
use uuid::Uuid;
use webauthn_rs::prelude::{
    DiscoverableKey, PublicKeyCredential, RegisterPublicKeyCredential, Webauthn, WebauthnBuilder,
    WebauthnError,
};
use webauthn_rs_core::internals::AuthenticatorData;
use webauthn_rs_core::proto::{Registration, ResidentKeyRequirement};

use crate::user_agent::browser_and_system;

/// A challenge lasts five minutes, and at most 1000 wait at once.
pub const CHALLENGE_LIMITS: ChallengeLimits = ChallengeLimits {
    lifetime: SignedDuration::from_mins(5),
    waiting: 1000,
};

/// The relying party's id for a backend whose public URL is `public`: its
/// host name. None for an IP address, for which browsers make no passkeys.
pub fn relying_party_id(public: &Url) -> Option<&str> {
    match public.host()? {
        Host::Domain(name) => Some(name),
        Host::Ipv4(_) | Host::Ipv6(_) => None,
    }
}

/// The relying party for one request: the public URL's host name, the
/// origins it accepts the browser's answers from, the public URL's and the
/// page's own, as the edge's origin check admits it (a development server
/// passes the page's requests on from another port).
pub struct RelyingParty(Webauthn);

impl RelyingParty {
    /// None when the backend offers no passkeys.
    pub fn new(public: &Url, page: Option<&Url>) -> Option<Self> {
        let id = relying_party_id(public)?;
        let origin = Url::parse(&public.origin().ascii_serialization()).ok()?;
        let mut builder = WebauthnBuilder::new(id, &origin)
            .inspect_err(|error| tracing::warn!("the public URL is no relying party: {error}"))
            .ok()?
            .rp_name("Demi");
        if let Some(page) = page.filter(|page| page.origin() != origin.origin()) {
            builder = builder.append_allowed_origin(page);
        }
        builder
            .build()
            .inspect_err(|error| tracing::warn!("the relying party could not be built: {error}"))
            .ok()
            .map(Self)
    }
}

/// Why a passkey operation failed in a way no request causes.
#[derive(Debug, thiserror::Error)]
pub enum PasskeyError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("WebAuthn failed: {0}")]
    Webauthn(#[from] WebauthnError),
}

/// Why a challenge could not be issued.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueRefusal {
    /// 1000 challenges wait already.
    TooMany,
}

/// Why a sign-in or an addition was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The challenge expired, was used, or is not one of this operation.
    Expired,
    /// No account has the passkey the browser signed with.
    UnknownPasskey(PasskeyId),
    /// The passkey is another account's than the one it must verify.
    OtherAccount,
    /// The browser's answer did not pass a check.
    Failed,
}

/// Issues a sign-in's challenge: the request options for any passkey of
/// this relying party, so neither the autofill nor the button needs an
/// email first.
pub async fn start_sign_in(
    control: &ControlService,
    party: &RelyingParty,
) -> Result<Result<SignInChallenge, IssueRefusal>, PasskeyError> {
    let (request, state) = party.0.start_discoverable_authentication()?;
    let id = challenge_id();
    if !control
        .issue_passkey_challenge(id.clone(), ChallengeState::SignIn(state), CHALLENGE_LIMITS)
        .await?
    {
        return Ok(Err(IssueRefusal::TooMany));
    }
    Ok(Ok(SignInChallenge {
        challenge_id: id,
        options: request.public_key,
    }))
}

/// Checks the browser's answer to the challenge `id` and answers the
/// account it signs in, recording the passkey's use. A verification of the
/// session of `account` takes only a passkey of that account; a sign-in
/// passes none.
pub async fn finish_sign_in(
    control: &ControlService,
    party: &RelyingParty,
    id: PasskeyChallengeId,
    credential: &PublicKeyCredential,
    account: Option<&UserId>,
) -> Result<Result<UserId, Refusal>, PasskeyError> {
    let Some(ChallengeState::SignIn(state)) = control.take_passkey_challenge(id).await? else {
        return Ok(Err(Refusal::Expired));
    };
    let Ok((handle, raw_id)) = party.0.identify_discoverable_authentication(credential) else {
        return Ok(Err(Refusal::Failed));
    };
    let Some(passkey_id) = credential_id(raw_id) else {
        return Ok(Err(Refusal::Failed));
    };
    let Some(record) = control.passkey(passkey_id.clone()).await? else {
        return Ok(Err(Refusal::UnknownPasskey(passkey_id)));
    };
    if account.is_some_and(|account| *account != record.user) {
        return Ok(Err(Refusal::OtherAccount));
    }
    // The handle the authenticator keeps beside the passkey names the
    // account it was made for.
    if user_handle(&record.user) != Some(handle) {
        return Ok(Err(Refusal::Failed));
    }
    let key = DiscoverableKey::from(&record.credential);
    let Ok(result) = party
        .0
        .finish_discoverable_authentication(credential, state, &[key])
    else {
        return Ok(Err(Refusal::Failed));
    };
    let mut updated = record.credential.clone();
    let changed = updated.update_credential(&result) == Some(true);
    control
        .passkey_used(record.id, changed.then_some(updated))
        .await?;
    Ok(Ok(record.user))
}

/// Issues an addition's challenge for `user`, whose password the caller
/// checked: the creation options of a discoverable passkey that verifies
/// its user, excluding the account's passkeys.
pub async fn start_registration(
    control: &ControlService,
    party: &RelyingParty,
    user: &UserDto,
) -> Result<Result<CreationChallenge, IssueRefusal>, PasskeyError> {
    let Some(handle) = user_handle(&user.id) else {
        return Err(PasskeyError::Webauthn(WebauthnError::InvalidUserUniqueId));
    };
    let excluded = control
        .passkeys(user.id.clone())
        .await?
        .into_iter()
        .map(|passkey| passkey.credential.cred_id().clone())
        .collect();
    let display_name = if user.nickname.is_empty() {
        user.email.as_str()
    } else {
        &user.nickname
    };
    let (mut creation, state) = party.0.start_passkey_registration(
        handle,
        user.email.as_str(),
        display_name,
        Some(excluded),
    )?;
    // `webauthn-rs` leaves a passkey's discoverability to the authenticator
    // and offers no way to ask for it; a sign-in that names no account
    // needs a discoverable one, so the options ask for it. Its check of the
    // answer does not depend on it.
    if let Some(selection) = creation.public_key.authenticator_selection.as_mut() {
        selection.resident_key = Some(ResidentKeyRequirement::Required);
        selection.require_resident_key = true;
    }
    let id = challenge_id();
    let challenge = ChallengeState::Registration {
        user: user.id.clone(),
        state,
    };
    if !control
        .issue_passkey_challenge(id.clone(), challenge, CHALLENGE_LIMITS)
        .await?
    {
        return Ok(Err(IssueRefusal::TooMany));
    }
    Ok(Ok(CreationChallenge {
        challenge_id: id,
        options: creation.public_key,
    }))
}

/// Checks the browser's new credential against `user`'s challenge `id` and
/// adds it to the account, named for its password manager or else for the
/// browser and system `user_agent` names.
pub async fn finish_registration(
    control: &ControlService,
    party: &RelyingParty,
    user: &UserId,
    id: PasskeyChallengeId,
    credential: &RegisterPublicKeyCredential,
    user_agent: Option<&str>,
) -> Result<Result<PasskeyRecord, Refusal>, PasskeyError> {
    let state = match control.take_passkey_challenge(id).await? {
        Some(ChallengeState::Registration { user: of, state }) if of == *user => state,
        _ => return Ok(Err(Refusal::Expired)),
    };
    let Ok(passkey) = party.0.finish_passkey_registration(credential, &state) else {
        return Ok(Err(Refusal::Failed));
    };
    let Some(passkey_id) = credential_id(passkey.cred_id()) else {
        return Ok(Err(Refusal::Failed));
    };
    let name = aaguid(credential)
        .and_then(manager_name)
        .map(str::to_owned)
        .or_else(|| browser_and_system(user_agent.unwrap_or_default()))
        .unwrap_or_else(|| "Passkey".to_owned());
    // A credential id another account has already is refused, as
    // `webauthn-rs` asks of its caller.
    match control
        .add_passkey(user.clone(), passkey_id, name, passkey)
        .await?
    {
        Some(record) => Ok(Ok(record)),
        None => Ok(Err(Refusal::Failed)),
    }
}

/// The WebAuthn user handle of the account `user`: the bytes of its id, a
/// UUID. None for an id of another form, which the backend never assigns.
pub fn user_handle(user: &UserId) -> Option<Uuid> {
    Uuid::parse_str(user.as_str()).ok()
}

/// `user`'s passkeys and user handle, as `GET /auth/passkeys` and the
/// synchronization channel's `passkeys` part give them.
pub async fn passkey_list(control: &ControlService, user: &UserId) -> Result<PasskeyList, StorageError> {
    let passkeys = control.passkeys(user.clone()).await?;
    Ok(PasskeyList {
        passkeys: passkeys.iter().map(PasskeyRecord::dto).collect(),
        user_handle: user_handle(user)
            .map(|handle| URL_SAFE_NO_PAD.encode(handle.as_bytes()))
            .unwrap_or_default(),
    })
}

fn challenge_id() -> PasskeyChallengeId {
    PasskeyChallengeId::try_from(Uuid::new_v4().to_string()).expect("a UUID is not empty")
}

/// The authenticator data of an attestation object: the one member of it
/// this module reads.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttestationObject<'a> {
    #[serde(borrow)]
    auth_data: &'a [u8],
}

/// The AAGUID of the authenticator that made `credential`, which names its
/// password manager; none for the zero AAGUID, which Apple's passkeys
/// carry. `webauthn-rs` keeps no AAGUID of a passkey registered without
/// attestation, so it is read from the attestation object `webauthn-rs`
/// has checked, with the CBOR decoder and the authenticator data parser
/// `webauthn-rs` reads it with.
fn aaguid(credential: &RegisterPublicKeyCredential) -> Option<Uuid> {
    let object: AttestationObject<'_> =
        serde_cbor_2::from_slice(credential.response.attestation_object.as_ref()).ok()?;
    let data = AuthenticatorData::<Registration>::try_from(object.auth_data).ok()?;
    let aaguid = Uuid::from_bytes(data.acd?.aaguid);
    (!aaguid.is_nil()).then_some(aaguid)
}

/// The password managers most people use, by the AAGUID each one's vendor
/// gives in its own code. A manager whose vendor publishes none is named by
/// the browser and system instead.
const MANAGERS: [(Uuid, &str); 5] = [
    // Chromium, device/fido/enclave/enclave_protocol_utils.cc: "AAGUID
    // value for GPM".
    (
        Uuid::from_u128(0xea9b8d66_4d01_1d21_3ce4_b6b48cb575d4),
        "Google Password Manager",
    ),
    // Chromium, device/fido/mac/util.mm: the AAGUID Google Chrome gives its
    // Touch ID platform authenticator's credentials.
    (
        Uuid::from_u128(0xadce0002_35bc_c60a_648b_0b25f1f05503),
        "Chrome on Mac",
    ),
    // bitwarden/clients,
    // libs/common/src/platform/services/fido2/fido2-authenticator.service.ts.
    (
        Uuid::from_u128(0xd548826e_79b4_db40_a3d8_11116f7e8349),
        "Bitwarden",
    ),
    // keepassxreboot/keepassxc, src/browser/BrowserPasskeys.cpp.
    (
        Uuid::from_u128(0xfdb141b2_5d84_443e_8a35_4698c205a502),
        "KeePassXC",
    ),
    // protonpass/proton-pass-common,
    // proton-pass-common/src/passkey/passkey_handling.rs.
    (
        Uuid::from_u128(0x50726f74_6f6e_5061_7373_50726f746f6e),
        "Proton Pass",
    ),
];

fn manager_name(aaguid: Uuid) -> Option<&'static str> {
    MANAGERS
        .iter()
        .find(|(known, _)| *known == aaguid)
        .map(|(_, name)| *name)
}
