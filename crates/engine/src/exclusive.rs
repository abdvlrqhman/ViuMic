//! Exclusive-mode WASAPI output into ViuMic's virtual cable.
//!
//! Screen-share audio (Discord's "Entire screen", OBS desktop audio) records every app's playback
//! through Windows loopback, which only sees streams mixed by the shared audio engine. ViuMic's
//! clean voice goes *into* the cable as playback, so in shared mode it leaked into screen shares
//! even with Discord muted. Exclusive streams bypass the shared engine. Measured on the user's
//! PC: a test tone played into "ViuMic Feed" reached "ViuMic Microphone" at full level both ways,
//! but a Discord-style process-loopback capture got it at -13 dBFS in shared mode and nothing
//! (-180 dBFS) in exclusive mode.

use crate::{Player, SAMPLE_RATE, Shared};
use anyhow::{Result, anyhow, bail};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, S_OK, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_DEVICE_IN_USE,
    AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED, AUDCLNT_SHAREMODE_EXCLUSIVE,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK, DEVICE_STATE_ACTIVE, IAudioClient, IAudioRenderClient,
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX, WAVEFORMATEXTENSIBLE,
    WAVEFORMATEXTENSIBLE_0, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, STGM_READ,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::GUID;

const FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
const PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
/// How long the render loop waits for the device before re-checking whether to stop.
const WAIT_MS: u32 = 200;

#[derive(Clone, Copy)]
enum Sample {
    F32,
    I16,
    I32,
}

/// The running output. Dropping it stops and joins the render thread.
pub(crate) struct Renderer {
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl Renderer {
    /// Opens `device` (exact friendly name) in exclusive mode and starts rendering from `player`.
    /// On failure the player comes back (when possible) so the caller can fall back to a shared
    /// stream.
    pub(crate) fn start(
        device: &str,
        channels: usize,
        player: Player,
        sh: Arc<Shared>,
    ) -> Result<Self, (anyhow::Error, Option<Player>)> {
        let (ready_tx, ready_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (name, stop_flag) = (device.to_string(), stop.clone());
        let spawned = thread::Builder::new()
            .name("viumic-out".into())
            .spawn(move || {
                let mut player = player;
                // SAFETY: COM calls on this thread only; every interface stays on it.
                match unsafe { Output::open(&name, channels) } {
                    Ok(out) => {
                        let _ = ready_tx.send(Ok(()));
                        out.run(&mut player, &sh, &stop_flag);
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err((e, player)));
                    }
                }
            });
        let thread = spawned.map_err(|e| (anyhow!("starting the output thread: {e}"), None))?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                thread: Some(thread),
                stop,
            }),
            Ok(Err((e, player))) => {
                let _ = thread.join();
                Err((e, Some(player)))
            }
            Err(_) => Err((anyhow!("the output thread died"), None)),
        }
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Output {
    client: IAudioClient,
    render: IAudioRenderClient,
    event: HANDLE,
    frames: u32,
    channels: usize,
    sample: Sample,
}

impl Output {
    unsafe fn open(name: &str, channels: usize) -> Result<Self> {
        unsafe {
            // S_FALSE (already initialised) is fine; this is a fresh thread anyway.
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let device = find(name)?;
            let mut client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
            let (format, sample) = negotiate(&client, channels)?;

            let mut period = 0i64;
            client.GetDevicePeriod(Some(&mut period), None)?;
            let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
            if let Err(e) = client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                flags,
                period,
                period,
                &format.Format,
                None,
            ) {
                if e.code() != AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED {
                    return Err(explain(e));
                }
                // Routine for exclusive mode: the period must match the device's frame size.
                // Recompute it from the size the device offered and retry on a fresh client.
                let frames = client.GetBufferSize()?;
                let aligned = (10_000_000.0 * frames as f64 / SAMPLE_RATE as f64).round() as i64;
                client = device.Activate(CLSCTX_ALL, None)?;
                client
                    .Initialize(
                        AUDCLNT_SHAREMODE_EXCLUSIVE,
                        flags,
                        aligned,
                        aligned,
                        &format.Format,
                        None,
                    )
                    .map_err(explain)?;
            }
            let event = CreateEventW(None, false, false, None)?;
            client.SetEventHandle(event)?;
            Ok(Self {
                frames: client.GetBufferSize()?,
                render: client.GetService()?,
                client,
                event,
                channels,
                sample,
            })
        }
    }

    fn run(self, player: &mut Player, sh: &Shared, stop: &AtomicBool) {
        crate::boost_thread_priority();
        // Two device buffers are in flight in event-driven exclusive mode.
        let buffer_us = 2 * self.frames as u64 * 1_000_000 / SAMPLE_RATE as u64;
        sh.playback_us.store(buffer_us as u32, Relaxed);
        // SAFETY: interfaces were created on this thread and the buffer pointer is only used
        // between GetBuffer and ReleaseBuffer, for exactly `frames` frames.
        unsafe {
            if let Err(e) = self.client.Start() {
                sh.fail(format!("audio device: {e}"));
                return;
            }
            while sh.running.load(Relaxed) && !stop.load(Relaxed) {
                if WaitForSingleObject(self.event, WAIT_MS) != WAIT_OBJECT_0 {
                    continue;
                }
                let data = match self.render.GetBuffer(self.frames) {
                    Ok(p) => p,
                    Err(e) => {
                        sh.fail(format!("audio device: {e}"));
                        break;
                    }
                };
                let (ch, sample) = (self.channels, self.sample);
                player.fill(sh, self.frames as usize, |frame, s| {
                    for c in 0..ch {
                        let k = frame * ch + c;
                        match sample {
                            Sample::F32 => *data.cast::<f32>().add(k) = s,
                            Sample::I16 => *data.cast::<i16>().add(k) = (s * 32767.0) as i16,
                            Sample::I32 => {
                                *data.cast::<i32>().add(k) = (s as f64 * 2_147_483_647.0) as i32
                            }
                        }
                    }
                });
                if let Err(e) = self.render.ReleaseBuffer(self.frames, 0) {
                    sh.fail(format!("audio device: {e}"));
                    break;
                }
            }
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

/// The render endpoint whose friendly name is exactly `name` (as cpal reports it).
unsafe fn find(name: &str) -> Result<IMMDevice> {
    unsafe {
        let all: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let endpoints = all.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        for i in 0..endpoints.GetCount()? {
            let device = endpoints.Item(i)?;
            let props = device.OpenPropertyStore(STGM_READ)?;
            if props.GetValue(&PKEY_Device_FriendlyName)?.to_string() == name {
                return Ok(device);
            }
        }
        bail!("“{name}” not found")
    }
}

/// First stereo-or-wider 48 kHz format the device takes natively in exclusive mode.
unsafe fn negotiate(
    client: &IAudioClient,
    channels: usize,
) -> Result<(WAVEFORMATEXTENSIBLE, Sample)> {
    let candidates = [
        (32, 32, FLOAT, Sample::F32),
        (16, 16, PCM, Sample::I16),
        (32, 24, PCM, Sample::I32),
        (32, 32, PCM, Sample::I32),
    ];
    for (bits, valid, sub, sample) in candidates {
        let format = extensible(channels as u16, bits, valid, sub);
        // SAFETY: `format` lives for the call.
        let hr =
            unsafe { client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, &format.Format, None) };
        if hr == S_OK {
            return Ok((format, sample));
        }
        if hr == AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED || hr == AUDCLNT_E_DEVICE_IN_USE {
            return Err(explain(hr.into()));
        }
    }
    bail!("the device accepts none of ViuMic's 48 kHz formats in exclusive mode")
}

fn extensible(channels: u16, bits: u16, valid: u16, sub: GUID) -> WAVEFORMATEXTENSIBLE {
    let block = channels * bits / 8;
    WAVEFORMATEXTENSIBLE {
        Format: WAVEFORMATEX {
            wFormatTag: 0xFFFE, // WAVE_FORMAT_EXTENSIBLE
            nChannels: channels,
            nSamplesPerSec: SAMPLE_RATE,
            nAvgBytesPerSec: SAMPLE_RATE * block as u32,
            nBlockAlign: block,
            wBitsPerSample: bits,
            cbSize: 22,
        },
        Samples: WAVEFORMATEXTENSIBLE_0 {
            wValidBitsPerSample: valid,
        },
        dwChannelMask: if channels == 2 { 0x3 } else { 0 },
        SubFormat: sub,
    }
}

/// Turns the two errors users can fix into plain words.
fn explain(e: windows::core::Error) -> anyhow::Error {
    match e.code() {
        AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED => anyhow!(
            "Windows doesn't allow exclusive control of ViuMic Feed (Sound settings → ViuMic Feed → Advanced)"
        ),
        AUDCLNT_E_DEVICE_IN_USE => anyhow!("another app is holding ViuMic Feed exclusively"),
        _ => anyhow::Error::new(e).context("opening ViuMic Feed in exclusive mode"),
    }
}
