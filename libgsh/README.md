# libgsh

This is a SDK library for building server applications that interact seamlessly with the **gsh** client.
It provides essential tools and abstractions to enable efficient communication between the server and the **gsh** graphical shell client application.

## Features

- **Server-Client Communication**: Simplifies the process of establishing and managing connections with the **gsh** client.
- **Data Serialization**: Includes utilities for encoding and decoding data exchanged between the server and client.
- **Extensibility**: Designed to be modular and extensible, allowing developers to build custom server-side functionality.

This library is an integral part of the **gsh** ecosystem, enabling developers to create robust and interactive server applications that leverage the graphical capabilities of the **gsh** client.

## Build a Server

Do you want to build your own service with **gsh**?
These instructions detail how to set up a server application that can communicate with the **gsh** client.

## Prerequisites

Install the following dependencies before proceeding with development:

- Install [Rust](https://www.rust-lang.org/tools/install)
- Install [Visual Studio 2022](https://visualstudio.microsoft.com/downloads/) version 17
  - In the "Workloads" tab enable "Desktop development with C++"
  - Click Modify at the bottom right
- Install [`protoc`](https://github.com/protocolbuffers/protobuf/releases/) version 30.2 *(`shared` dependency)*
