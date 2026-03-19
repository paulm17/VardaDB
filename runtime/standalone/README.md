# Restate Standalone

`restate-standalone` uses a standalone-owned configuration surface. It does not accept
legacy multi-node configuration keys. Only the standalone runtime settings documented below are
supported.

## Supported Operator Surface

- Admin HTTP API:
  - `GET /health`
  - `GET /version`
  - `GET /`
- Ingress HTTP API:
  - `POST /rpc`

## Example Configuration

```toml
node-name = "standalone"
base-dir = "restate-data"
shutdown-timeout = "60s"

[storage]
sqlite-dir = "sqlite"

[admin]
bind-address = "127.0.0.1:9070"
listen-mode = "tcp"

[ingress]
bind-address = "127.0.0.1:8080"
listen-mode = "tcp"
```

The runtime keeps a single SQLite database at
`<base-dir>/<node-name>/<storage.sqlite-dir>/standalone.sqlite3` when `sqlite-dir` is relative.
