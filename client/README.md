# Graphical Shell Client

This is the client application for **gsh**.
It provides a cross-platform native window to interface with graphical server applications in a seamless and intuitive manner.

## Build from Source

To build the **gsh client** from source, install the [required dependencies](#development) then run:

```bash
git clone https://github.com/WilliamRagstad/gsh && cd gsh/client
cargo build --release
```

The compiled binary will be located in `target/release/gsh` and can be moved to a directory in your PATH for easier access.

## Usage

Connect to a server application using its IP address or hostname and the port number (default is `1122`):

```bash
gsh 127.0.0.1
```

A window will open, displaying any graphical interface presented by the server application.\
You can interact with the server application through this window, and any user input will be sent to the server for processing.

You can also use the `--insecure` flag to disable TLS certificate **verification** for testing purposes.

```bash
gsh localhost --insecure
```

> TLS certificate might not be valid when connecting **via IP address** due to certificates usually are issued for wildcard/fully qualified (common) **domain names**. - [StackOverflow](https://stackoverflow.com/a/1119269)

See all available options by running:

```bash
gsh --help
```

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
