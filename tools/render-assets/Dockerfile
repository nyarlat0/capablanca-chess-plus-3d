FROM ubuntu:24.04 AS builder

ARG DEBIAN_FRONTEND=noninteractive
ARG IBL_SAMPLER_COMMIT=08fa12250f6f5f5969c1846aaa266e6eea887731
ARG KTX_SOFTWARE_COMMIT=936b655d10fe75f900967f524ba31005bebcbb47

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        cmake \
        git \
        libvulkan-dev \
        ninja-build \
        pkg-config \
        python3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY ibl-sampler.patch /src/ibl-sampler.patch

# The sampler keeps its own tested KTX revision as a submodule. It is built
# independently from the newer KTX command-line tools below.
RUN git clone --recurse-submodules https://github.com/KhronosGroup/glTF-IBL-Sampler.git ibl-sampler \
    && cd ibl-sampler \
    && git checkout "${IBL_SAMPLER_COMMIT}" \
    && git submodule update --init --recursive \
    && git apply /src/ibl-sampler.patch \
    && cmake -S . -B build -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_LIBRARY_OUTPUT_DIRECTORY=/opt/ibl \
        -DCMAKE_RUNTIME_OUTPUT_DIRECTORY=/opt/ibl \
        -DIBLSAMPLER_EXPORT_SHADERS=OFF \
        -DIBLSAMPLER_SHADERS_DIR=shaders \
    && cmake --build build --parallel \
    && mkdir -p /opt/ibl/shaders \
    && cp -a lib/source/shaders/. /opt/ibl/shaders/ \
    && find build/Ktx -name 'libktx.so*' -exec cp -P '{}' /opt/ibl/ \;

RUN git clone --recurse-submodules https://github.com/KhronosGroup/KTX-Software.git ktx-software \
    && cd ktx-software \
    && git checkout "${KTX_SOFTWARE_COMMIT}" \
    && git submodule update --init --recursive \
    && cmake -S . -B build -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX=/opt/ktx \
        -DBUILD_TESTING=OFF \
        -DKTX_FEATURE_DOC=OFF \
        -DKTX_FEATURE_GL_UPLOAD=OFF \
        -DKTX_FEATURE_LOADTEST_APPS=OFF \
        -DKTX_FEATURE_TESTS=OFF \
        -DKTX_FEATURE_TOOLS=ON \
        -DKTX_FEATURE_VK_UPLOAD=OFF \
    && cmake --build build --parallel \
    && cmake --install build

FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        ffmpeg \
        imagemagick \
        libvulkan1 \
        mesa-vulkan-drivers \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /opt/ibl /opt/ibl
COPY --from=builder /opt/ktx /opt/ktx
COPY rebuild.sh /usr/local/bin/rebuild-render-assets

ENV PATH="/opt/ktx/bin:/opt/ibl:${PATH}"
# KTX CLI uses the current library. rebuild.sh overrides this only for the
# legacy glTF-IBL-Sampler binary, which must use its bundled libktx ABI.
ENV LD_LIBRARY_PATH="/opt/ktx/lib:/opt/ibl"

WORKDIR /opt/ibl

ENTRYPOINT ["/usr/local/bin/rebuild-render-assets"]
