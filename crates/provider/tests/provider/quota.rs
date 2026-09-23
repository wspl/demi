//! An account's quota snapshot: the merge of probes and observations, and
//! the helpers a family reads its vendor's windows with (`usage-and-quota.md`
//! § Vendor quota).

use std::sync::{Arc, Mutex};

use demi_core::{QuotaPlan, QuotaSeverity, QuotaUnit, QuotaWindow, SnapshotSource, Timestamp};
use demi_provider::{
    quota::{
        MemorySnapshots, Observation, ProbeCost, ProbeReading, ProviderQuota, QuotaError,
        QuotaSource, clamp_used_percent, header_number, rfc3339, severity, unix_seconds,
        used_percent_from_ratio,
    },
    testing::FixedClock,
};
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderValue, StatusCode};

fn window(id: &str, used_percent: f64) -> QuotaWindow {
    QuotaWindow {
        id: id.into(),
        label: id.into(),
        used_percent: Some(used_percent),
        used: None,
        limit: None,
        unit: Some(QuotaUnit::Percent),
        resets_at: None,
        severity: severity(Some(used_percent)),
        scope: None,
    }
}

fn plan(id: &str) -> QuotaPlan {
    QuotaPlan {
        id: id.into(),
        label: id.into(),
    }
}

/// A family whose probes and observations are scripted.
struct Scripted {
    cost: Option<ProbeCost>,
    probes: Mutex<Vec<ProbeReading>>,
    observations: Mutex<Vec<Option<Vec<QuotaWindow>>>>,
}

impl QuotaSource for Scripted {
    fn probe_cost(&self) -> Option<ProbeCost> {
        self.cost
    }

    fn probe(&self) -> BoxFuture<'_, Result<ProbeReading, QuotaError>> {
        let reading = self.probes.lock().unwrap().remove(0);
        Box::pin(async { Ok(reading) })
    }

    fn observe(&self, _observation: Observation<'_>) -> Option<Vec<QuotaWindow>> {
        self.observations.lock().unwrap().remove(0)
    }
}

const NOW: &str = "2026-09-18T14:00:00.000Z";

fn quota(source: Scripted) -> ProviderQuota {
    let clock = Arc::new(FixedClock(NOW.parse::<Timestamp>().unwrap()));
    ProviderQuota::new(Box::new(source), Arc::new(MemorySnapshots::new()), clock)
}

fn observe(quota: &ProviderQuota) {
    let headers = HeaderMap::new();
    quota.observe(Observation::Response {
        status: StatusCode::OK,
        headers: &headers,
    });
}

fn ids_and_shares(windows: &[QuotaWindow]) -> Vec<(String, Option<f64>)> {
    windows.iter().map(|window| (window.id.clone(), window.used_percent)).collect()
}

#[tokio::test]
async fn an_observation_replaces_its_windows_and_keeps_the_probed_plan_and_label() {
    let quota = quota(Scripted {
        cost: Some(ProbeCost::Free),
        probes: Mutex::new(vec![ProbeReading {
            plan: Some(plan("pro")),
            account_label: Some("person@example.com".into()),
            windows: vec![window("monthly", 25.0), window("rpm", 10.0)],
        }]),
        observations: Mutex::new(vec![Some(vec![window("rpm", 40.0), window("tpm", 5.0)])]),
    });
    assert!(quota.latest().is_none());
    let probed = quota.probe().await.unwrap();
    assert_eq!((probed.source, probed.observed_at), (SnapshotSource::Probe, NOW.parse().unwrap()));

    observe(&quota);
    let observed = quota.latest().unwrap();
    assert_eq!(observed.source, SnapshotSource::Observation);
    assert_eq!(observed.plan, Some(plan("pro")));
    assert_eq!(observed.account_label.as_deref(), Some("person@example.com"));
    assert_eq!(
        ids_and_shares(&observed.windows),
        [("monthly".into(), Some(25.0)), ("rpm".into(), Some(40.0)), ("tpm".into(), Some(5.0))]
    );
}

#[tokio::test]
async fn a_probe_says_the_plan_and_keeps_the_windows_it_does_not_name() {
    let quota = quota(Scripted {
        cost: Some(ProbeCost::Free),
        probes: Mutex::new(vec![
            ProbeReading {
                plan: Some(plan("pro")),
                account_label: None,
                windows: vec![window("weekly", 40.0)],
            },
            // A later probe that names no plan clears it.
            ProbeReading::default(),
        ]),
        observations: Mutex::new(vec![Some(vec![window("requests", 5.0)]), None]),
    });
    observe(&quota);
    let probed = quota.probe().await.unwrap();
    assert_eq!(probed.plan, Some(plan("pro")));
    assert_eq!(ids_and_shares(&probed.windows), [("requests".into(), Some(5.0)), ("weekly".into(), Some(40.0))]);
    // A response the source cannot read changes nothing.
    observe(&quota);
    assert_eq!(quota.latest().unwrap(), probed);
    let again = quota.probe().await.unwrap();
    assert_eq!((again.plan.as_ref(), again.windows.len()), (None, 2));
}

#[tokio::test]
async fn a_family_that_cannot_probe_for_free_is_never_probed() {
    for (cost, error) in [(None, QuotaError::Unsupported), (Some(ProbeCost::Inference), QuotaError::RequiresInference)] {
        let quota = quota(Scripted {
            cost,
            probes: Mutex::new(Vec::new()),
            observations: Mutex::new(Vec::new()),
        });
        assert_eq!(quota.probe().await, Err(error));
        assert!(quota.latest().is_none());
    }
}

#[test]
fn shares_severities_and_reset_times_read_in_their_declared_units() {
    assert_eq!(clamp_used_percent(150.0), Some(100.0));
    assert_eq!(clamp_used_percent(-1.0), Some(0.0));
    assert_eq!(clamp_used_percent(35.5), Some(35.5));
    assert_eq!(clamp_used_percent(f64::NAN), None);
    assert_eq!(used_percent_from_ratio(25.0, 100.0), Some(25.0));
    assert_eq!(used_percent_from_ratio(1.0, 0.0), None);
    let severities = [(Some(50.0), Some(QuotaSeverity::Normal)), (Some(80.0), Some(QuotaSeverity::Warning)), (Some(90.0), Some(QuotaSeverity::Warning)), (Some(95.0), Some(QuotaSeverity::Critical)), (None, None)];
    for (share, expected) in severities {
        assert_eq!(severity(share), expected, "{share:?}");
    }

    let time = |text: &str| text.parse::<Timestamp>().unwrap();
    assert_eq!(unix_seconds(1_700_000_000.0), Some(time("2023-11-14T22:13:20.000Z")));
    assert_eq!(unix_seconds(1_700_000_000.5), Some(time("2023-11-14T22:13:20.500Z")));
    assert_eq!(unix_seconds(1e20), None);
    assert_eq!(unix_seconds(f64::NAN), None);
    assert_eq!(rfc3339("2026-08-21T10:45:24.951512+00:00"), Some(time("2026-08-21T10:45:24.951Z")));
    assert_eq!(rfc3339("2026-08-01T00:00:00Z"), Some(time("2026-08-01T00:00:00.000Z")));
    assert_eq!(rfc3339("soon"), None);
    assert_eq!(rfc3339("1790062659"), None);

    let mut headers = HeaderMap::new();
    for (name, value) in [("x-used", " 35.5 "), ("x-word", "full"), ("x-blank", ""), ("x-infinite", "inf")] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    assert_eq!(header_number(&headers, "x-used"), Some(35.5));
    for name in ["x-word", "x-blank", "x-infinite", "x-absent"] {
        assert_eq!(header_number(&headers, name), None, "{name}");
    }
}
