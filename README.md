# Capablanca Chess Plus 3D

A 3D chess application for Capablanca-family variants and Grand Chess, written
in Rust with Bevy 0.19. It runs as a native desktop application or as a WebAssembly
frontend in the browser, with Local, Fairy-Stockfish, and online multiplayer modes.

The project is under active development. The rules engine and multiplayer server
validate moves independently from the presentation layer; the Bevy frontend is
responsible for interaction, animation, sound, and rendering.

## Highlights

- Seven playable variants on 10x8 and 10x10 boards.
- Complete legal move generation, check, mate, stalemate, castling, en passant,
  promotion, repetition tracking, and extended FEN support.
- A Bevy 0.19 frontend with animated pieces, captured-piece trays, a 3D promotion
  picker, automatic camera turns, mouse and touch controls, sound, and game-state UI.
- Marble and wood PBR materials, image-based lighting, a nebula skybox, bloom,
  shadows, and optimized planar reflections of the pieces on the board.
- Local pass-and-play, configurable Fairy-Stockfish AI, and server-authoritative
  multiplayer over WebSockets.
- Browser reconnection through per-player tokens stored in `localStorage`; only
  token hashes and the authoritative move history are stored in PostgreSQL.

## Variants

| Variant | Board | Back rank / arrangement | Castling |
| --- | --- | --- | --- |
| Capablanca | 10x8 | `RNABQKBCNR` | King `f` to `c` or `i` |
| Gothic | 10x8 | `RNBQCKABNR` | King `f` to `c` or `i` |
| Embassy | 10x8 | `RNBQKCABNR` | King `e` to `b` or `h` |
| Schoolbook | 10x8 | `RQNBAKBNCR` | King `f` to `c` or `i` |
| Bird | 10x8 | `RNBCQKABNR` | King `f` to `c` or `i` |
| Carrera | 10x8 | `RCNBKQBNAR` | None under the historical preset |
| Grand | 10x10 | Grand Chess arrangement | None |

`A` is the archbishop/cardinal (bishop + knight). `C` is the
chancellor/marshal (rook + knight). Grand Chess promotion uses captured
material: an eligible captured piece is returned to play when selected for
promotion.

## Workspace layout

| Path | Purpose |
| --- | --- |
| [`engine`](engine/) | Dependency-free rules, game state, move generation, FEN, and reference search engine. |
| [`bevy-front`](bevy-front/) | Bevy desktop/WASM client, rendering, UI, animation, audio, Fairy-Stockfish integration, and multiplayer client. |
| [`multiplayer-protocol`](multiplayer-protocol/) | Shared versioned WebSocket message types. |
| [`backend`](backend/) | Actix WebSocket server with authoritative validation and PostgreSQL persistence. |
| [`tools`](tools/) | Reproducible KTX2 board-texture, skybox, and IBL generation. |

## Requirements

- Rust 1.95 or newer for the complete workspace and Bevy frontend.
- The usual [Bevy Linux dependencies](https://github.com/bevyengine/bevy/blob/main/docs/linux_dependencies.md)
  when building the native frontend on Linux.
- PostgreSQL for multiplayer development or deployment.
- Docker only when rebuilding generated render assets; it is not needed for an
  ordinary build.

The bundled native Fairy-Stockfish executable is for x86-64 Linux. On another
desktop architecture, set `FAIRY_STOCKFISH_PATH` to a compatible
Fairy-Stockfish executable. Browser AI uses the bundled WebAssembly worker and
does not depend on the server CPU architecture.

## Quick start

Run the desktop client from the workspace root:

```sh
cargo run -p bevy-front
```

Run all tests:

```sh
cargo test --workspace
```

The first build is large because Bevy and its renderer must be compiled. The
workspace uses the normal Cargo `target` directory so subsequent builds can
reuse it.

## Controls

### Desktop

- Left click: select a piece or destination.
- Left drag: orbit the camera.
- Right drag: pan the camera.
- Mouse wheel: zoom.
- Middle click: smoothly recenter the complete camera on the current player in
  Local mode, or on the local player in AI and Multiplayer modes.
- `Escape`: cancel the current selection.

### Touch

- Tap: select a piece or destination.
- One-finger drag: orbit the camera.
- Two-finger drag: pan the camera.
- Pinch with two fingers: zoom.

## Game modes

- **Local** is pass-and-play on one device. The camera begins on White's side
  and turns after each completed move animation.
- **AI** runs Fairy-Stockfish in a native child process or a dedicated browser
  Web Worker. Difficulty is selected in the new-game menu.
- **Multiplayer** creates or joins a room using its public game ID. Each color
  receives a separate secret player token for reconnecting without accounts.

## Multiplayer development

The server exposes `GET /health` and `GET /ws`. Create a PostgreSQL database,
then start the backend:

```sh
createdb capablanca
export DATABASE_URL=postgres://127.0.0.1/capablanca
cargo run -p backend
```

`BIND_ADDR` defaults to `127.0.0.1:8080`. The native client connects to
`ws://127.0.0.1:8080/ws` by default. Override it when needed:

```sh
CAPABLANCA_WS_URL=ws://127.0.0.1:9000/ws cargo run -p bevy-front
```

The server applies database migrations at startup. A client sends only a move
and its current revision; the server authenticates its player token, checks the
turn and legality, commits the move, and then broadcasts the accepted revision.
The complete history is sent only on join, reconnect, or resynchronization.
The Bevy client does not apply an online move before server confirmation.

See [`backend/README.md`](backend/README.md) for environment and security details.

## Browser build

Install the WebAssembly target and build the frontend:

```sh
rustup target add wasm32-unknown-unknown
cargo build --locked --release -p bevy-front --target wasm32-unknown-unknown
```

Package the resulting WASM binary with a `wasm-bindgen-cli` version matching
`Cargo.lock`, and serve it together with the complete runtime `assets`
directory. If the frontend and backend share an origin, leave
`CAPABLANCA_WS_URL` unset and the client will derive `ws://.../ws` or
`wss://.../ws` from the page URL.

Fairy-Stockfish uses WebAssembly threads and `SharedArrayBuffer`. A production
server must use HTTPS and return these headers for the page and its assets:

```text
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

The reverse proxy must forward `/ws` to the Actix backend with WebSocket
upgrade headers. Building the frontend and cross-compiling the small backend on
a workstation allows deployment to a weak RISC-V server without installing a
Rust toolchain or compiling on the server.

See [`bevy-front/README.md`](bevy-front/README.md) for the complete browser,
Fairy-Stockfish, and asset requirements.

## Render assets

Generated browser and native KTX2 textures are committed under
`bevy-front/assets/textures/generated`. Rebuild only the affected group:

```sh
./tools/rebuild-render-assets.sh board
./tools/rebuild-render-assets.sh environment
./tools/rebuild-render-assets.sh all
```

The script uses a pinned Docker toolchain and keeps board textures separate
from the expensive skybox and IBL pipeline.

## Inspiration and acknowledgements

The Bevy frontend originally took its starting point from
[Stefan Salewski's Bevy-3D-Chess](https://github.com/StefanSalewski/Bevy-3D-Chess).
Many thanks to Dr. Stefan Salewski for publishing that approachable 3D chess
example. This project's frontend has since been extensively reworked for Bevy
0.19, Capablanca-family rules, a modular architecture, modern rendering,
Fairy-Stockfish, and online play.

The upstream project is MIT-licensed. Its required copyright and permission
notice is preserved in [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).

AI mode uses [Fairy-Stockfish](https://github.com/fairy-stockfish/Fairy-Stockfish).
Exact native and browser versions, upstream sources, authors, and the GPLv3
license are documented in
[`bevy-front/assets/engine/THIRD_PARTY.md`](bevy-front/assets/engine/THIRD_PARTY.md).

## License

Except for third-party components identified in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) or carrying their own
license notice, this project is released into the public domain under
[The Unlicense](LICENSE).

Third-party code, dependencies, models, textures, audio, fonts, and bundled
binaries remain subject to their respective license terms and are not
relicensed by the project's Unlicense.
