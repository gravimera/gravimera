use std::path::PathBuf;

fn main() {
    // Avoid requiring developers/CI to have a system `protoc` installed.
    if let Ok(path) = protoc_bin_vendored::protoc_bin_path() {
        std::env::set_var("PROTOC", path);
    }

    let proto_root = PathBuf::from("proto");

    let protos = [
        proto_root.join("gravimera/common/v1/uuid.proto"),
        proto_root.join("gravimera/terrain/v1/terrain.proto"),
        proto_root.join("gravimera/scene/v1/scene.proto"),
    ];

    let proto_includes = [proto_root];

    let mut config = prost_build::Config::new();
    config
        .compile_protos(&protos, &proto_includes)
        .expect("compile protobuf schemas");

    for proto in protos {
        println!("cargo:rerun-if-changed={}", proto.display());
    }
}
