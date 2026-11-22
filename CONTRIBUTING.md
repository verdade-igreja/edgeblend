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
cargo test -p edgeblend-config -p edgeblend-geom -p edgeblend-remote
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

## Phone remote page

The phone page lives in `crates/edgeblend-remote/web` (Vite + Vue 3 + TypeScript + Tailwind +
shadcn-vue). It is built into **one self-contained file**, `web/dist/index.html`, which the Rust
crate embeds with `include_str!`. The built file is committed, so `cargo build` never needs Node;
CI fails if it is stale.

```
cd crates/edgeblend-remote/web
npm ci
npm run build        # type-check, build dist/index.html, check it has no external URLs / v-html
```

**Commit `dist/index.html` together with any change under `web/`.**

Live development against a running server (any OS, no Windows app needed):

```
cargo run -p edgeblend-remote --example serve          # prints http://127.0.0.1:7878/?t=<token>
cd crates/edgeblend-remote/web
npm run dev                                            # Vite proxies /api to 127.0.0.1:7878
```

Open `http://localhost:5173/?t=<token>` with the token the example printed. Set `EDGEBLEND_URL`
(e.g. `EDGEBLEND_URL=http://192.168.1.20:7878 npm run dev`) to proxy to the real app on a Windows PC.
The example uses a scratch config in the temp folder (pass a path as the second argument to change it).

Add shadcn-vue components with `npx shadcn-vue@latest add <name>`. The generated components target
newer reka-ui attributes; after adding one, make sure its `data-*:` variants match what reka-ui
emits (`data-[state=checked]`, `data-[state=active]`, `data-[orientation=horizontal]`, ...), and
remove any Google Fonts `@import` the CLI adds to `src/style.css`: the page must work without
internet.

## Releasing

1. Bump `[workspace.package] version` in `Cargo.toml` and commit.
2. `git tag vX.Y.Z && git push origin vX.Y.Z`.

The `release` workflow checks the tag against the version, runs the tests, builds the installer and
publishes the GitHub release with `edgeblend-setup-X.Y.Z.exe`, `edgeblend-setup.exe` and
`SHA256SUMS.txt`.
