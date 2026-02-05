use thiserror::Error;

#[derive(Debug, Error)]
pub enum SerialError {
    #[error("serial number command failed: {0}")]
    CommandFailed(String),
    #[error("no serial number configured")]
    NotConfigured,
}

/// Resolve the device serial number from config.
/// If a static serial_number is set, use it directly.
/// Otherwise, run serial_number_command and use its stdout.
pub fn resolve_serial(
    serial_number: Option<&str>,
    serial_number_command: Option<&str>,
) -> Result<String, SerialError> {
    if let Some(sn) = serial_number {
        return Ok(sn.to_string());
    }

    if let Some(cmd) = serial_number_command {
        return run_serial_command(cmd);
    }

    Err(SerialError::NotConfigured)
}

fn run_serial_command(cmd: &str) -> Result<String, SerialError> {
    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .output()
        .map_err(|e| SerialError::CommandFailed(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SerialError::CommandFailed(format!(
            "exit code {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    let serial = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if serial.is_empty() {
        return Err(SerialError::CommandFailed(
            "command produced empty output".to_string(),
        ));
    }

    Ok(serial)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn static_serial() {
        let result = resolve_serial(Some("device-1234"), None).unwrap();
        assert_eq!(result, "device-1234");
    }

    #[test]
    fn static_serial_takes_priority() {
        let result = resolve_serial(Some("static"), Some("echo dynamic")).unwrap();
        assert_eq!(result, "static");
    }

    #[test]
    fn command_serial() {
        let result = resolve_serial(None, Some("echo test-serial-42")).unwrap();
        assert_eq!(result, "test-serial-42");
    }

    #[test]
    fn command_strips_whitespace() {
        let result = resolve_serial(None, Some("echo '  spaced  '")).unwrap();
        assert_eq!(result, "spaced");
    }

    #[test]
    fn failing_command() {
        let result = resolve_serial(None, Some("false"));
        assert!(result.is_err());
    }

    #[test]
    fn empty_output_fails() {
        let result = resolve_serial(None, Some("printf ''"));
        assert!(result.is_err());
    }

    #[test]
    fn no_config_fails() {
        let result = resolve_serial(None, None);
        assert!(result.is_err());
    }
}
