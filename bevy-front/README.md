# Capablanca Chess Plus 3D frontend

This crate is the Bevy 0.19 frontend for `capablanca-engine`. It supports all
seven built-in variants, runtime 10×8 and 10×10 boards, human or engine control
for either side, legal-move highlighting, promotion choices, and asynchronous
engine searches.

## Requirements

- Rust 1.95 or newer (required by Bevy 0.19)
- The usual Bevy native dependencies for your platform
- Docker, only when rebuilding the optimized render textures

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
- Middle mouse drag: orbit the camera
- Right mouse drag: pan
- Mouse wheel: zoom
- The startup menu selects Local/AI mode, variant, and player color
- The corner arrow opens the in-game menu and its New game button
- Promotion is selected from the 3D popup
- `Escape`: cancel the current selection

## Render assets

The application loads browser-friendly KTX2 textures from
`assets/textures/generated`. Rebuild them from the source PNG/JPEG maps with:

```sh
./tools/rebuild-render-assets.sh
```

Run that command from the workspace root. The first run builds a pinned Docker
image containing Khronos KTX-Software and glTF-IBL-Sampler; subsequent runs
reuse Docker's build cache. The pipeline:

- builds the nebula skybox and its mip chain;
- prefilters diffuse and specular image-based lighting on the CPU via Lavapipe;
- builds mipmapped color, normal, and roughness maps for the board;
- emits direct ETC2 for WebGL2 plus UASTC variants for native GPU transcoding;
- applies the correct sRGB/linear transfer functions and validates every KTX2.

The six `*_2K_TEX.png` files are the default environment source. If
`assets/textures/environment.hdr` or `environment.exr` exists, it is preferred
for IBL while the six PNG faces remain the visible skybox.

The main visual controls are centralized in `src/render_tuning.rs`: environment
rotation/brightness, IBL intensity, lights and shadow distance, bloom, color
grading, vignette, and texture anisotropy. Board-material roughness and
planar-reflection strength remain next to their materials at the top of
`src/board.rs`.
