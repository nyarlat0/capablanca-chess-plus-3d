# Fairy-Stockfish

This directory contains binaries from Fairy-Stockfish, licensed under the GNU
General Public License version 3.

## Native Linux engine

- Version: Fairy-Stockfish 14, large-board build
- Source tag: <https://github.com/fairy-stockfish/Fairy-Stockfish/tree/fairy_sf_14>
- Build: `make -j2 build ARCH=x86-64 COMP=gcc largeboards=yes`

The executable is a portable x86-64 Linux build. Set the
`FAIRY_STOCKFISH_PATH` environment variable to use another compatible
Fairy-Stockfish executable.

## Browser engine

- npm package: `fairy-stockfish-nnue.wasm@1.1.11`
- Package integrity: `sha512-D5pocLErreRW+S1EPhprfeKo7iVWzLBIgWp5e8oRySDuNXopill2fRamUBRW4syWLAo4GMRr4noiE5b5F2sRkQ==`
- Exact upstream commit: <https://github.com/fairy-stockfish/fairy-stockfish.wasm/tree/5589ea54f322e8e76c199440e55ae39fe5d3b09c>

The unmodified `stockfish.js`, `stockfish.wasm`, and
`stockfish.worker.js` files come from that package. The small
`fairy-stockfish-client.worker.js` bridge is part of this frontend.

The full GPLv3 license text is in `Copying.txt` next to this file.
