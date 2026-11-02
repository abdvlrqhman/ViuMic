//! ViuMic command line: run the engine headless, or process WAV files offline.

use anyhow::{Context, Result, bail};
use std::io::Write;
use std::{env, thread, time::Duration};
use viumic_engine::{
    self as engine, Config, Deepen, Denoiser, Engine, Model, SAMPLE_RATE, UNLIMITED_DB,
};

const USAGE: &str = "\
ViuMic: AI noise removal for your microphone

usage:
  viumic devices
  viumic run     [--input NAME] [--output NAME] [OPTIONS]
  viumic denoise IN.wav OUT.wav [OPTIONS]
  viumic record  OUT.wav [--input NAME] [--seconds N]   raw mic, for bug reports

options:
  --model dpdfnet|rnnoise   dpdfnet = best quality (default), rnnoise = very light on CPU
  --strength DB         max noise attenuation in dB (default: unlimited)
  --deep SEMITONES      lower your voice by this many semitones (default: 0 = off)
  --hum 50|60|off       remove mains hum: 50 Hz (default; most of the world), 60 Hz (the Americas)
  --bypass              turn noise removal off
  --output NAME         default: VB-CABLE's \"CABLE Input\"; use your headphones to hear yourself
";

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let opt = |name: &str| {
        let i = args.iter().position(|a| a == name)?;
        args.get(i + 1).cloned()
    };
    let num = |name: &str, default: f32| -> Result<f32> {
        opt(name).map_or(Ok(default), |v| {
            v.parse()
                .with_context(|| format!("{name} expects a number"))
        })
    };
    let cfg = Config {
        input: opt("--input"),
        output: opt("--output"),
        model: match opt("--model").as_deref() {
            None | Some("dpdfnet") => Model::DpdfNet,
            Some("rnnoise") => Model::RNNoise,
            Some(m) => bail!("unknown model \"{m}\" (use dpdfnet or rnnoise)"),
        },
        strength_db: num("--strength", UNLIMITED_DB)?,
        enabled: !args.iter().any(|a| a == "--bypass"),
        depth: num("--deep", 0.0)?,
        hum_hz: match opt("--hum").as_deref() {
            Some("off") => None,
            _ => Some(num("--hum", 50.0)?),
        },
    };
    match (args.first().map(String::as_str), args.get(1), args.get(2)) {
        (Some("devices"), ..) => devices(),
        (Some("run"), ..) => run(&cfg),
        (Some("denoise"), Some(i), Some(o)) => denoise(i, o, &cfg),
        (Some("record"), Some(o), _) => record(o, cfg.input.as_deref(), num("--seconds", 30.0)?),
        _ => {
            eprint!("{USAGE}");
            std::process::exit(2)
        }
    }
}

fn devices() -> Result<()> {
    let d = engine::list_devices()?;
    let tag = |n: &String, def: &Option<String>| {
        if def.as_ref() == Some(n) {
            " (default)"
        } else {
            ""
        }
    };
    println!("Microphones:");
    for n in &d.inputs {
        println!("  {n}{}", tag(n, &d.default_input));
    }
    println!("Outputs:");
    for n in &d.outputs {
        println!("  {n}{}", tag(n, &d.default_output));
    }
    if !d.has_cable() {
        println!(
            "\nVB-CABLE not found. Install it from {}",
            engine::CABLE_URL
        );
    }
    Ok(())
}

fn run(cfg: &Config) -> Result<()> {
    let engine = Engine::start(cfg)?;
    println!(
        "{} -> {}   (Ctrl+C to stop)",
        engine.input_name, engine.output_name
    );
    loop {
        thread::sleep(Duration::from_millis(250));
        if let Some(e) = engine.take_error() {
            eprintln!("\n{e}");
        }
        let s = engine.stats();
        print!(
            "\rin {}  out {}  latency {:>3.0} ms (devices {:.0})  load {:>3.0}%  dropped {}  underruns {}  xruns {}   ",
            meter(s.in_peak),
            meter(s.out_peak),
            s.latency_ms,
            s.device_ms,
            s.load * 100.0,
            s.dropped,
            s.underruns,
            s.xruns
        );
        std::io::stdout().flush()?;
    }
}

/// -60..0 dBFS as 12 bars.
fn meter(peak: f32) -> String {
    let db = 20.0 * peak.max(1e-6).log10();
    let bars = ((db + 60.0) / 5.0).clamp(0.0, 12.0) as usize;
    format!("{:<12}", "#".repeat(bars))
}

fn record(output: &str, input: Option<&str>, secs: f32) -> Result<()> {
    println!("Recording {secs:.0} s of raw mic audio. Talk, pause, breathe, talk quietly...");
    let (samples, channels, sample_rate) = engine::record(input, secs)?;
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(output, spec)?;
    for s in samples {
        w.write_sample(s)?;
    }
    w.finalize()?;
    println!("wrote {output} ({channels} ch, {sample_rate} Hz)");
    Ok(())
}

fn denoise(input: &str, output: &str, cfg: &Config) -> Result<()> {
    let mut reader =
        hound::WavReader::open(input).with_context(|| format!("can't open {input}"))?;
    let spec = reader.spec();
    if spec.sample_rate != SAMPLE_RATE {
        bail!(
            "{input} is {} Hz; convert it to 48000 Hz first",
            spec.sample_rate
        );
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        hound::SampleFormat::Int => {
            let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 * scale))
                .collect::<Result<_, _>>()?
        }
    };
    let ch = spec.channels as usize;
    let mut audio: Vec<f32> = samples
        .chunks_exact(ch)
        .map(|f| f.iter().sum::<f32>() / ch as f32)
        .collect();

    if cfg.enabled {
        let mut den = Denoiser::new(cfg.model, cfg.strength_db, cfg.hum_hz)?;
        audio = engine::process_buffer(&audio, den.delay(), |i, o| den.process(i, o))?;
    }
    if cfg.depth > 0.0 {
        let mut deep = Deepen::default();
        audio = engine::process_buffer(&audio, deep.delay(), |i, o| {
            deep.process(cfg.depth, i, o);
            Ok(())
        })?;
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SAMPLE_RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(output, spec)?;
    for s in audio {
        w.write_sample(s)?;
    }
    w.finalize()?;
    println!("wrote {output}");
    Ok(())
}
