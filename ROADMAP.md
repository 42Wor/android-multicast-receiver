# OmniCast Roadmap

Semantic versioning starts at **`v0.1.0-alpha`**. Milestones below track the path from a single-device GPU shell to a multi-device production receiver.

## Phase 1 — Repository & architecture *(complete)*

- [x] Cargo workspace with clear crate boundaries
- [x] Community docs (`README`, `ROADMAP`, `CONTRIBUTING`, MIT license)
- [x] Core session / messaging types
- [x] Render + app scaffolding

## Phase 2 — Milestone 1: Single-device MVP — `v0.1.0-alpha`

**Goal:** Prove the end-to-end desktop pipeline with one phone-shaped session.

### Mobile 1 — Live discovery + RTSP handshake *(complete)*

- [x] mDNS advertisement so LAN phones can see **OmniCast**
- [x] Live TCP/RTSP handshake listener (`omnicast-protocol`)
- [x] Terminal logging of incoming packets and session parameters
- [x] Multi-window map + `wgpu` blit + mock 60 FPS path
- [ ] Real Android device completes RTSP → first **decoded** video frame (next media milestone)

### Stream telemetry overlay & settings *(complete)*

- [x] Thread-safe `StreamMetrics` (FPS, bitrate, uptime, frames/drops) in `omnicast-core`
- [x] `MetricsRegistry` for protocol → UI updates
- [x] `egui` + `egui-wgpu` translucent HUD over the video surface
- [x] Live FPS / bitrate / uptime (and packet stats) on the device window
- [x] Settings panel: display toggles, always-on-top, aspect lock, listen port, buffer slider, borderless
- [x] HUD pin / auto-hide (shortcut **H**; **S** opens settings)

```bash
cargo run -p omnicast-app -- --demo
# Hover the top of the window for the HUD, or press H to pin / S for settings
```

## Phase 3 — Milestone 2: Real media path — `v0.2.0`

- [ ] Production H.264 decode backend (platform HW where available)
- [ ] NV12 / YUV → RGB GPU conversion path hardened
- [ ] Stable RTSP SETUP/PLAY against at least one OEM mirror stack or Cast sink profile
- [ ] Session teardown / reconnect without leaking GPU resources
- [x] Basic metrics: FPS, bitrate, drop counters in UI overlay

## Phase 4 — Milestone 3: Multi-device beta — `v0.3.0`

- [ ] Concurrent sessions, each with isolated window + surface + decoder
- [ ] Backpressure between RTP ingest and GPU upload
- [ ] Discovery conflict handling and receiver rename
- [ ] Linux CI + Windows CI smoke (`cargo check`, demo headless where possible)
- [ ] Packaging sketch (MSI / portable zip)

## Phase 5 — Milestone 4: Multi-device production — `v1.0.0`

- [ ] Broad OEM validation matrix (Pixel, Samsung Smart View, etc.)
- [ ] Documented protocol capability matrix (Cast vs Miracast/WFD)
- [ ] Secure defaults, firewall guidance, signed releases
- [ ] Optional reverse-input explicitly out-of-scope unless separately specified
- [ ] Stable crate APIs and contributor onboarding complete

## Non-goals (near term)

- Companion APK requirement on the phone
- Cloud relay / WAN casting
- DRM circumvention or HDCP bypass tooling

## Notes for Android testing

- Allow **UDP 5353** (mDNS) and **TCP 8554** (RTSP) / **UDP 5004** (RTP) through Windows Firewall.
- Native **Google Cast** UI may still require Cast TLS (port 8009) beyond mDNS — RTSP clients and some OEM mirror stacks will hit the handshake logged here.
- Prefer the same Wi‑Fi LAN (not guest/AP isolation).
