fix:
    cargo clippy --fix --allow-staged --allow-dirty --workspace
    cargo fmt --all

fmt:
    cargo fmt --all

check:
    cargo clippy --workspace -- -D warnings
    cargo fmt --all --check
