fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    unsafe { std::env::set_var("PROTOC", protoc) };
    let mut config = prost_build::Config::new();
    config.enum_attribute(".", "#[allow(clippy::large_enum_variant)]");
    config
        .compile_protos(
            &[
                "../../schemas/worker.proto",
                "../../schemas/thread_ir.proto",
                "../../schemas/semantic_facts.proto",
                "../../schemas/local_cfg.proto",
                "../../schemas/edit_ir.proto",
                "../../schemas/transaction.proto",
            ],
            &["../../schemas"],
        )
        .expect("compile worker protocol");
}
