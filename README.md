<div align="center">
  <img src="assets/logo.png" alt="Graphical Shell Logo" height="130">
</div>
<p align="center">
	<a href="https://github.com/WilliamRagstad/gsh/actions"><img src="https://img.shields.io/github/actions/workflow/status/WilliamRagstad/gsh/rust.yml?style=flat-square&color=6b0" alt="Build Status"></a>
    <img src="https://img.shields.io/badge/built_with-Rust-dca282.svg?style=flat-square" alt="Built with Rust">
    <a href="https://github.com/WilliamRagstad/gsh/releases/latest"><img src="https://img.shields.io/github/v/release/WilliamRagstad/gsh?color=%23ff00a0&include_prereleases&label=client&sort=semver&style=flat-square" alt="Client Version"></a>
	<a href="https://crates.io/crates/libgsh"><img src="https://img.shields.io/crates/v/libgsh?color=6b0&label=libgsh&style=flat-square" alt="Rust Crate Version"></a>
	<a href="https://crates.io/crates/libgsh"><img src="https://img.shields.io/crates/d/libgsh?color=6b0&label=libgsh%20downloads&style=flat-square" alt="Rust Crate Downloads"></a>
    <a href="https://github.com/WilliamRagstad/gsh/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-00bfff.svg?style=flat-square" alt="License"></a>
</p>
<br>

## What is **Graphical Shell**?

It is a versatile framework designed to empower developers and enthusiasts to create custom graphical server interfaces, applications, services and experiences.
Whether you're building a personal server, a graphical Bulletin Board System (BBS), or exploring new interactive experiences, Graphical Shell provides the tools to bring your ideas to life.

It shippes with a SSH-like client application (`gsh`) that allows users to connect to a server and interact with it through a graphical interface.
All rendering and graphical elements are handled by the server, while the client seamlessly streams user interactions and input to the server for processing.

> ## [Client Application](client/README.md)
>
> The client application is a cross-platform native window that interfaces with graphical server applications in a seamless and intuitive manner.
>
> See latest release binaries for `Linux`, `MacOS`, and `Windows` in the [releases](https://github.com/WilliamRagstad/gsh/releases).

## Features

- **Interactivity**: Integrates graphical elements, allowing users interact with server applications and services.

- **Customizable**: Developers can create their own server applications using the provided library, enabling tailored experiences for specific use cases.

- **Cross-Platform Compatibility**: Client application on Linux, macOS, and Windows, ensuring a consistent experience across different operating systems.

- **Security**: TLS 1.3 encryption ensures secure communication between the client and server, protecting sensitive data and user interactions.

### What are you gonna build? ✨

- ### [Get Started](libgsh/README.md) 🔨

- ### [Examples](examples/) 🎓

- ### [Community](COMMUNITY.md) 👪

## Contributing

If you want to contribute to the development of **gsh**, follow these steps:

- Install [Rust](https://www.rust-lang.org/tools/install)
- Install [Visual Studio 2022](https://visualstudio.microsoft.com/downloads/) version 17
  - In the "Workloads" tab enable "Desktop development with C++"
  - Click Modify at the bottom right
- Install [`protoc`](https://github.com/protocolbuffers/protobuf/releases/) version 30.2 *(`shared` dependency)*
- Install [`cmake`](https://cmake.org/download/) version 3.31.7 *(`sdl2` dependency)*
- Clone this repository and `cd gsh`

Manual system test for the `colors` example:

1. In `examples/colors/`: `cargo run -q`
2. In `client/`: `cargo run -q -- localhost --insecure`
