fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Hermetic build: avoid system `protoc` dependency.
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    std::env::set_var("PROTOC", protoc);

    tonic_build::configure()
        .build_server(true)
        .build_client(false)
        .compile(&["../../proto/command.proto"], &["../../proto"])?;
    Ok(())
}
