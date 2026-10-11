<div align="center">
  <img src="assets/logo.png" alt="Graphical Shell Logo" height="130">
</div>
<p align="center">
	<a href="https://github.com/WilliamRagstad/gsh/actions"><img src="https://img.shields.io/github/actions/workflow/status/WilliamRagstad/gsh/rust.yml?style=flat-square&color=6b0" alt="Build Status"></a>
    <img src="https://img.shields.io/badge/built_with-Rust-cf6242.svg?style=flat-square" alt="Built with Rust">
    <a href="https://github.com/WilliamRagstad/gsh/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-009faf.svg?style=flat-square" alt="License"></a>
	<br>
    <a href="https://github.com/WilliamRagstad/gsh/releases/latest"><img src="https://img.shields.io/github/v/release/WilliamRagstad/gsh?color=df00a0&include_prereleases&label=client&sort=semver&style=flat-square" alt="Client Version"></a>
	<a href="https://github.com/WilliamRagstad/gsh/releases/latest"><img src="https://img.shields.io/github/downloads/WilliamRagstad/gsh/total?color=6b0&label=client%20dls&style=flat-square" alt="Client Downloads"></a>
	&nbsp;
	<a href="https://crates.io/crates/libgsh"><img src="https://img.shields.io/crates/v/libgsh?color=df00a0&label=libgsh&style=flat-square" alt="Rust Crate Version"></a>
	<a href="https://crates.io/crates/libgsh"><img src="https://img.shields.io/crates/d/libgsh?color=6b0&label=libgsh%20dls&style=flat-square" alt="Rust Crate Downloads"></a>
</p>
<br>

## Graphical Shell

Graphical Shell streams server-rendered graphics to the `gsh` client and sends user input back to the server. Build services with [`libgsh`](libgsh/README.md), using TLS 1.3 with optional password or public-key authentication.

- [Install the client](client/README.md#quick-install)
- [Build a server](libgsh/README.md)
- [API reference](https://docs.rs/libgsh/latest/libgsh/)
- [Examples](examples/) · [Benchmarks](benches/) · [Community](COMMUNITY.md)

![Architecture](assets/graph.png)
