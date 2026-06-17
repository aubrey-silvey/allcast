#!/usr/bin/env bash
# Stream from Sunshine (this workstation) to the Pi's TV, fullscreen, on bare
# KMS. Stops the Pi desktop (labwc/lightdm) so Moonlight can own the display,
# and restores it on exit. Run from the workstation; uses ssh -t for sudo.
#
# Why bare KMS + software decode: Moonlight can't get DRM master inside the
# labwc compositor, and the Pi 5's HW HEVC decode path is unusable
# (see ../docs/allcast-known-issues.md). Software decode handles 1080p60.
set -euo pipefail

PI="${PI:-ghostvox@192.168.0.208}"
PI_USER="${PI%@*}"
HOST_IP="${HOST_IP:-192.168.0.129}"
RES="${RES:-1080}"          # 720 | 1080 | 1440 | 4K
FPS="${FPS:-60}"
BITRATE="${BITRATE:-20000}" # Kbps
KMS_CARD="${KMS_CARD:-card1}"          # display GPU (card0 is v3d render, no outputs)
KMS_CONNECTOR="${KMS_CONNECTOR:-HDMI-A-2}"  # the HDMI port the TV is on

case "$RES" in
  720)  MODE="1280x720" ;;
  1080) MODE="1920x1080" ;;
  1440) MODE="2560x1440" ;;
  4K)   MODE="3840x2160" ;;
  *)    MODE="1920x1080" ;;
esac

echo "==> Streaming $HOST_IP -> $PI TV (${RES}p${FPS}, ${BITRATE}kbps, SW decode). Ctrl-C to stop."
ssh -t "$PI" "sudo PI_USER='$PI_USER' HOST_IP='$HOST_IP' RES='$RES' FPS='$FPS' BITRATE='$BITRATE' KMS_CARD='$KMS_CARD' KMS_CONNECTOR='$KMS_CONNECTOR' MODE='$MODE' bash -s" <<'REMOTE'
set -e
trap 'systemctl start lightdm' EXIT INT TERM
systemctl stop lightdm; sleep 2
mkdir -p /run/user/0
printf '{ "device": "/dev/dri/%s", "outputs": [ { "name": "%s", "mode": "%s" } ] }' \
  "$KMS_CARD" "$KMS_CONNECTOR" "$MODE" > /root/eglfs-kms.json
HOME="/home/$PI_USER" XDG_RUNTIME_DIR=/run/user/0 \
  QT_QPA_PLATFORM=eglfs QT_QPA_EGLFS_INTEGRATION=eglfs_kms \
  QT_QPA_EGLFS_KMS_CONFIG=/root/eglfs-kms.json SDL_VIDEODRIVER=kmsdrm \
  moonlight-qt stream "$HOST_IP" "Desktop" \
    --"$RES" --fps "$FPS" --bitrate "$BITRATE" \
    --video-codec HEVC --video-decoder software --display-mode fullscreen
REMOTE
echo "==> Stream ended; Pi desktop restored."
