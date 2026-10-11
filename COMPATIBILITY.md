# Compatibility

## Versioning

| Component                  | Format              | Source of truth         |
| -------------------------- | ------------------- | ----------------------- |
| **Client, `gsh`**          | `MAJOR.MINOR.PATCH` | Git tag & release asset |
| **Server crate, `libgsh`** | `MAJOR.MINOR.PATCH` | Git tag & crates.io     |

Both use Semantic Versioning: major versions break compatibility, minor versions add compatible features, and patch versions fix bugs.

## Operating Systems

- Linux binaries are built on **Ubuntu 22.04** / `glibc 2.35`, which runs on any newer distro and most enterprise LTS releases.
- Windows binaries are built with **MSVC v143** and run on **Windows 10 1809** or later.
