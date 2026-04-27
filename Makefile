# Makefile for groundwork
#
# Mirrors the CI suite so contributors can reproduce CI failures locally.

.PHONY: help fmt fmt-check clippy test build clean \
        ci ci-format ci-lint ci-test ci-audit ci-coverage ci-deny ci-package

help:
	@echo "Usage: make <target>"
	@echo ""
	@echo "Development:"
	@echo "  fmt            Format code"
	@echo "  fmt-check      Check code formatting"
	@echo "  clippy         Run clippy lints"
	@echo "  test           Run tests"
	@echo "  build          Build the crate"
	@echo "  clean          Clean build artifacts"
	@echo ""
	@echo "CI mirrors (reproduce CI failures locally):"
	@echo "  ci             Full CI suite"
	@echo "  ci-format      Check formatting"
	@echo "  ci-lint        Run clippy"
	@echo "  ci-test        Run tests via nextest"
	@echo "  ci-audit       Security audit"
	@echo "  ci-coverage    Coverage check"
	@echo "  ci-deny        Dependency license audit"
	@echo "  ci-package     Validate crate packaging"

# ============================================================================
# Development
# ============================================================================

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --release

clean:
	cargo clean

# ============================================================================
# CI suite targets (mirrors .gitea/workflows/ci.yml)
# ============================================================================

ci-format:
	cargo fmt --all -- --check

ci-lint:
	cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings

ci-test:
	cargo nextest run --all-features

ci-audit:
	cargo audit

ci-coverage:
	cargo llvm-cov nextest --all-features --lcov --output-path lcov.info \
		--fail-under-lines 80
	cargo llvm-cov report --summary-only

ci-deny:
	cargo deny check licenses

ci-package:
	cargo package --registry brefwiz --no-verify --allow-dirty

ci: ci-format ci-lint ci-test ci-audit ci-deny ci-package
	@echo "✅ All CI checks passed"

.PHONY: pre-commit
pre-commit: ci-format ci-lint ci-test ## Run all pre-commit checks (ADR-0021)
