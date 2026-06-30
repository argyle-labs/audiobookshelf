# TODO: base image + build for audiobookshelf. Mirror jellyfin/Dockerfile conventions.
FROM debian:12-slim
LABEL org.opencontainers.image.source="https://github.com/argyle-labs/audiobookshelf"
EXPOSE 13378
