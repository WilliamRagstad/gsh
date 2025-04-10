# gsh: Graphical Shell

**gsh** is an innovative graphical shell designed to provide users with an intuitive and efficient command-line interface, enhanced with graphical capabilities for improved usability and productivity.

## Features

- **Graphical Enhancements**: Integrates graphical elements, allowing users to visualize and interact with server applications and services.

- **Cross-Platform Compatibility**: Client application on Linux, macOS, and Windows, ensuring a consistent experience across different operating systems.

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
gsh 192.168.0.1  # Replace with your server's IP address
```

This will initiate a connection to the server at the specified IP address.
A window will open, displaying any graphical interface presented by the server application.
You can interact with the server application through this window, and any user input will be sent to the server for processing.

## Build a Server

Do you want to build your own server application with **gsh**?
Read the [Server lib README](lib/README.md) for instructions on how to set up a server application that can communicate with the **gsh** client.
