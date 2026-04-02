// Demo brain module: demo.belligerent.v1
//
// Guest ABI: `docs/intelligence_wasm_brains.md`
//
// Behavior (simplified v1):
// - Aggressively targets the nearest hostile unit within a notice radius.
// - When it has a target: attacks every tick and chases if melee.
// - Otherwise: short rest/wander loop.

const MAX_OBS_BYTES: usize = 65536;
const MAX_OUT_BYTES: usize = 8192;

const TAG_UNIT: u64 = 1 << 0;
const TAG_ATTACK_RANGED: u64 = 1 << 3;

static mut OBS_BUF: [u8; MAX_OBS_BYTES] = [0; MAX_OBS_BYTES];
static mut OUT_BUF: [u8; MAX_OUT_BYTES] = [0; MAX_OUT_BYTES];
static mut ALLOC_PHASE: u32 = 0;

// Persistent per-instance state.
static mut MODE_UNTIL_TICK: u64 = 0;
static mut HAS_WANDER_TARGET: u32 = 0;
static mut WANDER_TARGET: [f32; 3] = [0.0, 0.0, 0.0];
static mut LAST_HEALTH: i32 = -1;

static mut HAS_TARGET: u32 = 0;
static mut TARGET_ID_LO: u64 = 0;
static mut TARGET_ID_HI: u64 = 0;
static mut TARGET_POS: [f32; 3] = [0.0, 0.0, 0.0];
static mut TARGET_LAST_SEEN_TICK: u64 = 0;

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
    let is_ranged = (obs.self_tags_bits & TAG_ATTACK_RANGED) != 0;
    let notice_m = if is_ranged { 14.0 } else { 10.0 };
    let forget_target_ticks = 480u64;

    let mut rng = SplitMix64::new(obs.rng_seed ^ 0x1A45_5A6B_81E3_901C);

    // Detect attack via health drop.
    let attacked = match (obs.self_health, LAST_HEALTH) {
        (h, prev) if h >= 0 && prev >= 0 => h < prev,
        _ => false,
    };
    LAST_HEALTH = obs.self_health;

    // Refresh target from current snapshot, or forget.
    if HAS_TARGET != 0 {
        if let Some(seen) = find_entity_by_id(&obs, TARGET_ID_LO, TARGET_ID_HI) {
            TARGET_POS = add_pos(self_pos, seen.rel_pos);
            TARGET_LAST_SEEN_TICK = tick;
        } else if tick.wrapping_sub(TARGET_LAST_SEEN_TICK) > forget_target_ticks {
            HAS_TARGET = 0;
        }
    }

    // If attacked, prefer nearest hostile as target.
    if attacked {
        if let Some(hostile) = nearest_hostile_unit(&obs, None) {
            HAS_TARGET = 1;
            TARGET_ID_LO = hostile.entity_id_lo;
            TARGET_ID_HI = hostile.entity_id_hi;
            TARGET_POS = add_pos(self_pos, hostile.rel_pos);
            TARGET_LAST_SEEN_TICK = tick;
        }
    }

    // If no target: pick nearest hostile within notice radius.
    if HAS_TARGET == 0 {
        let max2 = notice_m * notice_m;
        if let Some(hostile) = nearest_hostile_unit(&obs, Some(max2)) {
            HAS_TARGET = 1;
            TARGET_ID_LO = hostile.entity_id_lo;
            TARGET_ID_HI = hostile.entity_id_hi;
            TARGET_POS = add_pos(self_pos, hostile.rel_pos);
            TARGET_LAST_SEEN_TICK = tick;
        }
    }

    // If we have a target, attack (and chase if melee).
    if HAS_TARGET != 0 {
        let mut count = 0u32;
        write_u32(out, 0, 0); // placeholder

        write_attack_target(out, 0, TARGET_ID_LO, TARGET_ID_HI, tick.wrapping_add(30));
        count += 1;
        if !is_ranged {
            write_move_to(out, count as usize, TARGET_POS, tick.wrapping_add(60));
            count += 1;
        }
        write_sleep(out, count as usize, 6);
        count += 1;

        write_u32(out, 0, count);
        return 4 + count * 32;
    }

    // No target: short rest/wander loop (wanders a bit more).
    if tick >= MODE_UNTIL_TICK {
        let r = rng.next_f32();
        if r < 0.60 {
            MODE_UNTIL_TICK = tick.wrapping_add(rng.gen_range_u32(90, 180) as u64);
            HAS_WANDER_TARGET = 1;
            WANDER_TARGET = random_wander_target(self_pos, 6.0, &mut rng);
        } else {
            MODE_UNTIL_TICK = tick.wrapping_add(rng.gen_range_u32(60, 150) as u64);
            HAS_WANDER_TARGET = 0;
        }
    }

    if HAS_WANDER_TARGET == 0 {
        let remaining = MODE_UNTIL_TICK.wrapping_sub(tick);
        let sleep = remaining.min(48).max(12) as u32;
        return write_sleep_only(out, sleep);
    }

    let arrival_m = 0.9f32;
    let arrival2 = arrival_m * arrival_m;
    let dx = WANDER_TARGET[0] - self_pos[0];
    let dz = WANDER_TARGET[2] - self_pos[2];
    let d2 = dx * dx + dz * dz;
    if !d2.is_finite() || d2 <= arrival2 {
        WANDER_TARGET = random_wander_target(self_pos, 6.0, &mut rng);
    }
    write_move_to_and_sleep(out, WANDER_TARGET, tick.wrapping_add(60), 18)
}

#[derive(Clone, Copy)]
struct Obs<'a> {
    bytes: &'a [u8],
    tick_index: u64,
    rng_seed: u64,
    self_kind_lo: u64,
    self_kind_hi: u64,
    self_tags_bits: u64,
    self_pos: [f32; 3],
    self_health: i32,
    nearby_count: usize,
}

#[derive(Clone, Copy)]
struct NearbyEntityView {
    entity_id_lo: u64,
    entity_id_hi: u64,
    kind_lo: u64,
    kind_hi: u64,
    rel_pos: [f32; 3],
    tags_bits: u64,
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
        let self_tags_bits = read_u64(bytes, 36);
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
            self_tags_bits,
            self_pos,
            self_health,
            nearby_count,
        })
    }

    fn nearby(&self, index: usize) -> Option<NearbyEntityView> {
        if index >= self.nearby_count {
            return None;
        }
        let base = 88 + index * 88;
        let entity_id_lo = read_u64(self.bytes, base + 0);
        let entity_id_hi = read_u64(self.bytes, base + 8);
        let kind_lo = read_u64(self.bytes, base + 16);
        let kind_hi = read_u64(self.bytes, base + 24);
        let rel_pos = [
            read_f32(self.bytes, base + 32),
            read_f32(self.bytes, base + 36),
            read_f32(self.bytes, base + 40),
        ];
        let tags_bits = read_u64(self.bytes, base + 76);
        Some(NearbyEntityView {
            entity_id_lo,
            entity_id_hi,
            kind_lo,
            kind_hi,
            rel_pos,
            tags_bits,
        })
    }
}

fn find_entity_by_id(obs: &Obs<'_>, id_lo: u64, id_hi: u64) -> Option<NearbyEntityView> {
    for i in 0..obs.nearby_count {
        let Some(e) = obs.nearby(i) else { continue };
        if e.entity_id_lo == id_lo && e.entity_id_hi == id_hi {
            return Some(e);
        }
    }
    None
}

fn nearest_hostile_unit(obs: &Obs<'_>, max2: Option<f32>) -> Option<NearbyEntityView> {
    let mut best: Option<(NearbyEntityView, f32)> = None;
    for i in 0..obs.nearby_count {
        let Some(e) = obs.nearby(i) else { continue };
        if (e.tags_bits & TAG_UNIT) == 0 {
            continue;
        }
        if e.kind_lo == obs.self_kind_lo && e.kind_hi == obs.self_kind_hi {
            continue;
        }
        let d2 = dist2_xz(e.rel_pos);
        if !d2.is_finite() {
            continue;
        }
        if let Some(m) = max2 {
            if d2 > m {
                continue;
            }
        }
        best = Some(match best {
            None => (e, d2),
            Some((prev, prev_d2)) => {
                if d2 < prev_d2 {
                    (e, d2)
                } else {
                    (prev, prev_d2)
                }
            }
        });
    }
    best.map(|(e, _)| e)
}

fn add_pos(self_pos: [f32; 3], rel: [f32; 3]) -> [f32; 3] {
    [self_pos[0] + rel[0], self_pos[1] + rel[1], self_pos[2] + rel[2]]
}

fn dist2_xz(rel: [f32; 3]) -> f32 {
    rel[0] * rel[0] + rel[2] * rel[2]
}

fn random_wander_target(self_pos: [f32; 3], radius: f32, rng: &mut SplitMix64) -> [f32; 3] {
    let a = rng.next_f32() * (core::f32::consts::PI * 2.0);
    let r = radius * rng.next_f32().sqrt();
    [self_pos[0] + a.cos() * r, self_pos[1], self_pos[2] + a.sin() * r]
}

fn write_sleep_only(out: &mut [u8], ticks: u32) -> u32 {
    if out.len() < 36 {
        return 0;
    }
    write_u32(out, 0, 1);
    write_sleep(out, 0, ticks);
    36
}

fn write_move_to_and_sleep(out: &mut [u8], pos: [f32; 3], valid_until: u64, sleep_ticks: u32) -> u32 {
    if out.len() < 4 + 2 * 32 {
        return 0;
    }
    write_u32(out, 0, 2);
    write_move_to(out, 0, pos, valid_until);
    write_sleep(out, 1, sleep_ticks);
    4 + 2 * 32
}

fn write_move_to(out: &mut [u8], index: usize, pos: [f32; 3], valid_until: u64) {
    let base = 4 + index * 32;
    write_u32(out, base + 0, 1);
    write_f32(out, base + 4, pos[0]);
    write_f32(out, base + 8, pos[1]);
    write_f32(out, base + 12, pos[2]);
    write_u64(out, base + 16, valid_until);
    for b in out[base + 24..base + 32].iter_mut() {
        *b = 0;
    }
}

fn write_attack_target(out: &mut [u8], index: usize, id_lo: u64, id_hi: u64, valid_until: u64) {
    let base = 4 + index * 32;
    write_u32(out, base + 0, 3);
    write_u64(out, base + 4, id_lo);
    write_u64(out, base + 12, id_hi);
    write_u64(out, base + 20, valid_until);
    for b in out[base + 28..base + 32].iter_mut() {
        *b = 0;
    }
}

fn write_sleep(out: &mut [u8], index: usize, ticks: u32) {
    let base = 4 + index * 32;
    write_u32(out, base + 0, 5);
    write_u32(out, base + 4, ticks);
    for b in out[base + 8..base + 32].iter_mut() {
        *b = 0;
    }
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
