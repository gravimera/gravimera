fn main() {
    println!(
        "wgpu enabled backends: {:?}",
        wgpu::Instance::enabled_backend_features()
    );
    let instance = wgpu::Instance::default();
    let adapters = instance.enumerate_adapters(wgpu::Backends::all());
    println!("adapters(all): {}", adapters.len());
    for (idx, adapter) in adapters.iter().enumerate() {
        let info = adapter.get_info();
        println!(
            "{idx}: {:?} | {:?} | {} | {:?}",
            info.backend, info.device_type, info.name, info.driver
        );
    }
    let metal_adapters = instance.enumerate_adapters(wgpu::Backends::METAL);
    println!("adapters(metal): {}", metal_adapters.len());
    for (idx, adapter) in metal_adapters.iter().enumerate() {
        let info = adapter.get_info();
        println!(
            "metal {idx}: {:?} | {:?} | {} | {:?}",
            info.backend, info.device_type, info.name, info.driver
        );
    }
}
