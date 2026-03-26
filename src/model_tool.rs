pub(crate) fn run(args: Vec<String>) -> ! {
    if args.is_empty() || matches!(args[0].as_str(), "help" | "--help" | "-h") {
        print_help();
        std::process::exit(0);
    }

    eprintln!(
        "Gravimera model-tool: this utility is temporarily unavailable while the object/scene \
         format is being refactored.\n\n\
         Run: `cargo run -- --help` for game options."
    );
    std::process::exit(2);
}

fn print_help() {
    println!(
        "Gravimera model-tool\n\n\
Usage:\n\
  cargo run -- model-tool help\n\n\
Notes:\n\
  - The previous `type.dat` tooling has been removed as part of the prefab-based object system.\n\
  - A new scene/object import/export tool will be added after the new `scene.grav` format lands.\n"
    );
}
