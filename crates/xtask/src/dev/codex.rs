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

#[cfg(test)]
mod tests {
    use super::*;

    fn read(value: Option<&str>, home: &Path) -> Result<Option<CodexSignIn>, String> {
        CodexSignIn::read(
            |name| (name == AUTH).then(|| value.map(str::to_owned)).flatten(),
            Some(home.to_owned()),
        )
    }

    #[test]
    fn the_sign_in_reaches_the_backend_without_its_refresh_token() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".codex")).unwrap();
        let auth = home.path().join(".codex/auth.json");
        let sign_in = json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {
                "id_token": "id-1",
                "access_token": "access-1",
                "refresh_token": "refresh-1",
                "account_id": "acct-1",
            },
            "last_refresh": "2026-10-18T03:00:00.000000Z",
        });
        std::fs::write(&auth, sign_in.to_string()).unwrap();

        assert!(read(None, home.path()).unwrap().is_none());
        let read = read(Some("~/.codex/auth.json"), home.path()).unwrap().unwrap();
        assert_eq!(read.path, auth);
        let handed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(read.backend_file()).unwrap()).unwrap();
        assert_eq!(
            handed,
            json!({ "accessToken": "access-1", "idToken": "id-1", "accountId": "acct-1" })
        );
    }

    #[test]
    fn a_path_that_cannot_be_read_or_holds_no_access_token_stops_the_command() {
        let home = tempfile::tempdir().unwrap();
        let missing = home.path().join("missing.json");
        let error = read(Some(missing.to_str().unwrap()), home.path()).err().unwrap();
        assert!(
            error.starts_with(&format!("{AUTH} names {}, which cannot be read", missing.display())),
            "{error}"
        );

        let api_key = home.path().join("api-key.json");
        std::fs::write(&api_key, r#"{"OPENAI_API_KEY":"sk-secret-key","tokens":null}"#).unwrap();
        let error = read(Some(api_key.to_str().unwrap()), home.path()).err().unwrap();
        assert_eq!(
            error,
            format!("{} holds no access token; `codex login` signs in", api_key.display())
        );

        let garbled = home.path().join("garbled.json");
        std::fs::write(&garbled, r#"{"tokens":{"access_token":12}}"#).unwrap();
        let error = read(Some(garbled.to_str().unwrap()), home.path()).err().unwrap();
        assert!(error.starts_with(&format!("{} is not a Codex CLI sign-in", garbled.display())), "{error}");
        assert!(!error.contains("12"), "{error}");
    }
}
