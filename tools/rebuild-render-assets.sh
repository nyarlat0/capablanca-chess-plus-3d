#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "${script_dir}/.." && pwd)
image_name=capablanca-render-assets:ktx-4.4.2

docker build \
    --tag "${image_name}" \
    "${script_dir}/render-assets"

docker run --rm \
    --user "$(id -u):$(id -g)" \
    --volume "${repo_root}/bevy-front/assets/textures:/workspace/textures" \
    "${image_name}"
