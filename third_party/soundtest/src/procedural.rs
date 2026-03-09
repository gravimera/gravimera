use crate::backend::AudioOut;
use anyhow::{Result, anyhow};
use rand::Rng;
use rodio::buffer::SamplesBuffer;

pub const SAMPLE_RATE: u32 = 44_100;

pub fn play_token_text(audio: &AudioOut, token_text: &str) -> Result<()> {
    let samples = render_token_text(token_text)?;
    let source = SamplesBuffer::new(1, SAMPLE_RATE, samples);
    audio.play(source)
}

pub fn render_token_text(token_text: &str) -> Result<Vec<f32>> {
    let mut events = Vec::new();
    let mut buf = String::new();
    let mut chars = token_text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch.is_ascii_alphabetic() {
            buf.push(ch.to_ascii_lowercase());
            continue;
        }

        flush_token(&mut events, &mut buf);

        match ch {
            ',' => events.push(Event::PauseMs(120)),
            '?' | '!' => events.push(Event::PauseMs(180)),
            '.' => {
                if chars.peek() == Some(&'.') {
                    chars.next();
                    if chars.peek() == Some(&'.') {
                        chars.next();
                    }
                    events.push(Event::PauseMs(320));
                } else {
                    events.push(Event::PauseMs(220));
                }
            }
            '\n' => events.push(Event::PauseMs(250)),
            _ => {}
        }
    }

    flush_token(&mut events, &mut buf);

    if events.is_empty() {
        return Err(anyhow!("procedural text produced no events"));
    }

    let mut rng = rand::rng();
    let mut out = Vec::new();

    for event in events {
        match event {
            Event::Token(t) => out.extend(render_token(&t, &mut rng)),
            Event::PauseMs(ms) => out.extend(silence(ms)),
        }
    }

    Ok(out)
}

#[derive(Debug)]
enum Event {
    Token(String),
    PauseMs(u32),
}

fn flush_token(events: &mut Vec<Event>, buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    let token = std::mem::take(buf);
    events.push(Event::Token(token));
}

fn render_token(token: &str, rng: &mut impl Rng) -> Vec<f32> {
    match token {
        "beep" => tone(880.0, 120, 0.25),
        "boop" => tone(440.0, 120, 0.25),
        "whirr" => sweep(180.0, 780.0, 320, 0.22),
        "buzz" => square(220.0, 240, 0.18),
        "click" => noise_burst(12, 0.6, rng),

        "wind" => wind(650, 0.14, rng),
        "whoosh" => whoosh(480, 0.25, rng),
        "rustle" => rustle(0.22, rng),
        "creak" => sweep(90.0, 170.0, 420, 0.18),
        "rumble" => rumble(820, 0.16, rng),
        "crack" => noise_burst(20, 0.8, rng),

        _ => Vec::new(),
    }
}

fn samples_len(ms: u32) -> usize {
    ((ms as u64) * (SAMPLE_RATE as u64) / 1000) as usize
}

fn silence(ms: u32) -> Vec<f32> {
    vec![0.0; samples_len(ms)]
}

fn tone(freq: f32, ms: u32, amp: f32) -> Vec<f32> {
    let n = samples_len(ms);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let s = (2.0 * std::f32::consts::PI * freq * t).sin();
        out.push(s * amp);
    }
    apply_fade(out, 6)
}

fn square(freq: f32, ms: u32, amp: f32) -> Vec<f32> {
    let n = samples_len(ms);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let s = (2.0 * std::f32::consts::PI * freq * t).sin();
        out.push(if s >= 0.0 { amp } else { -amp });
    }
    apply_fade(out, 6)
}

fn sweep(start_hz: f32, end_hz: f32, ms: u32, amp: f32) -> Vec<f32> {
    let n = samples_len(ms);
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for i in 0..n {
        let p = i as f32 / (n.max(1) as f32);
        let freq = start_hz + (end_hz - start_hz) * p;
        phase += 2.0 * std::f32::consts::PI * freq / SAMPLE_RATE as f32;
        out.push(phase.sin() * amp);
    }
    apply_fade(out, 10)
}

fn noise_burst(ms: u32, amp: f32, rng: &mut impl Rng) -> Vec<f32> {
    let n = samples_len(ms);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let v: f32 = rng.random_range(-1.0..=1.0);
        out.push(v * amp);
    }
    apply_fade(out, 2)
}

fn wind(ms: u32, amp: f32, rng: &mut impl Rng) -> Vec<f32> {
    lowpass(noise(ms, amp, rng), 0.04)
}

fn whoosh(ms: u32, amp: f32, rng: &mut impl Rng) -> Vec<f32> {
    let mut out = noise(ms, amp, rng);
    apply_envelope(&mut out, 30, 80);
    lowpass(out, 0.06)
}

fn rustle(amp: f32, rng: &mut impl Rng) -> Vec<f32> {
    let mut out = Vec::new();
    for _ in 0..7 {
        out.extend(noise_burst(26, amp, rng));
        out.extend(silence(22));
    }
    out
}

fn rumble(ms: u32, amp: f32, rng: &mut impl Rng) -> Vec<f32> {
    let mut out = tone(60.0, ms, amp);
    let n = out.len();
    for i in 0..n {
        let v: f32 = rng.random_range(-1.0..=1.0);
        out[i] += v * (amp * 0.12);
    }
    lowpass(out, 0.02)
}

fn noise(ms: u32, amp: f32, rng: &mut impl Rng) -> Vec<f32> {
    let n = samples_len(ms);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let v: f32 = rng.random_range(-1.0..=1.0);
        out.push(v * amp);
    }
    apply_fade(out, 12)
}

fn lowpass(mut samples: Vec<f32>, alpha: f32) -> Vec<f32> {
    let mut prev = 0.0f32;
    for s in &mut samples {
        prev = prev + alpha * (*s - prev);
        *s = prev;
    }
    samples
}

fn apply_fade(mut samples: Vec<f32>, fade_ms: u32) -> Vec<f32> {
    let fade = samples_len(fade_ms).min(samples.len() / 2);
    if fade == 0 {
        return samples;
    }
    for i in 0..fade {
        let gain = i as f32 / fade as f32;
        samples[i] *= gain;
        let j = samples.len() - 1 - i;
        samples[j] *= gain;
    }
    samples
}

fn apply_envelope(samples: &mut [f32], attack_ms: u32, release_ms: u32) {
    let attack = samples_len(attack_ms).min(samples.len());
    let release = samples_len(release_ms).min(samples.len());
    for i in 0..attack {
        let gain = i as f32 / attack.max(1) as f32;
        samples[i] *= gain;
    }
    for i in 0..release {
        let idx = samples.len().saturating_sub(1 + i);
        let gain = i as f32 / release.max(1) as f32;
        samples[idx] *= 1.0 - gain;
    }
}
