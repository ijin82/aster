# Aster — сборка, тесты, установка и упаковка релиза.
APP     := aster
VERSION := $(shell xargs < VERSION 2>/dev/null)
CARGO   ?= cargo

.DEFAULT_GOAL := build

.PHONY: all build check test run debug install rebuild uninstall package release clean

all: build

build:
	$(CARGO) build --release --locked

check:
	$(CARGO) check --locked

test:
	$(CARGO) test --locked

run:
	$(CARGO) run --release --locked

# Запуск под gdb с фатальными GLib-критическими ошибками.
debug:
	$(CARGO) build --locked
	G_DEBUG=fatal-criticals gdb -q -ex run -ex bt --args \
		./target/debug/$(APP) $(ARGS)

install:
	./install.sh install

rebuild:
	./install.sh rebuild

uninstall:
	./install.sh uninstall

# Удобные алиасы для упаковки.
package:
	./package-release.sh $(VERSION)

release: package

clean:
	$(CARGO) clean
	rm -rf dist
