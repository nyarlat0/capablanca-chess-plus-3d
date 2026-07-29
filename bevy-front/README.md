# Capablanca Chess Plus 3D frontend

This crate is the Bevy 0.19 frontend for `capablanca-engine`. It supports all
seven built-in variants, runtime 10×8 and 10×10 boards, human or engine control
for either side, legal-move highlighting, promotion choices, and asynchronous
engine searches.

## Requirements

- Rust 1.95 or newer (required by Bevy 0.19)
- The usual Bevy native dependencies for your platform

With rustup, install and select a suitable toolchain with:

```sh
rustup toolchain install 1.95
rustup override set 1.95
```

Then run from the workspace root:

```sh
cargo run -p bevy-front
```

## Controls

- Left click: select a piece and its destination
- Middle mouse drag: orbit the camera
- Right mouse drag: pan
- Mouse wheel: zoom
- `1` / `2`: toggle human or computer control for White / Black
- Arrow up/down or `+`/`-`: change engine search depth
- `N`: restart the current variant
- `F1` through `F7`: Capablanca, Gothic, Embassy, Schoolbook, Bird, Carrera,
  and Grand Chess
- `Escape`: cancel selection or promotion
- Promotion: `Q`, `C`, `A`, `R`, `B`, or `N` (knight); Space keeps a Grand
  Chess pawn unpromoted where the rules allow it
