<br/>
<div align="center">
  ———————————————
  <br>
  <img src="assets/logo.png" alt="Graphical Shell Logo" height="100">
  <br>
  ———————————————
</div>
<br/>

# Graphical Shell

Graphical Shell is a versatile framework designed to empower developers and enthusiasts to create custom graphical server interfaces, applications, services and experiences.
Whether you're building a personal server, a graphical Bulletin Board System (BBS), or exploring new interactive experiences, Graphical Shell provides the tools to bring your ideas to life.

It shippes with a SSH-like client application (`gsh`) that allows users to connect to a server and interact with it through a graphical interface.
All rendering and graphical elements are handled by the server, while the client seamlessly streams user interactions and input to the server for processing.

## Features

- **Interactivity**: Integrates graphical elements, allowing users interact with server applications and services.

- **Customizable**: Developers can create their own server applications using the provided library, enabling tailored experiences for specific use cases.

- **Cross-Platform Compatibility**: Client application on Linux, macOS, and Windows, ensuring a consistent experience across different operating systems.

- **Security**: TLS 1.3 encryption ensures secure communication between the client and server, protecting sensitive data and user interactions.

<div align="center">
  <h3>
   What are you gonna build? ✨
 &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;
 <a href="COMMUNITY.md">Explore community creations!</a> 🖌️
  </h3>
</div>

## Installation

To install **gsh**, follow the steps appropriate for your operating system:

```bash
git clone https://github.com/WilliamRagstad/gsh
cd gsh/client
cargo build --release
```

The compiled binary will be located in `target/release/gsh` and can be moved to a directory in your PATH for easier access.

## Usage

To start **gsh**, run the following command in your terminal:

```bash
gsh 192.168.0.1  # Replace with your server's IP address or hostname
```

This will initiate a connection to the server on the default port (`1122`).
A window will open, displaying any graphical interface presented by the server application.
You can interact with the server application through this window, and any user input will be sent to the server for processing.

## Build a Server

Do you want to build your own server application with **gsh**?
Read the [Server lib README](lib/README.md) for instructions on how to set up a server application that can communicate with the **gsh** client.

## Development

If you want to contribute to the development of **gsh**, follow these steps:

- Install [`protoc`](https://github.com/protocolbuffers/protobuf/releases/) *(`shared` dependency)*
- Install [`cmake`](https://cmake.org/download/) *(`sdl2` dependency)*
- Clone this repository and `cd gsh`

Manual system test:

1. In `example_server`: `cargo run -q`
2. In `client`: `cargo run -q -- localhost --insecure`
