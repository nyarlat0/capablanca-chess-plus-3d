#!/usr/bin/env bash
set -euo pipefail

source_dir=${SOURCE_DIR:-/workspace/textures}
output_dir=${OUTPUT_DIR:-${source_dir}/generated}
scope=${RENDER_ASSET_SCOPE:-all}
work_dir=$(mktemp -d)
trap 'rm -rf "${work_dir}"' EXIT

case "${scope}" in
    all)
        build_environment=true
        build_board=true
        ;;
    environment)
        build_environment=true
        build_board=false
        ;;
    board)
        build_environment=false
        build_board=true
        ;;
    *)
        echo "Unknown render asset scope: ${scope} (expected all, environment, or board)" >&2
        exit 2
        ;;
esac

skybox_names=(
    Right_4K_TEX.png
    Left_4K_TEX.png
    Up_4K_TEX.png
    Down_4K_TEX.png
    Front_4K_TEX.png
    Back_4K_TEX.png
)
pbr_names=(
    Marble001_1K-PNG_Color.png
    Marble001_1K-PNG_NormalGL.png
    Marble001_1K-PNG_Roughness.png
    Marble002_1K-PNG_Color.png
    Marble002_1K-PNG_NormalGL.png
    Marble002_1K-PNG_Roughness.png
    Wood066_1K-PNG_Color.png
    Wood066_1K-PNG_NormalGL.png
    Wood066_1K-PNG_Roughness.png
)

mkdir -p "${work_dir}/faces" "${work_dir}/pbr" "${work_dir}/out"

if [[ "${build_environment}" == true ]]; then
    for name in "${skybox_names[@]}"; do
        if [[ ! -f "${source_dir}/${name}" ]]; then
            echo "Missing source texture: ${source_dir}/${name}" >&2
            exit 1
        fi
    done

    first_geometry=$(identify -format '%wx%h' "${source_dir}/${skybox_names[0]}")
    face_size=${first_geometry%x*}
    face_height=${first_geometry#*x}
    if [[ "${face_size}" != "${face_height}" ]]; then
        echo "Skybox faces must be square; got ${first_geometry}" >&2
        exit 1
    fi
    for name in "${skybox_names[@]}"; do
        geometry=$(identify -format '%wx%h' "${source_dir}/${name}")
        if [[ "${geometry}" != "${first_geometry}" ]]; then
            echo "Skybox face ${name} is ${geometry}; expected ${first_geometry}" >&2
            exit 1
        fi
    done

    # The original runtime loader flopped every face before assembling the GPU
    # cubemap. Preserve that established orientation in the generated asset.
    for index in "${!skybox_names[@]}"; do
        convert "${source_dir}/${skybox_names[index]}" -flop \
            "${work_dir}/faces/${index}.png"
    done

    ktx create \
        --format R8G8B8A8_SRGB \
        --assign-tf srgb \
        --cubemap \
        --generate-mipmap \
        --mipmap-filter lanczos4 \
        --encode uastc \
        --uastc-quality 2 \
        --uastc-rdo \
        --uastc-rdo-l 0.75 \
        --zstd 18 \
        "${work_dir}/faces/0.png" \
        "${work_dir}/faces/1.png" \
        "${work_dir}/faces/2.png" \
        "${work_dir}/faces/3.png" \
        "${work_dir}/faces/4.png" \
        "${work_dir}/faces/5.png" \
        "${work_dir}/out/space_skybox.native.ktx2"

    # WebGL2 guarantees ETC2. Transcoding once here avoids shipping the C++ Basis
    # Universal transcoder inside the wasm module and removes startup CPU work.
    ktx transcode --target etc-rgb --zstd 18 \
        "${work_dir}/out/space_skybox.native.ktx2" \
        "${work_dir}/out/space_skybox.ktx2"

    # glTF-IBL-Sampler accepts an equirectangular panorama. Prefer a supplied HDR
    # source, but retain the six-face PNG workflow by converting it deterministically.
    if [[ -f "${source_dir}/environment.hdr" ]]; then
        ibl_source="${source_dir}/environment.hdr"
    elif [[ -f "${source_dir}/environment.exr" ]]; then
        convert "${source_dir}/environment.exr" "${work_dir}/environment.hdr"
        ibl_source="${work_dir}/environment.hdr"
    else
        montage \
            "${work_dir}/faces/0.png" \
            "${work_dir}/faces/1.png" \
            "${work_dir}/faces/2.png" \
            "${work_dir}/faces/3.png" \
            "${work_dir}/faces/4.png" \
            "${work_dir}/faces/5.png" \
            -tile 3x2 -geometry "${face_size}x${face_size}+0+0" \
            "${work_dir}/cubemap-c3x2.png"
        ffmpeg -hide_banner -loglevel error -y \
            -i "${work_dir}/cubemap-c3x2.png" \
            -vf "v360=input=c3x2:output=equirect:in_forder=rludfb:w=2048:h=1024" \
            -frames:v 1 "${work_dir}/environment.png"
        ibl_source="${work_dir}/environment.png"
    fi

    # Lavapipe makes the Vulkan filtering step work on machines without a GPU
    # passed through to Docker. The work is offline, so the slower CPU path is fine.
    lavapipe_icd=$(find /usr/share/vulkan/icd.d -name 'lvp_icd*.json' -print -quit)
    if [[ -z "${lavapipe_icd}" ]]; then
        echo "Mesa Lavapipe Vulkan driver was not found in the container" >&2
        exit 1
    fi
    export VK_DRIVER_FILES="${lavapipe_icd}"
    ln -s /opt/ibl/shaders "${work_dir}/shaders"

    (
        cd "${work_dir}"
        LD_LIBRARY_PATH="/opt/ibl:/opt/ktx/lib" cli \
            -inputPath "${ibl_source}" \
            -outCubeMap "${work_dir}/space_diffuse.raw.ktx2" \
            -outLUT "${work_dir}/diffuse-lut.png" \
            -distribution Lambertian \
            -sampleCount 1024 \
            -cubeMapResolution 64 \
            -mipLevelCount 1 \
            -targetFormat R16G16B16A16_SFLOAT

        LD_LIBRARY_PATH="/opt/ibl:/opt/ktx/lib" cli \
            -inputPath "${ibl_source}" \
            -outCubeMap "${work_dir}/space_specular.raw.ktx2" \
            -outLUT "${work_dir}/specular-lut.png" \
            -distribution GGX \
            -sampleCount 1024 \
            -cubeMapResolution 512 \
            -mipLevelCount 10 \
            -targetFormat R16G16B16A16_SFLOAT
    )

    ktx deflate --zstd 18 \
        "${work_dir}/space_diffuse.raw.ktx2" "${work_dir}/out/space_diffuse.ktx2"
    ktx deflate --zstd 18 \
        "${work_dir}/space_specular.raw.ktx2" "${work_dir}/out/space_specular.ktx2"
fi

create_color_texture() {
    local source_name=$1
    local output_stem=$2
    local output_name=${output_stem}.ktx2
    local intermediate="${work_dir}/out/${output_stem}.native.ktx2"
    convert "${source_dir}/${source_name}" -alpha off -colorspace sRGB \
        "${work_dir}/pbr/${output_stem}.png"
    ktx create \
        --format R8G8B8A8_SRGB \
        --assign-tf srgb \
        --generate-mipmap --mipmap-filter lanczos4 --mipmap-wrap wrap \
        --encode uastc --uastc-quality 2 --uastc-rdo --uastc-rdo-l 0.75 \
        --zstd 18 \
        "${work_dir}/pbr/${output_stem}.png" "${intermediate}"
    ktx transcode --target etc-rgb --zstd 18 \
        "${intermediate}" "${work_dir}/out/${output_name}"
}

create_normal_texture() {
    local source_name=$1
    local output_stem=$2
    local output_name=${output_stem}.ktx2
    local intermediate="${work_dir}/out/${output_stem}.native.ktx2"
    # Normal maps contain linear vector data even though JPEG commonly labels
    # them as sRGB. --assign-tf avoids altering those stored vector components.
    convert "${source_dir}/${source_name}" -alpha off \
        "${work_dir}/pbr/${output_stem}.png"
    ktx create \
        --format R8G8B8A8_UNORM --assign-tf linear --normalize \
        --generate-mipmap --mipmap-filter lanczos4 --mipmap-wrap wrap \
        --encode uastc --uastc-quality 2 --uastc-rdo --uastc-rdo-l 0.5 \
        --zstd 18 \
        "${work_dir}/pbr/${output_stem}.png" "${intermediate}"
    ktx transcode --target etc-rgb --zstd 18 \
        "${intermediate}" "${work_dir}/out/${output_name}"
}

create_roughness_texture() {
    local source_name=$1
    local output_stem=$2
    local output_name=${output_stem}.ktx2
    local intermediate="${work_dir}/out/${output_stem}.native.ktx2"
    # Expand grayscale to RGB: StandardMaterial samples roughness from G.
    convert "${source_dir}/${source_name}" -alpha off -colorspace Gray \
        "${work_dir}/pbr/${output_stem}.png"
    ktx create \
        --format R8G8B8A8_UNORM --assign-tf linear \
        --generate-mipmap --mipmap-filter lanczos4 --mipmap-wrap wrap \
        --encode uastc --uastc-quality 2 --uastc-rdo --uastc-rdo-l 0.75 \
        --zstd 18 \
        "${work_dir}/pbr/${output_stem}.png" "${intermediate}"
    ktx transcode --target etc-rgb --zstd 18 \
        "${intermediate}" "${work_dir}/out/${output_name}"
}

if [[ "${build_board}" == true ]]; then
    for name in "${pbr_names[@]}"; do
        if [[ ! -f "${source_dir}/${name}" ]]; then
            echo "Missing source texture: ${source_dir}/${name}" >&2
            exit 1
        fi
    done

    create_color_texture Marble001_1K-PNG_Color.png white_marble_color
    create_color_texture Marble002_1K-PNG_Color.png black_marble_color
    create_color_texture Wood066_1K-PNG_Color.png wood_color
    create_normal_texture Marble001_1K-PNG_NormalGL.png white_marble_normalgl
    create_normal_texture Marble002_1K-PNG_NormalGL.png black_marble_normalgl
    create_normal_texture Wood066_1K-PNG_NormalGL.png wood_normalgl
    create_roughness_texture Marble001_1K-PNG_Roughness.png white_marble_roughness
    create_roughness_texture Marble002_1K-PNG_Roughness.png black_marble_roughness
    create_roughness_texture Wood066_1K-PNG_Roughness.png wood_roughness
fi

for texture in "${work_dir}/out"/*.ktx2; do
    ktx validate "${texture}"
done

mkdir -p "${output_dir}"
for texture in "${work_dir}/out"/*.ktx2; do
    install -m 0644 "${texture}" "${output_dir}/$(basename "${texture}")"
done

echo "Generated ${scope} render assets in ${output_dir}"
