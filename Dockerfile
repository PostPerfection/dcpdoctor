ARG FFMPEG_URL=https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-n8.1-latest-linux64-gpl-8.1.tar.xz

FROM ubuntu:24.04 AS builder
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential cmake curl ca-certificates pkg-config git \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=/root/.cargo/bin:$PATH
WORKDIR /src
COPY . .
RUN cargo build --release -p dcpdoctor-cli --manifest-path rust/Cargo.toml

FROM ubuntu:24.04
ARG FFMPEG_URL
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3t64 libxml2-utils openjdk-21-jre-headless tini ca-certificates curl xz-utils \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL --retry 5 --retry-all-errors -o /tmp/ffmpeg.tar.xz "$FFMPEG_URL" \
    && tar -xJf /tmp/ffmpeg.tar.xz -C /tmp \
    && install -m 755 /tmp/ffmpeg-*/bin/ffmpeg /tmp/ffmpeg-*/bin/ffprobe /usr/local/bin/ \
    && rm -rf /tmp/ffmpeg.tar.xz /tmp/ffmpeg-*
COPY scripts/fetch_photon.sh /tmp/fetch_photon.sh
RUN bash /tmp/fetch_photon.sh /opt/photon && rm /tmp/fetch_photon.sh
ENV PHOTON_DIR=/opt/photon
COPY schemas /usr/local/share/dcpdoctor/schemas
ENV DCPDOCTOR_SCHEMA_DIR=/usr/local/share/dcpdoctor/schemas
COPY --from=builder /src/rust/target/release/dcpdoctor /usr/local/bin/dcpdoctor
RUN useradd -m -s /bin/bash dcpdoctor
USER dcpdoctor
WORKDIR /data
EXPOSE 8080
# dcpdoctor as pid 1 ignores SIGTERM
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/dcpdoctor"]
CMD ["--help"]
