# prism-ml

Machine-learning features for Prism (layer L4).

## Super Zoom (Real-ESRGAN)

`upscale(&Runtime, Model, &Rgba8, &dyn Control) -> Result<Rgba8>` enlarges an 8-bit RGBA image
4× with Real-ESRGAN. It runs the official
[`realesrgan-ncnn-vulkan`](https://github.com/xinntao/Real-ESRGAN-ncnn-vulkan) executable (MIT)
as a child process on the GPU (Vulkan; MoltenVK on macOS), so no native inference code is linked
and the crate has no `unsafe`.

- `Model::X4Plus` (`realesrgan-x4plus`, photographs) and `Model::X4PlusAnime`
  (`realesrgan-x4plus-anime`, illustrations); both BSD-3-Clause, from
  [Real-ESRGAN](https://github.com/xinntao/Real-ESRGAN). Both are 4× only: other factors resample
  the 4× result (the executable's `-s 2/3` scrambles these models' tiles).
- `Runtime::find(model)` looks in `PRISM_REALESRGAN_DIR`, then a copy bundled beside the app
  (`<exe dir>/realesrgan`, `Prism.app/Contents/Resources/realesrgan`), then
  `<user dir>/realesrgan` once the app called `runtime::configure(Some(user_dir))`.
- `install::install(&ctl)` downloads the pinned upstream package for macOS or Windows
  (Real-ESRGAN v0.2.5.0 release, size and SHA-256 checked), unpacks only the executable and the
  two models, and replaces `<user dir>/realesrgan` in one rename. Without `configure` it refuses,
  so tests and headless engines never download.

The executable exits with status 0 even when it fails, so the result PNG is always checked
(present, decodable, exactly 4× the input), and its stderr is quoted in the error.
