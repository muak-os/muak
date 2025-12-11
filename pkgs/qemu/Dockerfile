# QEMU MUAK Extension
# Bundle dynamic QEMU with all dependencies for bare-metal deployment

FROM alpine:latest AS builder
WORKDIR /build

# Install QEMU and ALL runtime dependencies
RUN apk add --no-cache \
  qemu-system-x86_64 \
  qemu-system-aarch64 \
  qemu-img \
  qemu-ui-gtk \
  qemu-ui-sdl \
  qemu-ui-spice-core \
  # Core libraries
  glib \
  pixman \
  gnutls \
  zlib \
  libpng \
  libjpeg-turbo \
  libsasl \
  snappy \
  lzo \
  libseccomp \
  libfdt \
  numactl \
  libcap-ng \
  # Graphics and display support
  mesa-dri-gallium \
  mesa-gl \
  libdrm \
  libx11 \
  libxext \
  libxfixes \
  libxrandr \
  libxrender \
  libxdamage \
  libxcomposite \
  libxcursor \
  libxi \
  libxinerama \
  libxkbcommon \
  libxshmfence \
  gtk+3.0 \
  sdl2 \
  spice \
  virglrenderer \
  # Audio support
  alsa-lib \
  pulseaudio \
  # USB support
  libusb \
  usbredir \
  # Network support
  libslirp \
  # Additional utilities
  coreutils \
  bash

# Create directory structure for organized bundling
RUN mkdir -p /output/usr/bin \
  /output/usr/lib \
  /output/usr/share/qemu \
  /output/lib

# Copy QEMU binaries
RUN cp /usr/bin/qemu-system-x86_64 /output/usr/bin/ && \
  cp /usr/bin/qemu-system-aarch64 /output/usr/bin/ && \
  cp /usr/bin/qemu-img /output/usr/bin/

# Copy ALL shared libraries (comprehensive approach)
# This ensures we don't miss any dependencies
RUN cp -a /usr/lib/* /output/usr/lib/ || true

# Copy musl dynamic linker (critical for Alpine-based binaries)
RUN cp /lib/ld-musl-x86_64.so.1 /output/lib/

# Copy QEMU firmware and BIOS files (CRITICAL - QEMU won't work without these!)
RUN cp -r /usr/share/qemu/* /output/usr/share/qemu/

# Copy other potentially needed shared data
RUN mkdir -p /output/usr/share/X11 && \
  cp -r /usr/share/X11/* /output/usr/share/X11/ 2>/dev/null || true

# Copy GTK/GDK data for GUI support
RUN mkdir -p /output/usr/share/glib-2.0 && \
  cp -r /usr/share/glib-2.0/* /output/usr/share/glib-2.0/ 2>/dev/null || true

RUN mkdir -p /output/usr/share/gtk-3.0 && \
  cp -r /usr/share/gtk-3.0/* /output/usr/share/gtk-3.0/ 2>/dev/null || true

# Final image - scratch base with bundled runtime
FROM scratch

# Copy all binaries
COPY --from=builder /output/usr/bin /usr/bin/

# Copy all libraries
COPY --from=builder /output/usr/lib /usr/lib/
COPY --from=builder /output/lib /lib/

# Copy QEMU firmware/BIOS files (essential for booting VMs)
COPY --from=builder /output/usr/share/qemu /usr/share/qemu/

# Copy display/GUI support files
COPY --from=builder /output/usr/share/X11 /usr/share/X11/
COPY --from=builder /output/usr/share/glib-2.0 /usr/share/glib-2.0/
COPY --from=builder /output/usr/share/gtk-3.0 /usr/share/gtk-3.0/

# OCI labels for metadata
LABEL org.opencontainers.image.title="qemu"
LABEL org.opencontainers.image.description="QEMU - Open source machine emulator and virtualizer with full graphics support"
LABEL org.opencontainers.image.version="9.2.0"
LABEL org.muak.extension.name="qemu"
LABEL org.muak.extension.version="9.2.0"
LABEL org.muak.extension.binaries="/usr/bin/qemu-system-x86_64,/usr/bin/qemu-system-aarch64,/usr/bin/qemu-img"
LABEL org.muak.extension.firmware="/usr/share/qemu"
