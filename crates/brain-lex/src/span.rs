/// Byte range in the source file plus the 1-based line/column of its start.
///
/// The file name is not repeated in every span: it lives once in `Component::file`
/// (and in every `Diag`), which keeps the AST small.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub col: u32,
}

impl Span {
    /// Span covering `self` up to the end of `other`.
    pub fn to(self, other: Span) -> Span {
        Span { end: other.end.max(self.end), ..self }
    }
}

/// Maps byte offsets to 1-based line and column (columns count chars, not bytes).
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
    /// Offsets of UTF-8 continuation bytes, so a column is a subtraction plus a binary search instead of
    /// counting every char from the line start (which made long single-line files quadratic).
    continuation: Vec<u32>,
    len: usize,
}

impl LineIndex {
    pub fn new(src: &str) -> Self {
        let mut starts = vec![0];
        let mut continuation = Vec::new();
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            } else if b & 0xC0 == 0x80 {
                continuation.push(i as u32);
            }
        }
        LineIndex { starts, continuation, len: src.len() }
    }

    pub fn line_col(&self, src: &str, offset: usize) -> (u32, u32) {
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.starts[line];
        let end = offset.min(src.len());
        let col = if src.len() == self.len && start <= end && src.is_char_boundary(start) && src.is_char_boundary(end) {
            let below = |at: usize| self.continuation.partition_point(|&c| (c as usize) < at);
            end - start - (below(end) - below(start))
        } else {
            src.get(start..end).map(|s| s.chars().count()).unwrap_or(end - start)
        };
        (line as u32 + 1, col as u32 + 1)
    }

    pub fn span(&self, src: &str, start: usize, end: usize) -> Span {
        let (line, col) = self.line_col(src, start);
        Span { start: start as u32, end: end as u32, line, col }
    }
}
