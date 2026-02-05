use crate::auth::shared_secret::SharedSecretAuth;
use crate::channel::{ChannelBuilder, Message};
use crate::config::{AuthConfig, Config};
use crate::firmware::{self, UpdateInfo};
use crate::serial;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};
use tracing::{debug, error, info, warn};
use tungstenite::http;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("connection failed: {0}")]
    Connection(String),
    #[error("websocket error: {0}")]
    WebSocket(String),
    #[error("join rejected: {0}")]
    JoinRejected(String),
    #[error("serial number error: {0}")]
    Serial(#[from] serial::SerialError),
    #[error("auth error: {0}")]
    Auth(String),
    #[error("firmware error: {0}")]
    Firmware(#[from] firmware::FirmwareError),
    #[error("channel closed")]
    ChannelClosed,
}

/// Events that the client can emit to the caller.
#[derive(Debug)]
pub enum ClientEvent {
    Connected,
    Joined,
    UpdateAvailable(UpdateInfo),
    FirmwareDownloaded(std::path::PathBuf),
    FirmwareApplied,
    RebootRequested,
    Disconnected(String),
}

/// The NervesHub device client.
pub struct NervesHubClient {
    config: Config,
    serial: String,
}

impl NervesHubClient {
    pub fn new(config: Config) -> Result<Self, ClientError> {
        let serial = serial::resolve_serial(
            config.serial_number.as_deref(),
            config.serial_number_command.as_deref(),
        )?;
        info!(serial = %serial, "resolved device serial number");
        Ok(Self { config, serial })
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Build the join payload with firmware metadata.
    pub fn join_payload(&self) -> serde_json::Value {
        json!({
            "device_api_version": self.config.device_api_version(),
            "nerves_fw_uuid": self.config.firmware.uuid,
            "nerves_fw_version": self.config.firmware.version,
            "nerves_fw_platform": self.config.firmware.platform,
            "nerves_fw_architecture": self.config.firmware.architecture,
            "nerves_fw_product": self.config.firmware.product,
        })
    }

    /// Connect to the NervesHub server and run the event loop.
    /// Sends events through the returned channel.
    pub async fn run(
        &self,
        event_tx: mpsc::Sender<ClientEvent>,
    ) -> Result<(), ClientError> {
        let ws_stream = self.connect().await?;
        let _ = event_tx.send(ClientEvent::Connected).await;

        let (mut write, mut read) = ws_stream.split();

        let topic = format!("device:{}", self.serial);
        let channel = ChannelBuilder::new(topic.clone());

        // Send join
        let join_msg = channel.join(self.join_payload());
        write
            .send(tungstenite::Message::Text(join_msg.to_json().into()))
            .await
            .map_err(|e| ClientError::WebSocket(e.to_string()))?;
        info!(topic = %topic, "sent channel join");

        // Wait for join reply
        let join_reply = Self::wait_for_reply(&mut read, &channel.join_ref).await?;
        if !join_reply.reply_ok() {
            let reason = join_reply
                .payload
                .get("response")
                .and_then(|r| r.get("reason"))
                .and_then(|r| r.as_str())
                .unwrap_or("unknown");
            return Err(ClientError::JoinRejected(reason.to_string()));
        }
        info!("joined device channel");
        let _ = event_tx.send(ClientEvent::Joined).await;

        // Event loop: heartbeat + message handling
        let heartbeat_interval = Duration::from_secs(self.config.heartbeat_interval_secs());
        let mut next_heartbeat = Instant::now() + heartbeat_interval;

        loop {
            tokio::select! {
                msg = read.next() => {
                    match msg {
                        Some(Ok(tungstenite::Message::Text(text))) => {
                            match Message::from_json(&text) {
                                Ok(msg) => {
                                    self.handle_message(msg, &channel, &mut write, &event_tx).await?;
                                }
                                Err(e) => {
                                    warn!(error = %e, "failed to parse message");
                                }
                            }
                        }
                        Some(Ok(tungstenite::Message::Close(_))) | None => {
                            info!("connection closed");
                            let _ = event_tx.send(ClientEvent::Disconnected("connection closed".to_string())).await;
                            return Ok(());
                        }
                        Some(Ok(_)) => {
                            // Ping/Pong/Binary - ignore
                        }
                        Some(Err(e)) => {
                            error!(error = %e, "websocket error");
                            let _ = event_tx.send(ClientEvent::Disconnected(e.to_string())).await;
                            return Err(ClientError::WebSocket(e.to_string()));
                        }
                    }
                }
                _ = tokio::time::sleep_until(next_heartbeat) => {
                    let hb = channel.heartbeat();
                    write
                        .send(tungstenite::Message::Text(hb.to_json().into()))
                        .await
                        .map_err(|e| ClientError::WebSocket(e.to_string()))?;
                    debug!("sent heartbeat");
                    next_heartbeat = Instant::now() + heartbeat_interval;
                }
            }
        }
    }

    async fn connect(
        &self,
    ) -> Result<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        ClientError,
    > {
        let url = self.config.socket_url();
        info!(url = %url, "connecting to NervesHub");

        match &self.config.auth {
            AuthConfig::Mtls {
                cert_path,
                key_path,
                ca_cert_path,
            } => {
                let tls_config =
                    crate::auth::mtls::build_tls_config(cert_path, key_path, ca_cert_path)
                        .map_err(|e| ClientError::Auth(e.to_string()))?;

                let connector =
                    tokio_tungstenite::Connector::Rustls(tls_config);

                let (ws_stream, _response) =
                    tokio_tungstenite::connect_async_tls_with_config(
                        &url,
                        None,
                        false,
                        Some(connector),
                    )
                    .await
                    .map_err(|e| ClientError::Connection(e.to_string()))?;

                Ok(ws_stream)
            }
            AuthConfig::SharedSecret { key, secret } => {
                let auth = SharedSecretAuth::new(key.clone(), secret.clone());
                let headers = auth
                    .auth_headers(&self.serial)
                    .map_err(|e| ClientError::Auth(e.to_string()))?;

                let mut request = http::Request::builder()
                    .uri(&url)
                    .header("Host", &self.config.host);

                for (name, value) in &headers {
                    request = request.header(name, value);
                }

                let request = request
                    .body(())
                    .map_err(|e| ClientError::Connection(e.to_string()))?;

                let (ws_stream, _response) =
                    tokio_tungstenite::connect_async(request)
                        .await
                        .map_err(|e| ClientError::Connection(e.to_string()))?;

                Ok(ws_stream)
            }
        }
    }

    async fn wait_for_reply<S>(
        read: &mut S,
        join_ref: &str,
    ) -> Result<Message, ClientError>
    where
        S: StreamExt<Item = Result<tungstenite::Message, tungstenite::Error>> + Unpin,
    {
        // Wait up to 30 seconds for a join reply
        let timeout = Duration::from_secs(30);
        let deadline = Instant::now() + timeout;

        loop {
            tokio::select! {
                msg = read.next() => {
                    match msg {
                        Some(Ok(tungstenite::Message::Text(text))) => {
                            if let Ok(msg) = Message::from_json(&text) {
                                if msg.is_reply() && msg.msg_ref.as_deref() == Some(join_ref) {
                                    return Ok(msg);
                                }
                            }
                        }
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => return Err(ClientError::WebSocket(e.to_string())),
                        None => return Err(ClientError::ChannelClosed),
                    }
                }
                _ = tokio::time::sleep_until(deadline) => {
                    return Err(ClientError::Connection("join reply timeout".to_string()));
                }
            }
        }
    }

    async fn handle_message<S>(
        &self,
        msg: Message,
        channel: &ChannelBuilder,
        write: &mut S,
        event_tx: &mpsc::Sender<ClientEvent>,
    ) -> Result<(), ClientError>
    where
        S: SinkExt<tungstenite::Message> + Unpin,
        S::Error: std::fmt::Display,
    {
        match msg.event.as_str() {
            "update" => {
                info!("received firmware update");
                match UpdateInfo::from_payload(&msg.payload) {
                    Ok(update_info) => {
                        let _ = event_tx
                            .send(ClientEvent::UpdateAvailable(update_info.clone()))
                            .await;
                        self.handle_update(update_info, channel, write, event_tx)
                            .await?;
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to parse update message");
                    }
                }
            }
            "reboot" => {
                info!("received reboot command");
                // Acknowledge reboot
                let ack = channel.push("rebooting", json!({}));
                let _ = write
                    .send(tungstenite::Message::Text(ack.to_json().into()))
                    .await;
                let _ = event_tx.send(ClientEvent::RebootRequested).await;
            }
            "phx_reply" => {
                debug!(
                    ref_id = ?msg.msg_ref,
                    status = ?msg.reply_status(),
                    "received reply"
                );
            }
            "phx_error" => {
                warn!(topic = %msg.topic, "channel error");
            }
            "phx_close" => {
                info!(topic = %msg.topic, "channel closed by server");
                let _ = event_tx
                    .send(ClientEvent::Disconnected(
                        "channel closed by server".to_string(),
                    ))
                    .await;
                return Err(ClientError::ChannelClosed);
            }
            other => {
                debug!(event = other, "unhandled event");
            }
        }
        Ok(())
    }

    async fn handle_update<S>(
        &self,
        update_info: UpdateInfo,
        channel: &ChannelBuilder,
        write: &mut S,
        event_tx: &mpsc::Sender<ClientEvent>,
    ) -> Result<(), ClientError>
    where
        S: SinkExt<tungstenite::Message> + Unpin,
        S::Error: std::fmt::Display,
    {
        info!(
            uuid = %update_info.firmware_meta.uuid,
            version = %update_info.firmware_meta.version,
            "downloading firmware"
        );

        // Download firmware
        let data_dir = self
            .config
            .data_dir
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/hub_link"));
        tokio::fs::create_dir_all(&data_dir)
            .await
            .map_err(|e| firmware::FirmwareError::Io(e))?;

        let channel_topic = channel.topic.clone();
        let channel_join_ref = channel.join_ref.clone();

        // We need to send progress updates. We'll collect them and send after download.
        let mut last_reported_percent: u8 = 0;
        let (progress_tx, mut progress_rx) = mpsc::channel::<u8>(16);

        let url = update_info.firmware_url.clone();
        let data_dir_clone = data_dir.clone();

        let download_handle = tokio::spawn(async move {
            firmware::download_firmware(&url, &data_dir_clone, |downloaded, total| {
                let pct = firmware::progress_percent(downloaded, total);
                let _ = progress_tx.try_send(pct);
            })
            .await
        });

        // Forward progress while download is running
        loop {
            tokio::select! {
                pct = progress_rx.recv() => {
                    match pct {
                        Some(pct) if pct > last_reported_percent + 4 || pct == 100 => {
                            last_reported_percent = pct;
                            let progress_msg = json!({"value": pct});
                            // Build a push message manually to avoid borrow issues
                            let push = Message {
                                join_ref: Some(channel_join_ref.clone()),
                                msg_ref: Some("0".to_string()), // Progress doesn't need unique refs
                                topic: channel_topic.clone(),
                                event: "fwup_progress".to_string(),
                                payload: progress_msg,
                            };
                            let _ = write.send(tungstenite::Message::Text(push.to_json().into())).await;
                        }
                        Some(_) => {} // Skip small increments
                        None => break, // Channel closed, download done
                    }
                }
            }
        }

        let firmware_path = download_handle
            .await
            .map_err(|e| ClientError::Connection(format!("download task failed: {}", e)))?
            .map_err(ClientError::Firmware)?;

        info!(path = %firmware_path.display(), "firmware downloaded");
        let _ = event_tx
            .send(ClientEvent::FirmwareDownloaded(firmware_path.clone()))
            .await;

        // Apply firmware
        firmware::apply_firmware(
            &firmware_path,
            self.config.fwup_devpath(),
            self.config.fwup_task(),
        )
        .await
        .map_err(ClientError::Firmware)?;

        let _ = event_tx.send(ClientEvent::FirmwareApplied).await;

        // Report completion
        let status_msg = channel.push("status_update", json!({"status": "update-handled"}));
        let _ = write
            .send(tungstenite::Message::Text(status_msg.to_json().into()))
            .await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuthConfig, FirmwareMetadata};

    fn test_config() -> Config {
        Config {
            host: "example.com".to_string(),
            auth: AuthConfig::SharedSecret {
                key: "test-key".to_string(),
                secret: "test-secret".to_string(),
            },
            serial_number: Some("test-device-001".to_string()),
            serial_number_command: None,
            fwup_devpath: None,
            fwup_task: None,
            firmware: FirmwareMetadata {
                uuid: "fw-uuid-123".to_string(),
                version: "1.0.0".to_string(),
                platform: "rpi4".to_string(),
                architecture: "arm".to_string(),
                product: "test-product".to_string(),
            },
            heartbeat_interval_secs: None,
            data_dir: None,
            device_api_version: None,
        }
    }

    #[test]
    fn client_creation() {
        let client = NervesHubClient::new(test_config()).unwrap();
        assert_eq!(client.serial(), "test-device-001");
    }

    #[test]
    fn join_payload_contains_metadata() {
        let client = NervesHubClient::new(test_config()).unwrap();
        let payload = client.join_payload();
        assert_eq!(payload["nerves_fw_uuid"], "fw-uuid-123");
        assert_eq!(payload["nerves_fw_version"], "1.0.0");
        assert_eq!(payload["nerves_fw_platform"], "rpi4");
        assert_eq!(payload["nerves_fw_architecture"], "arm");
        assert_eq!(payload["nerves_fw_product"], "test-product");
        assert_eq!(payload["device_api_version"], "2.3.0");
    }

    #[test]
    fn join_payload_custom_api_version() {
        let mut config = test_config();
        config.device_api_version = Some("2.0.0".to_string());
        let client = NervesHubClient::new(config).unwrap();
        let payload = client.join_payload();
        assert_eq!(payload["device_api_version"], "2.0.0");
    }
}
