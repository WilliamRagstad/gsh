# Repository guidance

- Always read the documents in `llm/` first (if any).
- Validate workspace changes with `cargo build --workspace` and `cargo test --workspace`.
- Linux builds require the native development dependencies listed in `.github/workflows/rust.yml`, including those used by the remote-desktop example.
