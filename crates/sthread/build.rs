fn main() {
    let protoc = protoc_bin_vendored::protoc_bin_path().expect("vendored protoc");
    unsafe { std::env::set_var("PROTOC", protoc) };
    prost_build::compile_protos(&["../../schemas/worker.proto"], &["../../schemas"])
        .expect("compile worker protocol");
}
