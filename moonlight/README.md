# Moonlight + Sunshine receiver (alternative to allcast)

A working, ready-to-use screen-cast path for the **Silvey Pi-on-TV setup**, as an
alternative to the in-house `allcast` pipeline. Uses [Sunshine](https://github.com/LizardByte/Sunshine)
on the sender (workstation) and [Moonlight](https://github.com/moonlight-stream/moonlight-qt)
on the receiver (Raspberry Pi 5 on the TV).

It is **complete and works today**, where allcast currently can't capture the
desktop on Linux (see `../docs/allcast-known-issues.md`).

## Topology

```
workstation (Sunshine host)  ──HEVC over the LAN──▶  Pi 5 on TV (moonlight-qt)
  Intel VAAPI HEVC encode                              software HEVC decode → KMS
```

## Hard-won facts baked into these scripts

- **Encoder = Intel VAAPI HEVC**, not NVENC. On this hybrid Intel+NVIDIA laptop
  the NVIDIA GPU has no attached display, so Sunshine's NVENC path fails
  (`Couldn't find monitor [0]`). VAAPI on the Intel iGPU (which drives the
  display) works. `host-setup.sh` pins `encoder = vaapi`.
- **Pi decode is software.** The Pi 5's hardware HEVC decoder emits a tiled
  SAND buffer that nothing here can scan out correctly (garbage + DRM wedge);
  forced hardware decode fails. So Moonlight runs `--video-decoder software`
  (the A76 cores handle 1080p60 fine). See `../docs/allcast-known-issues.md`.
- **The Pi runs on bare KMS.** Moonlight can't render inside the labwc
  compositor (can't get DRM master), so `pi-stream.sh` stops `lightdm` and runs
  Moonlight via `eglfs`/`kmsdrm` as root, then restarts the desktop on exit.
- **eglfs must target the display GPU + connected HDMI port.** Default eglfs
  picks `card0` (the v3d render node, no outputs) → "no screens". We pin
  `/dev/dri/card1` and the connected connector (here `HDMI-A-2`). Override with
  `KMS_CARD` / `KMS_CONNECTOR` if your wiring differs.
- **Audio doesn't work when run as root** (can't reach the user's PipeWire,
  `error 524`). Video-only for now; see TODO in `pi-stream.sh`.

## Usage

1. **On the workstation** (Arch/CachyOS), once:
   ```sh
   ./host-setup.sh
   ```
   Installs Sunshine, pins VAAPI HEVC, sets web creds, opens the firewall,
   enables the user service.

2. **On the Pi**, once (run from the workstation, or copy the script over):
   ```sh
   PI=ghostvox@192.168.0.208 ./pi-setup.sh
   ```
   Installs `moonlight-qt` + `qt6-wayland` from Moonlight's apt repo.

3. **Pair** the Pi to the host, once:
   ```sh
   PI=ghostvox@192.168.0.208 HOST_IP=192.168.0.129 ./pi-pair.sh
   ```

4. **Stream** (each session):
   ```sh
   PI=ghostvox@192.168.0.208 HOST_IP=192.168.0.129 ./pi-stream.sh
   ```
   Stops the Pi desktop, plays fullscreen on the TV, restores the desktop on
   Ctrl-C.

## Config

All scripts read env vars (with sensible defaults for this deployment):

| var | default | meaning |
|---|---|---|
| `PI` | `ghostvox@192.168.0.208` | ssh target for the Pi |
| `HOST_IP` | `192.168.0.129` | workstation LAN IP Moonlight connects to |
| `SUN_USER` / `SUN_PASS` | `silvey` / `Silvey371` | Sunshine web-manager creds |
| `RES` / `FPS` / `BITRATE` | `1080` / `60` / `20000` | stream profile (Kbps) |
| `KMS_CARD` / `KMS_CONNECTOR` | `card1` / `HDMI-A-2` | Pi display device + port |
| `LAN_CIDR` | `192.168.0.0/24` | firewall scope on the host |
