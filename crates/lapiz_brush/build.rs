use lapiz_image::image;
use wesl::{PkgBuilder, Wesl};

fn main() {
    println!("cargo:rerun-if-changed=shaders");

    let mut compiler = Wesl::new("shaders");
    compiler.add_package(&image::PACKAGE);

    compiler.build_artifact(
        &"package::compose_stroke_preview".parse().unwrap(),
        "compose_stroke_preview",
    );

    PkgBuilder::new("brush")
        .scan_root("shaders")
        .unwrap()
        .validate()
        .inspect_err(|e| panic!("{}", e))
        .unwrap()
        .build_artifact()
        .unwrap();
}
