#!/usr/bin/env bash
# Usage:  tag_image.sh  <src-ref>  [dst-tag]
#   <src-ref>  — tag (v1.2.3) | digest (sha256:…) | empty → latest
#   [dst-tag]  — the tag to write locally/remotely (defaults to *stable*)

set -euo pipefail

SRC=${1:-latest}        # what to pull
DST=${2:-stable}        # what tag to push
IMAGE=ghcr.io/verse-pbc/groups_relay

# Normalise the source reference
if [[ $SRC =~ ^sha256:[0-9A-Fa-f]{64}$ ]]; then
  REF="@${SRC}"         # content-addressable digest
else
  REF=":${SRC}"         # ordinary tag
fi

# Pull exactly that object, then retag & push
docker pull --platform linux/amd64 "${IMAGE}${REF}"
docker tag  "${IMAGE}${REF}"  "${IMAGE}:${DST}"
docker push "${IMAGE}:${DST}"
