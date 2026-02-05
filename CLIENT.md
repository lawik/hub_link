# NervesHub Client Protocol Reference

This document describes the NervesHub protocol as implemented by `nerves_hub_link`,
covering only the parts relevant to our minimal Rust client.

## Connection

The client connects via WebSocket to a NervesHub server. Two authentication methods
are supported. They use the same endpoint.

- **mTLS**:
- **Shared Secret**:

Endpoint: `wss://{host}/device-socket/websocket`

They both use the same Phoenix Channels protocol after the connection is established.

## Phoenix Channels Protocol

All messages are JSON arrays with 5 elements:

```
[join_ref, ref, topic, event, payload]
```

- `join_ref`: Set during join, null for server pushes
- `ref`: Incrementing message reference (string of integer), null for server broadcasts
- `topic`: Channel topic string
- `event`: Event name string
- `payload`: JSON object

### Lifecycle Events

**Join** (client -> server):
```json
["1", "1", "device:SERIAL", "phx_join", { ...metadata }]
```

**Join Reply** (server -> client):
```json
["1", "1", "device:SERIAL", "phx_reply", {"status": "ok", "response": {}}]
```

**Heartbeat** (client -> server):
```json
[null, "2", "phoenix", "heartbeat", {}]
```

**Heartbeat Reply** (server -> client):
```json
[null, "2", "phoenix", "phx_reply", {"status": "ok", "response": {}}]
```

**Server Push** (server -> client):
```json
[null, null, "device:SERIAL", "update", { ...payload }]
```

**Client Push** (client -> server):
```json
["1", "3", "device:SERIAL", "fwup_progress", { ...payload }]
```

**Error Reply**:
```json
["1", "1", "device:SERIAL", "phx_reply", {"status": "error", "response": {"reason": "..."}}]
```

**Close**:
```json
[null, null, "device:SERIAL", "phx_close", {}]
```

## Authentication

### mTLS

The device presents its client certificate during the TLS handshake. The server
validates the certificate against its CA chain and extracts the device identity
from the certificate.

Configuration needed:
- Device certificate (PEM or DER)
- Device private key (PEM or DER, or OpenSSL engine reference)
- CA certificate chain (PEM)
- Server CA for verification

### Shared Secret

The device connects with HMAC-based authentication headers on the WebSocket upgrade request.

**Headers:**
```
x-nh-alg:       NH1-HMAC-sha256-1000-32
x-nh-key:       <key_identifier>
x-nh-time:      <unix_timestamp_seconds>
x-nh-signature: <hmac_signature>
```

**Algorithm string format:** `NH1-HMAC-{digest}-{iterations}-{key_length}`

**Signature generation:**
1. Build salt string:
   ```
   NH1:device-socket:shared-secret:connect\n\nx-nh-alg={alg}\nx-nh-key={key}\nx-nh-time={timestamp}
   ```
2. Derive signing key using PBKDF2:
   - Secret: the shared secret
   - Salt: the salt string above
   - Iterations: from algorithm (e.g. 1000)
   - Key length: from algorithm (e.g. 32)
   - Digest: from algorithm (e.g. sha256)
3. HMAC-sign the device identifier (serial number) using the derived key
4. Base64-encode the signature

The server verifies the signature and checks that `x-nh-time` is within 90 seconds.

## Device Channel

### Topic

`device:{identifier}` where identifier is the device serial number.

### Join Payload

```json
{
  "device_api_version": "2.3.0",
  "fwup_version": "1.10.1",
  "nerves_fw_uuid": "<current-firmware-uuid>",
  "nerves_fw_version": "<current-firmware-version>",
  "nerves_fw_platform": "<platform>",
  "nerves_fw_architecture": "<architecture>",
  "nerves_fw_product": "<product-name>"
}
```

### Server Events (server -> client)

#### `update` - Firmware Update Available

```json
{
  "firmware_url": "https://s3.example.com/firmware.fw?signed-params",
  "firmware_meta": {
    "uuid": "new-firmware-uuid",
    "version": "1.1.0",
    "platform": "rpi4",
    "architecture": "arm",
    "product": "my-product"
  }
}
```

#### `reboot` - Reboot Command

```json
{}
```

### Client Events (client -> server)

#### `fwup_progress` - Download/Apply Progress

```json
{
  "value": 50
}
```

#### `status_update` - Update Status

```json
{
  "status": "update-rescheduled"
}
```

Valid statuses: `"update-rescheduled"`, `"update-failed"`, `"update-handled"`

#### `rebooting` - Reboot Acknowledgment

```json
{}
```

## Firmware Update Flow

1. Server pushes `update` event with `firmware_url` and `firmware_meta`
2. Client downloads firmware from the pre-signed URL via HTTPS
3. Client reports progress via `fwup_progress` events (0-100)
4. Client writes downloaded firmware to a temporary file
5. Client runs `fwup` CLI to apply the firmware:
   ```
   fwup -a -d /dev/mmcblk0 -i /tmp/firmware.fw -t upgrade
   ```
6. Client reports completion via `status_update`
7. Client may reboot (or wait for server `reboot` command)

## Heartbeat

The client must send heartbeat messages at regular intervals (default: 30 seconds).
If the server doesn't receive a heartbeat within the timeout window, it considers
the device disconnected.

## Reconnection

On disconnect, the client should reconnect with exponential backoff:
- Initial delay: 1 second
- Maximum delay: 60 seconds
- Jitter: up to 50% of delay

For Shared Secret auth, the signature must be regenerated on each connection attempt
(the timestamp changes).

## Device Serial Number

The device serial number is used as the channel topic identifier and (for shared secret)
as the signed data. It can be obtained by running a configurable command (e.g. reading
from a file or running a script).
