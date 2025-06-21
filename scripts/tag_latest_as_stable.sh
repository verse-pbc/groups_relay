#!/usr/bin/env bash
# Pull the requested image (tag or digest) and retag it as :stable.

set -euo pipefail

SOURCE=${1:-latest}              # allow tag, digest or omit (=latest)
IMAGE="ghcr.io/verse-pbc/groups_relay"

# Decide whether the arg is a digest (sha256:…) or a normal tag
if [[ $SOURCE =~ ^sha256:[A-Fa-f0-9]{64}$ ]]; then
  REF="@${SOURCE}"               # digest → use @sha256:...
else
  REF=":${SOURCE}"               # tag → use :tag
fi

# Pull the exact reference, then tag & push as :stable
docker pull --platform linux/amd64 "${IMAGE}${REF}"
docker tag "${IMAGE}${REF}"      "${IMAGE}:stable"
docker push                      "${IMAGE}:stable"
