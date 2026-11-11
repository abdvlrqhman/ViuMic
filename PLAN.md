# ViuMic plan

Open-source AI noise cancellation for your microphone, built with Rust and Tauri.

**Goal:** the noise-removal part of NVIDIA Broadcast. ViuMic takes your real mic, removes background noise with a neural model, and exposes the clean audio as a microphone that Discord, Zoom, Teams and OBS can pick. It runs on any CPU. No RTX needed.

**Constraint:** completely free. No paid services, no code or driver signing.

**Scope v1:** Windows 11, mic-side noise removal.

---

## 1. Decisions

| Area | Choice | Why |
|---|---|---|
| Denoise (default) | **DPDFNet-2 48 kHz** (Ceva, 2026, Apache-2.0), then an 80 Hz high-pass and hum notches | The user rated it "perfect" on their noisy laptop mic, removing the electrical noise under the voice that RNNoise and DFN3 left. Rust port of the upstream `StreamEnhancer`, matching it to 1e-7. Adds 50 ms latency; live load ~25–30% of one core per 10 ms hop (debug build). Model embedded (10.5 MB; see models/README.md). |
| Denoise (heavier option, not wired) | DPDFNet-8 48 kHz (15 MB, ~2.5× the CPU) | The user rated -2 and -8 both "perfect"; add -8 only if -2 ever falls short. |
| Denoise (light) | **RNNoise** (`nnnoiseless`) + the same filters + a downward expander | For weak CPUs. Leaves electrical noise under quiet speech and passes knocks. Adds 10 ms. |
| Removed | DeepFilterNet3 (+ the RNNoise-with-DFN-judge hybrid, the gates) | Robotic and ~8 dB quieter on the user's mic, now superseded by DPDFNet, which builds on DFN2. That also dropped the stale `deep_filter`/tract dependency. |
| De-hum | Notches at 50/100/150/200 Hz (4 Hz wide), `--hum 50\|60\|off`, default 50 | The user's laptop mic has a steady 49.99 Hz mains hum (+16 dB line) that RNNoise only halves. The notch removes it (−5 dB, i.e. below its neighbours); voice at 125/300/1000 Hz is untouched (test). |
| Deeper voice | **Removed** (user request, 2026-11-02) | That also dropped the C++ pitch shifter and the LLVM build requirement. |
| Inference runtime | **ONNX Runtime** via `ort` =2.0.0-rc.15 (pinned RC), CPU, 1 thread; FFT via `realfft` | Same runtime the upstream package uses. Statically linked: no DLL to ship. It downloads prebuilt ONNX Runtime at build time, so building needs network. No GPU needed. |
| Audio I/O | `cpal` 0.18 (WASAPI) | Standard. Since 0.18 it reroutes default streams, reports xruns and doesn't panic when a device is lost. |
| Ring buffers | `rtrb` 0.4 (lock-free single-producer/single-consumer) | Audio callbacks must never lock or allocate. |
| Resampling | `rubato`, only when a device isn't 48 kHz | Mics can show up at 16, 44.1 or 48 kHz. The engine runs at 48 kHz mono. |
| Virtual mic | **VB-CABLE bundled in the ViuMic installer**, renamed **"ViuMic Microphone"** / "ViuMic Feed" | The user wanted no separate download and an NVIDIA-Broadcast-style name. Windows only creates mics through Microsoft-signed kernel drivers (paid EV cert), so we ship VB-CABLE unmodified and run its silent setup (`-i -h`) from an NSIS post-install hook. Windows still asks once to trust the driver. ViuMic renames the endpoints at runtime through `IPolicyConfig` (what Sound settings uses; no admin needed; redone on every start). **Licensing:** the package's own readme says integrating it into another installer needs the author's agreement, while VB-Audio's licensing page allows it for free apps under the donationware model with visible credit. The app and installer show the credit; **email VB-Audio for written confirmation before a public release.** |
| UI shell | **Tauri 2.12.x** (3.0 is alpha) | Small binary, Rust backend in the same process. |
| Frontend | **Svelte 5 + TypeScript + Vite** (no SvelteKit) | About six controls. Svelte compiles down to almost nothing and contributors can read it. |
| License | **MIT OR Apache-2.0** | Rust convention. Compatible with DPDFNet (Apache-2.0; releases must ship its license text) and nnnoiseless (BSD-3). VB-CABLE stays under VB-Audio's donationware license (see Virtual mic). |

### Rejected

- **Our own virtual audio driver** (HushCore, SYSVAD, VirtualDrivers forks). Kernel drivers need paid signing or test mode.
- **Capture APO on the physical mic.** Windows 11 expects APOs to ship in the device's driver package and pass HLK validation. Registry injection breaks on driver updates.
- **NVIDIA Maxine / CUDA.** NVIDIA-only and proprietary. It stays the fallback if an open model ever stops being good enough; OBS shows the way (it loads NVIDIA's SDK if the user installed it).
- **GTCRN.** Tiny, but its README doesn't state the sample rate (it may be 16 kHz), which would lower quality.

---

## 2. Architecture

```
 physical mic                                                          VB-CABLE
 ──WASAPI──▶ [capture cb] ──rtrb──▶ [DSP thread] ──rtrb──▶ [output cb] ──WASAPI──▶ "CABLE Input"
                                     │ downmix → mono                               │
                                     │ resample → 48 kHz (if needed)                ▼
                                     │ denoise: DPDFNet | RNNoise | bypass    "CABLE Output" = the mic
                                     │ deeper voice (optional pitch shift)    Discord / Zoom / Teams / OBS
                                     │ peak meters → atomics
                                     └ resample → device rate (if needed)
```

- **One process.** Tauri hosts the engine. Closing the window hides it to the tray and audio keeps running.
- **Live controls** (on/off, strength, voice depth) are atomics the DSP thread reads. **Structural changes** (device, model) stop the engine and start it again.
- **Strength** = DFN attenuation limit in dB. For RNNoise it is a wet/dry mix.

### Repo layout

```
Cargo.toml            workspace
crates/engine/        lib: devices, pipeline, denoisers. No Tauri dependency.
crates/cli/           bin: headless run, wav→wav denoise, latency measurement
src-tauri/            Tauri app: commands, tray, settings, meters channel   (Phase 2)
ui/                   Svelte frontend                                       (Phase 2)
```

### Real-time rules

1. cpal callbacks only move samples into or out of `rtrb`. No `Mutex`, no allocation, no logging.
2. Inference runs on its own thread, in fixed 10 ms (480-sample) hops.
3. **Two clocks drift** (the mic and the cable are separate devices). The output callback waits until the buffer holds about 20 ms before it plays, and waits again after an underrun. The DSP thread drops a hop when the buffer grows past the high watermark.
4. When a device is lost, the engine reports an error. The app shows it and restarts the engine.

### Latency (measured: Realtek mic → VB-CABLE)

| Stage | Measured |
|---|---|
| Windows/device buffers (outside ViuMic's control) | 40–51 ms |
| ViuMic ring buffers | ~10 ms |
| DPDFNet (1 hop of framing + 4 of model lookahead) | 50 ms |
| RNNoise (light model) | 10 ms |
| **Total, DPDFNet** | **90–101 ms** |

The original target was ≤ 80 ms total. DPDFNet overshoots it by ~15 ms, a trade the user accepted for "perfect" quality. Calls feel natural up to ~150 ms one-way.

**How it's measured:** cpal 0.18 timestamps report the device latencies, the DSP thread reports ring fill, and the models report their exact delay. `viumic run` prints the live total.

---

## 3. Phases

### Phase 0: spike + gate ✅ done (2026-11-02)

- `deep_filter` is a git dependency pinned to `v0.5.6` (crates.io 0.2.5 predates DFN3). Builds on Rust 1.95.
- `viumic devices | run | denoise`, both models, deep voice, live strength/bypass, peak meters, latency stats.
- Test: low-passed noise comes out > 15 dB quieter at the same length, and a 6 dB strength limit holds.
- Noisy TTS speech: noise in pauses drops from −27 to −57 dBFS (DFN3) / −65 dBFS (RNNoise). DFN3 runs at ~0.14× real time on one core.
- Deep voice: `--deep 3` measured −2.9 semitones.
- **Gate: go.** DFN3 adds 30 ms (≤ 40). Still open: a 10-min glitch soak into VB-CABLE (needs VB-CABLE installed).

### Phase 1: engine hardening (1–2 days)

1. ✅ 10-min soak into VB-CABLE (debug build): 0 dropped, 2 underruns, 9 device xruns, latency 69–104 ms (VB-CABLE's device share is 51–62 ms). Re-run with a release build. If underruns persist, raise `PRIME` from 20 to 30 ms.
   - ✅ Live test 2 (user): robotic sometimes, random cut-offs, breath audible in pauses. Fix: replaced DFN's hard gate with our own smooth gate (open at local SNR −8 dB in 1 ms, hold 150 ms, release −6 dB/10 ms; active only at Max strength). Added `viumic record` for raw-mic bug reports.
   - Raw recording of the user's Realtek mic: **clips** (peaks −0.1 dBFS at 13/24/27/28 s); noise floor −40 dBFS with broadband hiss up to 24 kHz (mic boost too high); channels 99.9% identical, so downmix is fine. DFN removes loud breaths (RNNoise passes them) but lowers the voice ~8 dB. Both models remove a −32 dBFS "voiced" stretch at 16–18 s (quiet speech or hum?).
   - User ear test on their own recording: DFN (gated or not) = robotic, cut, quiet. **RNNoise = "completely perfect"**, except knocks in pauses and a "low digital wave". Measured: RNNoise lets 40–80 Hz rumble through while the user talks → added the 80 Hz high-pass. RNNoise is now the default.
   - Gate experiments on RNNoise: its own VAD removed 16 pitched hops that the approved version kept, so it was rejected. Gating with DFN's speech estimate never closed on this mic, so it was removed. Lesson: compare candidates against the user-approved output, not against "digital silence" (−77 dBFS and −inf both sound silent).
   - Combined version: user says knocks (11–14 s) are still audible and the "wave" under quiet speech is only slightly better. Knocks trade off against voice: DFN-guide slope 1 / 2 / 3 → knocks −40 / −47 / −52 dBFS, voice (quietest 5%) −4.5 / −7.6 / −10.6 dB. The user picks by ear. The wave survived a 20 dB rumble cut, so it's probably not rumble. Open question: is it already in the raw mic, or is it RNNoise's low-SNR artifact? If it's the artifact, a limited strength (some original sound mixed back) should mask it.
   - The user says the wave is in the raw mic too (audible all through, masked when talking loud). At 0.18 Hz resolution: one real hum line at 49.99 Hz; nothing at exact harmonics. Added de-hum. If the wave remains, it's the mic's broadband hiss (floor −40 dBFS, gain high enough to clip): a setup problem (input level, Mic Boost, the "Microphone Array" input), not a model one.
   - De-hum didn't remove the wave. The user says it rides on the voice (worse when quiet) and isn't hiss. So it's already in the raw driver signal. Main suspect: the Realtek/Windows mic effects (AGC or noise suppression). Test: turn "Audio enhancements" off and re-record raw. If confirmed, the real fix is capturing in WASAPI raw mode (`AUDCLNT_STREAMOPTIONS_RAW`, which bypasses the driver's effects for our stream only). cpal 0.18 doesn't expose it, so it needs our own WASAPI capture path or a cpal patch.
   - ✅ Enhancements off (`raw2.wav`) makes the noise constant instead of riding on the voice, which confirms the driver effects caused the "wave with voice". Noise-only recording (`noise.wav`): 72% of the energy is 20–80 Hz rumble swelling by ±7 dB, plus 50/100 Hz hum (+21/+19 dB lines, swelling by up to 21 dB); hiss above 2 kHz is perfectly steady. ViuMic's chain removes 63 dB of it (−36 → −100 dBFS), even without the dynamics. README step 4 now says to turn enhancements off. Phase 2: the app should detect and warn about this; later, WASAPI raw-mode capture.
   - ✅ **Screen-share leak (2026-11-07).** The user's mic was audible in Discord screen shares even
     while muted in Discord or with the ASUS key. Cause: Discord's whole-screen share records
     every process except Discord (process loopback, not tied to one output device), and
     ViuMic's playback into the cable was a shared-mode stream. Reproduced with a scratch
     tool doing Discord's capture: tone into ViuMic Feed in shared mode came through at
     −13 dBFS; in **exclusive mode**, nothing (−180 dBFS), while "ViuMic Microphone" still
     got it at full level. The engine now renders into the cable in exclusive mode
     (`crates/engine/src/exclusive.rs`, 48 kHz, format negotiated, 16-bit on VB-CABLE, default
     10 ms period). It falls back to shared mode with a visible reason. Side effect: Windows'
     share of the delay drops from 37–47 ms to ~30 ms. A 10-min exclusive run: 0 dropped,
     2 underruns.
     Mute: the ASUS key mutes the Windows default mic, which was NVIDIA Broadcast's (unused).
     ViuMic now mirrors Windows mute of the chosen mic and of ViuMic Microphone (polled every
     200 ms, sends silence, shows Muted). It offers one click to make ViuMic Microphone the
     default mic for all roles. VB-CABLE's install had also made ViuMic Feed the
     *communications* speaker; there's a one-click fix for that too. Both use
     IPolicyConfig::SetDefaultEndpoint and only run when the user clicks.
     CLI binary renamed `viumic-cli`: `viumic.exe` and `ViuMic.exe` are the same file on
     Windows, and the app build overwrote the CLI.
   - Measuring lessons: coarse spectra near a voice's pitch lie; `cargo test`/`clippy` don't rebuild `viumic.exe`, so run `cargo build` before using the CLI for measurements.
   - ✅ Earlier: user heard words cut off. Cause: DFN zeroes every 10 ms hop whose local SNR is under −10 dB (a hard gate). Fix: gate at −12 dB plus a 200 ms hangover after speech. On test speech, chopped voiced hops went 6 → 0 and noise in pauses went −57 → −53 dBFS.
2. Resample mics that aren't 48 kHz with `rubato` (the engine errors on them today). WASAPI already converts the output side.
3. Device-loss recovery: restart the engine when a device disappears or comes back.
4. **DPDFNet gates:** a release-build 10-min soak into VB-CABLE (watch `load`, `dropped`, `underruns`), and an ear test on desk knocks (DPDFNet hasn't been judged on knocks yet).
5. Output limiter: the user's mic clips at the input and output peaks reach 0 dBFS. Add a simple peak limiter before the cable.

### Phase 2: Tauri app ✅ built 2026-11-02 (user testing now)

**Round 2 (user feedback):** shadcn-svelte (vega, zinc, dark only, Geist) with a custom title bar (frameless window; minimize/maximize/close-to-tray) and responsive layout (1 column, 2 from 640 px). The mic choice is saved on first run and never silently switched (it waits for that mic if unplugged). Delay/CPU show the median of the last 3 s, eased. CPU is now ViuMic's share of the whole processor (Task Manager style), 1.8% on the test laptop. The exe is `ViuMic.exe` with version info "ViuMic", so Task Manager groups the model and WebView2 under ViuMic. Deeper voice removed.

Root pnpm workspace (`pnpm dev` / `pnpm build`). `src-tauri` is a workspace crate; `ui` is Svelte 5 + Vite + TS (no SvelteKit), with the Saira font bundled locally.

1. ✅ Commands: `get_state`, `set_settings` (save and apply live; device/model/hum restart the engine), `watch_meters` (`ipc::Channel`, 30 Hz).
2. ✅ Settings: JSON in `%APPDATA%\net.spacie.viumic\settings.json` (confirmed written).
3. ✅ Tray: "Noise removal" check item, Open, Quit; left click opens; closing the window hides it.
4. ✅ Single instance; Start with Windows (autostart plugin, launches with `--hidden` into the tray).
5. ✅ Notes when VB-CABLE is missing, and when it's the Windows default **output** (the user hears nothing). A cable *default input* is fine: the app's "Automatic" mic skips it and picks the first real mic.
6. ✅ Background supervisor: starts the engine and restarts it every 2 s after an "audio device" error (unplugged mic).
7. ✅ Engine: output peak limiter (−1 dBFS ceiling; the user's output had hit 0 dBFS). Adaptive jitter buffer (20 ms, growing 5 ms per underrun up to 50 ms) after the release soak showed 4 underruns in 10 min with VB-CABLE. Re-soak to confirm.
8. Design: logo palette with meaning (lavender = raw mic, mint = clean voice). The one bold element is the live strip (raw level behind clean level; the gap is the removed noise). Everything else is quiet controls, with rare settings under "More settings".

```
┌ ViuMic ─────────────────────────────── ● ON ┐
│ Microphone   [ Realtek Mic (default)     ▾ ] │
│ Input   ▮▮▮▮▮▮▯▯▯▯▯▯                          │
│ Output  ▮▮▮▯▯▯▯▯▯▯▯▯   → "CABLE Output"       │
│ Model   (•) Best — DeepFilterNet  ( ) Light   │
│ Strength ──────────────●  Max                 │
│ Deep voice ●──────────── off  (+40 ms)        │
│ [x] Start with Windows                        │
│ In Discord/Zoom pick "CABLE Output" as mic.   │
└───────────────────────────────────────────────┘
```

### Phase 3: release v0.1.0 (2026-11-11: built, see docs/RELEASING.md)

1. ✅ GitHub Actions on `windows-latest` (`.github/workflows/ci.yml`): UI check, `cargo fmt --check`, `clippy -D warnings`, `cargo test`. Actions pinned by SHA, Dependabot weekly.
2. ✅ Release on tag push (`release.yml`, tauri-action v1): draft release with the NSIS installer, signed `latest.json` + `.sig`, `SHA256SUMS.txt`. Fails if the tag and versions differ.
3. ✅ The installer is unsigned. The README explains the SmartScreen "More info → Run anyway" step and why: signing costs money.
4. ✅ LICENSE-MIT, LICENSE-APACHE, THIRD-PARTY-NOTICES.md, all three shipped inside the installer too.
5. ✅ Auto-updates (`tauri-plugin-updater`, release builds only): checks on start and every 6 h, shows "Update ready", installs only on click (passive NSIS, one UAC prompt since the install is per-machine). Public repo abdvlrqhman/ViuMic, identifier `net.spacie.viumic`, publisher Spacie.
6. **Before publishing:** written OK from VB-Audio to bundle VB-CABLE in the installer (see Decisions → Virtual mic).
7. Test the installer on a clean Windows VM: VB-CABLE silent install, the driver-trust prompt, the rename on first start, autostart, uninstall.
8. Prove an update end to end: install vN from GitHub, publish vN+1, click Install.

---

## 4. Later (not v1)

- **Echo cancellation** (laptop speakers): `sonora-aec3` (pure-Rust WebRTC AEC3, BSD-3), using a WASAPI loopback recording of the speakers as the reference.
- **Linux:** a PipeWire virtual source needs no driver, so it's the easiest port. **macOS** needs an AudioServerPlugIn plus notarization (paid), so it's out under the free rule.
- **Speaker-side denoise** needs a second virtual cable. On Windows only paid cables exist (VB-CABLE A+B), so it's Linux-only for now.

## 5. Risks

1. **DFN upstream is stale.** Last release v0.5.6 on 2023-08-31, last commit 2024-10-17. Mitigation: git pin, then vendor or fork, then the RNNoise fallback.
2. **Cable UX.** Users must install VB-CABLE and select "CABLE Output" in each app. Mitigation: in-app guidance.
3. **SmartScreen warning** on the unsigned installer. Mitigation: README steps; reputation builds over downloads.
4. **Clock drift clicks.** Mitigation: rule 3, then adaptive resampling.

## Sources

- DeepFilterNet: https://github.com/Rikorose/DeepFilterNet
- nnnoiseless: https://docs.rs/nnnoiseless
- cpal: https://github.com/RustAudio/cpal/releases
- VB-CABLE licensing: https://vb-audio.com/Services/licensing.htm
- Windows 11 APO rules: https://learn.microsoft.com/windows-hardware/drivers/audio/windows-11-apis-for-audio-processing-objects
- sonora-aec3: https://docs.rs/crate/sonora-aec3/latest
- HushCore (driver reference, rejected for signing): https://github.com/IACBI/hushcore
