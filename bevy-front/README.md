# Capablanca Chess Plus 3D frontend

This crate is the Bevy 0.19 frontend for `capablanca-engine`. It supports all
seven built-in variants, runtime 10×8 and 10×10 boards, human or engine control
for either side, two-player online rooms, legal-move highlighting, promotion
choices, and asynchronous Fairy-Stockfish searches.

## Requirements

- Rust 1.95 or newer (required by Bevy 0.19)
- The usual Bevy native dependencies for your platform
- Docker, only when rebuilding the optimized render textures

An x86-64 Linux Fairy-Stockfish 14 large-board executable is bundled for native
development. To use another build (including another operating system or CPU),
set `FAIRY_STOCKFISH_PATH` to a UCI-compatible Fairy-Stockfish executable. The
frontend still supplies its bundled `variants.ini` to that executable.

With rustup, install and select a suitable toolchain with:

```sh
rustup toolchain install 1.95
rustup override set 1.95
```

Make sure `~/.cargo/bin` appears before `/usr/bin` in `PATH`; otherwise a
distribution-provided older `cargo` can shadow the rustup toolchain.

Then run from the workspace root:

```sh
cargo run -p bevy-front
```

## Controls

- Left click: select a piece and its destination
- Left mouse drag: orbit the camera
- Middle click: smoothly recenter on the current player in Local mode, or on the
  local player in AI and Multiplayer modes
- Right mouse drag: pan
- Mouse wheel: zoom
- The startup menu selects Local/AI/Multiplayer mode, variant, player color, and
  the discrete Fairy-Stockfish strength in AI mode
- In Multiplayer, leave Game ID empty to create a room or paste an existing ID
  to join it; the browser remembers the secret player token for reconnection
- The corner arrow opens the in-game menu and its New game button
- The square-corner button below it toggles native or browser fullscreen
- Promotion is selected from the 3D popup
- `Escape`: cancel the current selection

## Browser / WASM setup

The browser engine is the pinned `fairy-stockfish-nnue.wasm@1.1.11` build and
runs in its own Web Worker. Build the deployable frontend from the workspace
root with:

```sh
./tools/build-web.sh
```

The script writes `dist/web`, embeds a content-addressed asset root in both
Bevy's asset loader and the Fairy-Stockfish worker URL, omits native/source-only
assets, and creates precompressed Brotli/Gzip files. Use the matching
[`../deploy/Caddyfile`](../deploy/Caddyfile) so the generated immutable paths
are cached while `index.html` continues to discover new releases.

Fairy-Stockfish uses WebAssembly threads and `SharedArrayBuffer`. The production
server (and the local development server) must send these headers on the page:

```text
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Serve the application over HTTPS, or from `localhost` during development; do
not open the HTML via `file://`. All engine files should be served from the same
origin. The bundled native and browser engines, their exact upstream source
links, and the GPLv3 license are documented in `assets/engine/THIRD_PARTY.md`.

The multiplayer client uses `wss://<current host>/ws` (or `ws://` on HTTP) by
default. Put the Actix backend behind that same-origin route in production. To
connect a browser development build directly to another address, set the URL
while compiling because it is embedded in the WASM module:

```sh
CAPABLANCA_WS_URL=ws://127.0.0.1:8080/ws \
  cargo build -p bevy-front --target wasm32-unknown-unknown
```

Native builds read `CAPABLANCA_WS_URL` at runtime and otherwise use
`ws://127.0.0.1:8080/ws`. Backend and PostgreSQL setup is documented in
[`../backend/README.md`](../backend/README.md).

## Render assets

The application loads browser-friendly KTX2 textures from
`assets/textures/generated`. Rebuild them from the source PNG/JPEG maps with:

```sh
./tools/rebuild-render-assets.sh
```

Run that command from the workspace root. The first run builds a pinned Docker
image containing Khronos KTX-Software and glTF-IBL-Sampler; subsequent runs
reuse Docker's build cache. The pipeline:

- builds the nebula skybox and its mip chain, with display-only contrast and
  saturation enhancement kept separate from scene lighting;
- prefilters diffuse and specular image-based lighting on the CPU via Lavapipe;
- builds mipmapped color, normal, and roughness maps for the board;
- emits direct ETC2 for WebGL2 plus UASTC variants for native GPU transcoding;
- applies the correct sRGB/linear transfer functions and validates every KTX2.

The optional target avoids rebuilding unrelated assets:

```sh
./tools/rebuild-render-assets.sh board
./tools/rebuild-render-assets.sh environment
./tools/rebuild-render-assets.sh all
```

With no target, `all` is used for backward compatibility. Use `board` after
changing marble or wood; it does not read, encode, or overwrite the skybox and
IBL files. Use `environment` only after changing the skybox or HDR environment.

The six `*_4K_TEX.png` files are the default environment source. If
`assets/textures/environment.hdr` or `environment.exr` exists, it is preferred
for IBL while the six PNG faces remain the visible skybox.

The main visual controls are centralized in `src/render_tuning.rs`: environment
rotation/brightness, IBL intensity, lights and shadow distance, bloom, color
grading, vignette, and texture anisotropy. Board-material roughness,
planar-reflection strength, and the maximum roughness blur radius remain next
to their materials at the top of `src/board.rs`.
