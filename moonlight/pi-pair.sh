#!/usr/bin/env bash
# Pair the Pi's moonlight-qt to Sunshine on this workstation, fully scripted:
# Moonlight generates the request with a chosen PIN; we submit the same PIN to
# Sunshine's local API. Run on the workstation (where Sunshine runs).
set -euo pipefail

PI="${PI:-ghostvox@192.168.0.208}"
HOST_IP="${HOST_IP:-192.168.0.129}"
SUN_USER="${SUN_USER:-silvey}"
SUN_PASS="${SUN_PASS:-Silvey371}"
PIN="${PIN:-$(printf '%04d' $(( (RANDOM*RANDOM) % 10000 )))}"

echo "==> Starting pairing from $PI to $HOST_IP with PIN $PIN"
ssh "$PI" "QT_QPA_PLATFORM=offscreen moonlight-qt pair '$HOST_IP' --pin '$PIN'" &
pair_pid=$!

sleep 4
echo "==> Submitting PIN to Sunshine API"
curl -sk -u "$SUN_USER:$SUN_PASS" -X POST https://localhost:47990/api/pin \
  -H "Content-Type: application/json" \
  -d "{\"pin\":\"$PIN\",\"name\":\"$PI\"}" -w '\nHTTP %{http_code}\n'

wait "$pair_pid" || true
echo "==> Verify:"
ssh "$PI" "QT_QPA_PLATFORM=offscreen moonlight-qt list '$HOST_IP' 2>/dev/null" || true
