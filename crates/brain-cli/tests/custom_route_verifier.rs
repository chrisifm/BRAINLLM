//! Custom route verification is explicit private source, checked before a deployment can serve requests.
use std::{path::{Path,PathBuf},process::{Command,Output,Stdio}};

struct Fixture(PathBuf);
impl Drop for Fixture {fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
fn cli(root:&Path,args:&[&str])->Output {
    Command::new(env!("CARGO_BIN_EXE_brain")).args(args).current_dir(root).env_remove("BRAIN_BACKEND_URL")
        .stdin(Stdio::null()).output().unwrap()
}
fn write(root:&Path,relative:&str,source:&str){let path=root.join(relative);std::fs::create_dir_all(path.parent().unwrap()).unwrap();std::fs::write(path,source).unwrap();}

#[test]
fn custom_verifier_requires_a_private_companion_without_inventing_signing_secret_fields() {
    let root=std::env::temp_dir().join(format!("brain-custom-verifier-check-{}",std::process::id()));
    std::fs::create_dir_all(&root).unwrap();let _cleanup=Fixture(root.clone());
    assert!(cli(&root,&["new","app","--json"]).status.success());
    let app=root.join("app");
    write(&app,"server/routes/events/route.jsbrain","<meta server>\nexport default { method: 'POST', path: '/events/custom', auth: 'public', body: 'raw', headers: ['x-event-signature'], verify: { scheme: 'custom' } }\n");
    write(&app,"server/routes/events/handler.jsbrain","<meta server>\nexport default async function handle(ctx) { return { accepted: true } }\n");
    let check=||{let output=cli(&app,&["check","--json"]);format!("{}{}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr))};
    let missing=check();assert!(missing.contains("B0735")&&missing.contains("verify.jsbrain"),"{missing}");
    assert!(!missing.contains("verify.header")&&!missing.contains("verify.secret"),"{missing}");
    write(&app,"server/routes/events/verify.jsbrain","export default async function verify(ctx) { return false }\n");
    let public=check();assert!(public.contains("verify.jsbrain")&&public.contains("private server code"),"{public}");
    write(&app,"server/routes/events/verify.jsbrain","<meta server>\nexport default async function verify(ctx) { return false }\n");
    let valid=check();assert!(!valid.contains("B0735")&&!valid.contains("B0790"),"{valid}");
    write(&app,"server/routes/events/verify.jsbrain","<meta server>\nexport default async function verify(ctx) { return ctx.headers['x-verifier-marker'] === 'accepted' }\n");
    write(&app,"server/routes/events/handler.jsbrain","<meta server>\nexport default async function handle(ctx) { return { marker: ctx.headers['x-handler-marker'] } }\n");
    let undeclared=check();
    assert!(undeclared.contains("B0816")&&undeclared.contains("x-verifier-marker")&&undeclared.contains("x-handler-marker"),"{undeclared}");
    write(&app,"server/routes/events/route.jsbrain","<meta server>\nexport default { method: 'POST', path: '/events/custom', auth: 'public', body: 'raw', headers: ['x-event-signature', 'x-verifier-marker', 'x-handler-marker'], verify: { scheme: 'custom' } }\n");
    let declared=check();assert!(!declared.contains("B0816"),"{declared}");
}
