fn main() {
    println!("cargo:rerun-if-changed=assets/vanta.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/vanta.ico")
            .compile()
            .expect("could not embed the Vanta Windows executable icon");
    }
}
