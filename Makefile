.DEFAULT_GOAL := help

ADOC_BIN := target/debug/adoc
FIXTURE_DIR := tests/fixtures
EXAMPLES_DIR := docs/examples
FIXTURES := $(wildcard $(FIXTURE_DIR)/*.adoc)
EXAMPLES := $(patsubst $(FIXTURE_DIR)/%.adoc,$(EXAMPLES_DIR)/%.html,$(FIXTURES))

.PHONY: help build build-release release release-version check test lint fmt fmt-check examples showcase clean ci

help: ## Show this help
	@awk 'BEGIN {FS = ":.*?## "; printf "Usage: make <target>\n\nTargets:\n"} \
		/^[a-zA-Z_-]+:.*?## / {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build the workspace (debug)
	cargo build

build-release: ## Build the workspace (release, optimised)
	cargo build --release

check: ## Type-check without building artefacts
	cargo check --all-targets

test: ## Run the test suite
	cargo test

lint: ## Run clippy across all crates and targets
	cargo clippy --all-targets -- -D warnings

fmt: ## Format all Rust sources in-place
	cargo fmt --all

fmt-check: ## Verify formatting without rewriting files
	cargo fmt --all -- --check

examples: $(EXAMPLES) ## Render tests/fixtures/*.adoc into docs/examples/*.html

showcase: docs/showcase.html ## Render the full-feature showcase to docs/showcase.html

docs/showcase.html: docs/showcase.adoc docs/showcase-snippet.adoc | $(ADOC_BIN)
	$(ADOC_BIN) $< -o $@

$(EXAMPLES_DIR)/%.html: $(FIXTURE_DIR)/%.adoc | $(ADOC_BIN) $(EXAMPLES_DIR)
	$(ADOC_BIN) $< > $@

$(ADOC_BIN): build

$(EXAMPLES_DIR):
	mkdir -p $@

clean: ## Remove build artefacts and generated examples
	cargo clean
	rm -rf $(EXAMPLES_DIR)
	rm -f docs/showcase.html

ci: fmt-check lint test ## Run the checks expected to pass in CI

# --- release cycle ---------------------------------------------------------
#
# `make release` bumps `Cargo.toml` to today's UTC calver version
# (YYYY.M.D, with no leading zeros — Cargo's SemVer parser rejects
# `2026.04.26` but accepts `2026.4.26`), commits the bump, tags it,
# and pushes both the branch and the tag. The tag push triggers the
# `Release` workflow under `.github/workflows/release.yml` which
# builds binaries, publishes a GitHub Release, and refreshes
# `Formula/adoc.rb`.
#
# Guards:
#   - working tree must be clean
#   - current branch must be `main`
#   - `make ci` (fmt-check + lint + test) must pass with the new
#     version applied — catches anything clippy notices once the
#     version string changes, and the test suite re-runs against
#     the bumped Cargo.toml.

CALVER := $(shell date -u +'%Y.%-m.%-d')

release-version: ## Print the calver string `make release` would use today
	@echo $(CALVER)

release: ## Cut a calver release: bump Cargo.toml, commit, tag, and push (triggers CI)
	@set -e; \
	v='$(CALVER)'; \
	echo "==> Cutting release $$v"; \
	if ! git diff --quiet --ignore-submodules HEAD; then \
		echo "error: working tree has uncommitted changes; commit or stash first." >&2; \
		exit 1; \
	fi; \
	branch="$$(git symbolic-ref --short HEAD)"; \
	if [ "$$branch" != "main" ]; then \
		echo "error: not on main (current branch: $$branch); releases are cut from main." >&2; \
		exit 1; \
	fi; \
	if git rev-parse "refs/tags/$$v" >/dev/null 2>&1; then \
		echo "error: tag $$v already exists. Delete it first or wait until tomorrow." >&2; \
		exit 1; \
	fi; \
	echo "==> Bumping Cargo.toml version to $$v"; \
	if [ "$$(uname)" = "Darwin" ]; then \
		sed -i '' "s/^version = .*/version = \"$$v\"/" Cargo.toml; \
	else \
		sed -i "s/^version = .*/version = \"$$v\"/" Cargo.toml; \
	fi; \
	echo "==> Refreshing Cargo.lock and validating"; \
	cargo check --quiet; \
	echo "==> Running CI checks against the bumped version"; \
	$(MAKE) ci; \
	echo "==> Committing and tagging"; \
	git add Cargo.toml Cargo.lock; \
	git commit -m "release: $$v"; \
	git tag "$$v"; \
	echo "==> Pushing main and tag $$v (this triggers the Release workflow)"; \
	git push origin main; \
	git push origin "$$v"; \
	echo "==> Release $$v triggered."; \
	echo "    Watch: https://github.com/grahambrooks/adoc/actions"
