<p align="center"><img src="assets/logo.svg" width="128" alt="ViuMic logo"></p>

# ViuMic

Free, open-source AI noise removal for your microphone. It runs on the CPU (no RTX needed), using
[DPDFNet](https://github.com/ceva-ip/DPDFNet), a 2026 48 kHz speech-enhancement model, and gives
your apps a clean **ViuMic Microphone**.

Status: early. The desktop app works; the first release is next. See [PLAN.md](PLAN.md).

## Use it

1. Run the ViuMic installer. It also installs the virtual microphone (VB-CABLE by VB-Audio,
   donationware: [vb-cable.com](https://vb-audio.com/Cable/)); Windows asks once to trust its driver.
2. Open ViuMic and pick your real microphone. ViuMic remembers it.
3. In Discord, Zoom, OBS or any app, choose **ViuMic Microphone** and turn off the app's own noise
   suppression.
4. Windows Settings → System → Sound → your mic → **Audio enhancements: Off**. Driver effects
   (common on Realtek laptop mics) pump the background noise up and down with your voice, and
   ViuMic can't undo that. Keep the input volume low enough that loud speech doesn't clip.

Closing the window keeps ViuMic running in the tray; Quit is in the tray menu.

## CLI

```
viumic devices                          list mics and outputs
viumic run --output "Headphones"        hear yourself instead of sending to the ViuMic microphone
viumic run --model rnnoise              much lighter model, for weak CPUs
viumic run --hum 60                     mains hum is 60 Hz where you live (the Americas)
viumic denoise in.wav out.wav           offline, 48 kHz WAV
viumic record raw.wav --seconds 20      raw mic sample, for bug reports
```

## Build

Requires Rust 1.95+, MSVC (Visual Studio Build Tools), Node 22+ with pnpm, and network access on
the first build (the `ort` crate downloads a prebuilt ONNX Runtime and links it statically, so
there's no DLL to ship):

```
pnpm install
pnpm dev                  # desktop app with hot reload
pnpm build                # installer: target/release/bundle/nsis/
cargo build --release -p viumic-cli   # just the CLI
```

Layout: `crates/engine` (audio + models), `crates/cli`, `src-tauri` (desktop app backend),
`ui` (Svelte 5 + shadcn-svelte frontend), `models` (embedded DPDFNet), `vendor/vbcable`
(VB-CABLE driver pack, shipped unmodified).

## License

MIT OR Apache-2.0. Third-party parts keep their own licenses:

- DPDFNet model (Apache-2.0, © Ceva Inc.), embedded in the binary; see [models/README.md](models/README.md)
- VB-CABLE (© Vincent Burel / VB-Audio, donationware), bundled unmodified; see `vendor/vbcable/readme.txt`
- ONNX Runtime (MIT), nnnoiseless/RNNoise (BSD-3), shadcn-svelte (MIT)
