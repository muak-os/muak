# Cloud Hypervisor MUAK Extension

FROM alpine:3.23 AS downloader

ARG TARGETARCH
ARG CH_VERSION=v42.0
ARG EDK2_VERSION=ch-a54f262b09

ARG TARGET=${TARGETARCH/amd64/x86_64}
ARG TARGET=${TARGET/arm64/aarch64}

WORKDIR /download

# Download Cloud Hypervisor binary (architecture-aware)
# x86_64: cloud-hypervisor-static, aarch64: cloud-hypervisor-static-aarch64
RUN apk add --no-cache curl && \
  if [ "${TARGET}" = "aarch64" ]; then \
  curl -fsSL "https://github.com/cloud-hypervisor/cloud-hypervisor/releases/download/${CH_VERSION}/cloud-hypervisor-static-aarch64" -o cloud-hypervisor; \
  else \
  curl -fsSL "https://github.com/cloud-hypervisor/cloud-hypervisor/releases/download/${CH_VERSION}/cloud-hypervisor-static" -o cloud-hypervisor; \
  fi && \
  chmod +x cloud-hypervisor

# Download EDK2 firmware (architecture-aware)
# x86_64: CLOUDHV.fd, aarch64: CLOUDHV_EFI.fd
RUN if [ "${TARGET}" = "aarch64" ]; then \
  curl -fsSL "https://github.com/cloud-hypervisor/edk2/releases/download/${EDK2_VERSION}/CLOUDHV_EFI.fd" -o CLOUDHV.fd; \
  else \
  curl -fsSL "https://github.com/cloud-hypervisor/edk2/releases/download/${EDK2_VERSION}/CLOUDHV.fd" -o CLOUDHV.fd; \
  fi

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
