# Contributing

Bug reports and feature requests: please use the GitHub issues. Pull requests are welcome.

## Build

edgeblend is Windows-only (Direct3D 11, DXGI Desktop Duplication). On Windows install Rust stable
with the **MSVC** toolchain (`x86_64-pc-windows-msvc`) and the Visual Studio Build Tools (C++
workload), then:

```
cargo build --release
```

The GNU/MinGW toolchain is not supported.

## Tests

```
cargo test --workspace
```

runs everything on Windows (including a test that the shader compiles). The host-portable subset
also runs on macOS and Linux:

```
cargo test -p edgeblend-config -p edgeblend-geom
```

## Type-checking from macOS / Linux

```
rustup target add x86_64-pc-windows-msvc
cargo check --target x86_64-pc-windows-msvc --all-targets
```

This compiles and type-checks every crate (the `windows` crate is plain Rust declarations) but stops
before linking, so no `.exe` is produced. Runtime behaviour has to be tested on Windows.

## Dev loop

`scripts\dev.ps1` builds, starts the app with blending on and watches the sources. Editing
`crates/*/src/*.rs` or a `Cargo.toml` rebuilds and restarts; editing `blend.hlsl` reloads the shader
inside the running app. If PowerShell blocks the script:
`powershell -ExecutionPolicy Bypass -File .\scripts\dev.ps1`.

## Releasing

1. Bump `[workspace.package] version` in `Cargo.toml` and commit.
2. `git tag vX.Y.Z && git push origin vX.Y.Z`.

The `release` workflow checks the tag against the version, runs the tests, builds the installer and
publishes the GitHub release with `edgeblend-setup-X.Y.Z.exe`, `edgeblend-setup.exe` and
`SHA256SUMS.txt`.
