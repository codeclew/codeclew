fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    unsafe { std::env::set_var("PROTOC", protoc) };

    let mut config = prost_build::Config::new();
    config.message_attribute(
        ".",
        "#[derive(serde::Serialize, serde::Deserialize)] #[serde(rename_all = \"camelCase\")]",
    );
    config.enum_attribute(
        ".",
        "#[derive(serde::Serialize, serde::Deserialize)] #[serde(rename_all = \"SCREAMING_SNAKE_CASE\")]",
    );
    config
        .compile_protos(&["../../schemas/evidence_core.proto"], &["../../schemas"])
        .expect("compile evidence core protocol");

    println!("cargo:rerun-if-changed=../../schemas/evidence_core.proto");
}
