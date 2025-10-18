.PHONY: x86_64 arm64 all clean

x86_64:
	ARCH=x86_64 bash scripts/build.sh

arm64:
	ARCH=arm64 bash scripts/build.sh

all: x86_64 arm64

clean:
	rm -rf build output
