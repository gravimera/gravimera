use serde_json::json;

#[derive(Debug, Clone)]
pub struct EffectSpec {
    pub preset: String,
    pub amount: f32,
    pub speed: Option<f32>,
    pub pitch_semitones: Option<f32>,
    pub bass_db: Option<f32>,
    pub treble_db: Option<f32>,
    pub reverb: Option<f32>,
    pub distortion: Option<f32>,
}

impl Default for EffectSpec {
    fn default() -> Self {
        Self {
            preset: "neutral".to_owned(),
            amount: 1.0,
            speed: None,
            pitch_semitones: None,
            bass_db: None,
            treble_db: None,
            reverb: None,
            distortion: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EffectParams {
    pub speed: f32,
    pub pitch_semitones: f32,
    pub bass_db: f32,
    pub treble_db: f32,
    pub reverb: f32,
    pub distortion: f32,
}

impl EffectParams {
    pub fn neutral() -> Self {
        Self {
            speed: 1.0,
            pitch_semitones: 0.0,
            bass_db: 0.0,
            treble_db: 0.0,
            reverb: 0.0,
            distortion: 0.0,
        }
    }

    pub fn from_spec(spec: &EffectSpec) -> Self {
        let amount = clamp_f32(spec.amount, 0.0, 1.0);
        let base = preset_params(spec.preset.as_str());
        let neutral = Self::neutral();

        let mut out = Self {
            speed: lerp(neutral.speed, base.speed, amount),
            pitch_semitones: lerp(neutral.pitch_semitones, base.pitch_semitones, amount),
            bass_db: lerp(neutral.bass_db, base.bass_db, amount),
            treble_db: lerp(neutral.treble_db, base.treble_db, amount),
            reverb: lerp(neutral.reverb, base.reverb, amount),
            distortion: lerp(neutral.distortion, base.distortion, amount),
        };

        if let Some(speed) = spec.speed {
            out.speed = speed;
        }
        if let Some(pitch) = spec.pitch_semitones {
            out.pitch_semitones = pitch;
        }
        if let Some(bass_db) = spec.bass_db {
            out.bass_db = bass_db;
        }
        if let Some(treble_db) = spec.treble_db {
            out.treble_db = treble_db;
        }
        if let Some(reverb) = spec.reverb {
            out.reverb = reverb;
        }
        if let Some(distortion) = spec.distortion {
            out.distortion = distortion;
        }

        out.speed = clamp_f32(out.speed, 0.4, 1.8);
        out.pitch_semitones = clamp_f32(out.pitch_semitones, -24.0, 24.0);
        out.bass_db = clamp_f32(out.bass_db, -12.0, 18.0);
        out.treble_db = clamp_f32(out.treble_db, -12.0, 18.0);
        out.reverb = clamp_f32(out.reverb, 0.0, 1.0);
        out.distortion = clamp_f32(out.distortion, 0.0, 1.0);
        out
    }

    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "speed": self.speed,
            "pitch_semitones": self.pitch_semitones,
            "bass_db": self.bass_db,
            "treble_db": self.treble_db,
            "reverb": self.reverb,
            "distortion": self.distortion,
        })
    }
}

fn preset_params(preset: &str) -> EffectParams {
    match preset.trim().to_ascii_lowercase().as_str() {
        "dragon" => EffectParams {
            speed: 0.78,
            pitch_semitones: -10.0,
            bass_db: 16.0,
            treble_db: -5.0,
            reverb: 0.60,
            distortion: 0.35,
        },
        "robot" => EffectParams {
            speed: 1.06,
            pitch_semitones: -2.0,
            bass_db: -3.0,
            treble_db: 12.0,
            reverb: 0.05,
            distortion: 0.28,
        },
        "fairy" => EffectParams {
            speed: 1.18,
            pitch_semitones: 11.0,
            bass_db: -9.0,
            treble_db: 16.0,
            reverb: 0.40,
            distortion: 0.0,
        },
        "giant" => EffectParams {
            speed: 0.70,
            pitch_semitones: -12.0,
            bass_db: 18.0,
            treble_db: -6.0,
            reverb: 0.20,
            distortion: 0.10,
        },
        "ghost" => EffectParams {
            speed: 0.92,
            pitch_semitones: -5.0,
            bass_db: -10.0,
            treble_db: 8.0,
            reverb: 0.85,
            distortion: 0.0,
        },
        "radio" => EffectParams {
            speed: 1.0,
            pitch_semitones: 0.0,
            bass_db: -12.0,
            treble_db: -8.0,
            reverb: 0.0,
            distortion: 0.40,
        },
        _ => EffectParams::neutral(),
    }
}

pub fn apply_effects_mono(samples: &[f32], sample_rate: u32, params: &EffectParams) -> Vec<f32> {
    let mut out = samples.to_owned();
    if out.is_empty() {
        return out;
    }

    let pitch_factor = (2.0f32).powf(params.pitch_semitones / 12.0);
    if (pitch_factor - 1.0).abs() > 1e-3 {
        out = resample_linear(&out, pitch_factor);
    }

    let stretch = pitch_factor / params.speed;
    if (stretch - 1.0).abs() > 1e-3 {
        out = wsola_time_stretch(&out, stretch, sample_rate);
    }

    if params.bass_db.abs() > 0.01 {
        let mut filter = Biquad::low_shelf(sample_rate, 160.0, params.bass_db);
        filter.process_buffer(&mut out);
    }
    if params.treble_db.abs() > 0.01 {
        let mut filter = Biquad::high_shelf(sample_rate, 4200.0, params.treble_db);
        filter.process_buffer(&mut out);
    }

    if params.reverb > 0.001 {
        out = simple_reverb(&out, sample_rate, params.reverb);
    }

    if params.distortion > 0.001 {
        apply_distortion(&mut out, params.distortion);
    }

    apply_limiter(&mut out, 0.98);
    out
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn clamp_f32(v: f32, min: f32, max: f32) -> f32 {
    v.max(min).min(max)
}

fn resample_linear(input: &[f32], factor: f32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    if factor <= 0.0 || (factor - 1.0).abs() < 1e-6 {
        return input.to_owned();
    }

    let out_len = ((input.len() as f32) / factor).ceil().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f32 * factor;
        let idx = pos.floor() as usize;
        let frac = pos - (idx as f32);
        let s0 = input.get(idx).copied().unwrap_or(0.0);
        let s1 = input.get(idx + 1).copied().unwrap_or(s0);
        out.push(s0 * (1.0 - frac) + s1 * frac);
    }
    out
}

fn wsola_time_stretch(input: &[f32], stretch: f32, sample_rate: u32) -> Vec<f32> {
    if input.len() < 256 {
        return input.to_owned();
    }
    if stretch <= 0.0 || (stretch - 1.0).abs() < 1e-3 {
        return input.to_owned();
    }

    let mut frame_len = (sample_rate as f32 * 0.040).round() as usize;
    frame_len = frame_len.clamp(256, 4096);
    if input.len() < frame_len {
        return input.to_owned();
    }
    let overlap = (frame_len / 2).clamp(64, frame_len - 1);
    let hop_out = frame_len - overlap;
    let hop_in = (hop_out as f32) / stretch;

    let search = (sample_rate as f32 * 0.015).round() as isize;

    let out_len = ((input.len() as f32) * stretch).ceil() as usize + frame_len + 1;
    let mut out = vec![0.0f32; out_len];

    out[..frame_len].copy_from_slice(&input[..frame_len]);

    let mut out_pos = hop_out;
    let mut in_pos_f = hop_in;

    while out_pos + frame_len <= out.len() {
        let expected = in_pos_f.round() as isize;
        if expected < 0 {
            break;
        }
        if (expected as usize) + frame_len >= input.len() {
            break;
        }

        let cand = find_best_candidate(input, &out, out_pos, expected, search, overlap, frame_len);
        overlap_add(&mut out, out_pos, input, cand, overlap, frame_len);

        in_pos_f += hop_in;
        out_pos += hop_out;
    }

    out.truncate(out_pos + frame_len);
    out
}

fn find_best_candidate(
    input: &[f32],
    out: &[f32],
    out_pos: usize,
    expected_in: isize,
    search: isize,
    overlap: usize,
    frame_len: usize,
) -> usize {
    let out_overlap = &out[out_pos..out_pos + overlap];
    let mut best_pos = expected_in.max(0) as usize;
    let mut best_score = f32::NEG_INFINITY;

    for offset in -search..=search {
        let cand = expected_in + offset;
        if cand < 0 {
            continue;
        }
        let cand = cand as usize;
        if cand + frame_len >= input.len() {
            continue;
        }
        let in_overlap = &input[cand..cand + overlap];
        let score = normalized_correlation(out_overlap, in_overlap);
        if score > best_score {
            best_score = score;
            best_pos = cand;
        }
    }

    best_pos
}

fn normalized_correlation(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut ea = 0.0f32;
    let mut eb = 0.0f32;
    for i in 0..a.len().min(b.len()) {
        let x = a[i];
        let y = b[i];
        dot += x * y;
        ea += x * x;
        eb += y * y;
    }
    if ea <= 1e-9 || eb <= 1e-9 {
        return 0.0;
    }
    dot / ((ea * eb).sqrt() + 1e-9)
}

fn overlap_add(
    out: &mut [f32],
    out_pos: usize,
    input: &[f32],
    in_pos: usize,
    overlap: usize,
    frame_len: usize,
) {
    for i in 0..frame_len {
        let src = input[in_pos + i];
        let dst_idx = out_pos + i;
        if i < overlap {
            let t = i as f32 / overlap.max(1) as f32;
            out[dst_idx] = out[dst_idx] * (1.0 - t) + src * t;
        } else {
            out[dst_idx] = src;
        }
    }
}

#[derive(Debug, Clone)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn low_shelf(sample_rate: u32, freq_hz: f32, gain_db: f32) -> Self {
        let sr = sample_rate as f32;
        let a = (10.0f32).powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq_hz / sr;
        let cosw0 = w0.cos();
        let sinw0 = w0.sin();
        let s = 1.0f32;
        let alpha = sinw0 / 2.0 * (((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).max(0.0)).sqrt();
        let beta = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) - (a - 1.0) * cosw0 + beta);
        let b1 = 2.0 * a * ((a - 1.0) - (a + 1.0) * cosw0);
        let b2 = a * ((a + 1.0) - (a - 1.0) * cosw0 - beta);
        let a0 = (a + 1.0) + (a - 1.0) * cosw0 + beta;
        let a1 = -2.0 * ((a - 1.0) + (a + 1.0) * cosw0);
        let a2 = (a + 1.0) + (a - 1.0) * cosw0 - beta;

        Self::new(b0, b1, b2, a0, a1, a2)
    }

    fn high_shelf(sample_rate: u32, freq_hz: f32, gain_db: f32) -> Self {
        let sr = sample_rate as f32;
        let a = (10.0f32).powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * freq_hz / sr;
        let cosw0 = w0.cos();
        let sinw0 = w0.sin();
        let s = 1.0f32;
        let alpha = sinw0 / 2.0 * (((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).max(0.0)).sqrt();
        let beta = 2.0 * a.sqrt() * alpha;

        let b0 = a * ((a + 1.0) + (a - 1.0) * cosw0 + beta);
        let b1 = -2.0 * a * ((a - 1.0) + (a + 1.0) * cosw0);
        let b2 = a * ((a + 1.0) + (a - 1.0) * cosw0 - beta);
        let a0 = (a + 1.0) - (a - 1.0) * cosw0 + beta;
        let a1 = 2.0 * ((a - 1.0) - (a + 1.0) * cosw0);
        let a2 = (a + 1.0) - (a - 1.0) * cosw0 - beta;

        Self::new(b0, b1, b2, a0, a1, a2)
    }

    fn new(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        let inv_a0 = if a0.abs() < 1e-9 { 1.0 } else { 1.0 / a0 };
        Self {
            b0: b0 * inv_a0,
            b1: b1 * inv_a0,
            b2: b2 * inv_a0,
            a1: a1 * inv_a0,
            a2: a2 * inv_a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process_sample(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }

    fn process_buffer(&mut self, buf: &mut [f32]) {
        for s in buf {
            *s = self.process_sample(*s);
        }
    }
}

#[derive(Debug, Clone)]
struct DelayLine {
    buf: Vec<f32>,
    idx: usize,
    feedback: f32,
}

impl DelayLine {
    fn new(len: usize, feedback: f32) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            idx: 0,
            feedback,
        }
    }

    fn process_comb(&mut self, input: f32) -> f32 {
        let out = self.buf[self.idx];
        self.buf[self.idx] = input + out * self.feedback;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }

    fn process_allpass(&mut self, input: f32) -> f32 {
        let buf_out = self.buf[self.idx];
        let out = -input + buf_out;
        self.buf[self.idx] = input + buf_out * self.feedback;
        self.idx += 1;
        if self.idx >= self.buf.len() {
            self.idx = 0;
        }
        out
    }
}

fn simple_reverb(input: &[f32], sample_rate: u32, amount: f32) -> Vec<f32> {
    let amount = clamp_f32(amount, 0.0, 1.0);
    if amount <= 0.0 {
        return input.to_owned();
    }

    let wet = 0.08 + 0.60 * amount;
    let dry = 1.0 - wet;
    let feedback = 0.30 + 0.62 * amount;

    let scale = sample_rate as f32 / 44_100.0;
    let comb_delays = [1116, 1188, 1277, 1356];
    let allpass_delays = [556, 441];

    let mut combs: Vec<DelayLine> = comb_delays
        .iter()
        .map(|d| DelayLine::new(((*d as f32) * scale) as usize, feedback))
        .collect();
    let mut allpasses: Vec<DelayLine> = allpass_delays
        .iter()
        .map(|d| DelayLine::new(((*d as f32) * scale) as usize, 0.5))
        .collect();

    let mut out = Vec::with_capacity(input.len());
    for &x in input {
        let mut acc = 0.0f32;
        for comb in &mut combs {
            acc += comb.process_comb(x);
        }
        acc /= combs.len().max(1) as f32;
        for ap in &mut allpasses {
            acc = ap.process_allpass(acc);
        }
        out.push(dry * x + wet * acc);
    }
    out
}

fn apply_distortion(samples: &mut [f32], amount: f32) {
    let amount = clamp_f32(amount, 0.0, 1.0);
    let drive = 1.0 + amount * 30.0;
    let norm = drive.tanh().max(1e-6);
    for s in samples {
        let x = *s * drive;
        *s = x.tanh() / norm;
    }
}

fn apply_limiter(samples: &mut [f32], ceiling: f32) {
    let ceiling = clamp_f32(ceiling, 0.1, 1.0);
    let mut max_abs = 0.0f32;
    for &s in samples.iter() {
        max_abs = max_abs.max(s.abs());
    }
    if max_abs > ceiling {
        let gain = ceiling / max_abs;
        for s in samples {
            *s *= gain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_spec_clamps_values() {
        let mut spec = EffectSpec::default();
        spec.preset = "dragon".to_owned();
        spec.amount = 2.0;
        spec.speed = Some(9.9);
        spec.pitch_semitones = Some(99.0);
        spec.bass_db = Some(99.0);
        spec.treble_db = Some(-99.0);
        spec.reverb = Some(9.0);
        spec.distortion = Some(-9.0);

        let params = EffectParams::from_spec(&spec);
        assert!((params.speed - 1.8).abs() < 1e-6);
        assert!((params.pitch_semitones - 24.0).abs() < 1e-6);
        assert!((params.bass_db - 18.0).abs() < 1e-6);
        assert!((params.treble_db - (-12.0)).abs() < 1e-6);
        assert!((params.reverb - 1.0).abs() < 1e-6);
        assert!((params.distortion - 0.0).abs() < 1e-6);
    }

    #[test]
    fn apply_effects_handles_empty_and_short_inputs() {
        let params = EffectParams::neutral();
        let out = apply_effects_mono(&[], 44_100, &params);
        assert!(out.is_empty());

        let samples = vec![0.1f32; 128];
        let out = apply_effects_mono(&samples, 44_100, &params);
        assert_eq!(out.len(), samples.len());
    }

    #[test]
    fn pitch_and_speed_are_reasonably_independent() {
        let sample_rate = 44_100;
        let n = sample_rate as usize;
        let samples = vec![0.1f32; n];

        let mut params = EffectParams::neutral();
        params.speed = 1.0;
        params.pitch_semitones = 7.0;
        let pitched = apply_effects_mono(&samples, sample_rate, &params);
        let ratio = pitched.len() as f32 / samples.len() as f32;
        assert!((ratio - 1.0).abs() < 0.25);

        let mut slower = EffectParams::neutral();
        slower.speed = 0.8;
        let slow_out = apply_effects_mono(&samples, sample_rate, &slower);
        assert!(slow_out.len() > samples.len());

        let mut faster = EffectParams::neutral();
        faster.speed = 1.2;
        let fast_out = apply_effects_mono(&samples, sample_rate, &faster);
        assert!(fast_out.len() < samples.len());
    }

    #[test]
    fn wsola_does_not_panic_when_input_shorter_than_frame() {
        // Regression test: 40ms frame at 22_050Hz is 882 samples, so 288 would panic before bounds checks.
        let sample_rate = 22_050;
        let input = vec![0.1f32; 288];
        let out = wsola_time_stretch(&input, 1.25, sample_rate);
        assert_eq!(out.len(), input.len());
    }
}
