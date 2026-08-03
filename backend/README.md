# Multiplayer backend

The backend exposes `GET /health` and one WebSocket endpoint per player at
`GET /ws`. PostgreSQL migrations run automatically at startup, so the only
external service required for local multiplayer development is PostgreSQL.

Create an empty database, then either export the settings or copy
`.env.example` to the ignored `backend/.env` and edit it:

```bash
createdb capablanca
export DATABASE_URL=postgres://127.0.0.1/capablanca
cargo run -p backend
```

`BIND_ADDR` defaults to `127.0.0.1:8080`. In production, terminate TLS in a
reverse proxy and forward `/ws` to this service. The browser frontend then uses
the same-origin `wss://<host>/ws` endpoint automatically.

For native frontend development the default URL is
`ws://127.0.0.1:8080/ws`. Override it at runtime when necessary:

```bash
CAPABLANCA_WS_URL=ws://127.0.0.1:9000/ws cargo run -p bevy-front
```

For a browser build the override is compiled into the WASM module, so set it
for the build command. It is unnecessary when the reverse proxy exposes `/ws`
on the frontend origin:

```bash
CAPABLANCA_WS_URL=wss://play.example.com/ws \
  cargo build -p bevy-front --release --target wasm32-unknown-unknown
```

Each browser stores its returned player token in `localStorage`. The database
stores only SHA-256 token hashes. Keep PostgreSQL private and serve production
WebSockets over TLS, because possession of a player token grants control of
that color without an account.
