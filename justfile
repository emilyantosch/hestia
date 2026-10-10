default: strict-lint test fmt

run:
    cargo run -p hestia

check:
    cargo check --workspace --all-targets

strict-lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    cargo test --workspace

fmt:
    cargo fmt --all

# Pass through Cargo options for focused checks, without a second task runner.
cargo *args:
    cargo {{args}}
