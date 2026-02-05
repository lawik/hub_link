# hub_link

A minimal NervesHub OTA firmware update client written in Rust. Connects to a NervesHub server instance over WebSocket (Phoenix Channels) and receives firmware updates, downloading and applying them via `fwup`.

## Building

```
cargo build --release
```

The resulting binary is at `target/release/hub_link`.

## Running

```
hub_link /path/to/config.toml
```

If no path is given, it defaults to `/etc/hub_link/config.toml`.

Logging is controlled via the `RUST_LOG` environment variable:

```
RUST_LOG=debug hub_link config.toml
```

## Configuration

Configuration is a TOML file. See `examples/` for complete samples.

### Common fields

| Field | Required | Default | Description |
|-------|----------|---------|-------------|
| `host` | yes | | Server hostname (e.g. `devices.nervescloud.com`) |
| `serial_number` | * | | Static device serial number |
| `serial_number_command` | * | | Shell command that prints the serial number |
| `fwup_devpath` | no | `/dev/mmcblk0` | Block device for fwup to write to |
| `fwup_task` | no | `upgrade` | fwup task name |
| `heartbeat_interval_secs` | no | `30` | Seconds between heartbeats |
| `data_dir` | no | `/tmp/hub_link` | Directory for temporary firmware downloads |
| `device_api_version` | no | `2.3.0` | API version reported to the server |

\* One of `serial_number` or `serial_number_command` is required.

### Firmware metadata

The `[firmware]` section describes the currently running firmware:

```toml
[firmware]
uuid = "aaaa-bbbb-cccc"
version = "1.0.0"
platform = "rpi4"
architecture = "arm"
product = "my-product"
```

All fields are required.

### Authentication

Two methods are supported. Both connect to the same `/device-socket/websocket` endpoint.

#### Shared Secret

```toml
[auth]
type = "shared_secret"
key = "device-key-identifier"
secret = "the-shared-secret"
```

The `key` is the identifier registered with NervesHub. The `secret` is the corresponding shared secret. These are used to generate HMAC-signed headers on each connection.

#### mTLS

```toml
[auth]
type = "mtls"
cert_path = "/etc/hub_link/device-cert.pem"
key_path = "/etc/hub_link/device-key.pem"
ca_cert_path = "/etc/hub_link/ca.pem"
```

The device presents its client certificate during the TLS handshake. The server validates it against the CA chain.

### Serial number

The serial number identifies the device to the server. It can be set directly:

```toml
serial_number = "device-001"
```

Or read from a command:

```toml
serial_number_command = "cat /sys/firmware/devicetree/base/serial-number"
```

If both are set, `serial_number` takes priority.

## Behavior

On startup, hub_link:

1. Reads the config file
2. Resolves the device serial number
3. Connects to the server via WebSocket
4. Joins the `device:{serial}` channel with firmware metadata
5. Sends heartbeats every 30 seconds
6. Listens for `update` events containing a firmware URL
7. Downloads the firmware to `data_dir`
8. Applies it with `fwup -a -d {devpath} -i firmware.fw -t {task}`
9. Reports completion to the server

On disconnect, it reconnects with exponential backoff (1s to 60s with jitter).

## Requirements

- `fwup` must be available on `PATH` for firmware application
