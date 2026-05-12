fix:
    cargo clippy --fix --allow-staged --allow-dirty --workspace
    cargo fmt --all

fmt:
    cargo fmt --all

check:
    cargo clippy --workspace -- -D warnings
    cargo fmt --all --check

# Fuzzing
# Setup: cargo install cargo-fuzz && rustup toolchain install nightly
# Usage: just fuzz fuzz_bmff_parse 3600 24 /path/to/corpus
fuzz target="fuzz_decode_full" duration="60" jobs="1" corpus="" max_len="65536" rss_limit="20000":
    cd rawler/fuzz && ASAN_OPTIONS=allocator_may_return_null=1 cargo +nightly fuzz run {{target}} --release -j{{jobs}} -- -max_total_time={{duration}} -max_len={{max_len}} -rss_limit_mb={{rss_limit}} {{corpus}}

fuzz-all duration="60" jobs="1" max_len="65536" rss_limit="20000":
    #!/usr/bin/env bash
    for t in $(cd rawler/fuzz && cargo +nightly fuzz list 2>/dev/null); do
        echo "=== $t {{duration}}s j{{jobs}} ===" ; (cd rawler/fuzz && ASAN_OPTIONS=allocator_may_return_null=1 cargo +nightly fuzz run "$t" --release -j{{jobs}} -- -max_total_time={{duration}} -max_len={{max_len}} -rss_limit_mb={{rss_limit}}) || true
    done

fuzz-list:
    @cd rawler/fuzz && cargo +nightly fuzz list
