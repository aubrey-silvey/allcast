fn main() {
    // Use a vendored protoc so the build needs no system protobuf compiler
    // (the headless Pi receiver builds this crate too).
    if let Ok(protoc) = protoc_bin_vendored::protoc_bin_path() {
        std::env::set_var("PROTOC", protoc);
    }
    tonic_build::compile_protos("proto/telemetry.proto")
        .expect("failed to compile proto/telemetry.proto");
    println!("cargo:rerun-if-changed=proto/telemetry.proto");
}
