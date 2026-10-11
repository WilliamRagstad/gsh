# Graphical Shell Client

Native client for server-rendered graphical applications.

## Quick Install

Windows, in an administrator PowerShell:

```powershell
iwr https://raw.githubusercontent.com/WilliamRagstad/gsh/main/install.ps1 | iex
```

Linux:

```bash
curl -sSfL https://raw.githubusercontent.com/WilliamRagstad/gsh/main/install.sh | sh
```

[Release binaries](https://github.com/WilliamRagstad/gsh/releases) · [Compatibility](../COMPATIBILITY.md)

## Usage

```bash
gsh example.com
gsh localhost --insecure
gsh --help
```

The default port is `1122`. `--insecure` disables TLS certificate verification; use it for local examples with self-signed certificates.

## Build from Source

Install the [build requirements](../libgsh/README.md#build-requirements), then:

```bash
git clone https://github.com/WilliamRagstad/gsh
cd gsh
cargo build --release -p gsh
```

The binary is `target/release/gsh` (`gsh.exe` on Windows).

To test locally, run `cargo run -p colors`, then `cargo run -p gsh -- localhost --insecure` in another terminal.
