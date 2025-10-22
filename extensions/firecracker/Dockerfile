# Firecracker MUAK Extension
# Multi-stage build: download real Firecracker binary, package for MUAK

FROM alpine:latest AS downloader
ARG FIRECRACKER_VERSION=v1.7.0
WORKDIR /download

RUN apk add --no-cache curl tar && \
    curl -fsSL "https://github.com/firecracker-microvm/firecracker/releases/download/${FIRECRACKER_VERSION}/firecracker-${FIRECRACKER_VERSION}-x86_64.tgz" -o firecracker.tgz && \
    tar -xzf firecracker.tgz && \
    mv release-${FIRECRACKER_VERSION}-x86_64/firecracker-${FIRECRACKER_VERSION}-x86_64 /download/firecracker && \
    chmod +x /download/firecracker

# Verify it's a static binary
RUN file /download/firecracker && \
    ldd /download/firecracker 2>&1 | grep -q "not a dynamic executable" || (echo "ERROR: Firecracker is not static!" && exit 1)

# Final image - scratch base with only what we need
FROM scratch
COPY --from=downloader /download/firecracker /usr/bin/firecracker
COPY manifest.yaml /manifest.yaml

# OCI labels for metadata
LABEL org.opencontainers.image.title="firecracker"
LABEL org.opencontainers.image.description="AWS Firecracker microVM hypervisor"
LABEL org.opencontainers.image.version="1.7.0"
LABEL org.muak.extension.name="firecracker"
LABEL org.muak.extension.version="1.7.0"
