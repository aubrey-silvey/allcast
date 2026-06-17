#!/usr/bin/env bash
# Sunshine host setup for the workstation (Arch/CachyOS).
# Idempotent-ish: safe to re-run. Needs an interactive terminal for paru's sudo.
set -euo pipefail

SUN_USER="${SUN_USER:-silvey}"
SUN_PASS="${SUN_PASS:-Silvey371}"
LAN_CIDR="${LAN_CIDR:-192.168.0.0/24}"
# Pin VAAPI HEVC: NVENC fails on this hybrid Intel+NVIDIA laptop (the NVIDIA GPU
# has no attached display → "Couldn't find monitor [0]"). Unset ENCODER to let
# Sunshine auto-detect on a machine where NVENC has a display.
ENCODER="${ENCODER:-vaapi}"

echo "==> Installing Sunshine (paru)"
if ! command -v sunshine >/dev/null; then
  paru -S --needed sunshine
fi

echo "==> Writing ~/.config/sunshine/sunshine.conf"
mkdir -p "$HOME/.config/sunshine"
conf="$HOME/.config/sunshine/sunshine.conf"
if [ -n "$ENCODER" ]; then
  if grep -q '^encoder' "$conf" 2>/dev/null; then
    sed -i "s/^encoder.*/encoder = $ENCODER/" "$conf"
  else
    printf 'encoder = %s\n' "$ENCODER" >> "$conf"
  fi
fi

echo "==> Setting web-manager credentials"
sunshine --creds "$SUN_USER" "$SUN_PASS"

echo "==> Opening firewall (ufw) for $LAN_CIDR"
if systemctl is-active --quiet ufw; then
  sudo ufw allow from "$LAN_CIDR" to any port 47984:48010 proto tcp
  sudo ufw allow from "$LAN_CIDR" to any port 47998:48010 proto udp
else
  echo "    (ufw not active; skipping. Open 47984-48010/tcp + 47998-48010/udp if you use another firewall.)"
fi

echo "==> Enabling + starting the Sunshine user service"
systemctl --user enable --now app-dev.lizardbyte.app.Sunshine.service

echo "==> Done. Web UI: https://localhost:47990 (user: $SUN_USER)"
echo "    Encoder pinned to: ${ENCODER:-auto}. Pair a client with ./pi-pair.sh"
