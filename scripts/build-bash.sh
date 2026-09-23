#!/bin/sh
# Builds a bash release into target/bash-<version> for the end-to-end tests:
#
#   scripts/build-bash.sh 5.0
#   INKLINE_TEST_BASH=target/bash-5.0/bin/bash cargo test
#
# Needs the ncurses development files (Debian: libncurses-dev, Fedora:
# ncurses-devel). Without them bash falls back to its bundled termcap, which
# cannot read terminfo, and readline then treats the terminal as dumb.
set -eu
version=${1:?usage: scripts/build-bash.sh VERSION}
root=$(cd "$(dirname "$0")/.." && pwd)
prefix="$root/target/bash-$version"
work="$root/target/bash-$version-build"
[ -x "$prefix/bin/bash" ] && exit 0
mkdir -p "$work"
cd "$work"
curl -sSfLO "https://ftp.gnu.org/gnu/bash/bash-$version.tar.gz"
tar xzf "bash-$version.tar.gz"
cd "bash-$version"
# Older releases predate GCC 14's stricter defaults.
CFLAGS="-O2 -std=gnu17 -Wno-implicit-function-declaration -Wno-implicit-int -Wno-incompatible-pointer-types -Wno-int-conversion" \
    ./configure --prefix="$prefix" --without-bash-malloc >configure.log
if grep -q 'using gnutermcap' configure.log; then
    echo "no terminfo library found; install the ncurses development files" >&2
    exit 1
fi
make -j"$(nproc)" >make.log
make install >install.log
