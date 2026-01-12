SHELL := /bin/bash

.PHONY: help format lint test build imsg clean emacs-test remote-test

help:
	@printf "%s\n" \
		"make format  - swift format in-place" \
		"make lint    - swift format lint + swiftlint" \
		"make test    - sync version, patch deps, run swift test" \
		"make build   - universal release build into bin/" \
		"make rpc-app - update /Applications/IMsgRPC.app wrapper" \
		"make imsg    - clean rebuild + run debug binary (ARGS=...)" \
		"make emacs-test - run emacs ERT tests" \
		"make remote-test - run remote RPC smoke tests over SSH" \
		"make clean   - swift package clean"

format:
	swift format --in-place --recursive Sources Tests

lint:
	swift format lint --recursive Sources Tests
	swiftlint

test:
	scripts/generate-version.sh
	swift package resolve
	scripts/patch-deps.sh
	swift test
	scripts/run-emacs-tests.sh

build:
	scripts/generate-version.sh
	swift package resolve
	scripts/patch-deps.sh
	scripts/build-universal.sh

rpc-app:
	scripts/update-rpc-app.sh

imsg:
	scripts/generate-version.sh
	swift package resolve
	scripts/patch-deps.sh
	swift package clean
	swift build -c debug --product imsg
	./.build/debug/imsg $(ARGS)

clean:
	swift package clean

emacs-test:
	scripts/run-emacs-tests.sh

remote-test:
	scripts/remote-e2e.sh
