# VIVE XR Elite bootloader: facts and consequences

## Facts (verified Nov 2026)

- The XR Elite's bootloader **is unlockable** on firmware **1.0.999.738**: hidden developer
  settings exposed via ADB → OEM unlock toggle → `fastboot oem unlock`. Unlocking wipes
  user data; `fastboot oem unlock` automatically disables AVB (orange boot state) — no
  separate vbmeta flashing needed (flashing an invalid vbmeta causes a bootloop).
- **Firmware 2.0.999.960 patches the unlock method and re-locks the bootloader**; there is
  no known reversal. Devices on 1.0 should not accept the 2.0 OTA while experiments run.
- Root has been achieved on 1.0.999.738 (Android 10, Snapdragon XR2 Gen 1) via Magisk.
- Display: single JDI DSI panel, 3840x1920 (dual 1920x1920 per eye), 75/90/120 Hz,
  **DRM/KMS only — no legacy framebuffer**. Relevant for any custom recovery or Linux boot.
- `fastboot fetch` is unsupported; HTC's firmware images are in an encrypted custom format.

## What this changes in the vivelite plan

1. **A native-Linux boot path exists** (unlocked bootloader → custom boot image →
   GNU/Linux with DRM/KMS panel + freedreno/Turnip for the Adreno GPU + Monado as
   the OpenXR runtime). The hard blocker is 6DoF tracking: the XR2 SLAM stack is
   HTC-proprietary with no upstream driver, so native Linux today means 3DoF/IMU or
   external tracking — not the room-scale experience the headset sells.
2. The **pragmatic path stays primary**: proot + Wine inside Android (rooted), with
   HTC's own OpenXR runtime owning tracking/display. Root removes the proot overhead
   arguments and unlocks bind mounts, real UID handling, and kernel knobs (TSO prctl
   probing, cgroup CPU pinning for the Wine threads).
3. The native path becomes a **research branch**: boot a minimal Linux (postmarketOS-
   style), bring up DRM/KMS + msm/adreno, run Monado against the panel, and revisit
   tracking later (Marker-based / outside-in / ALVR-style streaming are all options).

## Order of operations

1. Unlock on 1.0.999.738, Magisk root — do **not** take the 2.0 OTA.
2. Rooted Android + vivelite proot stack → playable SteamOS components on-device.
3. Research branch: dual-boot Linux image; keep the Android system on the other slot.
