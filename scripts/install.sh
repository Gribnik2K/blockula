#!/bin/sh
# 🧛 Install the `blockula` node + CLI from GitHub releases.
#   curl -fsSL https://github.com/jackdishman/blockula/releases/latest/download/install.sh | sh
# Env: BLOCKULA_VERSION (e.g. v0.3.0; default latest), BLOCKULA_INSTALL_DIR (default ~/.blockula/bin)
set -eu

REPO=jackdishman/blockula
DIR=${BLOCKULA_INSTALL_DIR:-$HOME/.blockula/bin}
VERSION=${BLOCKULA_VERSION:-latest}

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  *) echo "Sorry, no prebuilt blockula for $(uname -s) $(uname -m) yet. Build from source: cargo build --release --manifest-path node/Cargo.toml" >&2; exit 1 ;;
esac

if [ "$VERSION" = latest ]; then base=https://github.com/$REPO/releases/latest/download
else base=https://github.com/$REPO/releases/download/$VERSION; fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
asset=blockula-$target.tar.gz
echo "🧛 Downloading ${asset}…"
curl -fsSL "$base/$asset" -o "$tmp/$asset"
curl -fsSL "$base/checksums.txt" -o "$tmp/checksums.txt"

expected=$(grep " $asset\$" "$tmp/checksums.txt" | awk '{print $1}')
if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$tmp/$asset" | awk '{print $1}')
else actual=$(shasum -a 256 "$tmp/$asset" | awk '{print $1}'); fi
if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
  echo "Checksum mismatch for $asset; refusing to install." >&2
  exit 1
fi

tar -xzf "$tmp/$asset" -C "$tmp"
mkdir -p "$DIR"
install -m 755 "$tmp/blockula-$target/blockula" "$DIR/blockula"
echo "✓ Installed $("$DIR/blockula" --version) to $DIR/blockula"

case ":$PATH:" in
  *":$DIR:"*) ;;
  *) echo "  Add it to your PATH:  echo 'export PATH=\"$DIR:\$PATH\"' >> ~/.zshrc  (or ~/.bashrc)" ;;
esac
echo "  Try:  blockula dev        (local chain + explorer at http://localhost:7070)"
echo "        blockula wallet new && blockula wallet faucet --rpc https://blockula-testnet.fly.dev"
