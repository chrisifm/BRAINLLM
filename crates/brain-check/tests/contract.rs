//! INTERNALS §43: reads/writes per component, contract cards, the reverse graph, `split` and B1010.

use brain_check::{contract, split, Ctx};
use std::path::Path;

fn parse(file: &str, src: &str) -> brain_parse::ast::Component {
    brain_parse::parse(src, Path::new(file)).unwrap_or_else(|d| panic!("{d:?}"))
}

const CART: &str = r#"<meta props="label:str" emits="checkout">

<script>
  import { cart } from '../channels/cart.jsbrain'
  import { Theme } from '../channels/theme.jsbrain'
  import { posts } from '../services/posts.jsbrain'
  let theme = use(Theme)
  let list = resource(posts.get)
  const clear = () => { cart.items = [] }
  const add = (x) => cart.items.push(x)
</script>

<p class={theme.mode}>{label}: {cart.total}</p>
<button on:click={clear}>Clear</button>
"#;

#[test]
fn deps_reads_and_writes() {
    let d = contract::deps(&parse("comp/cart.brain", CART));
    let reads: Vec<&str> = d.reads.iter().map(String::as_str).collect();
    assert_eq!(reads, ["Theme", "Theme.mode", "cart.total", "prop:label", "resource:posts.get"]);
    let writes: Vec<&str> = d.writes.iter().map(String::as_str).collect();
    assert_eq!(writes, ["cart.items"]);
}

#[test]
fn card_is_the_contract_only() {
    let j = contract::card(&parse("comp/cart.brain", CART), Some((0, 1))).stringify();
    for part in [
        r#""name":"Cart""#,
        r#""props":[{"name":"label","type":"str"}]"#,
        r#""emits":["checkout"]"#,
        r#""reads":["Theme","Theme.mode","cart.total","resource:posts.get"]"#,
        r#""writes":["cart.items"]"#,
        r#""status":{"errors":0,"warnings":1}"#,
    ] {
        assert!(j.contains(part), "{part} missing in {j}");
    }
    assert!(!j.contains("cart.items = []"), "the card carries no body: {j}");
}

#[test]
fn reverse_graph_and_filter() {
    let root = std::env::temp_dir().join(format!("brain-contract-{}", std::process::id()));
    std::fs::create_dir_all(root.join("comp")).unwrap();
    std::fs::create_dir_all(root.join("channels")).unwrap();
    std::fs::write(root.join("channels/cart.jsbrain"), "export const cart = store({ items: [], total: 0 })\n").unwrap();
    std::fs::write(root.join("comp/cart.brain"), CART).unwrap();
    std::fs::write(root.join("comp/badge.brain"), "<script>\n  import { cart } from '../channels/cart.jsbrain'\n</script>\n<span>{cart.total}</span>\n").unwrap();
    let j = contract::reverse_json(&root, Some("cart")).stringify();
    std::fs::remove_dir_all(&root).ok();
    assert_eq!(j, r#"{"reads":{"cart.total":["comp/badge.brain","comp/cart.brain"]},"writes":{"cart.items":["comp/cart.brain"]}}"#);
}

const TODO: &str = r#"<meta props="title:str='To do'">

<script>
  let items = sig([])
  let filter = sig('all')
  let left = drv(() => items.filter(it => !it.done).length)
  const toggle = (id) => { items = items.map(it => it.id === id ? { ...it, done: !it.done } : it) }
  const remove = (id) => { items = items.filter(it => it.id !== id) }
</script>

<section class="todo">
  <h2>{title}</h2>
  <ul>
    {#each items as it (it.id)}
      <li class="row" class:done={it.done}>
        <input type="checkbox" checked={it.done} on:change={toggle(it.id)} aria-label={`Mark ${it.text}`} />
        <button on:click={remove(it.id)} aria-label="Delete">×</button>
      </li>
    {/each}
  </ul>
  <footer>
    <span>{left} left</span>
    <button class:on={filter === 'all'} on:click={filter = 'all'}>All</button>
    <button class:on={filter === 'done'} on:click={filter = 'done'}>Done</button>
  </footer>
</section>

<style>
  .todo { display: grid; }
  .row { display: flex; }
  footer button.on { color: var(--accent); }
</style>
"#;

fn clean(file: &str, src: &str) -> Vec<String> {
    let c = parse(file, src);
    brain_check::check(&c, &Ctx::default()).into_iter().filter(|d| d.level == brain_parse::Severity::Error).map(|d| d.to_string()).collect()
}

#[test]
fn split_infers_props_emits_and_moves_css() {
    let c = parse("comp/todo.brain", TODO);
    let s = split::split(&c, "li.row", "TodoRow", "comp/todo-row.brain").unwrap();
    assert_eq!(s.props, ["it:any"]);
    assert_eq!(s.emits, ["toggle", "remove"]);
    assert!(s.extracted.starts_with("<meta props=\"it:any\" emits=\"toggle remove\">"), "{}", s.extracted);
    assert!(s.extracted.contains("on:change={emit('toggle', it.id)}") && s.extracted.contains(".row { display: flex; }"), "{}", s.extracted);
    assert!(s.original.contains("import TodoRow from './todo-row.brain'"), "{}", s.original);
    assert!(s.original.contains("<TodoRow it={it} on:toggle={toggle} on:remove={remove} />"), "{}", s.original);
    assert!(!s.original.contains(".row {"), "the moved rule leaves the parent: {}", s.original);
    assert_eq!(clean("comp/todo-row.brain", &s.extracted), Vec::<String>::new());

    // Writes to the parent's state become a model prop.
    let f = split::split(&c, "footer", "TodoFooter", "comp/todo-footer.brain").unwrap();
    assert_eq!(f.props, ["left:any", "filter:str~"]);
    assert!(f.emits.is_empty());
    assert!(f.original.contains("<TodoFooter left={left} bind:filter={filter} />"), "{}", f.original);
    assert!(f.extracted.contains("on:click={filter = 'all'}"), "{}", f.extracted);
    assert_eq!(clean("comp/todo-footer.brain", &f.extracted), Vec::<String>::new());
}

#[test]
fn split_refuses_ambiguous_or_missing() {
    let c = parse("comp/todo.brain", TODO);
    assert!(split::split(&c, "button", "B", "comp/b.brain").unwrap_err().contains("matches 3 elements"));
    assert!(split::split(&c, ".nope", "Nope", "comp/nope.brain").unwrap_err().contains("no element"));
    assert!(split::split(&c, "ul li", "X", "comp/x.brain").unwrap_err().contains("combinators"));
    assert!(split::split(&c, "footer", "footer", "comp/x.brain").unwrap_err().contains("PascalCase"));
}

#[test]
fn b1010_points_at_the_split() {
    let rows: String = (0..160).map(|i| format!("    <p>Line {i}</p>\n")).collect();
    let big = format!("<main>\n  <h1>Title</h1>\n  <div class=\"intro\">\n    <p>a</p>\n  </div>\n  <article class=\"body\">\n{rows}  </article>\n</main>\n");
    let c = parse("views/big.brain", &big);
    let d: Vec<_> = brain_check::check(&c, &Ctx::default()).into_iter().filter(|d| d.code == "B1010").collect();
    assert_eq!(d.len(), 1);
    assert!(d[0].fix.contains("brain split views/big.brain \"article.body\" Body"), "{}", d[0].fix);

    let card = "  <div class=\"card\">\n    <h2>T</h2>\n    <p>x</p>\n  </div>\n";
    let rep = format!("<main>\n{}{}</main>\n", card.repeat(2), "  <p>pad</p>\n".repeat(150));
    let d: Vec<_> = brain_check::check(&parse("views/rep.brain", &rep), &Ctx::default()).into_iter().filter(|d| d.code == "B1010").collect();
    assert!(d[0].fix.contains("line 2 repeats 2 times"), "{}", d[0].fix);

    assert!(brain_check::check(&parse("comp/todo.brain", TODO), &Ctx::default()).iter().all(|d| d.code != "B1010"));
}
