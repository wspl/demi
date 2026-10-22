//! The developer's Codex CLI sign-in `xtask dev` signs the master account's
//! Codex entry in with: `DEMI_DEV_CODEX_AUTH` names its `auth.json`, whose
//! access token, ID token and account ID, never its refresh token, go to the
//! backend in a file of this run's that only this user can read, named by
//! the backend's development flag. No token is ever printed.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;

/// The variable.
const AUTH: &str = "DEMI_DEV_CODEX_AUTH";

/// The Codex CLI's `auth.json`, as far as it is read: its other fields,
/// the refresh token among them, are left unread.
#[derive(Deserialize)]
struct AuthFile {
    #[serde(default)]
    tokens: Option<Tokens>,
}

#[derive(Deserialize)]
struct Tokens {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

/// The sign-in a run hands the backend.
pub struct CodexSignIn {
    /// The `auth.json` it was read from.
    pub path: PathBuf,
    /// The backend's copy, removed when the run ends.
    file: tempfile::NamedTempFile,
}

impl CodexSignIn {
    /// The sign-in the file `var` names, with a leading `~/` in `home`;
    /// none when the variable is unset; or why it cannot be used, naming
    /// the path.
    pub fn read(
        var: impl Fn(&str) -> Option<String>,
        home: Option<PathBuf>,
    ) -> Result<Option<Self>, String> {
        let Some(named) = var(AUTH).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let path = match named.strip_prefix("~/") {
            Some(rest) => home
                .ok_or_else(|| format!("{AUTH} names {named}, and the home directory is unknown"))?
                .join(rest),
            None => PathBuf::from(named),
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{AUTH} names {}, which cannot be read: {error}", path.display()))?;
        let document = backend_document(&path, &text)?;
        let written = (|| {
            let mut file = tempfile::Builder::new().prefix("demi-dev-codex-").tempfile()?;
            file.write_all(document.to_string().as_bytes())?;
            file.flush()?;
            Ok::<_, std::io::Error>(file)
        })();
        let file = written.map_err(|error| format!("the backend's copy of {}: {error}", path.display()))?;
        Ok(Some(Self { path, file }))
    }

    /// The file the backend's `--dev-codex-account` names.
    pub fn backend_file(&self) -> &Path {
        self.file.path()
    }
}

/// What the backend reads of the sign-in `text` of the file at `path`: its
/// access token, ID token and account ID.
fn backend_document(path: &Path, text: &str) -> Result<serde_json::Value, String> {
    // serde's message may quote a value, which may be a token: only where.
    let file: AuthFile = serde_json::from_str(text).map_err(|error| {
        format!(
            "{} is not a Codex CLI sign-in: it fails to parse at line {}, column {}",
            path.display(),
            error.line(),
            error.column()
        )
    })?;
    let tokens = file.tokens;
    let field = |pick: fn(&Tokens) -> Option<&String>, what: &str| {
        tokens
            .as_ref()
            .and_then(pick)
            .filter(|value| !value.is_empty())
            .cloned()
            .ok_or_else(|| format!("{} holds no {what}; `codex login` signs in", path.display()))
    };
    Ok(json!({
        "accessToken": field(|tokens| tokens.access_token.as_ref(), "access token")?,
        "idToken": field(|tokens| tokens.id_token.as_ref(), "ID token")?,
        "accountId": field(|tokens| tokens.account_id.as_ref(), "account ID")?,
    }))
}
