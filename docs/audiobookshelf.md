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
