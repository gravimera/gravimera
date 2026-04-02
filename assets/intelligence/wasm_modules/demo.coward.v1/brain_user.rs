// Demo brain module: demo.coward.v1
//
// Guest ABI: `docs/intelligence_wasm_brains.md`
//
// Behavior (simplified v1):
// - If a hostile unit is close (or we took damage): flee and sleep briefly.
// - Otherwise: wander/rest/look loop.

const MAX_OBS_BYTES: usize = 65536;
const MAX_OUT_BYTES: usize = 8192;

const TAG_UNIT: u64 = 1 << 0;

static mut OBS_BUF: [u8; MAX_OBS_BYTES] = [0; MAX_OBS_BYTES];
static mut OUT_BUF: [u8; MAX_OUT_BYTES] = [0; MAX_OUT_BYTES];
static mut ALLOC_PHASE: u32 = 0;

// Persistent per-instance state (this module instance is per unit).
static mut MODE: u32 = 0; // 0=wander, 1=rest, 2=look
static mut MODE_UNTIL_TICK: u64 = 0;
static mut HAS_WANDER_TARGET: u32 = 0;
static mut WANDER_TARGET: [f32; 3] = [0.0, 0.0, 0.0];
static mut LAST_HEALTH: i32 = -1;

#[no_mangle]
pub unsafe extern "C" fn brain_alloc_v1(len: u32) -> u32 {
    ALLOC_PHASE = ALLOC_PHASE.wrapping_add(1);
    if ALLOC_PHASE % 2 == 1 {
        if (len as usize) > MAX_OBS_BYTES {
            return 0;
        }
        core::ptr::addr_of_mut!(OBS_BUF) as *mut u8 as u32
    } else {
        if (len as usize) > MAX_OUT_BYTES {
            return 0;
        }
        core::ptr::addr_of_mut!(OUT_BUF) as *mut u8 as u32
    }
}

#[no_mangle]
pub unsafe extern "C" fn brain_tick_v1(
    obs_ptr: u32,
    obs_len: u32,
    out_ptr: u32,
    out_cap: u32,
) -> u32 {
    if obs_ptr == 0 || out_ptr == 0 {
        return 0;
    }

    let obs_bytes = core::slice::from_raw_parts(obs_ptr as *const u8, obs_len as usize);
    let Some(obs) = Obs::parse(obs_bytes) else {
        return 0;
    };

    let out = core::slice::from_raw_parts_mut(out_ptr as *mut u8, out_cap as usize);
    if out.len() < 4 + 32 {
        return 0;
    }

    let tick = obs.tick_index;
    let self_pos = obs.self_pos;
    let mut rng = SplitMix64::new(obs.rng_seed ^ 0xC0B4_12D3_4A6E_9F01);

    // Detect attack via health drop.
    let attacked = match (obs.self_health, LAST_HEALTH) {
        (h, prev) if h >= 0 && prev >= 0 => h < prev,
        _ => false,
    };
    LAST_HEALTH = obs.self_health;

    let flee_trigger_m = 2.8f32;
    let flee_distance_m = 12.0f32;

    let threat = nearest_hostile_unit(&obs);
    let close_threat = threat.is_some_and(|t| t.d2_xz <= flee_trigger_m * flee_trigger_m);

    if attacked || close_threat {
        let threat_rel = threat
            .map(|t| [t.rel_pos[0], t.rel_pos[1], t.rel_pos[2]])
            .unwrap_or_else(|| random_rel_dir(&mut rng));
        let goal = flee_from_rel(self_pos, threat_rel, flee_distance_m, &mut rng);
        return write_out_move_to_and_sleep(out, goal, tick.wrapping_add(30), 6);
    }

    // Idle behavior: wander/rest/look.
    if tick >= MODE_UNTIL_TICK {
        let r = rng.next_f32();
        if r < 0.55 {
            MODE = 0;
            MODE_UNTIL_TICK = tick.wrapping_add(rng.gen_range_u32(120, 240) as u64);
            HAS_WANDER_TARGET = 1;
            WANDER_TARGET = random_wander_target(self_pos, 8.0, &mut rng);
        } else if r < 0.85 {
            MODE = 1;
            MODE_UNTIL_TICK = tick.wrapping_add(rng.gen_range_u32(90, 210) as u64);
            HAS_WANDER_TARGET = 0;
        } else {
            MODE = 2;
            MODE_UNTIL_TICK = tick.wrapping_add(rng.gen_range_u32(30, 90) as u64);
            HAS_WANDER_TARGET = 0;
        }
    }

    match MODE {
        // Wander
        0 => {
            let arrival_m = 0.8f32;
            let arrival2 = arrival_m * arrival_m;
            if HAS_WANDER_TARGET == 0 {
                HAS_WANDER_TARGET = 1;
                WANDER_TARGET = random_wander_target(self_pos, 8.0, &mut rng);
            } else {
                let dx = WANDER_TARGET[0] - self_pos[0];
                let dz = WANDER_TARGET[2] - self_pos[2];
                let d2 = dx * dx + dz * dz;
                if !d2.is_finite() || d2 <= arrival2 {
                    WANDER_TARGET = random_wander_target(self_pos, 8.0, &mut rng);
                }
            }
            write_out_move_to_and_sleep(out, WANDER_TARGET, tick.wrapping_add(60), 18)
        }
        // Rest
        1 => {
            let remaining = MODE_UNTIL_TICK.wrapping_sub(tick);
            let sleep = remaining.min(30).max(6) as u32;
            write_out_sleep_only(out, sleep)
        }
        // Look
        _ => {
            let remaining = MODE_UNTIL_TICK.wrapping_sub(tick);
            let sleep = remaining.min(18).max(6) as u32;
            write_out_sleep_only(out, sleep)
        }
    }
}

#[derive(Clone, Copy)]
struct Obs<'a> {
    bytes: &'a [u8],
    tick_index: u64,
    rng_seed: u64,
    self_kind_lo: u64,
    self_kind_hi: u64,
    self_pos: [f32; 3],
    self_health: i32,
    nearby_count: usize,
}

#[derive(Clone, Copy)]
struct Nearby {
    kind_lo: u64,
    kind_hi: u64,
    rel_pos: [f32; 3],
    tags_bits: u64,
    d2_xz: f32,
}

impl<'a> Obs<'a> {
    fn parse(bytes: &'a [u8]) -> Option<Self> {
        const HEADER_BYTES: usize = 88;
        if bytes.len() < HEADER_BYTES {
            return None;
        }
        let tick_index = read_u64(bytes, 4);
        let rng_seed = read_u64(bytes, 12);
        let self_kind_lo = read_u64(bytes, 20);
        let self_kind_hi = read_u64(bytes, 28);
        let self_pos = [read_f32(bytes, 44), read_f32(bytes, 48), read_f32(bytes, 52)];
        let self_health = read_i32(bytes, 72);
        let nearby_count = read_u32(bytes, 84) as usize;
        let needed = HEADER_BYTES.saturating_add(nearby_count.saturating_mul(88));
        if bytes.len() < needed {
            return None;
        }
        Some(Self {
            bytes,
            tick_index,
            rng_seed,
            self_kind_lo,
            self_kind_hi,
            self_pos,
            self_health,
            nearby_count,
        })
    }

    fn nearby(&self, index: usize) -> Option<Nearby> {
        if index >= self.nearby_count {
            return None;
        }
        let base = 88 + index * 88;
        // kind_id u128 as two u64 words
        let kind_lo = read_u64(self.bytes, base + 16);
        let kind_hi = read_u64(self.bytes, base + 24);
        let rel_pos = [
            read_f32(self.bytes, base + 32),
            read_f32(self.bytes, base + 36),
            read_f32(self.bytes, base + 40),
        ];
        let tags_bits = read_u64(self.bytes, base + 80);
        let d2_xz = rel_pos[0] * rel_pos[0] + rel_pos[2] * rel_pos[2];
        Some(Nearby {
            kind_lo,
            kind_hi,
            rel_pos,
            tags_bits,
            d2_xz,
        })
    }
}

fn nearest_hostile_unit(obs: &Obs<'_>) -> Option<Nearby> {
    let mut best: Option<Nearby> = None;
    for i in 0..obs.nearby_count {
        let Some(e) = obs.nearby(i) else { continue };
        if (e.tags_bits & TAG_UNIT) == 0 {
            continue;
        }
        // Hostile: kind != self_kind.
        if e.kind_lo == obs.self_kind_lo && e.kind_hi == obs.self_kind_hi {
            continue;
        }
        if !e.d2_xz.is_finite() {
            continue;
        }
        best = Some(match best {
            None => e,
            Some(prev) => {
                if e.d2_xz < prev.d2_xz {
                    e
                } else {
                    prev
                }
            }
        });
    }
    best
}

fn flee_from_rel(self_pos: [f32; 3], threat_rel: [f32; 3], distance: f32, rng: &mut SplitMix64) -> [f32; 3] {
    let dx = -threat_rel[0];
    let dz = -threat_rel[2];
    if let Some((nx, nz)) = normalize_xz(dx, dz) {
        [self_pos[0] + nx * distance, self_pos[1], self_pos[2] + nz * distance]
    } else {
        let rel = random_rel_dir(rng);
        [self_pos[0] - rel[0] * distance, self_pos[1], self_pos[2] - rel[2] * distance]
    }
}

fn random_wander_target(self_pos: [f32; 3], radius: f32, rng: &mut SplitMix64) -> [f32; 3] {
    let a = rng.next_f32() * (core::f32::consts::PI * 2.0);
    let r = radius * rng.next_f32().sqrt();
    let dx = a.cos() * r;
    let dz = a.sin() * r;
    [self_pos[0] + dx, self_pos[1], self_pos[2] + dz]
}

fn random_rel_dir(rng: &mut SplitMix64) -> [f32; 3] {
    let a = rng.next_f32() * (core::f32::consts::PI * 2.0);
    [a.cos(), 0.0, a.sin()]
}

fn normalize_xz(dx: f32, dz: f32) -> Option<(f32, f32)> {
    let d2 = dx * dx + dz * dz;
    if !d2.is_finite() || d2 <= 1e-10 {
        return None;
    }
    let inv = 1.0 / d2.sqrt();
    Some((dx * inv, dz * inv))
}

fn write_out_sleep_only(out: &mut [u8], ticks: u32) -> u32 {
    // count=1, kind=5 (SleepForTicks)
    if out.len() < 4 + 32 {
        return 0;
    }
    write_u32(out, 0, 1);
    write_u32(out, 4, 5);
    write_u32(out, 8, ticks);
    for b in out[12..36].iter_mut() {
        *b = 0;
    }
    36
}

fn write_out_move_to_and_sleep(out: &mut [u8], pos: [f32; 3], valid_until: u64, sleep_ticks: u32) -> u32 {
    // count=2:
    //   [0] MoveTo
    //   [1] SleepForTicks
    if out.len() < 4 + 2 * 32 {
        return 0;
    }
    write_u32(out, 0, 2);

    // record 0
    write_u32(out, 4, 1);
    write_f32(out, 8, pos[0]);
    write_f32(out, 12, pos[1]);
    write_f32(out, 16, pos[2]);
    write_u64(out, 20, valid_until);
    for b in out[28..36].iter_mut() {
        *b = 0;
    }

    // record 1
    let o = 4 + 32;
    write_u32(out, o, 5);
    write_u32(out, o + 4, sleep_ticks);
    for b in out[o + 8..o + 32].iter_mut() {
        *b = 0;
    }

    4 + 2 * 32
}

struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        let mut z = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        self.state = z;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_f32(&mut self) -> f32 {
        let bits = (self.next_u64() >> 40) as u32; // 24 bits
        (bits as f32) / ((1u32 << 24) as f32)
    }

    fn gen_range_u32(&mut self, lo: u32, hi: u32) -> u32 {
        if hi <= lo {
            return lo;
        }
        let span = (hi - lo) as u64;
        lo + (self.next_u64() % span) as u32
    }
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(buf)
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[offset..offset + 4]);
    i32::from_le_bytes(buf)
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(buf)
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[offset..offset + 4]);
    f32::from_le_bytes(buf)
}

fn write_u32(out: &mut [u8], offset: usize, value: u32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(out: &mut [u8], offset: usize, value: u64) {
    out[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn write_f32(out: &mut [u8], offset: usize, value: f32) {
    out[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
