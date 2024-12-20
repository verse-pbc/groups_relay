#!/bin/bash
# Pull specified image (defaults to latest) and tag it as stable to trigger the deploy process

SOURCE=${1:-latest}  # Use first argument if provided, otherwise default to "latest"

docker pull --platform linux/amd64 ghcr.io/verse-pbc/groups_relay:$SOURCE && \
docker tag ghcr.io/verse-pbc/groups_relay:$SOURCE ghcr.io/verse-pbc/groups_relay:stable && \
docker push ghcr.io/verse-pbc/groups_relay:stable
