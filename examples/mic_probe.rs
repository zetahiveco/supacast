//! Records 3 seconds from the default mic and prints peak/RMS level and the
//! resulting WAV size — a quick probe for the "Nothing heard" dictate bug.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

fn main() {
    let host = cpal::default_host();
    let device = host.default_input_device().expect("no mic");
    println!("input device: {device}");
    let config = device.default_input_config().expect("no default config");
    println!("config: {config:?}");

    let samples: std::sync::Arc<std::sync::Mutex<Vec<f32>>> = Default::default();
    let buf = samples.clone();

    macro_rules! stream {
        ($typ:ty, $conv:expr) => {{
            let buf = buf.clone();
            device
                .build_input_stream(
                    config.clone().into(),
                    move |data: &[$typ], _: &cpal::InputCallbackInfo| {
                        let conv: fn($typ) -> f32 = $conv;
                        buf.lock().unwrap().extend(data.iter().map(|s| conv(*s)));
                    },
                    |e| eprintln!("input error: {e}"),
                    None,
                )
                .unwrap()
        }};
    }

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => stream!(f32, |s: f32| s),
        cpal::SampleFormat::I16 => stream!(i16, |s: i16| s as f32 / 32768.0),
        cpal::SampleFormat::U16 => stream!(u16, |s: u16| (s as f32 - 32768.0) / 32768.0),
        other => panic!("unsupported {other:?}"),
    };
    stream.play().unwrap();

    println!("recording 3s — say something…");
    std::thread::sleep(std::time::Duration::from_secs(3));
    drop(stream);

    let data = samples.lock().unwrap().clone();
    let peak = data.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len().max(1) as f32).sqrt();
    println!(
        "captured {} samples ({:.2}s @ {}Hz), peak={peak:.4}, rms={rms:.4}",
        data.len(),
        data.len() as f32 / config.sample_rate() as f32,
        config.sample_rate()
    );
    if peak < 0.001 {
        println!("SILENCE — capture is producing no audio (permission or device issue).");
    }
}
