#!/usr/bin/env bash
# Install moonlight-qt on the Pi (Debian/Raspberry Pi OS Trixie arm64), from
# Moonlight's Cloudsmith apt repo. Run from the workstation; uses ssh -t so the
# Pi's sudo can prompt. No passwords stored.
set -euo pipefail

PI="${PI:-ghostvox@192.168.0.208}"
KEY_URL="https://dl.cloudsmith.io/public/moonlight-game-streaming/moonlight-qt/gpg.2F6AE14E1C660D44.key"
KEYRING="/usr/share/keyrings/moonlight-game-streaming-moonlight-qt-archive-keyring.gpg"
# Trixie isn't published in the repo yet; bookworm arm64 builds are ABI-compatible.
REPO="deb [signed-by=$KEYRING] https://dl.cloudsmith.io/public/moonlight-game-streaming/moonlight-qt/deb/debian bookworm main"

echo "==> Installing moonlight-qt + qt6-wayland on $PI"
ssh -t "$PI" "set -e
  curl -1sLf '$KEY_URL' | gpg --dearmor | sudo tee '$KEYRING' >/dev/null
  echo '$REPO' | sudo tee /etc/apt/sources.list.d/moonlight-game-streaming-moonlight-qt.list >/dev/null
  sudo apt-get update
  sudo apt-get install -y moonlight-qt qt6-wayland
  echo 'installed:' \$(moonlight-qt --version 2>/dev/null | head -1 || which moonlight-qt)
"
echo "==> Done. Pair with: PI=$PI HOST_IP=<workstation-ip> ./pi-pair.sh"
