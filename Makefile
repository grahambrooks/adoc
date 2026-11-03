.DEFAULT_GOAL := help

ADOC_BIN := target/debug/adoc
FIXTURE_DIR := tests/fixtures
EXAMPLES_DIR := docs/examples
FIXTURES := $(wildcard $(FIXTURE_DIR)/*.adoc)
EXAMPLES := $(patsubst $(FIXTURE_DIR)/%.adoc,$(EXAMPLES_DIR)/%.html,$(FIXTURES))

.PHONY: help build build-release release release-version release-dry release-fetch check test lint fmt fmt-check examples showcase clean ci

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
# CalVer is YYYY.M.MICRO (no leading zero on the month: Cargo's SemVer parser
# rejects `2026.04.1`). MICRO is one past the highest MICRO already tagged on
# origin this month, starting at 1 to match auto-managed's monthly tags.
#
# The tag is the version (release-kit v2). `make release` only tags origin/main
# and pushes the tag. The release workflow (`.github/workflows/release.yml`,
# configured by `.release.env`) stamps the version into Cargo.toml for the
# build, publishes the GitHub Release, and merges a PR that bumps Cargo.toml and
# `Formula/adoc.rb`. Nothing is bumped or committed here, so nothing races the
# bot, and local uncommitted work or the current branch don't matter.

YEAR  := $(shell date -u +%Y)
MONTH := $(shell date -u +%-m)
MICRO  = $(shell last=$$(git tag -l 'v$(YEAR).$(MONTH).*' \
           | sed -n 's/^v[0-9][0-9]*\.[0-9][0-9]*\.\([0-9][0-9]*\)$$/\1/p' \
           | sort -n | tail -1); echo $$(( $${last:-0} + 1 )))
VERSION = $(YEAR).$(MONTH).$(MICRO)
TAG     = v$(VERSION)

# Tags come from origin so MICRO reflects what is published, not this clone.
# A prerequisite rather than a recipe line, so the fetch lands before $(TAG)
# is expanded.
release-fetch:
	@git fetch --quiet origin main --tags

release-version: release-fetch ## Print the version `make release` would tag next
	@echo $(VERSION)

release-dry: release-fetch ## Show what `make release` would do, without doing it
	@echo 'would tag origin/main ($(shell git rev-parse --short origin/main)) as $(TAG) and push the tag'
	@echo 'the tag push runs the release workflow: it builds the binaries, publishes'
	@echo 'a GitHub Release, and merges a PR that bumps Cargo.toml and the formula'

release: release-fetch ## Tag origin/main with the next CalVer and push the tag (triggers the release workflow)
	@! git rev-parse -q --verify 'refs/tags/$(TAG)' >/dev/null \
		|| { echo 'release: tag $(TAG) already exists' >&2; exit 1; }
	@echo '--> tagging origin/main as $(TAG)'
	git tag -a '$(TAG)' -m 'adoc $(VERSION)' origin/main
	git push --quiet origin '$(TAG)'
	@echo 'pushed $(TAG); watch https://github.com/grahambrooks/adoc/actions'
