mod auth;
mod channel;
mod client;
mod config;
mod firmware;
mod serial;

use client::{ClientEvent, NervesHubClient};
use config::Config;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

fn backoff_delay(attempt: u32) -> std::time::Duration {
    let base_secs: f64 = (2.0_f64).powi(attempt as i32).min(60.0);
    let jitter = rand::random::<f64>() * base_secs * 0.5;
    std::time::Duration::from_secs_f64(base_secs + jitter)
}

async fn run_daemon(config: Config) -> Result<(), Box<dyn std::error::Error>> {
    let client = NervesHubClient::new(config)?;
    let mut attempt: u32 = 0;

    loop {
        let (event_tx, mut event_rx) = mpsc::channel::<ClientEvent>(32);

        // Spawn event handler
        let event_handle = tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                match event {
                    ClientEvent::Connected => info!("connected to server"),
                    ClientEvent::Joined => info!("joined device channel"),
                    ClientEvent::UpdateAvailable(info) => {
                        info!(
                            uuid = %info.firmware_meta.uuid,
                            version = %info.firmware_meta.version,
                            "firmware update available"
                        );
                    }
                    ClientEvent::FirmwareDownloaded(path) => {
                        info!(path = %path.display(), "firmware downloaded");
                    }
                    ClientEvent::FirmwareApplied => {
                        info!("firmware applied successfully");
                    }
                    ClientEvent::RebootRequested => {
                        info!("reboot requested by server");
                        // In a real deployment, trigger system reboot here
                    }
                    ClientEvent::Disconnected(reason) => {
                        warn!(reason = %reason, "disconnected");
                    }
                }
            }
        });

        match client.run(event_tx).await {
            Ok(()) => {
                info!("connection ended cleanly");
                attempt = 0;
            }
            Err(e) => {
                error!(error = %e, "connection error");
            }
        }

        event_handle.abort();

        let delay = backoff_delay(attempt);
        info!(delay_secs = delay.as_secs_f64(), attempt, "reconnecting");
        tokio::time::sleep(delay).await;
        attempt = attempt.saturating_add(1).min(6); // Cap at ~60s base
    }
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/etc/hub_link/config.toml"));

    let config = match Config::from_file(&config_path) {
        Ok(c) => c,
        Err(e) => {
            error!(path = %config_path.display(), error = %e, "failed to load config");
            std::process::exit(1);
        }
    };

    info!(
        host = %config.host,
        "starting hub_link daemon"
    );

    if let Err(e) = run_daemon(config).await {
        error!(error = %e, "daemon failed");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_delay_increases() {
        let d0 = backoff_delay(0);
        let d1 = backoff_delay(1);
        let d3 = backoff_delay(3);
        // With jitter, we can't assert exact values, but the base increases
        // d0 base=1s, d1 base=2s, d3 base=8s
        // With up to 50% jitter, max is 1.5s, 3s, 12s
        assert!(d0.as_secs_f64() <= 1.5);
        assert!(d1.as_secs_f64() <= 3.0);
        assert!(d3.as_secs_f64() <= 12.0);
    }

    #[test]
    fn backoff_delay_caps() {
        let d10 = backoff_delay(10);
        // Base capped at 60s, with 50% jitter max is 90s
        assert!(d10.as_secs_f64() <= 90.0);
    }
}
