#!/bin/sh
# Builds bash 5.0, the oldest supported bash, into target/bash-5.0 for the
# end-to-end tests:
#
#   scripts/build-bash-5.0.sh
#   INKLINE_TEST_BASH=target/bash-5.0/bin/bash cargo test
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
prefix="$root/target/bash-5.0"
work="$root/target/bash-5.0-build"
[ -x "$prefix/bin/bash" ] && exit 0
mkdir -p "$work"
cd "$work"
curl -sSfLO https://ftp.gnu.org/gnu/bash/bash-5.0.tar.gz
tar xzf bash-5.0.tar.gz
cd bash-5.0
# bash 5.0 predates GCC 14's stricter defaults.
CFLAGS="-O2 -std=gnu17 -Wno-implicit-function-declaration -Wno-implicit-int -Wno-incompatible-pointer-types -Wno-int-conversion" \
    ./configure --prefix="$prefix" --without-bash-malloc >configure.log
make -j"$(nproc)" >make.log
make install >install.log
