# Compatibility

This document outlines the compatibility of various versions of the clients and servers via the libgsh library.
It provides information on the supported versions, dependencies, and any known issues or limitations.

<!-- ## Supported Versions

| **libgsh (server)** ↓ / **gsh (client)** → | 1.4-series | 1.5-series | 2.x    |
| ------------------------------------------ | ---------- | ---------- | ------ |
| **1.4**                                    | ✅ full     | ✅ limited* | ❌      |
| **1.5**                                    | ✅ full     | ✅ full     | ❌      |
| **2.0**                                    | ❌          | ❌          | ✅ full |

> **limited**: new optional commands introduced in 1.5 are ignored gracefully when an older 1.4 client connects; core functionality is unaffected. -->

## Version-numbering scheme

| Component                  | Format              | Source of truth         |
| -------------------------- | ------------------- | ----------------------- |
| **Client, `gsh`**          | `MAJOR.MINOR.PATCH` | Git tag & release asset |
| **Server crate, `libgsh`** | `MAJOR.MINOR.PATCH` | Git tag & crates.io     |

- **Both follow Semantic Versioning 2.0.0**: a bump in `MAJOR` communicates a breaking change; `MINOR` adds functionality in a backward-compatible way; `PATCH` is for bug fixes.
- Cargo enforces these SemVer rules during dependency resolution, so Rust code that depends on `libgsh = "1"` will automatically pick any `1.x.y` that is published.

## Operating Systems

- Linux binaries are built on **Ubuntu 22.04** / `glibc 2.35`, which runs on any newer distro and most enterprise LTS releases.
- Windows binaries are built with **MSVC v143** and run on **Windows 10 1809** or later.
- If you need to support **CentOS 7** (`glibc 2.17`) or similarly old platforms, build from source in a **RHEL 8** container.
