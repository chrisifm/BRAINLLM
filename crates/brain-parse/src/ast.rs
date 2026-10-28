//! The `.brain` AST — contract COMPILER → TOOLCHAIN (SPEC §4).
//!
//! Every node carries a `Span` (byte range + 1-based line/col). The file path is stored once in
//! `Component::file` instead of in every span.

pub use brain_css::CssSheet;
pub use brain_lex::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    /// PascalCase of the file stem: `user-card.brain` → `UserCard`.
    pub name: String,
    pub file: String,
    pub kind: FileKind,
    pub level: Level,
    pub props: Vec<Prop>,
    /// `<meta seo ...>` / `<meta export ...>` and any other `<meta>` attribute besides `props`.
    pub meta: Vec<MetaAttr>,
    pub script: Vec<Stmt>,
    pub template: Vec<Node>,
    /// Children of a view's top-level `<head>` block (§32): merged into the document head, never rendered in place.
    pub head: Vec<Node>,
    pub style: Option<CssSheet>,
    pub span: Span,
    /// The source text (spans index into it; the checker resolves `Diag::edits` against it).
    pub src: String,
}

/// What a source file is (SPEC §10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// `.brain` component or view.
    Component,
    /// `app.brain`: document shell (`<html>`, `<head>`, layout, `<slot />`).
    App,
    /// `.jsbrain`: BrainScript only, no markup.
    Module,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// `ui/`
    Micro,
    /// `comp/` (and anything outside the known folders)
    Comp,
    /// `views/`
    View,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetaAttr {
    pub name: String,
    pub value: Option<String>,
    /// `<meta seo title={props.post.title}>`: an expression over the `load` props (only in `<meta seo>`, §32).
    pub expr: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Prop {
    pub name: String,
    pub ty: PropType,
    /// `name:type?`
    pub optional: bool,
    pub default: Option<Expr>,
    /// `name:type~`: a model prop the child may write (the parent binds it with `bind:name`).
    pub bindable: bool,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropType {
    Str,
    Num,
    Bool,
    List,
    Obj,
    Fn,
    Any,
}

impl PropType {
    pub fn parse(s: &str) -> Option<PropType> {
        Some(match s {
            "str" => PropType::Str,
            "num" => PropType::Num,
            "bool" => PropType::Bool,
            "list" => PropType::List,
            "obj" => PropType::Obj,
            "fn" => PropType::Fn,
            "any" => PropType::Any,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            PropType::Str => "str",
            PropType::Num => "num",
            PropType::Bool => "bool",
            PropType::List => "list",
            PropType::Obj => "obj",
            PropType::Fn => "fn",
            PropType::Any => "any",
        }
    }
}

// ───────────────────────────── Markup ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(Text),
    Interp(Interp),
    If(IfBlock),
    Each(EachBlock),
    Await(AwaitBlock),
    ComponentRef(ComponentRef),
    Slot(Slot),
    /// `{#slot item, i}…{/slot}` inside `<Comp>`: the default slot reads what the child passes with `<slot item={x} />`.
    SlotScope(SlotScope),
}

impl Node {
    pub fn span(&self) -> Span {
        match self {
            Node::Element(n) => n.span,
            Node::Text(n) => n.span,
            Node::Interp(n) => n.span,
            Node::If(n) => n.span,
            Node::Each(n) => n.span,
            Node::Await(n) => n.span,
            Node::ComponentRef(n) => n.span,
            Node::Slot(n) => n.span,
            Node::SlotScope(n) => n.span,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
    pub span: Span,
}

/// `<UserCard user={u} on:select={pick}>children</UserCard>` — tag starts with an uppercase letter.
#[derive(Clone, Debug, PartialEq)]
pub struct ComponentRef {
    pub name: String,
    pub attrs: Vec<Attr>,
    pub children: Vec<Node>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    /// `<slot name="footer" />`; `None` = the default slot.
    pub name: Option<String>,
    /// Slot props: `<slot item={x} />` (every attribute but `name`), read by the parent with `{#slot item}`.
    pub attrs: Vec<Attr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SlotScope {
    pub names: Vec<Ident>,
    pub body: Vec<Node>,
    pub span: Span,
}

/// Raw source text (entities untouched).
#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub value: String,
    pub span: Span,
}

/// `{expr}`, or `{@html expr}` when `html` (raw HTML; the checker requires `sanitize(…)`, B0210).
#[derive(Clone, Debug, PartialEq)]
pub struct Interp {
    pub expr: Expr,
    pub html: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attr {
    pub kind: AttrKind,
    /// Name without the directive prefix: `on:click` → `click`, `class:hot` → `hot`.
    pub name: String,
    pub value: AttrValue,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrKind {
    /// `name="x"`, `name={expr}` or bare `name`
    Plain,
    /// `on:name={…}`
    On,
    /// `bind:name={sig}`
    Bind,
    /// `class:name={bool}`
    Class,
    /// `style:prop={v}`
    Style,
    /// `transition:fade|slide|scale` or `transition:fade={{ ms: 200 }}` (§41.6), on elements inside `{#if}`/`{#each}`.
    Transition,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrValue {
    /// Bare attribute: `disabled`
    None,
    /// `"text"` without interpolations
    Text(String),
    /// `{expr}`
    Expr(Expr),
    /// `"a {b} c"`
    Mixed(Vec<AttrPart>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrPart {
    Text(String),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfBlock {
    /// `{#if a}` then every `{:else if b}`, in order.
    pub branches: Vec<IfBranch>,
    pub otherwise: Option<Vec<Node>>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfBranch {
    pub test: Expr,
    pub body: Vec<Node>,
    pub span: Span,
}

/// `{#each list as item, i (key)}…{:empty}…{/each}`
#[derive(Clone, Debug, PartialEq)]
pub struct EachBlock {
    pub list: Expr,
    pub item: Pat,
    pub index: Option<Ident>,
    /// Required by the checker (B0202); `None` only so the parser can report it with a fix.
    pub key: Option<Expr>,
    pub body: Vec<Node>,
    pub empty: Option<Vec<Node>>,
    pub span: Span,
}

/// `{#await p}…{:then v}…{:catch e}…{/await}`
#[derive(Clone, Debug, PartialEq)]
pub struct AwaitBlock {
    pub promise: Expr,
    pub pending: Vec<Node>,
    pub then: Option<AwaitBranch>,
    pub catch: Option<AwaitBranch>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AwaitBranch {
    pub binding: Option<Pat>,
    pub body: Vec<Node>,
    pub span: Span,
}

// ─────────────────────────── BrainScript ───────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    Let,
    Const,
    /// Parsed only to report B0103.
    Var,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stmt {
    Var(VarDecl),
    Expr { expr: Expr, span: Span },
    If { test: Expr, cons: Box<Stmt>, alt: Option<Box<Stmt>>, span: Span },
    ForOf { kind: VarKind, pat: Pat, iter: Expr, body: Box<Stmt>, span: Span },
    Return { arg: Option<Expr>, span: Span },
    Block { body: Vec<Stmt>, span: Span },
    Func(Func),
    Try { block: Vec<Stmt>, param: Option<Pat>, handler: Option<Vec<Stmt>>, finalizer: Option<Vec<Stmt>>, span: Span },
    Throw { arg: Expr, span: Span },
    Break { span: Span },
    Continue { span: Span },
    Import(Import),
    /// `export <decl>`; `load` in views is `Export(Func)`.
    Export { decl: Box<Stmt>, span: Span },
    /// `export default <expr>` — only valid in `.jsbrain` modules.
    ExportDefault { expr: Expr, span: Span },
    Empty { span: Span },
    /// `server: async function f(ctx, …) {}` — runs only on the server; the client gets an RPC stub (SPEC §11.1).
    Server { func: Func, span: Span },
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Var(v) => v.span,
            Stmt::Func(f) => f.span,
            Stmt::Import(i) => i.span,
            Stmt::Expr { span, .. }
            | Stmt::If { span, .. }
            | Stmt::ForOf { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Block { span, .. }
            | Stmt::Try { span, .. }
            | Stmt::Throw { span, .. }
            | Stmt::Break { span }
            | Stmt::Continue { span }
            | Stmt::Export { span, .. }
            | Stmt::ExportDefault { span, .. }
            | Stmt::Server { span, .. }
            | Stmt::Empty { span } => *span,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarDecl {
    pub kind: VarKind,
    pub decls: Vec<Declarator>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Declarator {
    pub pat: Pat,
    pub init: Option<Expr>,
    pub span: Span,
}

/// `import Default, { a, b as c } from './x.brain'` / `import * as ns from './x.js'`
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub default: Option<Ident>,
    pub namespace: Option<Ident>,
    /// (imported, local)
    pub named: Vec<(Ident, Ident)>,
    pub source: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Func {
    pub name: Option<Ident>,
    pub params: Vec<Pat>,
    pub body: FnBody,
    pub is_arrow: bool,
    pub is_async: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FnBody {
    Expr(Box<Expr>),
    Block(Vec<Stmt>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pat {
    Ident(Ident),
    Object { props: Vec<PatProp>, rest: Option<Box<Pat>>, span: Span },
    Array { elems: Vec<Option<Pat>>, rest: Option<Box<Pat>>, span: Span },
    Default { pat: Box<Pat>, default: Box<Expr>, span: Span },
    /// `...rest` as the last function parameter.
    Rest { arg: Box<Pat>, span: Span },
}

impl Pat {
    pub fn span(&self) -> Span {
        match self {
            Pat::Ident(i) => i.span,
            Pat::Object { span, .. } | Pat::Array { span, .. } | Pat::Default { span, .. } | Pat::Rest { span, .. } => *span,
        }
    }
    /// Every identifier bound by this pattern.
    pub fn bound_names<'a>(&'a self, out: &mut Vec<&'a Ident>) {
        match self {
            Pat::Ident(i) => out.push(i),
            Pat::Object { props, rest, .. } => {
                for p in props {
                    p.value.bound_names(out);
                }
                if let Some(r) = rest {
                    r.bound_names(out);
                }
            }
            Pat::Array { elems, rest, .. } => {
                for p in elems.iter().flatten() {
                    p.bound_names(out);
                }
                if let Some(r) = rest {
                    r.bound_names(out);
                }
            }
            Pat::Default { pat, .. } => pat.bound_names(out),
            Pat::Rest { arg, .. } => arg.bound_names(out),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PatProp {
    pub key: PropKey,
    pub value: Pat,
    pub shorthand: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PropKey {
    Ident(Ident),
    Str { value: String, raw: String, span: Span },
    Num { raw: String, span: Span },
    Computed(Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Ident(Ident),
    /// Parsed only to report B0102.
    This(Span),
    Num { raw: String, span: Span },
    Str { value: String, raw: String, span: Span },
    Regex { raw: String, span: Span },
    Bool { value: bool, span: Span },
    Null(Span),
    /// `quasis.len() == exprs.len() + 1`; quasis are raw source text.
    Template { tag: Option<Box<Expr>>, quasis: Vec<String>, exprs: Vec<Expr>, span: Span },
    /// `None` elements are holes: `[a, , b]`.
    Array { elems: Vec<Option<Expr>>, span: Span },
    Object { props: Vec<ObjProp>, span: Span },
    Func(Box<Func>),
    Unary { op: &'static str, arg: Box<Expr>, span: Span },
    Update { op: &'static str, prefix: bool, arg: Box<Expr>, span: Span },
    /// Arithmetic, comparison and logical (`&&`, `||`, `??`) operators.
    Binary { op: &'static str, left: Box<Expr>, right: Box<Expr>, span: Span },
    Assign { op: &'static str, target: Box<Expr>, value: Box<Expr>, span: Span },
    Cond { test: Box<Expr>, cons: Box<Expr>, alt: Box<Expr>, span: Span },
    Call { callee: Box<Expr>, args: Vec<Expr>, optional: bool, span: Span },
    New { callee: Box<Expr>, args: Vec<Expr>, span: Span },
    Member { obj: Box<Expr>, prop: MemberProp, optional: bool, span: Span },
    Seq { exprs: Vec<Expr>, span: Span },
    Spread { arg: Box<Expr>, span: Span },
    Await { arg: Box<Expr>, span: Span },
    Paren { expr: Box<Expr>, span: Span },
}

#[derive(Clone, Debug, PartialEq)]
pub enum MemberProp {
    Name(Ident),
    Computed(Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ObjProp {
    KeyValue { key: PropKey, value: Expr, span: Span },
    Shorthand(Ident),
    /// `name(params) { body }` — value is a non-arrow `Func`.
    Method { key: PropKey, func: Func, span: Span },
    Spread { arg: Expr, span: Span },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Ident(i) => i.span,
            Expr::Func(f) => f.span,
            Expr::This(span) | Expr::Null(span) => *span,
            Expr::Num { span, .. }
            | Expr::Str { span, .. }
            | Expr::Regex { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Template { span, .. }
            | Expr::Array { span, .. }
            | Expr::Object { span, .. }
            | Expr::Unary { span, .. }
            | Expr::Update { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Assign { span, .. }
            | Expr::Cond { span, .. }
            | Expr::Call { span, .. }
            | Expr::New { span, .. }
            | Expr::Member { span, .. }
            | Expr::Seq { span, .. }
            | Expr::Spread { span, .. }
            | Expr::Await { span, .. }
            | Expr::Paren { span, .. } => *span,
        }
    }

    /// Strips redundant parentheses.
    pub fn unparen(&self) -> &Expr {
        match self {
            Expr::Paren { expr, .. } => expr.unparen(),
            e => e,
        }
    }
}
