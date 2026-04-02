use crate::intelligence::protocol::*;
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use wasmtime::{
    AsContext, AsContextMut, Config, Engine, Instance, Memory, Module, Store, TypedFunc,
};

const METADATA_FILE_NAME: &str = "module.json";
const WASM_FILE_NAME: &str = "brain.wasm";
const RUST_SOURCE_FILE_NAME: &str = "brain_user.rs";
const BUILD_DIR_NAME: &str = "build";
const BUILD_WASM_FILE_NAME: &str = "brain.wasm";

const ABI_VERSION_V1: u32 = 1;

const MAX_WASM_MODULE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_WASM_MEMORY_PAGES: u32 = 256; // 16 MiB
const MAX_OUT_BYTES_V1: u32 = 8 * 1024;
const FUEL_PER_TICK_V1: u64 = 2_000_000;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum WasmSourceKind {
    WasmOnly,
    RustSource,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct WasmModuleMetadata {
    module_id: String,
    abi_version: u32,
    source_kind: WasmSourceKind,
}

#[derive(Debug, Clone)]
struct WasmModuleRecord {
    module_dir: PathBuf,
    meta: WasmModuleMetadata,
}

pub(crate) struct WasmBrainsRuntime {
    engine: Engine,
    module_store_root: PathBuf,
    compiled: HashMap<String, std::sync::Arc<WasmBrainModule>>,
}

struct WasmBrainModule {
    module_id: String,
    abi_version: u32,
    module: Module,
}

pub(crate) struct WasmBrainInstance {
    module_id: String,
    abi_version: u32,
    store: Store<()>,
    memory: Memory,
    alloc_v1: TypedFunc<u32, u32>,
    tick_v1: TypedFunc<(u32, u32, u32, u32), u32>,
    _instance: Instance,
}

pub(crate) fn list_available_wasm_module_ids() -> Vec<String> {
    list_available_wasm_module_ids_in_dir(crate::paths::intelligence_wasm_modules_dir().as_path())
}

pub(crate) fn list_available_wasm_module_ids_in_dir(module_store_root: &Path) -> Vec<String> {
    scan_module_store(module_store_root)
        .unwrap_or_default()
        .into_iter()
        .map(|rec| rec.meta.module_id)
        .collect()
}

impl WasmBrainsRuntime {
    pub(crate) fn new() -> Result<Self, String> {
        Self::new_with_module_store_root(crate::paths::intelligence_wasm_modules_dir())
    }

    pub(crate) fn new_with_module_store_root(module_store_root: PathBuf) -> Result<Self, String> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config).map_err(|err| format!("wasmtime engine: {err}"))?;
        Ok(Self {
            engine,
            module_store_root,
            compiled: HashMap::new(),
        })
    }

    pub(crate) fn load_module(&mut self, module_id: &str) -> Result<(), String> {
        let module_id = module_id.trim();
        validate_module_id(module_id)?;

        let record = find_module_record(self.module_store_root.as_path(), module_id)?;
        let wasm_path = ensure_wasm_artifact(&record)?;
        let module = load_wasmtime_module(&self.engine, module_id, &wasm_path)?;

        self.compiled.insert(
            module_id.to_string(),
            std::sync::Arc::new(WasmBrainModule {
                module_id: module_id.to_string(),
                abi_version: record.meta.abi_version,
                module,
            }),
        );
        Ok(())
    }

    pub(crate) fn spawn_instance(&mut self, module_id: &str) -> Result<WasmBrainInstance, String> {
        let module_id = module_id.trim();
        validate_module_id(module_id)?;
        let compiled = match self.compiled.get(module_id) {
            Some(v) => v.clone(),
            None => {
                self.load_module(module_id)?;
                self.compiled
                    .get(module_id)
                    .cloned()
                    .ok_or_else(|| "Internal error: module not cached after load".to_string())?
            }
        };

        let mut store = Store::new(&self.engine, ());

        // No imports (obs-only, no WASI).
        let instance = Instance::new(&mut store, &compiled.module, &[])
            .map_err(|err| format!("instantiate `{module_id}`: {err}"))?;

        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| "Missing export `memory`".to_string())?;

        match compiled.abi_version {
            ABI_VERSION_V1 => {
                let alloc_v1 = instance
                    .get_typed_func::<u32, u32>(&mut store, "brain_alloc_v1")
                    .map_err(|err| format!("Missing/invalid export `brain_alloc_v1`: {err}"))?;
                let tick_v1 = instance
                    .get_typed_func::<(u32, u32, u32, u32), u32>(&mut store, "brain_tick_v1")
                    .map_err(|err| format!("Missing/invalid export `brain_tick_v1`: {err}"))?;

                Ok(WasmBrainInstance {
                    module_id: compiled.module_id.clone(),
                    abi_version: compiled.abi_version,
                    store,
                    memory,
                    alloc_v1,
                    tick_v1,
                    _instance: instance,
                })
            }
            other => Err(format!(
                "Unsupported wasm abi_version {other} (supported: {ABI_VERSION_V1})"
            )),
        }
    }
}

impl WasmBrainInstance {
    pub(crate) fn tick(
        &mut self,
        input: &TickInput,
        caps: BudgetCaps,
        capabilities: &HashSet<String>,
    ) -> Result<TickOutput, String> {
        match self.abi_version {
            ABI_VERSION_V1 => tick_v1(self, input, caps, capabilities),
            other => Err(format!(
                "Unsupported wasm abi_version {other} (supported: {ABI_VERSION_V1})"
            )),
        }
    }
}

fn tick_v1(
    instance: &mut WasmBrainInstance,
    input: &TickInput,
    caps: BudgetCaps,
    capabilities: &HashSet<String>,
) -> Result<TickOutput, String> {
    let obs = encode_obs_v1(input, caps);

    instance
        .store
        .set_fuel(FUEL_PER_TICK_V1)
        .map_err(|err| format!("{} wasmtime set_fuel: {err}", instance.module_id))?;

    let obs_ptr = instance
        .alloc_v1
        .call(&mut instance.store, obs.len() as u32)
        .map_err(|err| format!("{} brain_alloc_v1: {err}", instance.module_id))?;
    write_guest_memory(
        &instance.memory,
        &mut instance.store,
        obs_ptr,
        obs.as_slice(),
    )?;

    let out_ptr = instance
        .alloc_v1
        .call(&mut instance.store, MAX_OUT_BYTES_V1)
        .map_err(|err| format!("{} brain_alloc_v1(out): {err}", instance.module_id))?;

    let written = instance
        .tick_v1
        .call(
            &mut instance.store,
            (obs_ptr, obs.len() as u32, out_ptr, MAX_OUT_BYTES_V1),
        )
        .map_err(|err| format!("{} brain_tick_v1: {err}", instance.module_id))?;

    if written == 0 {
        return Err("brain_tick_v1 returned 0".into());
    }
    if written > MAX_OUT_BYTES_V1 {
        return Err(format!(
            "brain_tick_v1 wrote {written} bytes (out_cap={MAX_OUT_BYTES_V1})"
        ));
    }

    let raw = read_guest_memory(&instance.memory, &mut instance.store, out_ptr, written)?;
    let mut out = decode_out_v1(raw.as_slice())?;
    filter_commands_by_capabilities(&mut out.commands, capabilities);
    Ok(out)
}

fn filter_commands_by_capabilities(commands: &mut Vec<BrainCommand>, caps: &HashSet<String>) {
    commands.retain(|cmd| match cmd {
        BrainCommand::MoveTo { .. } | BrainCommand::SetMove { .. } => caps.contains("brain.move"),
        BrainCommand::AttackTarget { .. } => caps.contains("brain.combat"),
        BrainCommand::SleepForTicks { .. } => true,
        BrainCommand::Say { .. } => caps.contains("brain.talk"),
    });
}

fn encode_obs_v1(input: &TickInput, caps: BudgetCaps) -> Vec<u8> {
    let max_nearby = caps.max_nearby_entities as usize;
    let nearby = input.nearby_entities.iter().take(max_nearby);
    let nearby_count: u32 = nearby.clone().count().try_into().unwrap_or(u32::MAX);

    let mut out = Vec::with_capacity(64 + (nearby_count as usize) * 88);
    out.extend_from_slice(&input.dt_ms.to_le_bytes());
    out.extend_from_slice(&input.tick_index.to_le_bytes());
    out.extend_from_slice(&input.rng_seed.to_le_bytes());
    for v in input.self_state.pos {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&input.self_state.yaw.to_le_bytes());
    for v in input.self_state.vel {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&opt_i32(input.self_state.health).to_le_bytes());
    out.extend_from_slice(&opt_i32(input.self_state.health_max).to_le_bytes());
    out.extend_from_slice(&opt_i32(input.self_state.stamina).to_le_bytes());
    out.extend_from_slice(&nearby_count.to_le_bytes());

    for e in nearby {
        let entity_id = stable_u128_from_str(e.entity_instance_id.as_str());
        out.extend_from_slice(&(entity_id as u64).to_le_bytes());
        out.extend_from_slice(&((entity_id >> 64) as u64).to_le_bytes());

        let kind_id = stable_u128_from_str(e.kind.as_str());
        out.extend_from_slice(&(kind_id as u64).to_le_bytes());
        out.extend_from_slice(&((kind_id >> 64) as u64).to_le_bytes());

        for v in e.rel_pos {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for v in e.rel_vel {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&opt_i32(e.health).to_le_bytes());
        out.extend_from_slice(&opt_i32(e.health_max).to_le_bytes());
        out.extend_from_slice(&opt_f32(e.radius).to_le_bytes());

        match e.aabb_half_extents {
            Some([x, z]) => {
                out.extend_from_slice(&x.to_le_bytes());
                out.extend_from_slice(&z.to_le_bytes());
            }
            None => {
                out.extend_from_slice(&(-1.0f32).to_le_bytes());
                out.extend_from_slice(&(-1.0f32).to_le_bytes());
            }
        }

        let tag_bits = tags_to_bits(e.tags.iter().map(|s| s.as_str()));
        out.extend_from_slice(&tag_bits.to_le_bytes());

        // Padding for forward compatibility: 4 bytes.
        out.extend_from_slice(&0u32.to_le_bytes());
    }

    out
}

fn decode_out_v1(bytes: &[u8]) -> Result<TickOutput, String> {
    if bytes.len() < 4 {
        return Err("WASM output too small".into());
    }
    let count = u32::from_le_bytes(bytes[0..4].try_into().unwrap_or([0u8; 4])) as usize;
    let mut offset = 4usize;
    let mut commands = Vec::with_capacity(count.min(16));

    const RECORD_BYTES: usize = 32;
    for _ in 0..count {
        if offset + RECORD_BYTES > bytes.len() {
            return Err("WASM output truncated".into());
        }
        let kind = u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap_or([0u8; 4]));
        let payload = &bytes[offset + 4..offset + RECORD_BYTES];
        match kind {
            1 => {
                // MoveTo: f32 x,y,z + u64 valid_until_tick
                let x = f32::from_le_bytes(payload[0..4].try_into().unwrap_or([0u8; 4]));
                let y = f32::from_le_bytes(payload[4..8].try_into().unwrap_or([0u8; 4]));
                let z = f32::from_le_bytes(payload[8..12].try_into().unwrap_or([0u8; 4]));
                let valid = u64::from_le_bytes(payload[12..20].try_into().unwrap_or([0u8; 8]));
                commands.push(BrainCommand::MoveTo {
                    pos: [x, y, z],
                    valid_until_tick: (valid != u64::MAX).then_some(valid),
                });
            }
            2 => {
                // SetMove: f32 x,z + u64 valid_until_tick
                let x = f32::from_le_bytes(payload[0..4].try_into().unwrap_or([0u8; 4]));
                let z = f32::from_le_bytes(payload[4..8].try_into().unwrap_or([0u8; 4]));
                let valid = u64::from_le_bytes(payload[8..16].try_into().unwrap_or([0u8; 8]));
                commands.push(BrainCommand::SetMove {
                    vec2: [x, z],
                    valid_until_tick: (valid != u64::MAX).then_some(valid),
                });
            }
            3 => {
                // AttackTarget: u128 target_id + u64 valid_until_tick
                let lo = u64::from_le_bytes(payload[0..8].try_into().unwrap_or([0u8; 8]));
                let hi = u64::from_le_bytes(payload[8..16].try_into().unwrap_or([0u8; 8]));
                let valid = u64::from_le_bytes(payload[16..24].try_into().unwrap_or([0u8; 8]));
                let target_u128 = (hi as u128) << 64 | (lo as u128);
                let target_id = uuid::Uuid::from_u128(target_u128).to_string();
                commands.push(BrainCommand::AttackTarget {
                    target_id,
                    valid_until_tick: (valid != u64::MAX).then_some(valid),
                });
            }
            5 => {
                // SleepForTicks: u32 ticks
                let ticks = u32::from_le_bytes(payload[0..4].try_into().unwrap_or([0u8; 4]));
                commands.push(BrainCommand::SleepForTicks { ticks });
            }
            0 => {}
            other => return Err(format!("Unknown command kind {other}")),
        }
        offset += RECORD_BYTES;
    }

    Ok(TickOutput {
        commands,
        meta: TickOutputMeta::default(),
    })
}

fn write_guest_memory(
    memory: &Memory,
    store: &mut Store<()>,
    ptr: u32,
    bytes: &[u8],
) -> Result<(), String> {
    let data = memory.data_mut(store.as_context_mut());
    let start = ptr as usize;
    let end = start.saturating_add(bytes.len());
    if end > data.len() {
        return Err("Guest memory out of bounds".into());
    }
    data[start..end].copy_from_slice(bytes);
    Ok(())
}

fn read_guest_memory(
    memory: &Memory,
    store: &mut Store<()>,
    ptr: u32,
    len: u32,
) -> Result<Vec<u8>, String> {
    let data = memory.data(store.as_context());
    let start = ptr as usize;
    let len = len as usize;
    let end = start.saturating_add(len);
    if end > data.len() {
        return Err("Guest memory out of bounds".into());
    }
    Ok(data[start..end].to_vec())
}

fn opt_i32(v: Option<i32>) -> i32 {
    v.unwrap_or(-1)
}

fn opt_f32(v: Option<f32>) -> f32 {
    v.unwrap_or(-1.0)
}

fn tags_to_bits<'a>(tags: impl Iterator<Item = &'a str>) -> u64 {
    let mut bits = 0u64;
    for t in tags {
        match t {
            "unit" => bits |= 1 << 0,
            "build" => bits |= 1 << 1,
            "attack.melee" => bits |= 1 << 2,
            "attack.ranged" => bits |= 1 << 3,
            "enemy" => bits |= 1 << 4,
            "player" => bits |= 1 << 5,
            _ => {}
        }
    }
    bits
}

fn stable_u128_from_str(text: &str) -> u128 {
    let s = text.trim();
    if let Ok(id) = uuid::Uuid::parse_str(s) {
        return id.as_u128();
    }
    let mut h = sha2::Sha256::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    u128::from_le_bytes(digest[0..16].try_into().unwrap_or([0u8; 16]))
}

fn validate_module_id(module_id: &str) -> Result<(), String> {
    let module_id = module_id.trim();
    if module_id.is_empty() {
        return Err("Empty module_id".into());
    }
    if module_id.len() > 128 {
        return Err("module_id too long".into());
    }
    if module_id.contains("..") || module_id.contains('/') || module_id.contains('\\') {
        return Err("Invalid module_id".into());
    }
    if !module_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err("Invalid module_id".into());
    }
    Ok(())
}

fn scan_module_store(module_store_root: &Path) -> Result<Vec<WasmModuleRecord>, String> {
    let entries = match std::fs::read_dir(module_store_root) {
        Ok(v) => v,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("read_dir {}: {err}", module_store_root.display())),
    };

    let mut out = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(folder) = file_name.to_str() else {
            continue;
        };
        if validate_module_id(folder).is_err() {
            continue;
        }

        let meta_path = path.join(METADATA_FILE_NAME);
        let meta = match std::fs::read_to_string(&meta_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let meta: WasmModuleMetadata = match serde_json::from_str(&meta) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if meta.module_id.trim() != folder {
            continue;
        }
        if meta.abi_version != ABI_VERSION_V1 {
            continue;
        }

        // Basic presence checks only; deeper validation happens on load.
        match meta.source_kind {
            WasmSourceKind::WasmOnly => {
                if !path.join(WASM_FILE_NAME).is_file() {
                    continue;
                }
            }
            WasmSourceKind::RustSource => {
                if !path.join(RUST_SOURCE_FILE_NAME).is_file() {
                    continue;
                }
            }
        }

        out.push(WasmModuleRecord {
            module_dir: path,
            meta,
        });
    }
    out.sort_by(|a, b| a.meta.module_id.cmp(&b.meta.module_id));
    Ok(out)
}

fn find_module_record(
    module_store_root: &Path,
    module_id: &str,
) -> Result<WasmModuleRecord, String> {
    let module_id = module_id.trim();
    let records = scan_module_store(module_store_root)?;
    records
        .into_iter()
        .find(|r| r.meta.module_id == module_id)
        .ok_or_else(|| "Module not found".to_string())
}

fn ensure_wasm_artifact(record: &WasmModuleRecord) -> Result<PathBuf, String> {
    match record.meta.source_kind {
        WasmSourceKind::WasmOnly => Ok(record.module_dir.join(WASM_FILE_NAME)),
        WasmSourceKind::RustSource => {
            let src = record.module_dir.join(RUST_SOURCE_FILE_NAME);
            if !src.is_file() {
                return Err(format!("Missing {}", src.display()));
            }
            let out_dir = record.module_dir.join(BUILD_DIR_NAME);
            std::fs::create_dir_all(&out_dir)
                .map_err(|err| format!("create_dir_all {}: {err}", out_dir.display()))?;
            let out_wasm = out_dir.join(BUILD_WASM_FILE_NAME);
            compile_rust_to_wasm(src.as_path(), out_wasm.as_path())?;
            Ok(out_wasm)
        }
    }
}

fn compile_rust_to_wasm(source: &Path, out_wasm: &Path) -> Result<(), String> {
    let Some(rustc) = find_rustc() else {
        return Err("Rust toolchain not found (set GRAVIMERA_RUSTC or bundle a toolchain)".into());
    };

    let output = std::process::Command::new(rustc)
        .arg("--edition=2021")
        .arg("--crate-type=cdylib")
        .arg("--target")
        .arg("wasm32-unknown-unknown")
        .arg("-O")
        .arg("-C")
        .arg("panic=abort")
        .arg("-C")
        .arg("lto=thin")
        .arg("-C")
        .arg("strip=symbols")
        .arg("-o")
        .arg(out_wasm)
        .arg(source)
        .output()
        .map_err(|err| format!("spawn rustc: {err}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("rustc failed: {stderr}"));
    }
    Ok(())
}

fn find_rustc() -> Option<std::ffi::OsString> {
    if let Some(v) = std::env::var_os("GRAVIMERA_RUSTC").filter(|v| !v.is_empty()) {
        return Some(v);
    }

    find_in_path("rustc")
}

fn find_in_path(bin_name: &str) -> Option<std::ffi::OsString> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let mut candidate = dir.join(bin_name);
        if cfg!(windows) {
            candidate.set_extension("exe");
        }
        if candidate.is_file() {
            return Some(candidate.into_os_string());
        }
    }
    None
}

fn load_wasmtime_module(
    engine: &Engine,
    module_id: &str,
    wasm_path: &Path,
) -> Result<Module, String> {
    let meta = std::fs::metadata(wasm_path)
        .map_err(|err| format!("stat {}: {err}", wasm_path.display()))?;
    if meta.len() > MAX_WASM_MODULE_BYTES {
        return Err(format!(
            "WASM module too large: {} bytes (max {})",
            meta.len(),
            MAX_WASM_MODULE_BYTES
        ));
    }
    let bytes =
        std::fs::read(wasm_path).map_err(|err| format!("read {}: {err}", wasm_path.display()))?;
    let module =
        Module::new(engine, bytes).map_err(|err| format!("compile wasm `{module_id}`: {err}"))?;

    validate_wasm_module(module_id, &module)?;
    Ok(module)
}

fn validate_wasm_module(module_id: &str, module: &Module) -> Result<(), String> {
    if module.imports().len() > 0 {
        let imports: Vec<String> = module
            .imports()
            .map(|i| {
                let module = i.module();
                let name = i.name();
                format!("{module}::{name}")
            })
            .collect();
        return Err(format!(
            "WASM module `{module_id}` has imports (obs-only modules must have none): {imports:?}"
        ));
    }

    let mut has_memory = false;
    let mut has_alloc = false;
    let mut has_tick = false;

    for export in module.exports() {
        match export.name() {
            "memory" => {
                has_memory = true;
                if let wasmtime::ExternType::Memory(mem) = export.ty() {
                    let max = mem
                        .maximum()
                        .ok_or_else(|| "WASM memory must declare a maximum".to_string())?;
                    if max > u64::from(MAX_WASM_MEMORY_PAGES) {
                        return Err(format!(
                            "WASM memory maximum too large: {max} pages (max {MAX_WASM_MEMORY_PAGES})"
                        ));
                    }
                } else {
                    return Err("Export `memory` is not a memory".into());
                }
            }
            "brain_alloc_v1" => has_alloc = true,
            "brain_tick_v1" => has_tick = true,
            _ => {}
        }
    }

    if !has_memory {
        return Err("Missing export `memory`".into());
    }
    if !has_alloc {
        return Err("Missing export `brain_alloc_v1`".into());
    }
    if !has_tick {
        return Err("Missing export `brain_tick_v1`".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wasm_only_module(module_store_root: &Path, module_id: &str, wat: &str) {
        let module_dir = module_store_root.join(module_id);
        std::fs::create_dir_all(&module_dir).expect("create module dir");
        std::fs::write(
            module_dir.join(METADATA_FILE_NAME),
            serde_json::to_vec(&serde_json::json!({
                "module_id": module_id,
                "abi_version": 1,
                "source_kind": "wasm_only"
            }))
            .expect("encode module.json"),
        )
        .expect("write module.json");
        let wasm_bytes = wat::parse_str(wat).expect("wat parse");
        std::fs::write(module_dir.join(WASM_FILE_NAME), wasm_bytes).expect("write brain.wasm");
    }

    fn demo_tick_input() -> TickInput {
        TickInput {
            realm_id: "realm".into(),
            scene_id: "scene".into(),
            unit_instance_id: uuid::Uuid::new_v4().to_string(),
            dt_ms: 16,
            tick_index: 123,
            rng_seed: 999,
            self_state: SelfState {
                pos: [1.0, 2.0, 3.0],
                yaw: 0.2,
                vel: [0.0, 0.0, 0.0],
                health: Some(10),
                health_max: Some(10),
                stamina: None,
                kind: uuid::Uuid::new_v4().to_string(),
                tags: vec!["unit".into()],
            },
            nearby_entities: vec![],
            events: vec![],
            capabilities: vec!["brain.move".into()],
            meta: TickInputMeta::default(),
        }
    }

    fn minimal_wat_module_move_to() -> &'static str {
        r#"(module
  (memory (export "memory") 1 1)
  (global $heap (mut i32) (i32.const 1024))

  (func (export "brain_alloc_v1") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (local.get $ptr)
  )

  ;; brain_tick_v1(obs_ptr, obs_len, out_ptr, out_cap) -> bytes_written
  (func (export "brain_tick_v1") (param $obs_ptr i32) (param $obs_len i32) (param $out_ptr i32) (param $out_cap i32) (result i32)
    ;; output: u32 count = 1
    (i32.store (local.get $out_ptr) (i32.const 1))
    ;; record[0].kind = 1 (MoveTo)
    (i32.store offset=4 (local.get $out_ptr) (i32.const 1))
    ;; payload: x,y,z = (10, 0, -10)
    (i32.store offset=8 (local.get $out_ptr) (i32.reinterpret_f32 (f32.const 10)))
    (i32.store offset=12 (local.get $out_ptr) (i32.reinterpret_f32 (f32.const 0)))
    (i32.store offset=16 (local.get $out_ptr) (i32.reinterpret_f32 (f32.const -10)))
    ;; valid_until_tick = u64::MAX (none)
    (i64.store offset=20 (local.get $out_ptr) (i64.const -1))
    ;; bytes_written = 4 + 32
    (i32.const 36)
  )
)"#
    }

    fn minimal_wat_module_infinite_loop() -> &'static str {
        r#"(module
  (memory (export "memory") 1 1)
  (global $heap (mut i32) (i32.const 1024))

  (func (export "brain_alloc_v1") (param $len i32) (result i32)
    (local $ptr i32)
    (local.set $ptr (global.get $heap))
    (global.set $heap (i32.add (global.get $heap) (local.get $len)))
    (local.get $ptr)
  )

  (func (export "brain_tick_v1") (param $obs_ptr i32) (param $obs_len i32) (param $out_ptr i32) (param $out_cap i32) (result i32)
    (loop $l
      br $l
    )
    (i32.const 0)
  )
)"#
    }

    #[test]
    fn wasm_tick_v1_decodes_move_to() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let module_store_root = tmp.path().join("intelligence").join("wasm_modules");
        write_wasm_only_module(
            module_store_root.as_path(),
            "demo.wasm.v1",
            minimal_wat_module_move_to(),
        );

        let mut runtime =
            WasmBrainsRuntime::new_with_module_store_root(module_store_root).expect("runtime");
        runtime.load_module("demo.wasm.v1").expect("load_module");
        let mut instance = runtime
            .spawn_instance("demo.wasm.v1")
            .expect("spawn_instance");

        let out = instance
            .tick(
                &demo_tick_input(),
                BudgetCaps::default(),
                &["brain.move".to_string()].into_iter().collect(),
            )
            .expect("tick");

        assert!(
            matches!(
                out.commands.as_slice(),
                [BrainCommand::MoveTo {
                    pos: [10.0, 0.0, -10.0],
                    valid_until_tick: None
                }]
            ),
            "out={out:?}"
        );
    }

    #[test]
    fn wasm_tick_v1_respects_capabilities() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let module_store_root = tmp.path().join("intelligence").join("wasm_modules");
        write_wasm_only_module(
            module_store_root.as_path(),
            "demo.wasm.v1",
            minimal_wat_module_move_to(),
        );

        let mut runtime =
            WasmBrainsRuntime::new_with_module_store_root(module_store_root).expect("runtime");
        runtime.load_module("demo.wasm.v1").expect("load_module");
        let mut instance = runtime
            .spawn_instance("demo.wasm.v1")
            .expect("spawn_instance");

        let out = instance
            .tick(&demo_tick_input(), BudgetCaps::default(), &HashSet::new())
            .expect("tick");

        assert!(out.commands.is_empty(), "out={out:?}");
    }

    #[test]
    fn wasm_tick_v1_out_of_fuel_errors() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let module_store_root = tmp.path().join("intelligence").join("wasm_modules");
        write_wasm_only_module(
            module_store_root.as_path(),
            "demo.loop.v1",
            minimal_wat_module_infinite_loop(),
        );

        let mut runtime =
            WasmBrainsRuntime::new_with_module_store_root(module_store_root).expect("runtime");
        runtime.load_module("demo.loop.v1").expect("load_module");
        let mut instance = runtime
            .spawn_instance("demo.loop.v1")
            .expect("spawn_instance");

        let err = instance
            .tick(
                &demo_tick_input(),
                BudgetCaps::default(),
                &["brain.move".to_string()].into_iter().collect(),
            )
            .unwrap_err();

        assert!(err.contains("demo.loop.v1"), "err={err}");
    }
}
