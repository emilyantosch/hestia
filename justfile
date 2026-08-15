default: strict-lint test fmt

strict-lint:
    nix develop --command cargo clippy --workspace --all-targets --all-features -- -D warnings

test:
    nix develop --command cargo test --workspace

fmt:
    nix develop --command cargo fmt --all
