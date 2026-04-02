// Demo brain module: demo.orbit.v1
//
// Guest ABI: `docs/intelligence_wasm_brains.md`
//
// Obs header v1 offsets:
//   tick_index: u64 @ 4
//   self_pos.y: f32  @ 48

const MAX_OBS_BYTES: usize = 65536;
const MAX_OUT_BYTES: usize = 8192;

static mut OBS_BUF: [u8; MAX_OBS_BYTES] = [0; MAX_OBS_BYTES];
static mut OUT_BUF: [u8; MAX_OUT_BYTES] = [0; MAX_OUT_BYTES];
static mut ALLOC_PHASE: u32 = 0;

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
    const HEADER_BYTES: usize = 88;
    if obs_ptr == 0 || out_ptr == 0 {
        return 0;
    }
    let obs = core::slice::from_raw_parts(obs_ptr as *const u8, obs_len as usize);
    if obs.len() < HEADER_BYTES {
        return 0;
    }
    let out = core::slice::from_raw_parts_mut(out_ptr as *mut u8, out_cap as usize);
    if out.len() < 36 {
        return 0;
    }

    let tick_index = read_u64(obs, 4);
    let self_y = read_f32(obs, 48);

    let radius = 6.0f32;
    let rads_per_tick = 0.05f32;
    let a = (tick_index as f32) * rads_per_tick;
    let x = a.cos() * radius;
    let z = a.sin() * radius;
    let y = self_y;
    let valid_until = tick_index.wrapping_add(10);

    // out_v1:
    //   u32 command_count
    //   record[0]: kind=1 MoveTo { f32 x,y,z; u64 valid_until_tick; ... }
    write_u32(out, 0, 1);
    write_u32(out, 4, 1);
    write_f32(out, 8, x);
    write_f32(out, 12, y);
    write_f32(out, 16, z);
    write_u64(out, 20, valid_until);
    // zero-fill unused payload bytes for determinism.
    for b in out[28..36].iter_mut() {
        *b = 0;
    }

    36
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
