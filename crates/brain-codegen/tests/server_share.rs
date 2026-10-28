//! #276: the server module gets copies of pure top-level constants and the helpers only server code reaches; those
//! helpers never reach the client bundle (no false B0702/B0704, no `process.env` in the client).
use std::path::Path;

const VIEW: &str = "<meta seo title=\"Dashboard\" description=\"Revenue, orders and the latest orders at a glance.\">
<script>
  const LABELS = { paid: 'Paid', pending: 'Pending' }
  const LIMIT = 5
  const money = (n) => '$' + n.toFixed(2)
  async function fetchDashboard() {
    const res = await fetch(process.env.API_URL + '/dashboard')
    return res.json()
  }
  export async function load(ctx) {
    const d = await fetchDashboard()
    return { orders: d.orders.slice(0, LIMIT).map((o) => ({ id: o.id, status: LABELS[o.status], total: money(o.total) })) }
  }
  let open = sig(false)
</script>
<main><h1>Dashboard</h1><p>{LIMIT} latest · {money(10)}</p>{#each props.orders as o (o.id)}<p>{o.status}</p>{/each}<button on:click={open = !open}>More</button></main>
";

#[test]
fn constants_and_server_helpers_reach_the_server() {
    let c = brain_parse::parse(VIEW, Path::new("views/dashboard.brain")).unwrap();
    let d: Vec<_> = brain_check::check(&c, &brain_check::Ctx::default()).into_iter().filter(|d| d.level == brain_parse::Severity::Error).collect();
    assert!(d.is_empty(), "{d:#?}");
    let o = brain_codegen::compile(VIEW, Path::new("views/dashboard.brain")).unwrap_or_else(|d| panic!("{d:#?}"));
    // Server: the constants, the shared formatter and the env-reading helper, before load.
    for want in ["const LABELS = ", "const LIMIT = 5", "const money = ", "async function fetchDashboard()", "process.env.API_URL"] {
        assert!(o.server_js.contains(want), "server is missing `{want}`:\n{}", o.server_js);
    }
    assert!(o.server_js.find("function fetchDashboard").unwrap() < o.server_js.find("export async function load").unwrap());
    // Client: no env, no server-only helper; what the markup uses stays.
    assert!(!o.js.contains("process.env") && !o.js.contains("fetchDashboard"), "{}", o.js);
    assert!(o.js.contains("money"), "the client still formats with money(): {}", o.js);
}

#[test]
fn reactive_state_stays_out_of_the_server() {
    let src = "<meta seo title=\"X\" description=\"A view whose load reads client state.\">\n<script>\n  let count = sig(0)\n  export async function load() {\n    return { n: count }\n  }\n</script>\n<main><h1>{props.n}</h1></main>\n";
    let c = brain_parse::parse(src, Path::new("views/x.brain")).unwrap();
    assert!(brain_check::check(&c, &brain_check::Ctx::default()).iter().any(|d| d.code == "B0704"));
}
