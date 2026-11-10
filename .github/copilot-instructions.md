# Copilot Instructions for Vanta

## Build, test, and lint

Run these commands from the repository root:

```sh
cargo build --workspace
cargo build -p vanta-comp
cargo test --workspace
cargo test -p vanta-ipc validation_tests::rejects_empty_intent
cargo fmt --all -- --check
cargo clippy --workspace --all-targets
```
