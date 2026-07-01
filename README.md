<p align="center">
  <img src="assets/icon-256.png" width="120" alt="audiobookshelf" />
</p>

# audiobookshelf

Audiobookshelf is a self-hosted audiobook and podcast server with apps for every platform.

A first-party [orca](https://github.com/argyle-labs/orca) plugin (service-backend).

This repo is **self-contained** — the steps below run audiobookshelf **by hand, without orca**. orca automates exactly this (same image, ports, and data) through one generic surface.

---

## Run it without orca

### Docker / Podman

```yaml
# compose.yml
services:
  audiobookshelf:
    image: ghcr.io/advplyr/audiobookshelf:latest
    container_name: audiobookshelf
    restart: unless-stopped
    ports:
      - "13378:80/tcp"   # web UI
    volumes:
      - ./config:/config
      - ./metadata:/metadata
      - /path/to/audiobooks:/audiobooks
```

```sh
docker compose up -d
```

Podman: the same file with `podman-compose up -d`.

### Ports & data

| | |
|---|---|
| Default port | `13378` |
| Upstream | <https://www.audiobookshelf.org/> |
| Operator notes | [audiobookshelf.md](docs/audiobookshelf.md) |


### Backup & restore

Back up the config/data volume(s) above — that's the whole service state (stop the container first for a clean copy). Restore by putting them back and starting it.

> With orca this is **`service.backup` / `service.restore`** — location-agnostic (docker / podman / lxc / vm), one command regardless of where audiobookshelf runs. No per-service backup script.

## With orca

orca drives this plugin through the single generic `service.*` surface — no per-plugin tools:

```sh
orca service.deploy audiobookshelf      # render + launch on any supported runtime
orca service.status audiobookshelf      # health + rich diagnostics (typed payload)
orca service.backup audiobookshelf      # location-agnostic backup (tar; PBS on Proxmox)
orca service.configure audiobookshelf   # apply config via the upstream API
```

## Layout

- `src/` — the plugin (pure Rust): the `ServiceBackend` descriptor + `configure` / `status`.
- `docs/` — standalone operator notes.
- [CAPABILITIES.md](CAPABILITIES.md) — the service-backend contract checklist.
- `assets/` — plugin icon.
