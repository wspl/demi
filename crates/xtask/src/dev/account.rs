//! The master account `xtask dev` seeds (`backend.md` § One-command
//! development backend): the one `DEMI_DEV_EMAIL` and `DEMI_DEV_PASSWORD`
//! name, the same pair the web app's sign-in page fills in from the
//! repository's `.env`, or a fixed account when neither is set. Each run
//! starts on a fresh data directory, so the account is the database's first
//! user every time.

/// The variables, which the page reads from `.env` as well.
const EMAIL: &str = "DEMI_DEV_EMAIL";
const PASSWORD: &str = "DEMI_DEV_PASSWORD";

/// The account seeded when neither variable is set.
const DEFAULT_EMAIL: &str = "developer@example.test";
const DEFAULT_PASSWORD: &str = "development";

pub struct Account {
    pub email: String,
    pub password: String,
    /// Whether the variables name it, so the page fills it in by itself.
    pub from_env: bool,
}

impl Account {
    /// The account `var` names, the fixed one when it names neither
    /// variable, or which variable is missing.
    pub fn read(var: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let email = var(EMAIL).filter(|value| !value.is_empty());
        let password = var(PASSWORD).filter(|value| !value.is_empty());
        match (email, password) {
            (Some(email), Some(password)) => Ok(Self {
                email,
                password,
                from_env: true,
            }),
            (None, None) => Ok(Self {
                email: DEFAULT_EMAIL.to_owned(),
                password: DEFAULT_PASSWORD.to_owned(),
                from_env: false,
            }),
            (Some(_), None) => Err(format!("{EMAIL} is set without {PASSWORD}")),
            (None, Some(_)) => Err(format!("{PASSWORD} is set without {EMAIL}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(set: &[(&str, &str)]) -> Result<Account, String> {
        Account::read(|name| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn the_variables_name_the_account_or_neither_gives_the_fixed_one() {
        let named = read(&[(EMAIL, "me@example.test"), (PASSWORD, "secret-1")]).unwrap();
        assert_eq!(
            (named.email.as_str(), named.password.as_str(), named.from_env),
            ("me@example.test", "secret-1", true)
        );
        let fixed = read(&[]).unwrap();
        assert_eq!(
            (fixed.email.as_str(), fixed.password.as_str(), fixed.from_env),
            (DEFAULT_EMAIL, DEFAULT_PASSWORD, false)
        );
        assert_eq!(
            read(&[(EMAIL, "me@example.test")]).err().unwrap(),
            "DEMI_DEV_EMAIL is set without DEMI_DEV_PASSWORD"
        );
    }
}
