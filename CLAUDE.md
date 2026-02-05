This project you are responsible for executing is a reimplementation of nerves_hub_link, the client library for the open source OTA and firmware update service NervesHub.

You can find nerves_hub_link for reference at ../nerves_hub_link in the directory structure. You can find nerves_hub_web, the server implementation, at ../nerves_hub_web.

The goal is a minimal client that can be run as daemon to perform an mTLS connection or shared secret connection to a NervesHub server instance and receive firmware updates.

Console, extensions, health and geo are NOT IN SCOPE.

Reporting basic metadata is IN SCOPE.

The swappable client implementations and in-depth config are NOT IN SCOPE.

Configurable implementations we need:

mTLS with device certificate:
This can take a local OpenSSL private key reference and whatever else is necessary. Whether an HSM like an OpenSSL engine from cryptoauthlib or simply a local file.

Shared Secret:
Matching implementation for the Shared Secret existing in current nerves_hub_link.

General config:

- URI for the NervesHub service

We implement this is Rust to make a small performant cross-compileable binary.

Support for firmware formats and protocols:

Initially we only support fwup. We download via HTTPS expecting a pre-signed object storage link in the style of S3. We download it fully. Then we apply it using the fwup CLI tool that is assumed to be available through other means. This is intended to run on highly specific embedded devices.

Download restarts are done from scratch. Resuming is NOT IN SCOPE. Streaming download and apply is NOT IN SCOPE.

nerves_hub_link's protocol is Phoenix Channels, a WebSocket based JSON mechanism. We do not use the existing implementation of Phoenix Channels that exist in Rust. On last test they were outdated and suffered from fairly complex dependencies.

We are likely using Tokio, relevant websocket libraries and we need support for mTLS which probably involves some additional crypto libraries.

I don't know Rust, so I'll need clarifications on some choices.

Copy this plan to NOTES.md and mark progress and make notes there.

## Plan

[] Look through the nerves_hub_link project and build a project document explaining the parts we actually need. Call it CLIENT.md.

Reference CLIENT.md to build basic tests to cover the fundamental functionality we intend to have. Then build the implementation to make the test work. Don't start by building a daemon. Just implementation code and tests for these features:

  [] mTLS websocket connection.
  [] Alternate Shared Secret connection.
  [] Firmware URL delivery over the WebSocket.
  [] Pull device serial number by running a configured command.
  [] Configuration to select connection method, server URI and other config.
  [] Baisc firmware metadata reporting for device.
  [] Apply firmware using fwup.
  [] Run as daemon.

