//! `LineIndex` columns: exact (chars, not bytes) and not quadratic on long lines.

use brain_lex::LineIndex;
use std::time::Duration;

fn naive(src: &str, offset: usize) -> (u32, u32) {
    let start = src[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = src[..offset].matches('\n').count();
    (line as u32 + 1, src[start..offset].chars().count() as u32 + 1)
}

#[test]
fn columns_count_chars_on_mixed_utf8() {
    let src = "a\u{e9}b\n\u{1f600}x\u{4e2d}\u{6587}\n\nlast \u{fc}ber";
    let idx = LineIndex::new(src);
    for offset in (0..=src.len()).filter(|&o| src.is_char_boundary(o)) {
        assert_eq!(idx.line_col(src, offset), naive(src, offset), "offset {offset}");
    }
}

#[test]
fn spans_on_one_long_line_are_not_quadratic() {
    // 4 MB on a single line (10 bytes, 9 chars per unit), a span at every unit: counting chars from the line
    // start each time is ~800 GB of work; the index answers each in O(log n).
    let units = 400_000;
    let line: String = "\u{e9}abcdefg,".repeat(units);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let idx = LineIndex::new(&line);
        let mut last = 0;
        for offset in (0..line.len()).step_by(10) {
            last = idx.span(&line, offset, offset).col;
        }
        tx.send(last).unwrap();
    });
    let last = rx.recv_timeout(Duration::from_secs(20)).expect("LineIndex::span is quadratic on a long line");
    assert_eq!(last as usize, 9 * (units - 1) + 1);
}
