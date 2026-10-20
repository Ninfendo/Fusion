//! Compiles the Protobuf files into Rust with protox, a Protobuf compiler
//! written in Rust, so no `protoc` has to be installed.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let proto = "proto/fusion/v1/viewserver.proto";
    println!("cargo:rerun-if-changed={proto}");
    let files = protox::compile([proto], ["proto"])?;
    prost_build::Config::new().compile_fds(files)?;
    Ok(())
}
