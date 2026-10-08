# QADREN v0.1.0 Bootstrap

Status: first prototype; no cloud integration or file operations.

## Included
- Dependency-free Rust core with small, versioned C ABI.
- Idle Worker skeleton and capability-only storage adapter contract.
- Native SwiftUI window and MenuBarExtra (macOS 13+).
- Local build script producing `dist/QADREN.app` with Rust static library linked into executable.
- Rust tests, formatting and clippy script.

## Developer prerequisites
Mac with compatible Xcode Command Line Tools / Xcode, supported Rust toolchain with a macOS target. **End users do not install these tools.**

## Build on macOS
```bash
bash scripts/test-core.sh
bash scripts/build-macos.sh
open dist/QADREN.app
```

The application shows core version/status from **real Rust FFI calls**, and the local Worker status. Menu bar entry shows status and opens the window. No OAuth, iCloud entitlement, network, background scheduler or file transfer exists yet.

## Check list on Mac
- [ ] Rust tests pass.
- [ ] SwiftUI app builds and opens.
- [ ] Version shown in window is 0.1.0; ABI mismatch is signaled.
- [ ] Menu bar item opens window; Quit exits.
- [ ] Activity Monitor: idle CPU and memory measurements recorded.
- [ ] No application-originated downloads/uploads occur.
- [ ] App works without developer tools installed on destination Mac (after proper distribution/signing).

## Known limitations
The script builds only the host architecture (arm64 or x86_64), not universal binaries. It does not sign or notarize. Bundle identifier is temporary. SwiftUI launch, linker behavior and clean-Mac installation must be verified on macOS. Do not consider this a release, agent, live iCloud adapter or production backup engine.

## Next steps
After validating the bootstrap on Mac: move to an Xcode project if needed for signing/entitlements, test iCloud Documents, establish production identity, and add jobs/backup policies incrementally. Do not introduce inter-node connection in this phase.
