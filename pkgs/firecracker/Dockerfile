FROM alpine:3.23 AS downloader

ARG TARGETARCH
ARG FIRECRACKER_VERSION=v1.13.0
ARG KERNEL_VERSION=6.1.141

# Map Docker TARGETARCH to architecture prefix
ARG TARGET=${TARGETARCH/amd64/x86_64}
ARG TARGET=${TARGET/arm64/aarch64}

WORKDIR /download

RUN apk add --no-cache curl tar

# Download Firecracker binary
RUN curl -fsSL "https://github.com/firecracker-microvm/firecracker/releases/download/${FIRECRACKER_VERSION}/firecracker-${FIRECRACKER_VERSION}-${TARGET}.tgz" -o firecracker.tgz && \
    tar --no-same-owner -xzf firecracker.tgz && \
    mv release-${FIRECRACKER_VERSION}-${TARGET}/firecracker-${FIRECRACKER_VERSION}-${TARGET} /download/firecracker && \
    chmod +x /download/firecracker && \
    rm -rf firecracker.tgz release-${FIRECRACKER_VERSION}-${TARGET}

# Download Linux kernel from Firecracker CI artifacts
RUN curl -fsSL -o /download/vmlinux \
    "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.13/${TARGET}/vmlinux-${KERNEL_VERSION}" && \
    chmod 644 /download/vmlinux

# Download minimal rootfs from Firecracker CI artifacts
RUN curl -fsSL -o /download/rootfs.ext4 \
    "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.7/${TARGET}/ubuntu-22.04.ext4" && \
    chmod 644 /download/rootfs.ext4

FROM scratch

COPY --from=downloader /download/firecracker /usr/bin/firecracker
COPY --from=downloader /download/vmlinux /usr/share/firecracker/vmlinux
COPY --from=downloader /download/rootfs.ext4 /usr/share/firecracker/rootfs.ext4

LABEL org.opencontainers.image.title="firecracker"
LABEL org.opencontainers.image.description="AWS Firecracker microVM hypervisor with kernel and rootfs"
LABEL org.opencontainers.image.version="1.13.0"
LABEL org.muak.extension.name="firecracker"
LABEL org.muak.extension.version="1.13.0"
