# libgsh

<a href="https://crates.io/crates/libgsh"><img src="https://img.shields.io/crates/v/libgsh?color=6b0&label=libgsh&style=flat-square" alt="Rust Crate Version"></a>

This is a SDK library for building server applications that interact seamlessly with the **gsh** client.
It provides essential tools and abstractions to enable efficient communication between the server and the **gsh** graphical shell client application.

## Features

- **Server-Client Communication**: Simplifies the process of establishing and managing connections with the **gsh** client.
- **Data Serialization**: Includes utilities for encoding and decoding data exchanged between the server and client.
- **Extensibility**: Designed to be modular and extensible, allowing developers to build custom server-side functionality.

This library is an integral part of the **gsh** ecosystem, enabling developers to create robust and interactive server applications that leverage the graphical capabilities of the **gsh** client.

## Build a Server

Do you want to build your own service with **gsh**?
Choose between these *out-of-the-box* server implementations:

| Server   | Description                                                    | Technology             |
| -------- | -------------------------------------------------------------- | ---------------------- |
| `async`  | An asynchronous server that handle communication concurrently. | Tokio async runtime    |
| `simple` | A basic server that handle non-blocking communication          | Native multi-threading |

> **Recommendation**\
> The `async` server provides better performance and scalability for most applications.

View the [**examples**](../examples/) directory for service implementations using the `async` and `simple` servers.
There you can view how they interact with the **gsh** client.

## Prerequisites

Install the following dependencies before proceeding with setting up your development environment:

- Install [Rust](https://www.rust-lang.org/tools/install)
- Install [Visual Studio 2022](https://visualstudio.microsoft.com/downloads/) version 17
  - In the "Workloads" tab enable "Desktop development with C++"
  - Click Modify at the bottom right
- Install [`protoc`](https://github.com/protocolbuffers/protobuf/releases/) version 30.2 *(`shared` dependency)*
