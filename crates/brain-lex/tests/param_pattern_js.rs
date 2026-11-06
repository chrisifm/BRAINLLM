//! The trusted JS emitter must implement the native segment evaluator exactly.
use brain_lex::{Json, param_pattern::Pattern};
use std::{
    io::Write,
    process::{Command, Stdio},
};
#[test]
fn native_and_javascript_segment_matching_agree() {
    let mut rows = Vec::new();
    let alphabet = b"abxyz23709_-.%ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut seed = 29u64;
    for source in [
        "[2-9a-z]{7}",
        "[a-z]{1,8}-[0-9]{2,4}",
        "a{2}a{2}",
        "[a-z]{1,3}[0-9]{0,2}-",
        "[a-z]{256}",
        "\\.[a-z]{1,4}",
        "[a-z]{1,3}\\.[0-9]{1,2}",
        "[0-9]{0,3}a",
    ] {
        let pattern = Pattern::parse(source).unwrap();
        let mut samples = vec![
            "2abcdef".into(),
            "%32abcdef".into(),
            "a-23".into(),
            "abc23-".into(),
            ".abc".into(),
            "abc.23".into(),
            "aaaa".into(),
            "23a".into(),
            "".into(),
            "a".repeat(256),
            "a".repeat(257),
            "\u{e9}".into(),
            "a/b".into(),
            "abcdefg\n".into(),
            "abcdefg\r\n".into(),
            "23a\n".into(),
            "\t".into(),
            "\u{0000}".into(),
            "ab\\c".into(),
        ];
        for len in 0..48 {
            for _ in 0..12 {
                let mut text = String::new();
                for _ in 0..len {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    text.push(alphabet[(seed as usize >> 24) % alphabet.len()] as char);
                }
                samples.push(text);
            }
        }
        let cases = samples
            .iter()
            .map(|value| Json::Arr(vec![Json::str(value), Json::Bool(pattern.matches(value))]))
            .collect();
        rows.push(Json::obj([
            (
                "rx",
                Json::str(&format!("^(?:{})$", pattern.regex_source())),
            ),
            ("cases", Json::Arr(cases)),
        ]));
    }
    let input = Json::Arr(rows).stringify();
    let mut node=Command::new("node").args(["--input-type=module","-e","import assert from 'node:assert/strict';let input='';for await(const chunk of process.stdin)input+=chunk;for(const row of JSON.parse(input)){const rx=new RegExp(row.rx);for(const [value,want]of row.cases)assert.equal(rx.test(value),want,JSON.stringify({rx:row.rx,value}));}"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("Node is required for runtime equivalence");
    node.stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let out = node.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
