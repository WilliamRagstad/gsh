# Graphical Shell Client&nbsp; <a href="https://github.com/WilliamRagstad/gsh/releases/latest"><img src="https://img.shields.io/github/v/release/WilliamRagstad/gsh?color=%23ff00a0&include_prereleases&label=client&sort=semver&style=flat-square" alt="Client Version"></a> <a href="https://github.com/WilliamRagstad/gsh/releases/latest"><img src="https://img.shields.io/github/downloads/WilliamRagstad/gsh/total?color=6b0&label=client%20dls&style=flat-square" alt="Client Downloads"></a>

This is the client application for `gsh`.
It provides a cross-platform native window to interface with graphical server applications in a seamless and intuitive manner.
The client application is a cross-platform native window that interfaces with graphical server applications in a seamless and intuitive manner.

&nbsp;

## Quick Install

Use the one-liners below to install the latest release of `gsh` on your system.
The installation script will download the latest release and place the binary in your `PATH` for easy access.
You can also find release binaries on the [releases page](https://github.com/WilliamRagstad/gsh/releases) or build it yourself [from source](#build-from-source).
View the [compatibility](../COMPATIBILITY.md) document for more information on supported versions.

> ### Windows
>
> Run as administrator in `PowerShell`:
>
> ```powershell
> iwr https://raw.githubusercontent.com/WilliamRagstad/gsh/main/install.ps1 | iex
> ```

---

> ### Linux
>
> ```bash
> curl -sSfL https://raw.githubusercontent.com/WilliamRagstad/gsh/main/install.sh | sh
> ```

&nbsp;

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

&nbsp;

## Build from Source

To build the **gsh client** from source, install the [required dependencies](#contributing) then run:

```bash
git clone https://github.com/WilliamRagstad/gsh && cd gsh/client
cargo build --release
```

The compiled binary will be located in `./target/release/gsh` and can be moved to a directory in your PATH for easier access.

## Contributing

If you want to contribute to the development of `gsh`, follow these steps:

- Install [Rust](https://www.rust-lang.org/tools/install)
- Install [Visual Studio 2022](https://visualstudio.microsoft.com/downloads/) version 17
  - In the "Workloads" tab enable "Desktop development with C++"
  - Click Modify at the bottom right
- Install [`protoc`](https://github.com/protocolbuffers/protobuf/releases/) version 30.2 *(`shared` dependency)*
- Clone this repository and `cd gsh`

Manual system test for the `colors` example:

1. In `examples/colors/`: `cargo run -q`
2. In `client/`: `cargo run -q -- localhost --insecure`
