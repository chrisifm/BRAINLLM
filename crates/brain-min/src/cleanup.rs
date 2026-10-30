//! Typed, literal-free feedback for actual production cleanup operations.
use crate::lex::Tok;
use std::fmt;

pub const MAX_FACTS: usize = 4096;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Removed,
    InitializerRetained,
    SideEffectImportRetained,
    Retained,
}
impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::InitializerRetained => "initializer-retained",
            Self::SideEffectImportRetained => "side-effect-import-retained",
            Self::Retained => "retained",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Variable,
    Import,
    Function,
    Class,
    DebugLog,
    Scope,
}
impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Variable => "variable",
            Self::Import => "import",
            Self::Function => "function",
            Self::Class => "class",
            Self::DebugLog => "debug-log",
            Self::Scope => "scope",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupFact {
    pub action: Action,
    pub kind: Kind,
    /// Safe public identifier only, never a source literal or module specifier.
    pub binding: Option<String>,
    pub start: usize,
    pub end: usize,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub reason: &'static str,
    pub fix: &'static str,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub facts: Vec<CleanupFact>,
    pub removed_bindings: usize,
    pub removed_import_bindings: usize,
    pub retained_effects: usize,
    pub facts_truncated: bool,
    pub omitted_facts: usize,
    line_starts: Option<Vec<usize>>,
}
impl CleanupReport {
    pub(crate) fn record(
        &mut self,
        source: Option<&str>,
        token: &Tok,
        action: Action,
        kind: Kind,
        reason: &'static str,
        fix: &'static str,
    ) -> Result<(), CleanupError> {
        if action != Action::Retained && !matches!(kind, Kind::DebugLog | Kind::Scope) {
            self.removed_bindings += 1;
            if kind == Kind::Import {
                self.removed_import_bindings += 1;
            }
        }
        if matches!(action, Action::InitializerRetained | Action::SideEffectImportRetained) {
            self.retained_effects += 1;
        }
        if self.facts.len() == MAX_FACTS {
            self.facts_truncated = true;
            self.omitted_facts += 1;
            return Ok(());
        }
        let (line, column) = if let Some(src) = source.filter(|src| token.start <= src.len() && src.is_char_boundary(token.start)) {
            let lines = self
                .line_starts
                .get_or_insert_with(|| std::iter::once(0).chain(src.bytes().enumerate().filter_map(|(i, b)| (b == b'\n').then_some(i + 1))).collect());
            let index = lines.partition_point(|&n| n <= token.start).saturating_sub(1);
            (Some(index + 1), Some(token.start - lines[index] + 1))
        } else {
            (None, None)
        };
        let safe = token.text.len() <= 80
            && token.text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'$')
            && !["sk_", "sk-", "ghp_", "github_pat_", "AKIA", "AIza", "xoxb_"].iter().any(|p| token.text.starts_with(p));
        let binding = (safe && !matches!(kind, Kind::DebugLog | Kind::Scope)).then(|| token.text.clone());
        self.facts.push(CleanupFact { action, kind, binding, start: token.start, end: token.start + token.text.len(), line, column, reason, fix });
        Ok(())
    }
    pub(crate) fn append(&mut self, other: Self) -> Result<(), CleanupError> {
        self.removed_bindings += other.removed_bindings;
        self.removed_import_bindings += other.removed_import_bindings;
        self.retained_effects += other.retained_effects;
        let room = MAX_FACTS.saturating_sub(self.facts.len());
        self.omitted_facts += other.omitted_facts + other.facts.len().saturating_sub(room);
        self.facts_truncated |= other.facts_truncated || self.omitted_facts > 0;
        self.facts.extend(other.facts.into_iter().take(room));
        Ok(())
    }
    /// Deterministic cache/CLI JSON, without source bodies, literal values or runtime logs.
    pub fn to_json(&self) -> String {
        let facts = self
            .facts
            .iter()
            .map(|f| {
                format!(
                    "{{\"action\":{},\"kind\":{},\"binding\":{},\"start\":{},\"end\":{},\"line\":{},\"column\":{},\"reason\":{},\"fix\":{}}}",
                    quote(f.action.as_str()),
                    quote(f.kind.as_str()),
                    f.binding.as_deref().map(quote).unwrap_or_else(|| "null".into()),
                    f.start,
                    f.end,
                    num(f.line),
                    num(f.column),
                    quote(f.reason),
                    quote(f.fix)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"schemaVersion\":1,\"coordinateSpace\":\"input-js\",\"columnUnit\":\"UTF-8 bytes, 1-based\",\"precision\":\"input-token; original source mapping not inferred\",\"removedBindings\":{},\"removedImportBindings\":{},\"retainedEffects\":{},\"factsTruncated\":{},\"omittedFacts\":{},\"facts\":[{}]}}",
            self.removed_bindings, self.removed_import_bindings, self.retained_effects, self.facts_truncated, self.omitted_facts, facts
        )
    }
}
#[derive(Clone, Debug)]
pub struct Minified {
    pub code: String,
    pub report: CleanupReport,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupError {
    pub code: &'static str,
    pub stage: &'static str,
    pub offset: Option<usize>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub reason: &'static str,
    pub fix: &'static str,
}
impl CleanupError {
    pub(crate) fn new(code: &'static str, stage: &'static str, source: Option<&str>, offset: Option<usize>, reason: &'static str, fix: &'static str) -> Self {
        let (line, column) = offset.map(|n| position(source, n)).unwrap_or((None, None));
        Self { code, stage, offset, line, column, reason, fix }
    }
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schemaVersion\":1,\"code\":{},\"stage\":{},\"coordinateSpace\":\"input-js\",\"offset\":{},\"line\":{},\"column\":{},\"reason\":{},\"fix\":{}}}",
            quote(self.code),
            quote(self.stage),
            num(self.offset),
            num(self.line),
            num(self.column),
            quote(self.reason),
            quote(self.fix)
        )
    }
}
impl fmt::Display for CleanupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} [{}]{}: {}; fix: {}",
            self.code,
            self.stage,
            self.line.map(|n| format!(" at input {n}:{}", self.column.unwrap_or(1))).unwrap_or_default(),
            self.reason,
            self.fix
        )
    }
}
impl std::error::Error for CleanupError {}
fn num(value: Option<usize>) -> String {
    value.map(|n| n.to_string()).unwrap_or_else(|| "null".into())
}
fn position(source: Option<&str>, offset: usize) -> (Option<usize>, Option<usize>) {
    let Some(source) = source.filter(|s| offset <= s.len() && s.is_char_boundary(offset)) else { return (None, None) };
    let before = &source[..offset];
    let line = before.bytes().filter(|b| *b == b'\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or("").len() + 1;
    (Some(line), Some(column))
}
fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
