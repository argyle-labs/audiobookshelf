# Audiobookshelf

Audiobooks and podcasts server with progress tracking.

- **Port**: 13378 (configurable via `AUDIOBOOKSHELF_PORT`)
- **Image**: `ghcr.io/advplyr/audiobookshelf`
- **Compose**: [compose.yml](../compose.yml)

## Deploy

```bash
docker compose up -d
```

## Environment Variables

| Variable                       | Default            | Description               |
| ------------------------------ | ------------------ | ------------------------- |
| `TZ`                           | `Etc/UTC`   | Timezone                  |
| `AUDIOBOOKSHELF_IMAGE_TAG`     | `latest`           | Image tag                 |
| `AUDIOBOOKSHELF_CONFIG_PATH`   | `./config`         | Config directory          |
| `AUDIOBOOKSHELF_METADATA_PATH` | `./config/metadata`| Metadata directory        |
| `AUDIOBOOKSHELF_PORT`          | `13378`            | Host port                 |
| `MEDIA_PATH`                   | *(required)*       | Base path — audiobooks/podcasts/books subdirs are mounted |

## Initial Setup

Create user, add libraries for audiobooks and podcasts.

## Troubleshooting

```bash
docker compose logs audiobookshelf
```

## Scan-on-import integration (orca)

New audiobooks land on the willow share, so ABS's watcher does not fire — a push
scan is required. ABS has no native *arr connector, so scans are triggered by the
[download-client scan dispatcher](../../sabnzbd/docs/scan-dispatcher.md) on `books`/
`audiobooks` category completions.

**API:** `POST /login` `{username,password}` → `user.token`, then
`POST /api/libraries/{libraryId}/scan` with `Authorization: Bearer <token>`.
`GET /api/libraries` lists ids. Libraries: **Audiobooks**
`d1d6751d-cf80-4746-b301-c03b3ae1fe89`, **podcasts**
`13ba8800-1315-4363-84e2-7956fa118d23`. Server 2.36 also supports non-expiring API
keys (`/api/api-keys`) — the dispatcher currently does login→scan for durability
against token expiry.

**Creds:** 1Password `audiobookshelf (orca)` (orca vault). **Endpoint:**
`http://10.10.10.6:13378` (baldur).

**Follow-up:** Libation (Audible) writes audiobooks without going through the
download clients — it needs its own post-download hook to trigger a scan here.

**Future plugin capability:** expose `scan` + a `configure` that registers the
import hook and (optionally) mints a dedicated API key. See [CAPABILITIES.md](../CAPABILITIES.md).
