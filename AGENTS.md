# Repository guidance

- Do not add or restore a "No AI" badge or banner in the README. Its removal is intentional.
- Validate workspace changes with `cargo build --workspace` and `cargo test --workspace`.
- Linux builds require the native development dependencies listed in `.github/workflows/rust.yml`, including those used by the remote-desktop example.
