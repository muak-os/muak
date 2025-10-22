# Cloud Hypervisor MUAK Extension

FROM alpine:latest AS downloader
ARG CH_VERSION=v42.0
WORKDIR /download

RUN apk add --no-cache curl && \
  curl -fsSL "https://github.com/cloud-hypervisor/cloud-hypervisor/releases/download/${CH_VERSION}/cloud-hypervisor-static" -o cloud-hypervisor && \
  chmod +x cloud-hypervisor

# Final image - scratch base with only what we need
FROM scratch
COPY --from=downloader /download/cloud-hypervisor /usr/bin/cloud-hypervisor
COPY manifest.yaml /manifest.yaml

# OCI labels for metadata
LABEL org.opencontainers.image.title="cloud-hypervisor"
LABEL org.opencontainers.image.description="Cloud Hypervisor - Open source Virtual Machine Monitor (VMM)"
LABEL org.opencontainers.image.version="42.0"
LABEL org.muak.extension.name="cloud-hypervisor"
LABEL org.muak.extension.version="42.0"
