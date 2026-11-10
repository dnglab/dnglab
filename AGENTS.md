# Agent instructions

## Examples

Every example in an `examples/` directory of the workspace must have a step
in `.github/workflows/ci-rawdb.yaml` that runs it. `cargo test` only compiles
examples; that job runs them with the rawdb sample cache and the API key.
Steps write the output of examples to `$RUNNER_TEMP`, not into the checkout.

## Render pipeline documentation

`docs/render.md` describes the render pipeline from raw data to the output
files. Update it in the same change when you modify or extend the pipeline:
the plan and its operations in `rawler/src/imgop/render/`, the camera profile
render in `rawler/src/dng/profile_render/`, the processing steps and
`RenderOptions`, the color map, the highlight and HDR handling, the TIFF and
JPEG output in `rawler/src/imgop/develop.rs`, and the rendering for DNG
LinearRaw data and previews in `rawler/src/dng/convert.rs`. Keep the order of
the steps, the options table and the process-raw switches in it matching the
code.

