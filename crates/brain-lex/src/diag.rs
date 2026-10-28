use crate::{LineIndex, Span};
use std::fmt;

/// A compiler diagnostic in the SPEC §5 shape. `fix` is the exact change the LLM should apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diag {
    pub code: &'static str,
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub msg: String,
    pub fix: String,
    /// Exact text edits that apply `fix` (empty when the fix needs judgment). `brain fix` applies them.
    pub edits: Vec<Edit>,
    pub level: Severity,
}

/// INTERNALS §20/§21: errors block; warnings (`warning`) and `info` do not. An `info` with edits is a deterministic
/// normalization the compiler already applied on its own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Severity {
    #[default]
    Error,
    Warning,
    Info,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// Replace the source between `start` and `end` (byte offsets, end exclusive) with `text`. Line/column
/// (1-based, columns in chars, end exclusive) are filled by [`Diag::locate`] for the JSON.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub start: u32,
    pub end: u32,
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub text: String,
}

impl Diag {
    pub fn new(code: &'static str, file: &str, span: Span, msg: impl Into<String>, fix: impl Into<String>) -> Self {
        let level = crate::codes::default_level(code);
        Diag { code, file: file.to_string(), line: span.line, col: span.col, msg: msg.into(), fix: fix.into(), edits: Vec::new(), level }
    }

    /// Marks the diagnostic as a normalization (`info`): it never blocks and its edits are applied automatically.
    pub fn info(mut self) -> Self {
        self.level = Severity::Info;
        self
    }

    pub fn is_info(&self) -> bool {
        self.level == Severity::Info
    }

    /// Adds an edit over the byte range `start..end` (an insertion when they are equal).
    pub fn edit(mut self, start: u32, end: u32, text: impl Into<String>) -> Self {
        self.edits.push(Edit { start, end, text: text.into(), ..Edit::default() });
        self
    }

    /// Fills line/column of every edit from the source the offsets point into.
    pub fn locate(&mut self, src: &str) {
        if self.edits.is_empty() {
            return;
        }
        let idx = LineIndex::new(src);
        for e in &mut self.edits {
            (e.line, e.col) = idx.line_col(src, e.start as usize);
            (e.end_line, e.end_col) = idx.line_col(src, e.end as usize);
        }
    }

    /// Applies the edits to `src` (from the last to the first). `None` if they overlap or fall outside.
    pub fn apply(src: &str, edits: &[Edit]) -> Option<String> {
        let mut v: Vec<&Edit> = edits.iter().collect();
        v.sort_by_key(|e| (e.start, e.end));
        for w in v.windows(2) {
            if w[0].end > w[1].start {
                return None;
            }
        }
        let mut out = src.to_string();
        for e in v.iter().rev() {
            let (a, b) = (e.start as usize, e.end as usize);
            if a > b || b > out.len() || !out.is_char_boundary(a) || !out.is_char_boundary(b) {
                return None;
            }
            out.replace_range(a..b, &e.text);
        }
        Some(out)
    }

    /// Non-blocking: warnings (`warning`, e.g. B0201) and `info` normalizations never stop compilation.
    pub fn is_warning(&self) -> bool {
        self.level != Severity::Error
    }

    /// One JSON object, written by hand (the workspace has no serde).
    pub fn to_json(&self) -> String {
        let mut s = String::with_capacity(96 + self.msg.len() + self.fix.len());
        s.push_str("{\"code\":");
        push_json_str(&mut s, self.code);
        s.push_str(",\"file\":");
        push_json_str(&mut s, &self.file);
        s.push_str(&format!(",\"line\":{},\"col\":{},\"msg\":", self.line, self.col));
        push_json_str(&mut s, &self.msg);
        s.push_str(",\"fix\":");
        push_json_str(&mut s, &self.fix);
        s.push_str(",\"level\":");
        push_json_str(&mut s, self.level.as_str());
        s.push_str(",\"edits\":[");
        for (i, e) in self.edits.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("{{\"line\":{},\"col\":{},\"endLine\":{},\"endCol\":{},\"text\":", e.line, e.col, e.end_line, e.end_col));
            push_json_str(&mut s, &e.text);
            s.push('}');
        }
        s.push_str("]}");
        s
    }

    /// `[{...},{...}]` for `brain check --json`.
    pub fn list_to_json(diags: &[Diag]) -> String {
        let items: Vec<String> = diags.iter().map(Diag::to_json).collect();
        format!("[{}]", items.join(","))
    }
}

impl fmt::Display for Diag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{} {} {}", self.file, self.line, self.col, self.code, self.msg)?;
        if !self.fix.is_empty() {
            write!(f, "\n  fix: {}", self.fix)?;
        }
        Ok(())
    }
}

/// Codes that are warnings instead of errors.

pub fn push_json_str(out: &mut String, v: &str) {
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_locate_serialize_and_apply() {
        let src = "<script>\nvar n = 1\n</script>";
        let mut d = Diag::new("B0103", "c.brain", Span::default(), "m", "f").edit(9, 12, "let");
        d.locate(src);
        assert_eq!((d.edits[0].line, d.edits[0].col, d.edits[0].end_line, d.edits[0].end_col), (2, 1, 2, 4));
        assert!(d.to_json().ends_with(r#""edits":[{"line":2,"col":1,"endLine":2,"endCol":4,"text":"let"}]}"#), "{}", d.to_json());
        assert_eq!(Diag::apply(src, &d.edits).unwrap(), "<script>\nlet n = 1\n</script>");
    }

    #[test]
    fn json_escapes_quotes_backslashes_and_controls() {
        let d = Diag {
            code: "B0102",
            file: "comp/c.brain".into(),
            line: 7,
            col: 12,
            msg: "`this` \"does\" \\ not exist\n\u{1}".into(),
            fix: "use `n`".into(),
            edits: Vec::new(),
            level: Severity::Error,
        };
        assert_eq!(
            d.to_json(),
            r#"{"code":"B0102","file":"comp/c.brain","line":7,"col":12,"msg":"`this` \"does\" \\ not exist\n\u0001","fix":"use `n`","level":"error","edits":[]}"#
        );
        assert_eq!(Severity::Warning.as_str(), "warning");
    }
}
