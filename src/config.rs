use serde::Deserialize;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("missing required field: {0}")]
    Missing(&'static str),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthConfig {
    Mtls {
        cert_path: PathBuf,
        key_path: PathBuf,
        ca_cert_path: PathBuf,
    },
    SharedSecret {
        key: String,
        secret: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
pub struct FirmwareMetadata {
    pub uuid: String,
    pub version: String,
    pub platform: String,
    pub architecture: String,
    pub product: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub host: String,
    pub auth: AuthConfig,
    pub serial_number_command: Option<String>,
    pub serial_number: Option<String>,
    pub fwup_devpath: Option<String>,
    pub fwup_task: Option<String>,
    pub firmware: FirmwareMetadata,
    pub heartbeat_interval_secs: Option<u64>,
    pub data_dir: Option<PathBuf>,
    pub device_api_version: Option<String>,
}

impl Config {
    pub fn from_file(path: &std::path::Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_str(&content)
    }

    pub fn from_str(content: &str) -> Result<Self, ConfigError> {
        let config: Config = toml::from_str(content)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.host.is_empty() {
            return Err(ConfigError::Missing("host"));
        }
        if self.serial_number.is_none() && self.serial_number_command.is_none() {
            return Err(ConfigError::Missing(
                "either serial_number or serial_number_command",
            ));
        }
        Ok(())
    }

    pub fn socket_url(&self) -> String {
        format!("wss://{}/device-socket/websocket", self.host)
    }

    pub fn heartbeat_interval_secs(&self) -> u64 {
        self.heartbeat_interval_secs.unwrap_or(30)
    }

    pub fn fwup_devpath(&self) -> &str {
        self.fwup_devpath.as_deref().unwrap_or("/dev/mmcblk0")
    }

    pub fn fwup_task(&self) -> &str {
        self.fwup_task.as_deref().unwrap_or("upgrade")
    }

    pub fn device_api_version(&self) -> &str {
        self.device_api_version.as_deref().unwrap_or("2.3.0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mtls_config() {
        let toml = r#"
host = "devices.nerves-hub.org"
serial_number = "device-1234"

[auth]
type = "mtls"
cert_path = "/etc/hub_link/cert.pem"
key_path = "/etc/hub_link/key.pem"
ca_cert_path = "/etc/hub_link/ca.pem"

[firmware]
uuid = "aaaa-bbbb"
version = "1.0.0"
platform = "rpi4"
architecture = "arm"
product = "my-product"
"#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.host, "devices.nerves-hub.org");
        assert_eq!(
            config.socket_url(),
            "wss://devices.nerves-hub.org/device-socket/websocket"
        );
        assert!(matches!(config.auth, AuthConfig::Mtls { .. }));
        assert_eq!(config.firmware.uuid, "aaaa-bbbb");
    }

    #[test]
    fn parse_shared_secret_config() {
        let toml = r#"
host = "devices.nerves-hub.org"
serial_number = "device-1234"

[auth]
type = "shared_secret"
key = "my-key"
secret = "super-secret"

[firmware]
uuid = "aaaa-bbbb"
version = "1.0.0"
platform = "rpi4"
architecture = "arm"
product = "my-product"
"#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(
            config.socket_url(),
            "wss://devices.nerves-hub.org/device-socket/websocket"
        );
        assert!(matches!(config.auth, AuthConfig::SharedSecret { .. }));
    }

    #[test]
    fn missing_host_fails() {
        let toml = r#"
host = ""
serial_number = "device-1234"

[auth]
type = "shared_secret"
key = "k"
secret = "s"

[firmware]
uuid = "u"
version = "v"
platform = "p"
architecture = "a"
product = "pr"
"#;
        assert!(Config::from_str(toml).is_err());
    }

    #[test]
    fn missing_serial_fails() {
        let toml = r#"
host = "example.com"

[auth]
type = "shared_secret"
key = "k"
secret = "s"

[firmware]
uuid = "u"
version = "v"
platform = "p"
architecture = "a"
product = "pr"
"#;
        assert!(Config::from_str(toml).is_err());
    }

    #[test]
    fn defaults() {
        let toml = r#"
host = "example.com"
serial_number = "dev-1"

[auth]
type = "shared_secret"
key = "k"
secret = "s"

[firmware]
uuid = "u"
version = "v"
platform = "p"
architecture = "a"
product = "pr"
"#;
        let config = Config::from_str(toml).unwrap();
        assert_eq!(config.heartbeat_interval_secs(), 30);
        assert_eq!(config.fwup_devpath(), "/dev/mmcblk0");
        assert_eq!(config.fwup_task(), "upgrade");
        assert_eq!(config.device_api_version(), "2.3.0");
    }
}
