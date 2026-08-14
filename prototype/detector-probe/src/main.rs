//! PROTOTYPE — THROWAWAY MEASUREMENT HARNESS. Not production code.
//!
//! Question it exists to answer (three constants that argument cannot settle):
//!
//!   1. How large can `k` get in the period-multiple refinement stage before the
//!      autocorrelation peak at lag ≈ k·T stops being findable on a REAL decaying string?
//!      Provisional cap is 30. This is load-bearing: if k tops out low, the ≤1 cent
//!      Measurement Resolution promise fails at E4 and ADR 0001 needs rewriting.
//!   2. What Clarity (NSDF peak height) separates a plucked note from mains hum and a
//!      strummed chord? Provisional 0.8.
//!   3. What Level floor separates silence from signal?
//!
//! Run one session per condition so segments stay clean:
//!
//!   cargo run -- --list
//!   cargo run -- --device Scarlett --channel 1 --tag note   --secs 20
//!   cargo run -- --device Scarlett --channel 1 --tag hum    --secs 10
//!   cargo run -- --device Scarlett --channel 1 --tag chord  --secs 15
//!   cargo run -- --device Scarlett --channel 1 --tag silence --secs 10
//!   cargo run -- --tag bass-low-e --record bass-low-e --secs 8 --k-table
//!
//! `--record` also makes this the tool that captures the test corpus.

mod detect;

use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use detect::{nearest_note, Detector, NoiseFloor};

const WINDOW_MS: f32 = 171.0;
const HOP_MS: f32 = 21.0;
const MIN_HZ: f32 = 30.87; // B0
const MAX_HZ: f32 = 1318.5; // E6

struct Args {
    list: bool,
    device: Option<String>,
    channel: usize,
    tag: String,
    record: Option<String>,
    secs: u64,
    k_table: bool,
    sweep: bool,
    synthetic: Option<f32>,
    weak_fundamental: bool,
    decay: Option<f32>,
    harmonic_decay: Option<f32>,
    inharmonicity: Option<f32>,
    partials: Option<usize>,
    wav_in: Option<String>,
    verbose: bool,
    csv: bool,
    lowpass: Option<f32>,
}

fn parse_args() -> Args {
    let mut a = Args {
        list: false,
        device: None,
        channel: 0,
        tag: "untagged".into(),
        record: None,
        secs: 20,
        k_table: false,
        sweep: false,
        synthetic: None,
        weak_fundamental: false,
        decay: None,
        harmonic_decay: None,
        inharmonicity: None,
        partials: None,
        wav_in: None,
        verbose: false,
        csv: false,
        lowpass: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--list" => a.list = true,
            "--k-table" => a.k_table = true,
            "--synthetic-sweep" => a.sweep = true,
            "--weak-fundamental" => a.weak_fundamental = true,
            "--synthetic" => a.synthetic = it.next().and_then(|v| v.parse().ok()),
            "--decay" => a.decay = it.next().and_then(|v| v.parse().ok()),
            "--harmonic-decay" => a.harmonic_decay = it.next().and_then(|v| v.parse().ok()),
            "--inharmonicity" => a.inharmonicity = it.next().and_then(|v| v.parse().ok()),
            "--partials" => a.partials = it.next().and_then(|v| v.parse().ok()),
            "--wav-in" => a.wav_in = it.next(),
            "--verbose" => a.verbose = true,
            "--csv" => a.csv = true,
            "--lowpass" => a.lowpass = it.next().and_then(|v| v.parse().ok()),
            "--device" => a.device = it.next(),
            "--channel" => {
                // 1-based on the command line, matching the labels on an interface's front panel.
                a.channel = it
                    .next()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(1)
                    .saturating_sub(1);
            }
            "--tag" => a.tag = it.next().unwrap_or_else(|| "untagged".into()),
            "--record" => a.record = it.next(),
            "--secs" => a.secs = it.next().and_then(|v| v.parse().ok()).unwrap_or(20),
            other => eprintln!("ignoring unknown arg: {other}"),
        }
    }
    a
}

/// One window of a harmonic stack at an exactly known frequency. Amplitudes roll off as 1/h,
/// phases are deterministic but not all-aligned (all-aligned is unrealistically easy).
/// `weak` attenuates the fundamental to 10%, mimicking a bass pickup — the case that makes
/// spectral peak-picking give octave errors.
fn synth(hz: f32, sr: f32, n: usize, args: &Args) -> Vec<f32> {
    let mut out = vec![0.0f32; n];
    // Capping partial count is what a low-pass filter does to a real string. Filling all the
    // way to Nyquist is unphysical: B0 would get 699 partials, and the highest ones carry the
    // most inharmonicity error while being inaudible.
    let max_h = (((sr * 0.45) / hz).floor() as usize).min(args.partials.unwrap_or(usize::MAX));
    let b = args.inharmonicity.unwrap_or(0.0);
    for h in 1..=max_h.max(1) {
        let amp = if h == 1 && args.weak_fundamental {
            0.10
        } else {
            1.0 / h as f32
        };
        let phase = (h as f32 * 1.7).sin() * std::f32::consts::PI;
        // Stiff-string inharmonicity: partial h sits at h·f0·sqrt(1 + B·h²), not h·f0. The
        // signal is therefore not strictly periodic, and the mismatch compounds with lag —
        // which is the mechanism that can actually bound k.
        let fh = hz * h as f32 * (1.0 + b * (h * h) as f32).sqrt();
        for (i, s) in out.iter_mut().enumerate() {
            let t = i as f32 / sr;
            // Higher partials damp faster on a real string.
            let env = match args.harmonic_decay {
                Some(tau) => (-t * h as f32 / tau).exp(),
                None => 1.0,
            };
            *s += amp * env * (2.0 * std::f32::consts::PI * fh * t + phase).sin();
        }
    }
    if let Some(tau) = args.decay {
        for (i, s) in out.iter_mut().enumerate() {
            *s *= (-(i as f32 / sr) / tau).exp();
        }
    }
    let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-9);
    for s in out.iter_mut() {
        *s /= peak * 1.05;
    }
    out
}

const SWEEP: [(&str, f32); 14] = [
    ("B0", 30.868),
    ("E1", 41.203),
    ("A1", 55.000),
    ("D2", 73.416),
    ("E2", 82.407),
    ("G2", 97.999),
    ("A2", 110.000),
    ("D3", 146.832),
    ("G3", 195.998),
    ("B3", 246.942),
    ("E4", 329.628),
    ("A4", 440.000),
    ("E5", 659.255),
    ("E6", 1318.510),
];

/// Accuracy validation against exactly-known truth. This is the claim ADR 0001 rests on:
/// coarse MPM degrades badly at high frequency, and k-multiple refinement recovers it.
fn run_synthetic(args: &Args) {
    let sr = 48_000.0f32;
    let window = (WINDOW_MS / 1000.0 * sr).round() as usize;
    let mut det = Detector::new(sr, window, MIN_HZ, MAX_HZ).with_lowpass(args.lowpass);

    println!(
        "synthetic accuracy sweep — sr {} Hz, window {window} ({:.1} ms)",
        sr as u32, WINDOW_MS
    );
    if args.weak_fundamental {
        println!("fundamental attenuated to 10% (bass pickup mimic)");
    }
    if let Some(d) = args.decay {
        println!("exponential decay, tau = {d} s");
    }
    if let Some(d) = args.harmonic_decay {
        println!("differential harmonic decay, tau = {d} s (partial h decays as tau/h)");
    }
    if let Some(b) = args.inharmonicity {
        println!("stiff-string inharmonicity, B = {b:e}");
    }
    if let Some(m) = args.lowpass {
        println!("low-pass before refinement at {m}x coarse f0");
    }
    if let Some(n) = args.partials {
        println!("partials limited to {n} (equivalent to a low-pass at {n}x f0)");
    }
    println!(
        "\n{:<5} {:>10}  {:>11}  {:>11}  {:>6} {:>5} {:>7}",
        "note", "truth Hz", "coarse err", "refined err", "k", "kwin", "clarity"
    );
    println!("{}", "-".repeat(66));

    let list: Vec<(&str, f32)> = match args.synthetic {
        Some(hz) => vec![("--", hz)],
        None => SWEEP.to_vec(),
    };

    let mut worst_refined = 0.0f32;
    for (name, truth) in list {
        let buf = synth(truth, sr, window, args);
        match det.analyse(&buf) {
            Some(r) => {
                let ce = 1200.0 * (r.coarse_hz / truth).log2();
                let re = 1200.0 * (r.refined_hz / truth).log2();
                worst_refined = worst_refined.max(re.abs());
                println!(
                    "{name:<5} {truth:>10.3}  {ce:>+9.2}\u{a2}  {re:>+9.2}\u{a2}  {:>6} {:>5} {:>7.3}",
                    r.k_used, r.k_max_window, r.clarity
                );
                if args.k_table {
                    for p in &r.probes {
                        println!(
                            "        k={:3} nsdf={:.3} implied={:9.3}Hz err={:+7.2}\u{a2} {}",
                            p.k,
                            p.nsdf,
                            p.implied_hz,
                            1200.0 * (p.implied_hz / truth).log2(),
                            if p.found { "found" } else { "LOST" }
                        );
                    }
                }
            }
            None => println!("{name:<5} {truth:>10.3}  {:>11}  {:>11}", "NO PITCH", "-"),
        }
    }
    println!("\nworst refined error across sweep: {worst_refined:.2}\u{a2}  (requirement: \u{2264}1.00\u{a2})");
}


/// Run the detector over a recorded file instead of live audio. Makes calibration reproducible:
/// thresholds can be swept over the same clip rather than asking someone to replay it.
fn run_wav(path: &str, args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let mut rd = hound::WavReader::open(path)?;
    let spec = rd.spec();
    let sr = spec.sample_rate as f32;
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            rd.samples::<i32>().filter_map(|s| s.ok()).map(|s| s as f32 / max).collect()
        }
        hound::SampleFormat::Float => rd.samples::<f32>().filter_map(|s| s.ok()).collect(),
    };
    let samples: Vec<f32> = if spec.channels > 1 {
        raw.iter().step_by(spec.channels as usize).copied().collect()
    } else {
        raw
    };

    let window = (WINDOW_MS / 1000.0 * sr).round() as usize;
    let hop = (HOP_MS / 1000.0 * sr).round() as usize;
    let tag = if args.tag == "untagged" {
        std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    } else {
        args.tag.clone()
    };

    if args.csv {
        // header emitted by the caller
    } else {
        println!("file        {path}");
    }
    if !args.csv {
    println!("audio       {} ch @ {} Hz, {:.2} s", spec.channels, sr as u32, samples.len() as f32 / sr);
    println!("framing     window {window} ({:.1} ms), hop {hop} ({:.1} ms)",
        window as f32 / sr * 1000.0, hop as f32 / sr * 1000.0);
    }

    let mut det = Detector::new(sr, window, MIN_HZ, MAX_HZ).with_lowpass(args.lowpass);
    let mut stats = Stats::default();
    // Gate 18 dB above the tracked floor, never above -50 dBFS, leaking up 0.1 dB/s.
    // A 1 dB/s leak drifted the floor 12 dB over a 12 s clip -- far faster than any real noise
    // floor moves, and it dragged the gate up during continuous playing.
    let mut floor = NoiseFloor::new(18.0, -50.0, 0.1, sr / hop as f32);
    let (mut passed, mut total) = (0usize, 0usize);
    let mut pos = 0usize;
    while pos + window <= samples.len() {
        let frame = &samples[pos..pos + window];
        let rms = (frame.iter().map(|s| s * s).sum::<f32>() / window as f32).sqrt();
        let db = if rms > 0.0 { 20.0 * rms.log10() } else { -120.0 };
        let r = det.analyse(frame);
        if args.csv {
            match &r {
                Some(r) => println!("{tag},{:.4},{db:.2},{:.4},{:.3},{}", pos as f32 / sr, r.clarity, r.refined_hz, r.k_used),
                None => println!("{tag},{:.4},{db:.2},,,", pos as f32 / sr),
            }
        }
        if args.verbose {
            let t = pos as f32 / sr;
            match &r {
                Some(r) => {
                    let (n, c) = nearest_note(r.refined_hz);
                    println!("t={t:5.2}  lvl={db:6.1}dB  clar={:.3}  {:8.2}Hz {n}{c:+6.1}c  k={:3}",
                        r.clarity, r.refined_hz, r.k_used);
                }
                None => println!("t={t:5.2}  lvl={db:6.1}dB  clar=  --"),
            }
        }
        floor.observe(db);
        total += 1;
        let clarity_ok = r.as_ref().map(|r| r.clarity >= 0.90).unwrap_or(false);
        if db > floor.gate_db() && clarity_ok {
            passed += 1;
        }
        stats.push(db, r.as_ref());
        pos += hop;
    }
    if !args.csv {
        stats.report(&tag, sr, window, hop);
        println!(
            "\nGATING  observed floor {:.1} dBFS  ->  gate {:.1} dBFS (+18 dB, ceiling -50)\n\
             \x20       frames passing BOTH gates (level + clarity>=0.90): {}/{} = {:.1}%",
            floor.floor_db(), floor.gate_db(), passed, total,
            100.0 * passed as f32 / total.max(1) as f32
        );
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args();

    if args.sweep || args.synthetic.is_some() {
        run_synthetic(&args);
        return Ok(());
    }

    if let Some(path) = args.wav_in.clone() {
        return run_wav(&path, &args);
    }

    let host = cpal::default_host();

    if args.list {
        println!("input devices:");
        for d in host.input_devices()? {
            let name = d.to_string();
            match d.default_input_config() {
                Ok(c) => println!(
                    "  {name}\n      {} ch, {} Hz, {:?}",
                    c.channels(),
                    c.sample_rate(),
                    c.sample_format()
                ),
                Err(e) => println!("  {name}\n      (no default input config: {e})"),
            }
        }
        return Ok(());
    }

    let device = match &args.device {
        Some(want) => host
            .input_devices()?
            .find(|d| d.to_string().to_lowercase().contains(&want.to_lowercase()))
            .ok_or_else(|| format!("no input device matching {want:?} — try --list"))?,
        None => host
            .default_input_device()
            .ok_or("no default input device — try --list")?,
    };

    let supported = device.default_input_config()?;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.config();
    let sample_rate = config.sample_rate as f32;
    let channels = config.channels as usize;

    if args.channel >= channels {
        return Err(format!(
            "channel {} requested but device exposes only {} channel(s)",
            args.channel + 1,
            channels
        )
        .into());
    }

    let window = (WINDOW_MS / 1000.0 * sample_rate).round() as usize;
    let hop = (HOP_MS / 1000.0 * sample_rate).round() as usize;

    println!("device      {}", device);
    println!(
        "config      {} ch @ {} Hz, {:?}, listening on channel {}",
        channels,
        sample_rate as u32,
        sample_format,
        args.channel + 1
    );
    println!(
        "framing     window {window} samples ({:.1} ms), hop {hop} samples ({:.1} ms) = {:.0} readings/sec",
        window as f32 / sample_rate * 1000.0,
        hop as f32 / sample_rate * 1000.0,
        sample_rate / hop as f32
    );
    println!("range       {MIN_HZ} Hz .. {MAX_HZ} Hz");
    println!("tag         {}", args.tag);
    println!("running for {} s — play now\n", args.secs);

    let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(sample_rate as usize * 2);
    let ch = args.channel;
    let err_fn = |e| eprintln!("stream error: {e}");

    // The callback does one thing: deinterleave the chosen channel into the queue. Same
    // discipline the real app will use — no allocation, no locks, no analysis in here.
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _: &_| {
                for frame in data.chunks(channels) {
                    if let Some(&s) = frame.get(ch) {
                        let _ = producer.push(s);
                    }
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _: &_| {
                for frame in data.chunks(channels) {
                    if let Some(&s) = frame.get(ch) {
                        let _ = producer.push(s as f32 / i16::MAX as f32);
                    }
                }
            },
            err_fn,
            None,
        )?,
        other => return Err(format!("unhandled sample format {other:?}").into()),
    };
    stream.play()?;

    let mut det = Detector::new(sample_rate, window, MIN_HZ, MAX_HZ).with_lowpass(args.lowpass);
    let det_max_lag = det.refine_max_lag();
    let mut hist: Vec<f32> = Vec::with_capacity(window * 4);
    let mut recorded: Vec<f32> = Vec::new();
    let mut stats = Stats::default();
    let started = Instant::now();
    let deadline = Duration::from_secs(args.secs);

    while started.elapsed() < deadline {
        while let Ok(s) = consumer.pop() {
            hist.push(s);
            if args.record.is_some() {
                recorded.push(s);
            }
        }
        while hist.len() >= window {
            let t = started.elapsed().as_secs_f32();
            let frame = &hist[..window];
            let rms = (frame.iter().map(|s| s * s).sum::<f32>() / window as f32).sqrt();
            let db = if rms > 0.0 {
                20.0 * rms.log10()
            } else {
                -120.0
            };

            match det.analyse(frame) {
                Some(r) => {
                    let (cn, cc) = nearest_note(r.coarse_hz);
                    let (rn, rc) = nearest_note(r.refined_hz);
                    println!(
                        "t={t:5.2}  lvl={db:6.1}dB  clar={:.3}  coarse={:8.2}Hz {cn}{cc:+5.1}c  \
                         refined={:8.2}Hz {rn}{rc:+5.1}c  k={:3}  kwin={:3}",
                        r.clarity, r.coarse_hz, r.refined_hz, r.k_used, r.k_max_window
                    );
                    if args.k_table {
                        println!(
                            "          coarse lag {:.2} samples, max lag {} — window allows k\u{2264}{}",
                            r.coarse_lag,
                            det_max_lag,
                            r.k_max_window
                        );
                        for p in &r.probes {
                            println!(
                                "          k={:3}  nsdf={:.3}  implied={:8.2}Hz  \
                                 vs_coarse={:+7.1}c  {}",
                                p.k,
                                p.nsdf,
                                p.implied_hz,
                                p.cents_vs_coarse,
                                if p.found { "found" } else { "LOST" }
                            );
                        }
                    }
                    stats.push(db, Some(&r));
                }
                None => {
                    println!("t={t:5.2}  lvl={db:6.1}dB  clar=  --    (no periodicity found)");
                    stats.push(db, None);
                }
            }
            hist.drain(..hop);
        }
        std::thread::sleep(Duration::from_millis(2));
    }

    drop(stream);

    if let (Some(name), false) = (&args.record, recorded.is_empty()) {
        let dir = std::path::Path::new("corpus-scratch-PROTOTYPE");
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{name}.wav"));
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: sample_rate as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec)?;
        for s in &recorded {
            w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
        }
        w.finalize()?;
        println!("\nwrote {} ({} samples)", path.display(), recorded.len());
    }

    stats.report(&args.tag, sample_rate, window, hop);
    Ok(())
}

#[derive(Default)]
struct Stats {
    db: Vec<f32>,
    clarity: Vec<f32>,
    k_max: Vec<f32>,
    k_win: Vec<f32>,
    refine_shift: Vec<f32>,
    silent_frames: usize,
    /// (level bucket index, k_max) — for seeing whether k degrades as a note decays.
    decay: Vec<(usize, f32)>,
}

const BUCKETS: [(&str, f32); 4] = [
    ("loud   >-20dB", -20.0),
    ("mid  -20..-30", -30.0),
    ("quiet-30..-40", -40.0),
    ("faint  <-40dB", f32::NEG_INFINITY),
];

impl Stats {
    fn push(&mut self, db: f32, r: Option<&detect::Reading>) {
        self.db.push(db);
        match r {
            Some(r) => {
                self.clarity.push(r.clarity);
                self.k_max.push(r.k_max_signal as f32);
                self.k_win.push(r.k_max_window as f32);
                let shift = 1200.0 * (r.refined_hz / r.coarse_hz).log2();
                self.refine_shift.push(shift.abs());
                let bucket = BUCKETS
                    .iter()
                    .position(|&(_, floor)| db > floor)
                    .unwrap_or(3);
                self.decay.push((bucket, r.k_max_signal as f32));
            }
            None => self.silent_frames += 1,
        }
    }

    fn report(&self, tag: &str, sr: f32, window: usize, hop: usize) {
        let total = self.db.len();
        println!("\n{}", "=".repeat(78));
        println!(
            "SUMMARY  tag={tag}  frames={total}  sr={} Hz  window={window} ({:.1} ms)  hop={hop}",
            sr as u32,
            window as f32 / sr * 1000.0
        );
        println!("{}", "=".repeat(78));
        if total == 0 {
            println!("no frames captured — was the device silent or the channel wrong?");
            return;
        }
        println!(
            "frames with no periodicity found: {} of {total} ({:.0}%)",
            self.silent_frames,
            100.0 * self.silent_frames as f32 / total as f32
        );
        row("level dBFS", &self.db);
        row("clarity", &self.clarity);
        row("k_max (signal)", &self.k_max);
        row("k_max (window)", &self.k_win);
        row("refine shift ¢", &self.refine_shift);

        println!("\nQ1 — does k survive a decaying string? k_max by level:");
        for (i, (label, _)) in BUCKETS.iter().enumerate() {
            let vals: Vec<f32> = self
                .decay
                .iter()
                .filter(|(b, _)| *b == i)
                .map(|(_, k)| *k)
                .collect();
            if vals.is_empty() {
                println!("  {label}   (no frames)");
            } else {
                let mut v = vals.clone();
                v.sort_by(f32::total_cmp);
                println!(
                    "  {label}   n={:4}  k_max p05={:.0} p50={:.0} p95={:.0}",
                    v.len(),
                    pct(&v, 0.05),
                    pct(&v, 0.50),
                    pct(&v, 0.95)
                );
            }
        }
    }
}

fn row(label: &str, vals: &[f32]) {
    if vals.is_empty() {
        println!("{label:>16}  (none)");
        return;
    }
    let mut v = vals.to_vec();
    v.sort_by(f32::total_cmp);
    println!(
        "{label:>16}  p05={:8.2}  p50={:8.2}  p95={:8.2}  min={:8.2}  max={:8.2}",
        pct(&v, 0.05),
        pct(&v, 0.50),
        pct(&v, 0.95),
        v[0],
        v[v.len() - 1]
    );
}

fn pct(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return f32::NAN;
    }
    let i = ((sorted.len() - 1) as f32 * p).round() as usize;
    sorted[i]
}
