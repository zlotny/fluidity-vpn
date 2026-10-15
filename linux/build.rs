fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let _ = embed_resource::compile("../windows/fluidity.rc", embed_resource::NONE).manifest_optional();
    }
}
