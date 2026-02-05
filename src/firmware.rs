use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum FirmwareError {
    #[error("download failed: {0}")]
    Download(String),
    #[error("fwup failed: {0}")]
    Fwup(String),
    #[error("invalid update message: {0}")]
    InvalidMessage(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Parsed firmware update info from the server's "update" event.
#[derive(Debug, Clone, Deserialize)]
pub struct UpdateInfo {
    pub firmware_url: String,
    pub firmware_meta: FirmwareMeta,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct FirmwareMeta {
    pub uuid: String,
    pub version: String,
    pub platform: String,
    pub architecture: String,
    pub product: String,
}

impl UpdateInfo {
    /// Parse an update message payload.
    pub fn from_payload(payload: &Value) -> Result<Self, FirmwareError> {
        serde_json::from_value(payload.clone())
            .map_err(|e| FirmwareError::InvalidMessage(e.to_string()))
    }
}

/// Download firmware from a pre-signed URL to a local file.
/// Returns the path to the downloaded file.
/// Reports progress via a callback: fn(bytes_downloaded, total_bytes_option).
pub async fn download_firmware<F>(
    url: &str,
    dest_dir: &Path,
    mut on_progress: F,
) -> Result<PathBuf, FirmwareError>
where
    F: FnMut(u64, Option<u64>),
{
    let client = reqwest::Client::new();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| FirmwareError::Download(e.to_string()))?;

    if !response.status().is_success() {
        return Err(FirmwareError::Download(format!(
            "HTTP {}",
            response.status()
        )));
    }

    let total_size = response.content_length();
    let dest_path = dest_dir.join("firmware.fw");
    let mut file = tokio::fs::File::create(&dest_path)
        .await
        .map_err(|e| FirmwareError::Io(e))?;

    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| FirmwareError::Download(e.to_string()))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| FirmwareError::Io(e))?;
        downloaded += chunk.len() as u64;
        on_progress(downloaded, total_size);
    }

    file.flush().await.map_err(|e| FirmwareError::Io(e))?;
    info!(downloaded_bytes = downloaded, path = %dest_path.display(), "firmware download complete");

    Ok(dest_path)
}

/// Apply firmware using the fwup CLI tool.
pub async fn apply_firmware(
    firmware_path: &Path,
    devpath: &str,
    task: &str,
) -> Result<(), FirmwareError> {
    info!(
        firmware = %firmware_path.display(),
        devpath,
        task,
        "applying firmware with fwup"
    );

    let output = tokio::process::Command::new("fwup")
        .arg("-a")
        .arg("-d")
        .arg(devpath)
        .arg("-i")
        .arg(firmware_path)
        .arg("-t")
        .arg(task)
        .output()
        .await
        .map_err(|e| FirmwareError::Fwup(format!("failed to execute fwup: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(stderr = %stderr, "fwup failed");
        return Err(FirmwareError::Fwup(format!(
            "fwup exit {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    info!("firmware applied successfully");
    Ok(())
}

/// Calculate progress percentage (0-100).
pub fn progress_percent(downloaded: u64, total: Option<u64>) -> u8 {
    match total {
        Some(total) if total > 0 => ((downloaded * 100) / total).min(100) as u8,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_update_info() {
        let payload = json!({
            "firmware_url": "https://s3.example.com/fw.fw?token=abc",
            "firmware_meta": {
                "uuid": "abc-123",
                "version": "1.1.0",
                "platform": "rpi4",
                "architecture": "arm",
                "product": "my-product"
            }
        });
        let info = UpdateInfo::from_payload(&payload).unwrap();
        assert_eq!(info.firmware_url, "https://s3.example.com/fw.fw?token=abc");
        assert_eq!(info.firmware_meta.uuid, "abc-123");
        assert_eq!(info.firmware_meta.version, "1.1.0");
        assert_eq!(info.firmware_meta.platform, "rpi4");
    }

    #[test]
    fn parse_invalid_update() {
        let payload = json!({"missing": "fields"});
        assert!(UpdateInfo::from_payload(&payload).is_err());
    }

    #[test]
    fn progress_calculation() {
        assert_eq!(progress_percent(0, Some(100)), 0);
        assert_eq!(progress_percent(50, Some(100)), 50);
        assert_eq!(progress_percent(100, Some(100)), 100);
        assert_eq!(progress_percent(200, Some(100)), 100); // clamped
        assert_eq!(progress_percent(50, None), 0);
        assert_eq!(progress_percent(50, Some(0)), 0);
    }
}
