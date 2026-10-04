//! The real model `xtask dev` can seed beside Echo (`backend.md` §
//! One-command development backend), from the `DEMI_DEV_PROVIDER_*`
//! variables: all four name one model of an OpenAI-compatible Chat
//! Completions endpoint, or none is set, and an optional fifth lists the
//! model's thinking efforts.

use std::num::NonZeroU32;

use serde_json::json;

/// The variables, in the order the entry needs them.
const NAMES: [&str; 4] = [
    "DEMI_DEV_PROVIDER_BASE_URL",
    "DEMI_DEV_PROVIDER_API_KEY",
    "DEMI_DEV_PROVIDER_MODEL",
    "DEMI_DEV_PROVIDER_CONTEXT_WINDOW",
];
/// The model's thinking efforts, comma-separated, as the endpoint names
/// them; without it the model has none, and the page offers no effort.
const EFFORTS: &str = "DEMI_DEV_PROVIDER_THINKING_EFFORTS";

/// The development model the variables name.
pub struct DevProvider {
    base_url: String,
    api_key: String,
    model: String,
    context_window: NonZeroU32,
    thinking_efforts: Vec<String>,
}

impl DevProvider {
    /// The model `var` names, none when it names none of the variables, or
    /// what is wrong with them. Read before anything is built, so a partial
    /// setting stops the command at once; the backend validates the URL when
    /// the entry is created.
    pub fn read(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, String> {
        let values = NAMES.map(|name| var(name).filter(|value| !value.is_empty()));
        let efforts = var(EFFORTS).filter(|value| !value.is_empty());
        if values.iter().all(Option::is_none) {
            return match efforts {
                Some(_) => Err(format!(
                    "{EFFORTS} is set, but the development provider needs all of {}",
                    NAMES.join(", ")
                )),
                None => Ok(None),
            };
        }
        let [Some(base_url), Some(api_key), Some(model), Some(window)] = values else {
            let missing: Vec<&str> = NAMES
                .iter()
                .zip(&values)
                .filter(|(_, value)| value.is_none())
                .map(|(name, _)| *name)
                .collect();
            return Err(format!(
                "{} not set; the development provider needs all of {}",
                missing.join(", "),
                NAMES.join(", ")
            ));
        };
        let context_window = window
            .parse::<NonZeroU32>()
            .map_err(|_| format!("{} must be a positive integer below 2^32", NAMES[3]))?;
        let thinking_efforts = efforts
            .iter()
            .flat_map(|list| list.split(','))
            .map(|effort| effort.trim().to_owned())
            .collect();
        Ok(Some(Self {
            base_url,
            api_key,
            model,
            context_window,
            thinking_efforts,
        }))
    }

    /// The model's name, as the page shows it.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// The provider entry the web API creates: an `openai` entry labeled
    /// Development that speaks Chat Completions, with the one model.
    pub fn entry(&self) -> serde_json::Value {
        json!({
            "source": "custom",
            "providerType": "openai",
            "wireApi": "chat-completions",
            "label": "Development",
            "apiKey": self.api_key,
            "baseUrl": self.base_url,
            "models": [{
                "id": self.model,
                "displayName": self.model,
                "contextWindow": self.context_window.get(),
                "outputLimit": null,
                "thinkingEfforts": self.thinking_efforts,
                "acceptedExtensions": null,
                "fastTier": null,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(set: &[(&str, &str)]) -> Result<Option<DevProvider>, String> {
        DevProvider::read(|name| {
            set.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        })
    }

    #[test]
    fn the_variables_name_a_model_only_when_all_four_are_set() {
        assert!(read(&[]).unwrap().is_none());
        let all = [
            (NAMES[0], "https://gateway.example/v1"),
            (NAMES[1], "key"),
            (NAMES[2], "deepseek/deepseek-v4.1-flash"),
            (NAMES[3], "1000000"),
        ];
        let provider = read(&all).unwrap().unwrap();
        assert_eq!(provider.entry()["models"][0]["contextWindow"], 1_000_000);
        assert_eq!(provider.entry()["wireApi"], "chat-completions");

        let error = read(&all[..2]).err().unwrap();
        assert!(error.starts_with("DEMI_DEV_PROVIDER_MODEL, DEMI_DEV_PROVIDER_CONTEXT_WINDOW not set"), "{error}");
        let zero = [all[0], all[1], all[2], (NAMES[3], "0")];
        assert!(read(&zero).err().unwrap().contains("positive integer"));
    }

    #[test]
    fn the_thinking_efforts_reach_the_entry_and_need_the_model() {
        let efforts = [
            (NAMES[0], "https://gateway.example/v1"),
            (NAMES[1], "key"),
            (NAMES[2], "deepseek/deepseek-v4.1-flash"),
            (NAMES[3], "1000000"),
            (EFFORTS, "low, medium,high"),
        ];
        let provider = read(&efforts).unwrap().unwrap();
        assert_eq!(
            provider.entry()["models"][0]["thinkingEfforts"],
            json!(["low", "medium", "high"])
        );
        assert!(read(&efforts[4..]).err().unwrap().starts_with(EFFORTS));
    }
}
