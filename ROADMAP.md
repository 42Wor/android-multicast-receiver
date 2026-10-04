# OmniCast Roadmap

Semantic versioning starts at **`v0.1.0-alpha`**. Milestones below track the path from a single-device GPU shell to a multi-device production receiver.

## Phase 1 — Repository & architecture *(complete in v0.1.0-alpha)*

- [x] Cargo workspace with clear crate boundaries
- [x] Community docs (`README`, `ROADMAP`, `CONTRIBUTING`, MIT license)
- [x] Core session / messaging types
- [x] Render + app scaffolding

## Phase 2 — Milestone 1: Single-device MVP — `v0.1.0-alpha`

**Goal:** Prove the end-to-end desktop pipeline with one phone-shaped session.

### Mobile 1 — Live discovery + RTSP handshake *(current)*

- [x] mDNS advertisement so LAN phones can see **OmniCast**
  - `_rtsp._tcp`, `_display._tcp`, `_googlecast._tcp`
  - Cast-style TXT (`fn`, `md`, `id`, …) + explicit LAN IPv4 when available
- [x] Live TCP/RTSP handshake listener (`omnicast-protocol`)
  - OPTIONS / DESCRIBE / ANNOUNCE / SETUP / PLAY / TEARDOWN
  - UDP RTP + interleaved TCP RTP logging
- [x] Terminal logging of incoming packets and session parameters
  - Peer IP/port, Transport, client/server RTP ports, video codec / payload type
- [x] `winit` `ApplicationHandler` multi-window map (`WindowId` → `DeviceContext`)
- [x] `wgpu` fullscreen texture blit (WGSL)
- [x] Synthetic/mock 60 FPS frame path for verification
- [ ] Real Android device completes RTSP → first **decoded** video frame (Milestone 2)

**Exit criteria (Mobile 1):** Run without `--demo`, phone sees **OmniCast**, tap connects, terminal shows RTSP handshake + session params.

```bash
cargo run -p omnicast-app
# On Android: Cast / Screen Mirroring / Smart View → select OmniCast
# Watch terminal for RTSP request/response and RTP lines
```

## Phase 3 — Milestone 2: Real media path — `v0.2.0`

- [ ] Production H.264 decode backend (platform HW where available)
- [ ] NV12 / YUV → RGB GPU conversion path hardened
- [ ] Stable RTSP SETUP/PLAY against at least one OEM mirror stack or Cast sink profile
- [ ] Session teardown / reconnect without leaking GPU resources
- [ ] Basic metrics: FPS, bitrate, drop counters in UI overlay

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
