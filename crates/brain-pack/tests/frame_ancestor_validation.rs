//! Every framing source is a complete origin, including the local HTTP exception.
use brain_pack::{frame_ancestor_problem, read_frame_ancestors};

#[test]
fn local_frame_ancestors_reject_paths_ports_and_policy_delimiters() {
    for source in [
        "http://localhost/", "http://localhost/path", "http://localhost:80/path",
        "http://localhost:bad", "http://localhost:", "http://localhost:65536",
        "http://localhost:80?x=1", "http://localhost:80#fragment",
        "http://localhost:80; script-src 'unsafe-inline'", "http://localhost:80\r\nX-Test: value",
        "http://127.0.0.1:80\"", "http://intranet:80'", "http://localhost:*suffix",
        "http://localhost:*; script-src 'unsafe-inline'", "http://localhost:*/path",
        "http://intranet:80; script-src 'unsafe-inline'",
        "http://intranet:*", "http://*.example.com", "http://example.com",
    ] {
        assert!(frame_ancestor_problem(source).is_some(), "accepted {source:?}");
    }
}

#[test]
fn exact_framing_origins_and_existing_loopback_wildcard_ports_remain_valid() {
    for source in [
        "'self'", "'none'", "https://embed.example.com", "https://embed.example.com:8443",
        "http://localhost", "http://localhost:8080", "http://localhost:*",
        "http://127.0.0.1", "http://127.0.0.1:*", "http://intranet:8080", "http://INTRANET:8080",
    ] {
        assert_eq!(frame_ancestor_problem(source), None, "{source}");
    }
}

#[test]
fn invalid_local_sources_are_reported_and_excluded_from_the_policy() {
    struct Scratch(std::path::PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    let root = Scratch(std::env::temp_dir().join(format!("brain-frame-origin-{}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    std::fs::write(root.0.join("brain.json"), r#"{"csp":{"frameAncestors":["'self'","http://localhost/path","http://localhost:bad","http://localhost:80; script-src 'unsafe-inline'","http://localhost:8080"]}}"#).unwrap();
    let (sources, problems) = read_frame_ancestors(&root.0);
    assert_eq!(sources.unwrap(), ["'self'", "http://localhost:8080"]);
    assert_eq!(problems.len(), 3);
    assert!(problems.iter().all(|p| p.contains("csp.frameAncestors") && p.contains("fix:")));
    std::fs::write(root.0.join("brain.json"), r#"{"csp":{"frameAncestors":["http://localhost/path"]}}"#).unwrap();
    let (sources, problems) = read_frame_ancestors(&root.0);
    assert_eq!(sources, None, "an entirely invalid list retains the default deny policy");
    assert_eq!(problems.len(), 1);
}
