use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Native microphone recorder — replaces the web `MediaRecorder` +
/// `AudioContext` analyser used by the Tauri/webview version.
///
/// Captures f32 samples from the default input device on a cpal audio
/// thread into a shared buffer, and publishes an RMS loudness level
/// (0..=1, bit-packed in an AtomicU32) so the dictate ring can pulse.
pub struct Recorder {
    stream: Option<cpal::Stream>,
    samples: Arc<Mutex<Vec<f32>>>,
    level: Arc<AtomicU32>,
    pub sample_rate: u32,
}

impl Recorder {
    pub fn start() -> Result<Recorder, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| "no microphone found".to_string())?;
        let config = device
            .default_input_config()
            .map_err(|e| format!("could not open mic config: {e}"))?;

        let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let level = Arc::new(AtomicU32::new(0));

        let err_fn = |e| eprintln!("audio input error: {e}");

        macro_rules! build_stream {
            ($typ:ty, $conv:expr) => {{
                let buf = samples.clone();
                let lvl = level.clone();
                device.build_input_stream(
                    config.clone().into(),
                    move |data: &[$typ], _: &cpal::InputCallbackInfo| {
                        let conv: fn($typ) -> f32 = $conv;
                        {
                            let mut out = buf.lock().unwrap();
                            out.extend(data.iter().map(|s| conv(*s)));
                        }
                        // RMS over this callback -> UI level meter.
                        let sum: f32 = data.iter().map(|s| conv(*s) * conv(*s)).sum();
                        let rms = (sum / data.len().max(1) as f32).sqrt();
                        let scaled = (rms * 4.0).min(1.0);
                        lvl.store(scaled.to_bits(), Ordering::Relaxed);
                    },
                    err_fn,
                    None,
                )
            }};
        }

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => build_stream!(f32, |s: f32| s).map_err(cpal_err)?,
            cpal::SampleFormat::I16 => {
                build_stream!(i16, |s: i16| s as f32 / 32768.0).map_err(cpal_err)?
            }
            cpal::SampleFormat::U16 => {
                build_stream!(u16, |s: u16| (s as f32 - 32768.0) / 32768.0).map_err(cpal_err)?
            }
            other => return Err(format!("unsupported mic sample format: {other:?}")),
        };
        stream.play().map_err(|e| format!("could not start mic: {e}"))?;

        Ok(Recorder {
            stream: Some(stream),
            samples,
            level,
            sample_rate: config.sample_rate(),
        })
    }

    /// Current input level, 0..=1 (for the pulsing orb).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Whether any non-zero sample has been captured so far. A denied mic
    /// permission on macOS yields a running but completely silent stream,
    /// so this is also the permission probe.
    pub fn has_signal(&self) -> bool {
        self.samples.lock().unwrap().iter().any(|&s| s != 0.0)
    }

    /// Stop capturing and return the recorded samples. Dropping the cpal
    /// stream detaches from the mic.
    pub fn stop(mut self) -> (Vec<f32>, u32) {
        self.stream.take(); // drop -> stop
        let samples = self.samples.lock().unwrap().clone();
        (samples, self.sample_rate)
    }
}

fn cpal_err(e: cpal::Error) -> String {
    format!("could not open mic stream: {e}")
}

/// Launch-time microphone check: opens the default input and waits up to
/// `max_wait_ms` for any non-zero sample. Returns false when there is no
/// input device, the stream fails, or the input stays silent (e.g. macOS
/// microphone permission denied) — callers should ask for permission then.
pub fn probe_input(max_wait_ms: u64) -> bool {
    let Ok(rec) = Recorder::start() else {
        return false;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(max_wait_ms);
    let mut ok = false;
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(150));
        if rec.has_signal() {
            ok = true;
            break;
        }
    }
    drop(rec); // release the mic
    ok
}

// ---------------------------------------------------------------------------
// WAV encode/decode (16-bit PCM, mono) — hand-rolled, no extra deps.
// ---------------------------------------------------------------------------

/// Encode mono f32 samples as a 16-bit PCM WAV file in memory.
pub fn encode_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36u32 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decode a 16-bit PCM WAV (as written by `encode_wav`) back to mono f32.
/// Returns `(samples, sample_rate)`; skips unknown chunks so WAV files from
/// other writers usually work too.
pub fn decode_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32), String> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut pos = 12usize;
    let mut sample_rate = 44100u32;
    let mut data: Option<&[u8]> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().unwrap()) as usize;
        let body_start = pos + 8;
        let body_end = (body_start + size).min(bytes.len());
        match id {
            b"fmt " => {
                if body_end - body_start >= 16 {
                    sample_rate = u32::from_le_bytes(
                        bytes[body_start + 4..body_start + 8].try_into().unwrap(),
                    );
                }
            }
            b"data" => data = Some(&bytes[body_start..body_end]),
            _ => {}
        }
        pos = body_start + size + (size & 1); // chunks are word-aligned
    }
    let data = data.ok_or("WAV missing data chunk")?;
    let samples: Vec<f32> = data
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect();
    Ok((samples, sample_rate))
}

// ---------------------------------------------------------------------------
// Native playback — replaces the <audio data:...> webview player.
// ---------------------------------------------------------------------------

/// Handle for a sound playing on the default output device. Drop to detach
/// (the sound keeps playing to completion on its thread). Supports the
/// inline player UI: pause/resume, seek and progress reporting.
pub struct PlaybackHandle {
    stop: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    /// Playback position in *source* samples (f64 bit-packed so the audio
    /// thread can advance it losslessly).
    pos: Arc<AtomicU64>,
    total_samples: usize,
    sample_rate: u32,
}

impl PlaybackHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn is_done(&self) -> bool {
        self.done.load(Ordering::Relaxed)
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }
    /// Total duration in seconds.
    pub fn duration_secs(&self) -> f64 {
        self.total_samples as f64 / self.sample_rate.max(1) as f64
    }
    /// Current playback position in seconds.
    pub fn position_secs(&self) -> f64 {
        f64::from_bits(self.pos.load(Ordering::Relaxed)) / self.sample_rate.max(1) as f64
    }
    /// Seek to a position in seconds (clamped to the track length).
    pub fn seek_secs(&self, secs: f64) {
        let s = (secs * self.sample_rate as f64).clamp(0.0, self.total_samples as f64);
        self.pos.store(s.to_bits(), Ordering::Relaxed);
        if s < self.total_samples as f64 {
            self.done.store(false, Ordering::Relaxed);
        }
    }
}

/// Play 16-bit PCM WAV bytes on the default output device.
pub fn play_wav(wav: &[u8]) -> Result<PlaybackHandle, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let (samples, rate) = decode_wav(wav)?;
    let total_samples = samples.len();
    let pos = Arc::new(AtomicU64::new(0.0f64.to_bits()));
    let stop = Arc::new(AtomicBool::new(false));
    let paused = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));

    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "no audio output device".to_string())?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("could not open output config: {e}"))?;
    let out_rate = config.sample_rate() as usize;

    // Naive linear resample from `rate` to the output device rate.
    let samples = Arc::new(samples);
    let step = rate as f64 / out_rate as f64;
    let channels = config.channels() as usize;

    let err_fn = |e| eprintln!("audio output error: {e}");
    let s_pos = pos.clone();
    let s_stop = stop.clone();
    let s_paused = paused.clone();
    let s_done = done.clone();
    let s_samples = samples.clone();

    let mix = move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
        if s_stop.load(Ordering::Relaxed) || s_paused.load(Ordering::Relaxed) {
            // Stopped or paused: output silence and don't advance position.
            out.fill(0.0);
            return;
        }
        // Position is tracked in source samples so seeking is exact.
        let mut pos = f64::from_bits(s_pos.load(Ordering::Relaxed));
        for frame in out.chunks_mut(channels) {
            let t = pos;
            let i0 = t as usize;
            let frac = (t - i0 as f64) as f32;
            let v = if i0 + 1 < s_samples.len() {
                s_samples[i0] * (1.0 - frac) + s_samples[i0 + 1] * frac
            } else if i0 < s_samples.len() {
                s_samples[i0]
            } else {
                0.0
            };
            frame.fill(v);
            pos += step;
        }
        s_pos.store(pos.to_bits(), Ordering::Relaxed);
        if pos >= s_samples.len() as f64 {
            s_done.store(true, Ordering::Relaxed);
        }
    };
    let stream = device
        .build_output_stream(config.into(), mix, err_fn, None)
        .map_err(|e| format!("could not open output stream: {e}"))?;
    stream.play().map_err(|e| format!("could not start playback: {e}"))?;

    // Keep the stream alive on a worker thread until done or stopped.
    let t_stop = stop.clone();
    let t_done = done.clone();
    std::thread::spawn(move || {
        let _stream = stream;
        loop {
            if t_done.load(Ordering::Relaxed) || t_stop.load(Ordering::Relaxed) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    });

    Ok(PlaybackHandle {
        stop,
        done,
        paused,
        pos,
        total_samples,
        sample_rate: rate,
    })
}
