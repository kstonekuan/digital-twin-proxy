# AI Proxy - Development Commands

# Run all quality checks (format, lint, build)
check: fmt clippy test build

# Format code with rustfmt
fmt:
    cargo fmt

# Run clippy linter
clippy:
    cargo clippy --all-targets --all-features

# Build the project
build:
    cargo build

# Run tests
test:
    cargo test

# Build release version
release:
    cargo build --release