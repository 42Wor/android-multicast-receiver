# OmniCast Roadmap

Semantic versioning starts at **`v0.1.0-alpha`**. Milestones below track the path from a single-device GPU shell to a multi-device production receiver.

## Phase 1 — Repository & architecture *(complete in v0.1.0-alpha)*

- [x] Cargo workspace with clear crate boundaries
- [x] Community docs (`README`, `ROADMAP`, `CONTRIBUTING`, MIT license)
- [x] Core session / messaging types
- [x] Render + app scaffolding

## Phase 2 — Milestone 1: Single-device MVP — `v0.1.0-alpha`

**Goal:** Prove the end-to-end desktop pipeline with one phone-shaped session.

- [x] mDNS advertisement (`_display._tcp` / `_googlecast._tcp`)
- [x] Tokio RTSP listener + RTP H.264 NAL extraction
- [x] `winit` `ApplicationHandler` multi-window map (`WindowId` → `DeviceContext`)
- [x] `wgpu` fullscreen texture blit (WGSL)
- [x] Synthetic/mock 60 FPS frame path for verification
- [ ] Real Android device completes RTSP → first decoded frame (Phase 2+)

**Exit criteria:** `cargo run -p omnicast-app -- --demo` sustains a resizable window at ~60 FPS.

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
