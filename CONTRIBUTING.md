# Contributing to socle

Thank you for your interest in contributing!

## Prerequisites

- Rust 1.85+ (`rustup update stable`)
- PostgreSQL (for integration tests that hit a real database)
- `cargo-llvm-cov` for coverage: `cargo install cargo-llvm-cov`
- `cargo-deny` for dependency audits: `cargo install cargo-deny`

## Development setup

```bash
git clone https://github.com/brefwiz/socle
cd socle
cp .env.example .env   # if present, otherwise set DATABASE_URL manually
cargo build
```

## Running tests

```bash
# Unit tests (no database required)
cargo test

# All tests including integration (requires DATABASE_URL)
DATABASE_URL=postgres://localhost/socle_test cargo test

# Coverage
cargo llvm-cov --all-features
```

## Code style

- `cargo fmt` — enforced by CI
- `cargo clippy --all-targets --all-features -- -D warnings` — enforced by CI
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`

## Pull requests

1. Fork the repo and create a branch from `main`.
2. Add or update tests for any behaviour you change.
3. Ensure `cargo fmt`, `cargo clippy`, and `cargo test` all pass locally.
4. Open a PR against `main` — the CI suite runs automatically.

There is no CLA. By submitting a pull request you agree to license your contribution under the [MIT License](LICENSE).
