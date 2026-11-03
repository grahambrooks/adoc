.DEFAULT_GOAL := help

# release, release-dry, release-version (release-kit; tag-only, date-based CalVer)
include release.mk

ADOC_BIN := target/debug/adoc
FIXTURE_DIR := tests/fixtures
EXAMPLES_DIR := docs/examples
FIXTURES := $(wildcard $(FIXTURE_DIR)/*.adoc)
EXAMPLES := $(patsubst $(FIXTURE_DIR)/%.adoc,$(EXAMPLES_DIR)/%.html,$(FIXTURES))

.PHONY: help build build-release check test lint fmt fmt-check hooks examples showcase clean ci

help: ## Show this help
	@awk 'BEGIN {FS = ":.*?## "; printf "Usage: make <target>\n\nTargets:\n"} \
		/^[a-zA-Z_-]+:.*?## / {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

build: ## Build the workspace (debug)
	cargo build

build-release: ## Build the workspace (release, optimised)
	cargo build --release

check: fmt-check lint test ## The pre-push gate: the same commands as CI's check job

test: ## Run the test suite (as CI)
	cargo test --workspace --all-features --locked

lint: ## Run clippy, warnings denied (as CI)
	cargo clippy --workspace --all-targets --all-features --locked -- -D warnings

fmt: ## Format all Rust sources in-place
	cargo fmt --all

fmt-check: ## Verify formatting without rewriting files
	cargo fmt --all --check

hooks: ## Install the git hooks (pre-commit and pre-push) from prek.toml
	prek install

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

ci: check ## Alias for check
