#!/usr/bin/env sh
# scripts/install.sh — install verifier-loop + verifier-verdict and the jewilo / jewije aliases.
#
# tasks.md §10.4. Cargo does not support multiple names per [[bin]] target natively, so the
# `jewilo` / `jewije` aliases are created as symlinks (or copies on filesystems without
# symlinks) after `cargo install --path .`.
#
# Usage:
#   ./scripts/install.sh                  # installs to ~/.local (default)
#   ./scripts/install.sh /opt/verifier    # installs to a custom --root
#
# The actual install + PATH wiring for the self-verify step is the leader's job; this
# script is the documented, repeatable path.

set -eu

ROOT="${1:-$HOME/.local}"

echo ">> cargo install --path . --root \"$ROOT\""
cargo install --path . --force --root "$ROOT"

BIN_DIR="$ROOT/bin"
install_alias() {
  src="$BIN_DIR/$1"
  dst="$BIN_DIR/$2"
  if [ -e "$dst" ] || [ -L "$dst" ]; then
    rm -f "$dst"
  fi
  if ln -s "$src" "$dst" 2>/dev/null; then
    echo ">> linked $dst -> $src"
  else
    cp "$src" "$dst"
    echo ">> copied $src -> $dst (symlink unavailable)"
  fi
}

install_alias verifier-loop jewilo
install_alias verifier-verdict jewije

echo ">> done. Ensure \"$BIN_DIR\" is on your PATH."
