#!/bin/bash
# Pull latest image, tag it as stable and push it to the registry so it triggers the deploy process

docker pull --platform linux/amd64 ghcr.io/verse-pbc/groups_relay:latest && \
docker tag ghcr.io/verse-pbc/groups_relay:latest ghcr.io/verse-pbc/groups_relay:stable && \
docker push ghcr.io/verse-pbc/groups_relay:stable
