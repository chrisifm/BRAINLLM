//! Library callers must not bypass the portable-output invariant by omitting prerender or enabling gateways.
use brain_pack::{BuildOptions, Input};
#[test]
fn portable_policy_requires_prerender_and_refuses_gateway_features_before_compiling() {
    let compile = |_: &str, _: &std::path::Path| panic!("policy must fail before compilation");
    let input = Input {
        root: std::path::PathBuf::new(), compile: &compile, runtime_core: String::new(),
        runtime_siblings: vec![], layer_order: String::new(), minify: true, site: None,
        shim: None, generator: String::new(), server_module: None, cache: Default::default(),
        server_mjs: None, progress: None, post: None,
    };
    let options = BuildOptions { static_export: true, ..Default::default() };
    let error = brain_pack::build_with_options(&input, &[], &|_| vec![], &|_| vec![], &options).err().expect("no shim must fail");
    assert!(error.contains("prerender DOM shim"), "{error}");
    for options in [
        BuildOptions { static_export: true, native_sockets: Some(brain_pack::NativeSocketOptions::new("https://example.com").unwrap()), ..Default::default() },
        BuildOptions { static_export: true, native_routes: vec![("/endpoint".into(), vec![])], ..Default::default() },
        BuildOptions { static_export: true, trust_proxy: vec!["127.0.0.1".into()], ..Default::default() },
        BuildOptions { static_export: true, access: Some("{}".into()), ..Default::default() },
    ] {
        let error = brain_pack::build_with_options(&input, &[], &|_| vec![], &|_| vec![], &options).err().expect("gateway must fail");
        assert!(error.contains("cannot enforce"), "{error}");
    }
}
