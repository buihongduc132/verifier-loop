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

# Build the two release binaries once, then copy those exact artifacts.  Using
# `cargo install` here would rebuild in Cargo's install pipeline and can produce
# binaries that differ from the release artifacts validated by the caller; it also
# writes `$ROOT/.crates.toml`.  A direct release build/copy avoids both problems,
# including the root-owned metadata case handled by older installations.
echo ">> cargo build --release --bins"
cargo build --release --bins

BIN_DIR="$ROOT/bin"
mkdir -p "$BIN_DIR"
install -m 755 target/release/verifier-loop "$BIN_DIR/verifier-loop"
install -m 755 target/release/verifier-verdict "$BIN_DIR/verifier-verdict"

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
