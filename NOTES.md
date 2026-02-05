# Hub Link - Implementation Notes

## Plan

- [x] Look through the nerves_hub_link project and build CLIENT.md
- [x] mTLS websocket connection
- [x] Alternate Shared Secret connection
- [x] Firmware URL delivery over the WebSocket
- [x] Pull device serial number by running a configured command
- [x] Configuration to select connection method, server URI and other config
- [x] Basic firmware metadata reporting for device
- [x] Apply firmware using fwup
- [x] Run as daemon

## Architecture

```
src/
  main.rs          - Entry point, daemon mode with reconnection
  config.rs        - Configuration (TOML file parsing)
  channel.rs       - Phoenix Channels protocol
  auth/
    mod.rs         - Auth module
    mtls.rs        - mTLS TLS config builder (cert/key/CA loading)
    shared_secret.rs - Shared Secret HMAC auth (PBKDF2 + HMAC-SHA256)
  client.rs        - NervesHub device client (connect, join, handle events, update flow)
  firmware.rs      - Firmware download (reqwest streaming) and fwup apply
  serial.rs        - Serial number retrieval (static or shell command)
```

## Dependencies

- tokio: async runtime
- tokio-tungstenite: WebSocket client
- rustls + rustls-pemfile + tokio-rustls: TLS with mTLS support
- serde + serde_json: JSON serialization
- reqwest: HTTPS firmware download with streaming
- hmac + sha2 + pbkdf2: Shared Secret crypto
- base64: encoding
- toml: config file parsing
- tracing + tracing-subscriber: structured logging
- thiserror: error types
- rand: jitter for backoff

## Tests (39 passing)

- config: TOML parsing, validation, defaults, both auth types
- channel: message parsing, building (join/heartbeat/push), roundtrip, error cases
- shared_secret: algorithm string, header generation, determinism, differentiation
- mtls: file loading error cases (missing, empty)
- serial: static, command, priority, whitespace, errors
- firmware: update message parsing, progress calculation
- client: creation, join payload with metadata
- main: backoff delay behavior

## Notes

- Both auth methods use same endpoint: /device-socket/websocket
- Phoenix Channels messages are JSON arrays: [join_ref, ref, topic, event, payload]
- Heartbeat interval: 30 seconds (configurable)
- Reconnect with exponential backoff: 1s -> 60s with 50% jitter
- Shared Secret signature has 90 second validity window
- Firmware downloads are full (no resume), applied via fwup CLI
- Progress reported every 5% increment
