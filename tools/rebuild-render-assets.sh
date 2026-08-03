#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "${script_dir}/.." && pwd)
image_name=capablanca-render-assets:ktx-4.4.2
scope=${1:-all}

if (( $# > 1 )); then
    echo "Usage: $0 [all|board|environment]" >&2
    exit 2
fi
case "${scope}" in
    all|board|environment) ;;
    *)
        echo "Usage: $0 [all|board|environment]" >&2
        exit 2
        ;;
esac

docker build \
    --tag "${image_name}" \
    "${script_dir}/render-assets"

docker run --rm \
    --user "$(id -u):$(id -g)" \
    --env "RENDER_ASSET_SCOPE=${scope}" \
    --volume "${repo_root}/bevy-front/assets/textures:/workspace/textures" \
    "${image_name}"
