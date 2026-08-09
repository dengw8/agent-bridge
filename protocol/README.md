# ABP v1 Golden Packet corpus

`golden/manifest.json` is the readable source of truth for the 22 legal ABP v1.0 message frames. `fuzz/manifest.json` describes invalid header, payload, enum, range, zero-value, truncation, and oversize cases. The `.bin` files are generated artifacts consumed unchanged by both the Rust/macOS and ESP-IDF C native tests.

Regenerate after an intentional protocol change:

```sh
node protocol/generate.mjs
```

Verify that checked-in artifacts still match their manifests:

```sh
node protocol/generate.mjs --check
protocol/test-native.sh
```

Protocol changes require updating `docs/AGENT_BRIDGE_PROTOCOL_V1.md` first. A breaking change uses a new major corpus rather than silently rewriting v1.0 packets.
