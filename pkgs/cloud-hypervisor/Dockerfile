# Cloud Hypervisor MUAK Extension

FROM alpine:latest AS downloader
ARG CH_VERSION=v42.0
ARG EDK2_VERSION=ch-a54f262b09
WORKDIR /download

RUN apk add --no-cache curl && \
  curl -fsSL "https://github.com/cloud-hypervisor/cloud-hypervisor/releases/download/${CH_VERSION}/cloud-hypervisor-static" -o cloud-hypervisor && \
  chmod +x cloud-hypervisor && \
  curl -fsSL "https://github.com/cloud-hypervisor/edk2/releases/download/${EDK2_VERSION}/CLOUDHV.fd" -o CLOUDHV.fd

# Final image - scratch base with only what we need
FROM scratch
COPY --from=downloader /download/cloud-hypervisor /usr/bin/cloud-hypervisor
COPY --from=downloader /download/CLOUDHV.fd /usr/share/muak/CLOUDHV.fd

# OCI labels for metadata
LABEL org.opencontainers.image.title="cloud-hypervisor"
LABEL org.opencontainers.image.description="Cloud Hypervisor - Open source Virtual Machine Monitor (VMM)"
LABEL org.opencontainers.image.version="42.0"
LABEL org.muak.extension.name="cloud-hypervisor"
LABEL org.muak.extension.version="42.0"
