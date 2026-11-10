fn main() {
    println!("cargo:rerun-if-changed=assets/logo_app.png");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let source = image::open("assets/logo_app.png").expect("Не удалось прочитать логотип");
    let sizes = [16, 24, 32, 48, 64, 128, 256];
    let images: Vec<_> = sizes
        .iter()
        .map(|&size| {
            source
                .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                .to_rgba8()
        })
        .collect();
    let frames: Vec<_> = images
        .iter()
        .zip(sizes)
        .map(|(image, size)| {
            image::codecs::ico::IcoFrame::as_png(
                image.as_raw(),
                size,
                size,
                image::ExtendedColorType::Rgba8,
            )
            .expect("Не удалось подготовить значок")
        })
        .collect();
    let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"))
        .join("rustterm.ico");
    let file = std::fs::File::create(&path).expect("Не удалось создать значок");
    image::codecs::ico::IcoEncoder::new(file)
        .encode_images(&frames)
        .expect("Не удалось записать значок");
    let mut resources = winresource::WindowsResource::new();
    resources
        .set("ProductName", "RustTerm")
        .set("FileDescription", "RustTerm")
        .set_icon(
            path.to_str()
                .expect("Путь значка содержит неподдерживаемые символы"),
        );
    resources
        .compile()
        .expect("Не удалось встроить значок Windows");
}
