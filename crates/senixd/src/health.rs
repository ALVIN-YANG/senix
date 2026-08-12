use std::{collections::HashMap, sync::Arc, time::Duration};

use senix_core::{GatewayRuntime, HealthCheckProtocol, HealthState, HealthTarget};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
    task::JoinSet,
    time::{Instant, sleep, timeout},
};
use tracing::{debug, warn};

const SCAN_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Debug)]
struct ProbeTracker {
    target: HealthTarget,
    next_due: Instant,
    consecutive_successes: u32,
    consecutive_failures: u32,
}

impl ProbeTracker {
    fn new(target: HealthTarget, now: Instant) -> Self {
        Self {
            target,
            next_due: now,
            consecutive_successes: 0,
            consecutive_failures: 0,
        }
    }

    fn observe(&mut self, succeeded: bool) -> Option<HealthState> {
        if succeeded {
            self.consecutive_failures = 0;
            self.consecutive_successes = self.consecutive_successes.saturating_add(1);
            (self.consecutive_successes >= self.target.check.healthy_threshold)
                .then_some(HealthState::Healthy)
        } else {
            self.consecutive_successes = 0;
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
            (self.consecutive_failures >= self.target.check.unhealthy_threshold)
                .then_some(HealthState::Unhealthy)
        }
    }
}

pub async fn run(runtime: Arc<GatewayRuntime>) {
    let mut trackers: HashMap<String, ProbeTracker> = HashMap::new();

    loop {
        let now = Instant::now();
        let targets = runtime.health_targets();
        trackers.retain(|id, _| targets.iter().any(|target| target.id == *id));
        for target in targets {
            match trackers.get_mut(&target.id) {
                Some(tracker) if tracker.target == target => {}
                Some(tracker) => *tracker = ProbeTracker::new(target, now),
                None => {
                    trackers.insert(target.id.clone(), ProbeTracker::new(target, now));
                }
            }
        }

        let mut probes = JoinSet::new();
        for tracker in trackers.values_mut() {
            if now < tracker.next_due {
                continue;
            }
            tracker.next_due = now + Duration::from_millis(tracker.target.check.interval_ms);
            let target = tracker.target.clone();
            probes.spawn(async move {
                let succeeded = probe(&target).await;
                (target, succeeded)
            });
        }

        while let Some(result) = probes.join_next().await {
            let Ok((target, succeeded)) = result else {
                warn!("health probe task failed");
                continue;
            };
            let Some(tracker) = trackers.get_mut(&target.id) else {
                continue;
            };
            if tracker.target != target {
                continue;
            }
            let Some(health) = tracker.observe(succeeded) else {
                continue;
            };
            if let Err(error) = runtime.report_health(&target.id, health) {
                debug!(instance = %target.id, %error, "discarded health result for old snapshot");
            }
        }

        sleep(SCAN_INTERVAL).await;
    }
}

async fn probe(target: &HealthTarget) -> bool {
    let probe = async {
        match target.check.protocol {
            HealthCheckProtocol::Tcp => TcpStream::connect(target.address).await.map(|_| ()),
            HealthCheckProtocol::Http => http_probe(target).await,
        }
    };
    timeout(Duration::from_millis(target.check.timeout_ms), probe)
        .await
        .is_ok_and(|result| result.is_ok())
}

async fn http_probe(target: &HealthTarget) -> std::io::Result<()> {
    let mut stream = TcpStream::connect(target.address).await?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        target.check.path, target.address
    );
    stream.write_all(request.as_bytes()).await?;

    let mut status_line = String::new();
    BufReader::new(stream).read_line(&mut status_line).await?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok());
    if status.is_some_and(|status| (200..400).contains(&status)) {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "unhealthy HTTP status line: {status_line:?}"
        )))
    }
}
