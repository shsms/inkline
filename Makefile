# make            build the release library
# make install    copy it to $(LIBDIR)
# make test       run the tests with the system bash
# make test-all   run them with the system bash and each of $(BASHES)
# make check      fmt, clippy and the unit tests
# make clean      remove target/, including the bashes built for testing

LIBDIR ?= $(HOME)/.local/lib
BASHES ?= 5.0 5.3

.PHONY: all build install test test-all check clean

all: build

build:
	cargo build --release

install: build
	mkdir -p $(LIBDIR)
	cp target/release/libinkline.so $(LIBDIR)/

test:
	cargo test

test-all: test $(BASHES:%=target/bash-%/bin/bash)
	for v in $(BASHES); do \
	    INKLINE_TEST_BASH=target/bash-$$v/bin/bash cargo test || exit 1; \
	done

target/bash-%/bin/bash:
	scripts/build-bash.sh $*

check:
	cargo fmt --check
	cargo clippy --all-targets --all-features -- -D warnings
	cargo test --lib

clean:
	cargo clean
