# ┌──────────────────────────────────────────────────────────────┐
# │  Trail — Local-first prolly-tree operation database         │
# │  Makefile: build · test · install · release · dev · ci       │
# └──────────────────────────────────────────────────────────────┘

# ── Configuration ──────────────────────────────────────────────────

PREFIX       ?= $(HOME)/.cargo
BINDIR       ?= $(PREFIX)/bin
DATADIR      ?= $(PREFIX)/share/trail
MANDIR       ?= $(PREFIX)/share/man/man1
CARGO        ?= cargo
RUSTC        ?= rustc

# Binary / package names
BIN_NAME     := trail
WORKSPACE_MEMBERS := trail prolly

# Feature flags for the main binary
FEATURES     ?= sqlite

# Build profile: debug | release
PROFILE      ?= release

# Extra flags passed through to cargo
BUILD_FLAGS  ?=
TEST_FLAGS   ?=

# Cross-compilation target (empty = host)
TARGET       ?=

# Version from Cargo.toml
VERSION      := $(shell grep '^version' Cargo.toml | head -1 | sed 's/.*"\(.*\)".*/\1/')

# OS / arch detection
UNAME_S      := $(shell uname -s)
UNAME_M      := $(shell uname -m)

# Colors
BOLD         := \033[1m
GREEN        := \033[32m
CYAN         := \033[36m
RESET        := \033[0m
CHECK        := $(GREEN)✓$(RESET)

# ── Help ───────────────────────────────────────────────────────────

.PHONY: help
help: ## Show this help
	@printf "$(BOLD)Trail $(VERSION)$(RESET) — Makefile targets\n\n"
	@grep -E '^[a-zA-Z_-]+.*:.*?## .*$$' $(MAKEFILE_LIST) \
	  | sort \
	  | awk 'BEGIN {FS = ":.*?## "}; {printf "  $(CYAN)%-24s$(RESET) %s\n", $$1, $$2}'

# ── Build ──────────────────────────────────────────────────────────

.PHONY: build
build: ## Build debug binary
	@printf "$(BOLD)Building $(BIN_NAME) (debug)...$(RESET)\n"
	$(CARGO) build -p $(BIN_NAME) $(BUILD_FLAGS)

.PHONY: release
release: ## Build optimized release binary
	@printf "$(BOLD)Building $(BIN_NAME) (release)...$(RESET)\n"
	$(CARGO) build -p $(BIN_NAME) --release $(BUILD_FLAGS)

.PHONY: all
all: build test ## Build + test (debug)

.PHONY: check
check: ## Fast compile check (no codegen)
	$(CARGO) check --workspace $(BUILD_FLAGS)

.PHONY: check-release
check-release: ## Fast compile check in release mode
	$(CARGO) check --workspace --release $(BUILD_FLAGS)

# ── Test ───────────────────────────────────────────────────────────

.PHONY: test
test: ## Run all tests
	@printf "$(BOLD)Running all tests...$(RESET)\n"
	$(CARGO) test --workspace $(TEST_FLAGS) -- --nocapture

.PHONY: test-release
test-release: ## Run tests in release mode
	$(CARGO) test --workspace --release $(TEST_FLAGS) -- --nocapture

.PHONY: test-trail
test-trail: ## Run trail crate tests
	$(CARGO) test -p trail $(TEST_FLAGS) -- --nocapture

.PHONY: test-prolly
test-prolly: ## Run prolly crate tests
	$(CARGO) test -p prolly-map $(TEST_FLAGS) -- --nocapture

.PHONY: test-e2e
test-e2e: ## Run only end-to-end tests
	$(CARGO) test -p trail e2e -- --nocapture

# ── Benchmarks ─────────────────────────────────────────────────────

.PHONY: bench
bench: ## Run all benchmarks
	$(CARGO) bench --workspace

.PHONY: bench-prolly
bench-prolly: ## Run prolly tree benchmarks
	$(CARGO) bench -p prolly

.PHONY: bench-cli-scale-smoke
bench-cli-scale-smoke: ## Run small CLI scale benchmark suitable for CI
	TRAIL_SCALE_FILES=$${TRAIL_SCALE_FILES:-1000} \
	TRAIL_SCALE_BASE=$${TRAIL_SCALE_BASE:-/tmp} \
	TRAIL_SCALE_LABEL=$${TRAIL_SCALE_LABEL:-ci-smoke} \
	TRAIL_SCALE_MATERIALIZED=$${TRAIL_SCALE_MATERIALIZED:-1} \
	TRAIL_SCALE_BACKUP=$${TRAIL_SCALE_BACKUP:-1} \
	scripts/cli-scale-bench.sh

.PHONY: bench-cli-scale
bench-cli-scale: ## Run local 10k CLI scale benchmark
	TRAIL_SCALE_FILES=$${TRAIL_SCALE_FILES:-10000} \
	TRAIL_SCALE_BASE=$${TRAIL_SCALE_BASE:-/Volumes/Workspace} \
	TRAIL_SCALE_LABEL=$${TRAIL_SCALE_LABEL:-local-10k} \
	scripts/cli-scale-bench.sh

.PHONY: bench-cli-scale-large
bench-cli-scale-large: ## Run large manual CLI scale benchmark (override TRAIL_SCALE_FILES for 1M)
	TRAIL_SCALE_FILES=$${TRAIL_SCALE_FILES:-100000} \
	TRAIL_SCALE_BASE=$${TRAIL_SCALE_BASE:-/Volumes/Workspace} \
	TRAIL_SCALE_LABEL=$${TRAIL_SCALE_LABEL:-manual-large} \
	scripts/cli-scale-bench.sh

.PHONY: bench-cli-scale-nightly
bench-cli-scale-nightly: ## Run manual/nightly 10k, 100k, and 1M CLI scale benchmark
	TRAIL_SCALE_FILES=$${TRAIL_SCALE_FILES:-10000,100000,1000000} \
	TRAIL_SCALE_BASE=$${TRAIL_SCALE_BASE:-/Volumes/Workspace} \
	TRAIL_SCALE_LABEL=$${TRAIL_SCALE_LABEL:-nightly-scale} \
	scripts/cli-scale-bench.sh

.PHONY: bench-cli-scale-1m-headless
bench-cli-scale-1m-headless: ## Run 1M no-materialize scale benchmark without backup/materialized workdirs
	TRAIL_SCALE_FILES=$${TRAIL_SCALE_FILES:-1000000} \
	TRAIL_SCALE_BASE=$${TRAIL_SCALE_BASE:-/Volumes/Workspace} \
	TRAIL_SCALE_LABEL=$${TRAIL_SCALE_LABEL:-manual-1m-headless} \
	TRAIL_SCALE_MATERIALIZED=$${TRAIL_SCALE_MATERIALIZED:-0} \
	TRAIL_SCALE_BACKUP=$${TRAIL_SCALE_BACKUP:-0} \
	scripts/cli-scale-bench.sh

# ── Lint & Format ──────────────────────────────────────────────────

.PHONY: fmt
fmt: ## Format all source code
	$(CARGO) fmt --all

.PHONY: fmt-check
fmt-check: ## Check formatting (CI)
	$(CARGO) fmt --all -- --check

.PHONY: clippy
clippy: ## Run clippy lints
	@printf "$(BOLD)Running clippy...$(RESET)\n"
	$(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

.PHONY: clippy-fix
clippy-fix: ## Auto-fix clippy suggestions
	$(CARGO) clippy --workspace --all-targets --all-features --fix --allow-dirty --allow-staged

.PHONY: lint
lint: fmt-check clippy ## Full lint pass (fmt-check + clippy)

# ── Docs ───────────────────────────────────────────────────────────

.PHONY: docs
docs: ## Generate rustdocs
	@printf "$(BOLD)Generating documentation...$(RESET)\n"
	$(CARGO) doc --workspace --no-deps --document-private-items

.PHONY: docs-open
docs-open: docs ## Open rustdocs in browser
	open target/doc/trail/index.html 2>/dev/null \
	  || xdg-open target/doc/trail/index.html 2>/dev/null \
	  || @printf "Open target/doc/trail/index.html manually\n"

# ── Install / Uninstall ────────────────────────────────────────────

.PHONY: install
install: release ## Install trail to $(BINDIR)
	@printf "$(BOLD)Installing $(BIN_NAME) $(VERSION) to $(BINDIR)...$(RESET)\n"
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 755 "target/release/$(BIN_NAME)" "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	@printf "  $(CHECK) $(DESTDIR)$(BINDIR)/$(BIN_NAME)\n"

.PHONY: install-debug
install-debug: build ## Install debug binary to $(BINDIR)
	@printf "$(BOLD)Installing $(BIN_NAME) (debug) to $(BINDIR)...$(RESET)\n"
	install -d "$(DESTDIR)$(BINDIR)"
	install -m 755 "target/debug/$(BIN_NAME)" "$(DESTDIR)$(BINDIR)/$(BIN_NAME)-debug"

.PHONY: uninstall
uninstall: ## Uninstall trail
	@printf "$(BOLD)Uninstalling $(BIN_NAME)...$(RESET)\n"
	rm -f "$(DESTDIR)$(BINDIR)/$(BIN_NAME)"
	@printf "  $(CHECK) removed $(DESTDIR)$(BINDIR)/$(BIN_NAME)\n"

# ── Release packaging ──────────────────────────────────────────────

DIST_DIR     := dist
TARBALL      := $(BIN_NAME)-$(VERSION)-$(shell uname -s | tr '[:upper:]' '[:lower:]')-$(UNAME_M).tar.gz
CHECKSUM     := $(TARBALL).sha256

.PHONY: dist
dist: release ## Create release tarball
	@printf "$(BOLD)Packaging $(TARBALL)...$(RESET)\n"
	@mkdir -p "$(DIST_DIR)"
	tar -czf "$(DIST_DIR)/$(TARBALL)" \
	  -C target/release "$(BIN_NAME)" \
	  -C ../../ README.md LICENSE 2>/dev/null || true
	cd "$(DIST_DIR)" && shasum -a 256 "$(TARBALL)" > "$(CHECKSUM)"
	@printf "  $(CHECK) $(DIST_DIR)/$(TARBALL)\n"
	@printf "  $(CHECK) $(DIST_DIR)/$(CHECKSUM)\n"

.PHONY: dist-clean
dist-clean: ## Remove dist artifacts
	rm -rf "$(DIST_DIR)"

.PHONY: release-check
release-check: ## Verify release readiness
	@printf "$(BOLD)Checking release readiness for v$(VERSION)...$(RESET)\n"
	@$(CARGO) fmt --all -- --check \
	  && printf "  $(CHECK) formatting ok\n" \
	  || { printf "  ✗ formatting issues\n"; exit 1; }
	@$(CARGO) clippy --workspace --all-features -- -D warnings \
	  && printf "  $(CHECK) clippy ok\n" \
	  || { printf "  ✗ clippy errors\n"; exit 1; }
	@$(CARGO) test --workspace \
	  && printf "  $(CHECK) tests pass\n" \
	  || { printf "  ✗ test failures\n"; exit 1; }
	@$(CARGO) build --release -p $(BIN_NAME) \
	  && printf "  $(CHECK) release build ok\n" \
	  || { printf "  ✗ build failure\n"; exit 1; }
	@printf "\n$(BOLD)$(GREEN)Release v$(VERSION) is ready.$(RESET)\n"

# ── Cross-compilation ──────────────────────────────────────────────

.PHONY: build-linux
build-linux: ## Cross-compile for x86_64-unknown-linux-gnu
	$(CARGO) build -p $(BIN_NAME) --release --target x86_64-unknown-linux-gnu $(BUILD_FLAGS)

.PHONY: build-macos-arm
build-macos-arm: ## Cross-compile for aarch64-apple-darwin
	$(CARGO) build -p $(BIN_NAME) --release --target aarch64-apple-darwin $(BUILD_FLAGS)

.PHONY: build-macos-x86
build-macos-x86: ## Cross-compile for x86_64-apple-darwin
	$(CARGO) build -p $(BIN_NAME) --release --target x86_64-apple-darwin $(BUILD_FLAGS)

# ── Dev workflow ───────────────────────────────────────────────────

.PHONY: watch
watch: ## Auto-rebuild on changes
	$(CARGO) watch -x "check --workspace" -x "test --workspace"

.PHONY: watch-run
watch-run: ## Auto-rebuild and run on changes
	$(CARGO) watch -x "run -p $(BIN_NAME) -- status"

.PHONY: run
run: build ## Build and run (pass ARGS="..." to override)
	$(CARGO) run -p $(BIN_NAME) -- $(or $(ARGS),--help)

.PHONY: run-release
run-release: release ## Build release and run (pass ARGS="..." to override)
	./target/release/$(BIN_NAME) $(or $(ARGS),--help)

# ── Dependencies ───────────────────────────────────────────────────

.PHONY: deps-update
deps-update: ## Update dependencies
	$(CARGO) update

.PHONY: deps-outdated
deps-outdated: ## Check for outdated dependencies
	$(CARGO) outdated 2>/dev/null || \
	  { printf "Install cargo-outdated: $(CARGO) install cargo-outdated\n"; exit 1; }

.PHONY: deps-audit
deps-audit: ## Audit dependencies for vulnerabilities
	$(CARGO) audit 2>/dev/null || \
	  { printf "Install cargo-audit: $(CARGO) install cargo-audit\n"; exit 1; }

.PHONY: deps-tree
deps-tree: ## Print dependency tree
	$(CARGO) tree

# ── Clean ──────────────────────────────────────────────────────────

.PHONY: clean
clean: ## Remove build artifacts
	$(CARGO) clean

.PHONY: clean-all
clean-all: clean dist-clean ## Wipe everything: target + dist
	@printf "  $(CHECK) all artifacts removed\n"

# ── Info ───────────────────────────────────────────────────────────

.PHONY: version
version: ## Print version
	@printf "$(BIN_NAME) $(VERSION)\n"

.PHONY: info
info: ## Show build environment info
	@printf "$(BOLD)Trail $(VERSION)$(RESET)\n\n"
	@printf "  Rust toolchain:  $(shell $(RUSTC) --version 2>/dev/null || echo 'not found')\n"
	@printf "  Cargo:           $(shell $(CARGO) --version 2>/dev/null || echo 'not found')\n"
	@printf "  Target:          $(or $(TARGET),host)\n"
	@printf "  Profile:         $(PROFILE)\n"
	@printf "  PREFIX:          $(PREFIX)\n"
	@printf "  OS:              $(UNAME_S) ($(UNAME_M))\n"

.PHONY: toolchain
toolchain: ## Install/verify the correct Rust toolchain
	@rustup show active-toolchain 2>/dev/null || rustup toolchain install stable
	@rustup component add clippy rustfmt 2>/dev/null || true
	@printf "  $(CHECK) toolchain ready\n"

# ── CI ─────────────────────────────────────────────────────────────

.PHONY: ci
ci: fmt-check clippy test release-check ## Full CI pipeline

.PHONY: ci-fast
ci-fast: fmt-check check test ## Fast CI (check instead of full clippy+release)

# ── Helpers ────────────────────────────────────────────────────────

.DEFAULT_GOAL := help
