# Contributing to omnicast-rs

Thanks for helping build a fast, open Android casting receiver.

## Development setup

1. Install a recent stable Rust toolchain (`rustup`).
2. Clone the repository and build the workspace:

```bash
cargo check --workspace
cargo run -p omnicast-app -- --demo
```

3. Before opening a PR:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Code style

| Tool | Expectation |
|------|-------------|
| `cargo fmt` | Required; rustfmt defaults |
| `cargo clippy` | Fix warnings; prefer idiomatic Rust |
| Edition | 2021 |
| Unsafe | Justify in comments; keep localized (prefer `omnicast-media` / `omnicast-render`) |

Guidelines:

- Keep crate boundaries clean: protocols must not create windows; rendering must not parse RTP.
- Cross-thread work uses `omnicast_core::messages` / channels — not ad-hoc globals.
- Prefer `tracing` over `println!` for diagnostics.
- Avoid blocking the `winit` thread; use `EventLoopProxy` for async → UI events.

## Commit guidelines

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(render): add NV12 texture upload path
fix(protocol): handle FU-A NAL reassembly edge case
docs: clarify mDNS service types
chore: bump wgpu
```

Scopes commonly used: `core`, `discovery`, `protocol`, `media`, `render`, `app`.

Keep commits focused. Large protocol spikes can be marked `WIP:` in the PR title.

## Architecture breakdown

```
discovery ──► app ◄── protocol ──► media ──► app ──► render
                 ▲
                 └── core (types / session ids / messages)
```

| Change type | Primary crate |
|-------------|----------------|
| Session enums, IDs, channels | `omnicast-core` |
| mDNS TXT / service types | `omnicast-discovery` |
| RTSP verbs, RTP headers, NAL parsing | `omnicast-protocol` |
| Decoders, color conversion | `omnicast-media` |
| WGSL, pipelines, surfaces | `omnicast-render` |
| Window lifecycle, demo harness | `omnicast-app` |

See [README.md](README.md) and [ROADMAP.md](ROADMAP.md) for milestone context.

## Pull requests

- Describe motivation and test plan (`--demo`, device model if applicable).
- Do not commit proprietary blobs, secrets, or DRM circumvention material.
- Update docs when behavior or crate boundaries change.

## License

Contributions are accepted under the MIT License.
