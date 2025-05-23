# libgsh&nbsp; <a href="https://crates.io/crates/libgsh"><img src="https://img.shields.io/crates/v/libgsh?color=df00a0&label=libgsh&style=flat-square" alt="Rust Crate Version"></a> <a href="https://crates.io/crates/libgsh"><img src="https://img.shields.io/crates/d/libgsh?color=6b0&label=libgsh%20dls&style=flat-square" alt="Rust Crate Downloads"></a>

This is a SDK library for building server applications that interact seamlessly with the `gsh` client.
It provides essential tools and abstractions to enable efficient communication between the server and the `gsh` graphical shell client application.

## Quick Install

Install the latest release of [`libgsh`](https://crates.io/crates/libgsh) using `cargo`:

```bash
cargo install libgsh
```

## Features

- **Server-Client Communication**: Simplifies the process of establishing and managing connections with the `gsh` client.
- **Data Serialization**: Includes utilities for encoding and decoding data exchanged between the server and client.
- **Extensibility**: Designed to be modular and extensible, allowing developers to build custom server-side functionality.

This library is an integral part of the `gsh` ecosystem, enabling developers to create robust and interactive server applications that leverage the graphical capabilities of the `gsh` client.

## Build a Server

Do you want to build your own service with `gsh`?
Choose between these *out-of-the-box* server implementations:

| Server   | Description                                                    | Technology             |
| -------- | -------------------------------------------------------------- | ---------------------- |
| `async`  | An asynchronous server that handle communication concurrently. | Tokio async runtime    |
| `simple` | A basic server that handle non-blocking communication          | Native multi-threading |

> **Recommendation**\
> The `async` server provides better performance and scalability for most applications.

View the [**examples**](../examples/) directory for service implementations using the `async` and `simple` servers.
There you can view how they interact with the `gsh` client.
View the [compatibility](../COMPATIBILITY.md) document for more information on supported versions.

## Architecture

The `libgsh` library is designed to be modular and extensible, allowing developers to build custom server-side functionality. It provides a set of abstractions and utilities that simplify the process of establishing and managing connections with the `gsh` client.

<div align="center">
  <img src="../assets/graph.png" alt="Architecture Diagram">
  <br>
  <strong>Overview diagram</strong>
  <br>
  <br>

```mermaid
sequenceDiagram
	participant Client as gsh client
	participant CGSH as client <br> gsh library
	participant SGSH as server <br> gsh library
	participant Server as libgsh service

	Client->>CGSH: Connect to server
	CGSH-->>SGSH: TLS + Handshake
	SGSH->>Server: Client Hello message
	SGSH-->>CGSH: Server Hello ACK message
	alt Server require Authentication
	rect rgb(190, 30, 50)
		CGSH<<->>Client: Enter credentials
		CGSH-->>SGSH: Send credentials
		SGSH<<->>Server: Verify credentials
		SGSH-->>CGSH: Authentication result
	end
	end
	CGSH->>Client: Server Hello ACK message
	par main event loop
    rect rgb(50, 60, 210)
		Client->>Server: User input
		Server->>Client: Visual/Media output
	end
	end
```

  <br>
  <strong>Detailed communication diagram</strong>
</div>

## Prerequisites

Install the following dependencies before proceeding with setting up your development environment:

- Install [Rust](https://www.rust-lang.org/tools/install)
- Install [Visual Studio 2022](https://visualstudio.microsoft.com/downloads/) version 17
  - In the "Workloads" tab enable "Desktop development with C++"
  - Click Modify at the bottom right
- Install [`protoc`](https://github.com/protocolbuffers/protobuf/releases/) version 30.2 *(`shared` dependency)*
- Install [`cmake`](https://cmake.org/download/) version 3.31.7 *(`sdl2` dependency)*
