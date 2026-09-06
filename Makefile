# Native Ubuntu builds. Run `make help` for the supported workflows.
.DEFAULT_GOAL := help
SHELL := /bin/bash
.SHELLFLAGS := -eu -o pipefail -c
.DELETE_ON_ERROR:

PREFIX ?= $(HOME)/.local
DESTDIR ?=
CARGO ?= cargo
PYTHON ?= python3
CARGO_TARGET_DIR ?= target
DIST_DIR ?= dist
LOCAL_DEPS ?= 0
export CARGO_TARGET_DIR CARGO PYTHON PREFIX DESTDIR DIST_DIR

# Keep the optional extracted-library environment identical for checks and builds.
ifeq ($(LOCAL_DEPS),1)
RUN := ./scripts/cargo-local --exec
else
RUN :=
endif

.PHONY: help setup deps toolchain local-deps doctor build install uninstall run \
        update upgrade update-source package install-deb fmt lint test test-core \
        test-install check clean

help: ## Show commands and configuration (the default; makes no changes)
	@printf 'Readero local installation\n\n'
	@awk 'BEGIN {FS = ":.*## "} /^[a-z-]+:.*## / {printf "  %-16s %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@printf '\nOptions: PREFIX=%s DESTDIR=<staging-root> LOCAL_DEPS=1\n' "$$HOME/.local"
	@printf '         CARGO=cargo PYTHON=python3 CARGO_TARGET_DIR=target DIST_DIR=dist\n'
	@printf '\nFirst install: make setup && make install\nLatest upstream: make update (requires a clean checkout and tracked branch)\n'

setup: ## Install Ubuntu build dependencies and stable Rust, then check readiness
	@$(MAKE) deps
	@$(MAKE) toolchain
	@$(MAKE) doctor

deps: ## Refresh apt metadata and install build/packaging dependencies (uses sudo)
	@"$$PYTHON" scripts/local-install.py deps

toolchain: ## Install/update stable Rust with rustfmt and Clippy (requires rustup)
	@command -v rustup >/dev/null || { echo 'Install rustup first, then rerun make toolchain.' >&2; exit 1; }
	rustup toolchain install stable --profile minimal --component rustfmt --component clippy

local-deps: ## Prepare the optional machine-specific /tmp development prefix
	"$$PYTHON" scripts/prepare-local-build.py

doctor: ## Check Rust and native library requirements without changing the system
	@$(RUN) "$$PYTHON" scripts/local-install.py doctor --cargo "$$CARGO"

build: doctor ## Build the current source as an ordinary optimized desktop executable
	$(RUN) "$$CARGO" build --locked --release --package readero --bin readero --no-default-features --features desktop

install: build ## Build and install executable, launcher, icon, and notices for this user
	"$$PYTHON" scripts/local-install.py install --binary "$${CARGO_TARGET_DIR}/release/readero" --prefix "$$PREFIX" --destdir "$$DESTDIR"

uninstall: ## Remove installed app files; preserve reading history and bookmarks
	"$$PYTHON" scripts/local-install.py uninstall --prefix "$$PREFIX" --destdir "$$DESTDIR"

run: build ## Build and launch the app (open documents from its file picker)
	"$${CARGO_TARGET_DIR}/release/readero"

update-source: ## Fast-forward the current branch from its upstream; refuse local changes
	@"$$PYTHON" scripts/local-install.py update-source

update: ## Fetch the latest upstream source, rebuild, and install locally
	@$(MAKE) update-source
	@$(MAKE) install

upgrade: update ## Alias for update

package: build ## Build a .deb with the manifest version and detected native dependencies
	./scripts/package-deb "$${CARGO_TARGET_DIR}/release/readero" "$$DIST_DIR"

install-deb: package ## Build and install the exact new .deb through apt (uses sudo)
	"$$PYTHON" scripts/local-install.py install-deb --dist-dir "$$DIST_DIR"

fmt: ## Check formatting of authored Rust code
	$(RUN) "$$CARGO" fmt --package readero --check

lint: doctor ## Run strict Clippy checks, including native smoke-test code
	$(RUN) "$$CARGO" clippy --package readero --no-deps --all-targets --features smoke --locked -- -D warnings

test: doctor ## Run the app's Rust tests with native dependencies
	$(RUN) "$$CARGO" test --package readero --locked

test-core: ## Run core Rust tests without GTK/Papers/WebKit development packages
	$(RUN) "$$CARGO" test --package readero --locked --no-default-features

test-install: ## Test staging, desktop integration, uninstall, and source-update safeguards
	"$$PYTHON" -m unittest discover -s tests -p 'test_local_install.py' -v

check: fmt lint test test-install ## Run formatting, lint, Rust, and installer checks

clean: ## Remove Cargo build products (preserves installed app, dist, and user data)
	$(RUN) "$$CARGO" clean
