# Agent instructions

## Examples

Every example in an `examples/` directory of the workspace must have a step
in `.github/workflows/ci-rawdb.yaml` that runs it. `cargo test` only compiles
examples; that job runs them with the rawdb sample cache and the API key.
Steps write the output of examples to `$RUNNER_TEMP`, not into the checkout.

