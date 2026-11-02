# Models

## `dpdfnet2_48khz_hr.onnx`

DPDFNet-2, high-resolution 48 kHz: causal speech enhancement (DeepFilterNet2 + dual-path RNN).
Embedded into the engine at build time (`crates/engine/src/dpdfnet.rs`).

| | |
|---|---|
| Source | https://huggingface.co/Ceva-IP/DPDFNet, `onnx/dpdfnet2_48khz_hr.onnx`, revision `main` |
| Fetched | 2026-11-02, via the `dpdfnet` 0.6.0 Python package |
| Size | 10,493,337 bytes |
| SHA-256 | `7F0575A5CEC0BA4FFD8F8BD657E06D007E4CCDD955D76FAAB922B9D3291DC14B` |
| License | Apache-2.0, © Ceva Inc. (code: https://github.com/ceva-ip/DPDFNet) |
| Paper | Rika et al., "DPDFNet: Boosting DeepFilterNet2 via Dual-Path RNN", arXiv:2512.16420 |

I/O: `spec` `[1,1,481,2]` + `state_in` `[56436]` → `spec_e` `[1,1,481,2]` + `state_out` `[56436]`.
The initial state comes from the model's metadata (`erb_norm_init`, `spec_norm_init`).
Frames are 960 samples with a Vorbis window and a 480 hop. Live delay is 2400 samples (50 ms).

The Rust port matches the Python `StreamEnhancer` to 1e-7 (max abs diff on real speech).
