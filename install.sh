#!/usr/bin/env bash
#! Installs the latest gsh client binary on Linux.
#! 1. Detects CPU architecture (x86 or x64)
#! 2. Downloads the matching asset from the latest GitHub release
#! 3. Installs it to ~/.local/share/gsh/bin and ensures that dir is on PATH
#!
#! Copyright (c) 2023 William Ragstad
#! Licensed under the MIT License.

set -euo pipefail

case "$(uname -m)" in
x86_64) ARCH="x64" ;;
i?86) ARCH="x86" ;;
*)
  echo "Unsupported architecture: $(uname -m)"
  exit 1
  ;;
esac

LATEST_JSON=$(curl -s "https://api.github.com/repos/WilliamRagstad/gsh/releases/latest" -H "User-Agent: gsh-installer")
VERSION=$(echo "$LATEST_JSON" | grep -oP '"tag_name":\s*"\K[^"]+' | sed 's/^v//')
PATTERN="gsh-${VERSION}-${ARCH}-linux"
ASSET_URL=$(echo "$LATEST_JSON" |
  grep -oP '"browser_download_url":\s*"\K[^"]+' |
  grep "gsh-${VERSION}-${ARCH}-linux" |
  head -n1)

if [[ -z "$ASSET_URL" ]]; then
  echo "No binary matching $PATTERN found in latest release"
  exit 1
fi

INSTALL_DIR="$HOME/.local/share/gsh/bin"
BIN_PATH="$INSTALL_DIR/gsh"

mkdir -p "$INSTALL_DIR"
curl -L --output "${BIN_PATH}" "$ASSET_URL"
chmod +x "${BIN_PATH}"

if ! grep -q "$INSTALL_DIR" <<<"$PATH"; then
  SHELL_RC="$HOME/.bashrc"
  echo -e "\n# added by gsh installer\nexport PATH=\"\$PATH:$INSTALL_DIR\"" >>"$SHELL_RC"
  echo "Added $INSTALL_DIR to PATH in $SHELL_RC"
  echo "(restart your terminal or run: source $SHELL_RC)"
fi

echo "✅ gsh $VERSION ($ARCH) installed to $BIN_PATH"
