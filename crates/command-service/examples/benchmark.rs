//! Reproducible synthetic transport benchmark; no model or product backend.
use bytes::Bytes;
use demi_command_service::{
    Client, Handler, InvocationContext, ServiceError,
    protocol::{Completion, Invocation, Record},
    serve_stdio,
};
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{process::Command, task::JoinSet};
use tokio_util::sync::CancellationToken;

type Error = Box<dyn std::error::Error + Send + Sync>;

struct Fixture;
impl Handler for Fixture {
    fn operations(&self) -> Vec<String> {
        vec!["echo".into(), "flood".into()]
    }
    fn invoke(
        &self,
        mut ctx: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async move {
            if ctx.request.operation == "flood" {
                let chunk = Bytes::from(vec![0xa5; 64 * 1024]);
                loop {
                    ctx.output.stdout(chunk.clone()).await?;
                }
            }
            while let Some(chunk) = ctx.input.next().await? {
                ctx.output.stdout(chunk).await?;
            }
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

fn request(operation: &str) -> Invocation {
    Invocation {
        edits: None,
        operation: operation.into(),
        invocation_id: "benchmark".into(),
        args: serde_json::json!({}),
        cwd: std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        env: BTreeMap::new(),
    }
}

async fn echo(client: &Client, size: usize, delay: Duration) -> Result<(), Error> {
    let (mut input, mut output) = client.invoke(&request("echo")).await?;
    let chunk = Bytes::from(vec![0xa5; size.min(64 * 1024)]);
    let mut sent = 0;
    let mut received = 0;
    let mut completed = false;
    while let Some(record) = output.next().await? {
        match record {
            Record::InputPull if sent < size => {
                let bytes = chunk.slice(..(size - sent).min(chunk.len()));
                sent += bytes.len();
                input.write(bytes).await?;
            }
            Record::InputPull => input.end()?,
            Record::Stdout(bytes) => {
                assert!(bytes.iter().all(|byte| *byte == 0xa5));
                received += bytes.len();
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
            }
            Record::Completion(result) => {
                assert_eq!(result.exit_code, 0);
                completed = true;
            }
            Record::Stderr(_) => panic!("unexpected stderr"),
        }
    }
    assert!(completed);
    assert_eq!(received, size);
    Ok(())
}

fn rss(pid: u32) -> Option<usize> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = text.lines().find(|line| line.starts_with("VmRSS:"))?;
    line.split_whitespace()
        .nth(1)?
        .parse::<usize>()
        .ok()
        .map(|kb| kb * 1024)
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<(), Error> {
    if std::env::args().nth(1).as_deref() == Some("--service") {
        serve_stdio(Arc::new(Fixture)).await?;
        return Ok(());
    }
    let start = Instant::now();
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--service")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;
    let child_pid = child.id().unwrap();
    let transport = tokio::io::join(child.stdout.take().unwrap(), child.stdin.take().unwrap());
    let (client, connection) = Client::connect(transport).await?;
    let driver = tokio::spawn(connection);
    client.info().await?;
    let startup_us = start.elapsed().as_micros();
    let stop = CancellationToken::new();
    let sample_stop = stop.clone();
    let peak = Arc::new(AtomicUsize::new(0));
    let sampled_peak = peak.clone();
    let sampler = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(10));
        loop {
            tokio::select! {
                _ = sample_stop.cancelled() => break,
                _ = interval.tick() => {
                    if let (Some(parent), Some(child)) = (rss(std::process::id()), rss(child_pid)) {
                        sampled_peak.fetch_max(parent + child, Ordering::Relaxed);
                    }
                }
            }
        }
    });
    let result = tokio::time::timeout(Duration::from_secs(60), async {
        for _ in 0..20 {
            echo(&client, 32, Duration::ZERO).await?;
        }
        let mut samples = Vec::new();
        for _ in 0..1000 {
            let start = Instant::now();
            echo(&client, 32, Duration::ZERO).await?;
            samples.push(start.elapsed().as_micros());
        }
        samples.sort_unstable();
        let concurrent = Instant::now();
        let mut calls = JoinSet::new();
        for _ in 0..16 {
            let client = client.clone();
            calls.spawn(async move {
                for _ in 0..100 {
                    echo(&client, 32, Duration::ZERO).await?;
                }
                Ok::<_, Error>(())
            });
        }
        while let Some(result) = calls.join_next().await {
            result??;
        }
        let calls_per_second = 1600.0 / concurrent.elapsed().as_secs_f64();
        let large = Instant::now();
        echo(&client, 8 * 1024 * 1024, Duration::ZERO).await?;
        let mib_per_second = 8.0 / large.elapsed().as_secs_f64();
        let slow = Instant::now();
        echo(&client, 8 * 1024 * 1024, Duration::from_millis(2)).await?;
        let slow_ms = slow.elapsed().as_millis();
        let (mut input, output) = client.invoke(&request("flood")).await?;
        input.end()?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let independent = Instant::now();
        echo(&client, 32, Duration::ZERO).await?;
        let blocked_peer_call_us = independent.elapsed().as_micros();
        let cancellation = Instant::now();
        input.cancel();
        drop(input);
        drop(output);
        // Reap the service after shutdown so the measurement includes handler
        // and process release, not just sending RST_STREAM or a shutdown reply.
        client.shutdown().await?;
        assert!(child.wait().await?.success());
        let cancel_and_shutdown_us = cancellation.elapsed().as_micros();
        Ok::<_, Error>(serde_json::json!({
            "startup_us": startup_us, "warm_calls": 1000,
            "warm_p50_us": samples[500], "warm_p95_us": samples[950],
            "concurrency": 16, "concurrent_calls_per_second": calls_per_second,
            "binary_mib_per_second": mib_per_second,
            "slow_consumer_8mib_ms": slow_ms,
            "blocked_peer_call_us": blocked_peer_call_us,
            "cancel_and_shutdown_us": cancel_and_shutdown_us,
        }))
    })
    .await;
    stop.cancel();
    sampler.await?;
    drop(client);
    match result {
        Ok(Ok(mut metrics)) => {
            driver.await??;
            metrics["peak_combined_rss_bytes"] = match peak.load(Ordering::Relaxed) {
                0 => serde_json::Value::Null,
                bytes => bytes.into(),
            };
            println!("{}", serde_json::to_string_pretty(&metrics)?);
            Ok(())
        }
        failure => {
            child.kill().await?;
            child.wait().await?;
            driver.abort();
            let _cancelled_driver = driver.await;
            Err(format!("benchmark failed: {failure:?}").into())
        }
    }
}
