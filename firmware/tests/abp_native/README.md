# ABP ESP-IDF native test

This host-native test compiles the ESP-IDF `abp` component with a system C11 compiler. It reads the exact same legal and invalid binary corpus as the Rust/macOS tests, checks byte-for-byte round trips, verifies rejection does not modify caller-visible state, exhaustively checks Control/ID/Phase/value boundaries, and exercises deterministic random frames.

Run from the repository root:

```sh
protocol/test-native.sh
```

No ESP32-S3, USB connection, or flashing is required for I01.
