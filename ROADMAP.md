# OmniCast Roadmap

Semantic versioning starts at **`v0.1.0-alpha`** (git tag present).

## Current status (summary)

| Area | State |
|------|--------|
| Phase 1 — workspace / crates / window+render shell | **Complete** |
| Milestone 1 — mDNS + live RTSP handshake + mock video path | **Complete** |
| Telemetry HUD + settings panel | **Complete** (landed; UX polish welcome) |
| HW H.264 decode → live pixels | **Next** |
| Multi-device production sessions | **Planned** |
| Miracast / Cast V2 interoperability | **Planned** |

---

## Phase 1 — Repository & architecture *(complete)*

- [x] Cargo workspace (`resolver = "2"`) with clear crate boundaries
- [x] Crates: `omnicast-core`, `discovery`, `protocol`, `media`, `render`, `ui`, `app`
- [x] Community docs (`README`, `ROADMAP`, `CONTRIBUTING`, MIT `LICENSE`)
- [x] Core session / device IDs / `AppEvent` messaging types
- [x] Initial `winit` multi-window shell + `wgpu` fullscreen blit
- [x] Protocol foundations: RTSP listener scaffold + RTP / H.264 NAL parser
- [x] mDNS discovery surface (`_rtsp._tcp`, `_display._tcp`, `_googlecast._tcp`)

---

## Phase 2 — Milestone 1: Single-device MVP — `v0.1.0-alpha`

**Goal:** One phone-shaped session from discovery → handshake → presented frames (mock or real).

### Mobile 1 — Live discovery + RTSP handshake *(complete)*

- [x] mDNS advertisement so LAN devices can see **OmniCast**
- [x] Live TCP/RTSP handshake (`OPTIONS` / `DESCRIBE` / `ANNOUNCE` / `SETUP` / `PLAY` / `TEARDOWN`)
- [x] Terminal logging of packets and session parameters (IP, ports, codec)
- [x] Multi-window map (`WindowId` → `DeviceContext`) + mock ~60 FPS path
- [ ] Real Android device completes RTSP → first **hardware-decoded** video frame

### Stream telemetry overlay & settings *(complete — recently landed)*

Shipped in-tree; treat remaining work as polish, not blockers:

- [x] Thread-safe `StreamMetrics` (FPS, bitrate, uptime, frames/drops)
- [x] `MetricsRegistry` for protocol → UI updates
- [x] `egui` + `egui-wgpu` translucent HUD over the video surface
- [x] Live FPS / bitrate / uptime (+ packet stats) on the device window
- [x] Settings modal: display toggles, always-on-top, aspect lock, listen port, buffer slider, borderless
- [x] HUD pin / auto-hide (**H** pin; **S** / ⚙ opens settings)
- [ ] Optional polish: persist settings to disk, aspect-ratio enforcement on resize

```bash
cargo run -p omnicast-app -- --demo
# Hover top edge for HUD · H pin · S settings
```

---

## Phase 3 — Milestone 2: Real media path — `v0.2.0`

- [ ] Production H.264 decode backend (platform HW where available)
- [ ] NV12 / YUV → RGB (or GPU convert) path hardened
- [ ] Stable SETUP/PLAY against at least one OEM mirror stack **or** Cast sink profile
- [ ] Session teardown / reconnect without leaking GPU resources
- [x] Basic metrics overlay (FPS, bitrate, drops) — done early in v0.1.0-alpha

---

## Phase 4 — Milestone 3: Multi-device sessions — `v0.3.0`

**Goal:** Several phones cast concurrently with isolated windows and media pipelines.

- [ ] Concurrent sessions: one window + surface + decoder + metrics per device
- [ ] Backpressure between RTP ingest and GPU upload
- [ ] Fair scheduling when N streams compete for decode/GPU
- [ ] Discovery conflict handling and receiver rename
- [ ] Stress demo: `cargo run -p omnicast-app -- --demo --devices N`
- [ ] Linux + Windows CI smoke (`cargo check` / tests)
- [ ] Packaging sketch (portable zip / MSI)

---

## Phase 5 — Protocol depth: Miracast & Cast V2 — `v0.4.0` → `v1.0.0`

### Google Cast V2 *(planned)*

- [ ] Cast TLS control channel (typical port **8009**) beyond mDNS TXT advertisement
- [ ] Receiver app / screen-mirroring session negotiation compatible with stock Cast UI
- [ ] Capability / status TXT records kept accurate (`fn`, `md`, `id`, `ca`, `st`, …)
- [ ] Document which Android “Cast” paths hit Cast V2 vs RTSP-only clients

### Miracast / Wi-Fi Display *(planned)*

- [ ] Research Windows WFD sink feasibility vs pure userspace RTSP
- [ ] Adapter interface under `omnicast-protocol` (no UI coupling)
- [ ] OEM matrix notes (Samsung Smart View, etc.)
- [ ] Honest capability matrix in docs (what works / what needs OS support)

### Production hardening — `v1.0.0`

- [ ] Broad device validation matrix
- [ ] Secure defaults, firewall guidance, signed releases
- [ ] Stable crate APIs and contributor onboarding
- [ ] Reverse input remains **out of scope** unless separately specified

---

## Non-goals (near term)

- Companion APK requirement on the phone
- Cloud relay / WAN casting
- DRM circumvention or HDCP bypass tooling

## Notes for Android testing

- Allow **UDP 5353** (mDNS), **TCP 8554** (RTSP), **UDP 5004** (RTP) through Windows Firewall.
- Stock Google Cast UI may require Cast V2 TLS — RTSP clients hit the handshake logged today.
- Prefer the same Wi‑Fi LAN (not guest / AP isolation).
