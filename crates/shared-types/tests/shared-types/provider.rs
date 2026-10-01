//! What the product keeps of a provider entry: a stored catalog and a stored
//! quota snapshot keep their wire shape and refuse what their contract does
//! not hold.

use demi_shared_types::{ProviderModelList, QuotaSnapshot, decode};
use serde_json::{Value, json};

fn catalog() -> Value {
    json!({
        "models": [{
            "id": "claude-opus-4-8",
            "displayName": "Claude Opus 4.8",
            "description": null,
            "contextWindow": 1_000_000,
            "outputLimit": 128_000,
            "supportsTools": true,
            "supportsAttachments": true,
            "supportsVideo": null,
            "acceptedExtensions": null,
            "supportsReasoning": true,
            "supportedThinkingEfforts": ["low", "high"],
            "defaultThinkingEffort": null,
            "canDisableThinking": false,
            "serviceTiers": [{ "id": "priority", "label": "Fast", "description": null, "fast": true }],
            "defaultServiceTierId": null,
            "cost": { "input": 5.0, "output": 25.0, "cacheRead": null, "cacheWrite": null }
        }],
        "defaultModelId": "claude-opus-4-8",
        "warnings": [],
        "sourceFetchedAt": "1970-01-01T00:00:00.000Z",
        "stale": false
    })
}

fn snapshot() -> Value {
    json!({
        "observedAt": "2026-09-23T10:04:12.000Z",
        "source": "observation",
        "plan": { "id": "pro", "label": "Pro" },
        "accountLabel": null,
        "windows": [{
            "id": "primary",
            "label": "5-hour",
            "usedPercent": 34.0,
            "used": null,
            "limit": null,
            "unit": "percent",
            "resetsAt": "2026-09-23T12:30:00.000Z",
            "severity": "normal",
            "scope": null
        }]
    })
}

#[test]
fn a_stored_catalog_and_quota_snapshot_keep_their_wire_shape() {
    let list: ProviderModelList = decode(&catalog().to_string()).unwrap();
    assert_eq!(serde_json::to_value(&list).unwrap(), catalog());
    let quota: QuotaSnapshot = decode(&snapshot().to_string()).unwrap();
    assert_eq!(serde_json::to_value(&quota).unwrap(), snapshot());
}

#[test]
fn a_stored_catalog_and_quota_snapshot_refuse_what_their_contract_does_not_hold() {
    let mut refused = Vec::new();
    let mut unknown = catalog();
    unknown["models"][0]["providerId"] = json!("anthropic");
    refused.push(unknown);
    let mut absent = catalog();
    absent["models"][0]
        .as_object_mut()
        .unwrap()
        .remove("description");
    refused.push(absent);
    let mut zero = catalog();
    zero["models"][0]["outputLimit"] = json!(0);
    refused.push(zero);
    for value in refused {
        assert!(
            decode::<ProviderModelList>(&value.to_string()).is_err(),
            "{value}"
        );
    }

    let mut over = snapshot();
    over["windows"][0]["usedPercent"] = json!(100.5);
    let mut cached = snapshot();
    cached["source"] = json!("cache");
    let mut raw = snapshot();
    raw["raw"] = json!({ "plan_type": "pro" });
    for value in [over, cached, raw] {
        assert!(
            decode::<QuotaSnapshot>(&value.to_string()).is_err(),
            "{value}"
        );
    }
}
