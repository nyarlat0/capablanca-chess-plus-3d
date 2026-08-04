#!/usr/bin/env bash
set -Eeuo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
asset_source="$repo_root/bevy-front/assets"
html_template="$repo_root/bevy-front/web/index.html.in"
publish_dir="$repo_root/dist/web"
staging_parent="$repo_root/dist"

cargo_command=${CAPABLANCA_CARGO:-cargo}
wasm_bindgen_command=${CAPABLANCA_WASM_BINDGEN:-wasm-bindgen}

for command in "$cargo_command" "$wasm_bindgen_command" sha256sum install rsync gzip; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Required command not found: $command" >&2
        exit 1
    fi
done

required_bindgen_version=$(sed -n '/^name = "wasm-bindgen"$/{n;s/^version = "\([^"]*\)"$/\1/p;q;}' "$repo_root/Cargo.lock")
installed_bindgen_version=$(
    "$wasm_bindgen_command" --version | sed -n 's/^wasm-bindgen //p'
)
if [[ -z "$required_bindgen_version" || "$installed_bindgen_version" != "$required_bindgen_version" ]]; then
    echo "wasm-bindgen-cli $required_bindgen_version is required; found ${installed_bindgen_version:-unknown}." >&2
    echo "Install it with: cargo install --locked wasm-bindgen-cli --version $required_bindgen_version" >&2
    exit 1
fi

mkdir -p "$staging_parent"
staging_dir=$(mktemp -d "$staging_parent/.web-build.XXXXXX")
cleanup() {
    rm -rf -- "$staging_dir"
}
trap cleanup EXIT

unversioned_assets="$staging_dir/assets"
public_dir="$staging_dir/public"
mkdir -p "$unversioned_assets/textures/generated" "$unversioned_assets/engine" "$public_dir"

# Copy only files that can be requested by the browser build. Source textures,
# native KTX2 variants, and the native Fairy-Stockfish executable do not belong
# in a web deployment.
for directory in models fonts sounds shaders; do
    rsync -a "$asset_source/$directory/" "$unversioned_assets/$directory/"
done
find "$asset_source/textures/generated" -maxdepth 1 -type f -name '*.ktx2' ! -name '*.native.ktx2' -print0 |
    sort -z |
    while IFS= read -r -d '' source_file; do
        install -m 0644 "$source_file" "$unversioned_assets/textures/generated/"
    done
for engine_file in \
    fairy-stockfish-client.worker.js \
    stockfish.js \
    stockfish.wasm \
    stockfish.worker.js \
    variants.ini; do
    install -m 0644 "$asset_source/engine/$engine_file" "$unversioned_assets/engine/$engine_file"
done

asset_hash=$(
    cd "$unversioned_assets"
    find . -type f -print0 |
        sort -z |
        xargs -0 sha256sum |
        sha256sum |
        cut -c1-16
)
asset_root="assets/$asset_hash"
mkdir -p "$public_dir/assets"
mv "$unversioned_assets" "$public_dir/$asset_root"

echo "Building Bevy WASM with asset root $asset_root"
(
    cd "$repo_root"
    CAPABLANCA_ASSET_ROOT="$asset_root" \
        "$cargo_command" build --locked --profile web-release -p bevy-front --target wasm32-unknown-unknown
)

bindgen_dir="$staging_dir/bindgen"
mkdir -p "$bindgen_dir"
"$wasm_bindgen_command" \
    --target web \
    --no-typescript \
    --remove-name-section \
    --remove-producers-section \
    --out-dir "$bindgen_dir" \
    --out-name capablanca \
    "$repo_root/target/wasm32-unknown-unknown/web-release/bevy-front.wasm"

release_hash=$(
    cd "$bindgen_dir"
    sha256sum capablanca.js capablanca_bg.wasm | sha256sum | cut -c1-16
)
release_dir="$public_dir/releases/$release_hash"
mkdir -p "$release_dir"
mv "$bindgen_dir/capablanca.js" "$bindgen_dir/capablanca_bg.wasm" "$release_dir/"

sed "s/__CAPABLANCA_RELEASE__/$release_hash/g" "$html_template" >"$public_dir/index.html"

# Precompression happens on the build machine, not on the weak production
# server. Caddy selects these sidecars without recompressing every response.
while IFS= read -r -d '' source_file; do
    gzip -9 -c "$source_file" >"$source_file.gz"
    if command -v brotli >/dev/null 2>&1; then
        brotli --quality=9 --force --output="$source_file.br" "$source_file"
    fi
done < <(
    find "$public_dir" -type f \
        \( -name '*.js' -o -name '*.wasm' -o -name '*.glb' -o -name '*.ttf' -o -name '*.wgsl' -o -name '*.ini' \) \
        -print0
)

mkdir -p "$publish_dir"
rsync -a --delete "$public_dir/" "$publish_dir/"

plain_size=$(du -sh "$publish_dir" | cut -f1)
echo "Web release $release_hash is ready in $publish_dir ($plain_size including precompressed files)."
echo "Asset release: $asset_hash"
