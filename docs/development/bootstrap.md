# QADREN v0.1.0 Bootstrap

Status: historical description of the first prototype. Current `main` includes Yandex metadata-only integration; see README for current status.

## Included
- Dependency-free Rust core with small, versioned C ABI.
- Idle Worker skeleton and capability-only storage adapter contract.
- Native SwiftUI window and MenuBarExtra (macOS 13+).
- Local build script that produces `dist/QADREN.app` with the Rust static library linked into the executable.
- Rust tests, formatting and clippy script.

## Developer prerequisites
Mac with compatible Xcode Command Line Tools / Xcode, supported Rust toolchain with a macOS target. **End users do not install these tools.**

## Build on macOS
```bash
bash scripts/test-core.sh
bash scripts/build-macos.sh
open dist/QADREN.app
```

The application shows the core version/status from **real Rust FFI calls**, and the local Worker status. The menu bar entry shows the status and opens the window. No OAuth, iCloud entitlement, network, background scheduler or file transfer exists.

## Check list on Mac
- [ ] Rust tests pass.
- [ ] SwiftUI app builds and opens.
- [ ] The window shows version 0.1.0; the system signals an ABI mismatch.
- [ ] The menu bar item opens the window; Quit exits.
- [ ] Activity Monitor: idle CPU and memory measurements are recorded.
- [ ] No application-originated downloads or uploads occur.
- [ ] The app works without developer tools on the destination Mac (after proper distribution/signing).

## Known limitations
The script builds only the host architecture (arm64 or x86_64), not universal binaries. The script does not sign or notarize. The bundle identifier is temporary. You must verify the SwiftUI launch, linker behavior and clean-Mac installation on macOS. Do not consider this a release, agent, live iCloud adapter or production backup engine.

## Next steps
See `docs/roadmap.md` for the current Scenario and Job development sequence. Earlier limitations document the bootstrap milestone, not the current entire product. Do not introduce inter-node connection in this phase.
