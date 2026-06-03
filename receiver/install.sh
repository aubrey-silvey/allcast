#!/bin/bash
# Install allcast-receiver as a user-level systemd service that auto-
# activates on RTP traffic. Run as the user that owns the labwc / Wayland
# session (typically `sis` on the Pi 5 kiosk).
#
#   ./install.sh                  # builds the binary and installs the service
#   ./install.sh --no-build       # skips cargo build (use pre-built target/)
#
# Requires sudo for the binary copy. Everything else is user-scope.

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DO_BUILD=1
for arg in "$@"; do
    case "$arg" in
        --no-build) DO_BUILD=0 ;;
        *) echo "unknown arg: $arg" >&2; exit 2 ;;
    esac
done

if [ "$DO_BUILD" = 1 ]; then
    echo "==> Building release binary"
    ( cd "$REPO_DIR" && cargo build --release -p allcast-receiver )
fi

BIN="$REPO_DIR/target/release/allcast-receiver"
[ -x "$BIN" ] || { echo "binary not found at $BIN — run without --no-build, or `cargo build --release -p allcast-receiver` first"; exit 1; }

echo "==> Installing binary to /usr/local/bin/allcast-receiver"
sudo install -m 755 "$BIN" /usr/local/bin/allcast-receiver

ENV_FILE="$HOME/.config/allcast-receiver.env"
if [ ! -f "$ENV_FILE" ]; then
    echo "==> Writing default env file at $ENV_FILE"
    mkdir -p "$(dirname "$ENV_FILE")"
    cat > "$ENV_FILE" <<'EOF'
# allcast-receiver runtime config.
# Reload after edits with:  systemctl --user restart allcast-receiver

LISTEN=5004
CODEC=h265
PT=96
JITTER_MS=10
SINK=waylandsink fullscreen=true
RCVBUF=8388608
IDLE_TIMEOUT_S=3
RUST_LOG=info
EOF
else
    echo "==> Keeping existing $ENV_FILE"
fi

UNIT_DIR="$HOME/.config/systemd/user"
mkdir -p "$UNIT_DIR"
install -m 644 "$REPO_DIR/receiver/contrib/allcast-receiver.service" "$UNIT_DIR/"

echo "==> Reloading systemd user units"
systemctl --user daemon-reload

echo "==> Enabling + starting allcast-receiver"
systemctl --user enable --now allcast-receiver.service

echo
echo "Done. Status:"
systemctl --user --no-pager status allcast-receiver.service | head -12
echo
echo "Useful:"
echo "  journalctl --user -u allcast-receiver -f          # follow logs"
echo "  systemctl --user restart allcast-receiver          # reload config"
echo "  systemctl --user stop allcast-receiver             # stop"
echo "  systemctl --user disable allcast-receiver          # don't start on session"
echo "  edit $ENV_FILE to tweak"
