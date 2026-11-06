//! Structured MCP parity for bounded operator CLI commands. Never shells out or publishes.
use serde_json::{Value,json};
use std::{path::{Path,Component},process::Stdio,time::Duration};
use tokio::io::AsyncReadExt;
const OUTPUT:usize=2*1024*1024;
fn arguments(tool:&str,request:&Value)->Result<Vec<String>,String>{
 let object=request.as_object().ok_or("operator request must be an object")?;
 let (prefix,allowed):(&[&str],&[&str])=match tool {
  "brain_db_audit"=>(&["db","audit"],&["environment","limit"]),
  "brain_backend_conform"=>(&["backend","conform"],&["url","paths"]),
  "brain_google_setup"=>(&["google","setup"],&["environment"]),
  "brain_seo_diff"=>(&["seo","diff"],&["from"]),
  "brain_vitals_report"=>(&["vitals","report"],&["environment","days","limit","offset"]),
  "brain_data_backfill"=>(&["data","backfill"],&["name","environment","limit","includeNull","apply","request","confirm"]),
  "brain_edge"=>(&["edge"],&["operation","provider","url","request","confirm"]),
  _=>return Err("unknown operator tool".into())
 };
 if object.keys().any(|key|!allowed.contains(&key.as_str())){return Err("unknown operator request field".into());}
 let text=|key:&str|->Result<Option<&str>,String>{match request.get(key){None=>Ok(None),Some(value)=>value.as_str().filter(|v|!v.is_empty()&&!v.starts_with('-')&&v.len()<=4096&&!v.chars().any(char::is_control)).map(Some).ok_or_else(||format!("{key} must be a bounded nonempty string"))}};
 let boolean=|key:&str|->Result<bool,String>{match request.get(key){None=>Ok(false),Some(value)=>value.as_bool().ok_or_else(||format!("{key} must be a boolean"))}};
 let mut args=prefix.iter().map(|v|v.to_string()).collect::<Vec<_>>();
 if tool=="brain_data_backfill" {
  let name=text("name")?.filter(|name|brain_backend::contract::identifier(name)).ok_or("backfill name must be a declared identifier")?;
  args.push(name.into());
  if text("environment")?.is_none(){return Err("backfill requires an explicit environment".into());}
  let apply=boolean("apply")?;
  if apply {if !boolean("confirm")? || text("request")?.is_none(){return Err("backfill apply requires confirm:true and a reviewed request file".into());}args.push("--apply".into());}
  else if request.get("request").is_some()||boolean("confirm")? {return Err("preview does not accept an apply request or confirmation".into());}
  if boolean("includeNull")?{args.push("--include-null".into());}
 }
 if tool=="brain_edge" {
  let operation=text("operation")?.unwrap_or("plan");
  if !matches!(operation,"plan"|"audit"|"check"|"apply"){return Err("edge operation must be plan, audit, check or apply".into());}
  args.push(operation.into());
  if let Some(provider)=text("provider")? {if provider!="cloudflare"{return Err("only the explicit cloudflare provider is supported".into());}}
  if operation=="apply" && (!boolean("confirm")? || text("provider")?!=Some("cloudflare") || text("request")?.is_none()) {return Err("edge apply requires explicit cloudflare, confirm:true and a reviewed request file".into());}
  if operation!="apply" && boolean("confirm")? {return Err("only edge apply accepts confirmation".into());}
 }
 for(key,flag)in[("environment","--env"),("url","--url"),("from","--from"),("provider","--provider"),("request","--request")] {
  if let Some(value)=text(key)? {
   if key=="environment" && (value.len()>64 || !value.as_bytes()[0].is_ascii_alphabetic() || !value.bytes().all(|c|c.is_ascii_alphanumeric()||matches!(c,b'_'|b'-'))){return Err("environment must be a declared environment name".into());}
   if key=="request" || key=="from" && !value.starts_with("http://") && !value.starts_with("https://") {
    if !Path::new(value).components().all(|part|matches!(part,Component::Normal(_))) || value.split(['/', '\\']).any(|part|part.starts_with('.')) {return Err("operator input files must be visible app-relative paths".into());}
   }
   args.extend([flag.into(),value.into()]);
  }
 }
 if tool=="brain_backend_conform" && text("url")?.is_none(){return Err("conform requires an explicit URL".into());}
 if tool=="brain_seo_diff" && text("from")?.is_none(){return Err("SEO diff requires an explicit baseline sitemap".into());}
 for(key,max)in[("days",90u64),("limit",if tool=="brain_db_audit"{200}else if tool=="brain_vitals_report"{500}else{1000}),("offset",10000)] {
  if let Some(value)=request.get(key){let number=value.as_u64().filter(|n|*n<=max&&(*n>0||key=="offset")).ok_or_else(||format!("{key} is outside its operator bounds"))?;args.extend([format!("--{key}"),number.to_string()]);}
 }
 if let Some(paths)=request.get("paths") {
  let paths=paths.as_array().filter(|v|!v.is_empty()&&v.len()<=8).ok_or("paths must contain one to eight concrete routes")?;
  for path in paths {let path=path.as_str().filter(|v|v.len()<=4096&&v.starts_with('/')&&!v.starts_with("//")&&!v.chars().any(char::is_control)).ok_or("paths must be bounded app-relative routes")?;args.extend(["--path".into(),path.into()]);}
 }
 args.push("--json".into());Ok(args)
}
async fn execute(root:&Path,executable:&Path,args:&[String],budget:Duration)->Result<Value,String>{
 let mut child=tokio::process::Command::new(executable).args(args).current_dir(root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().map_err(|_|"cannot start owned operator command")?;
 let stdout=child.stdout.take().ok_or("operator stdout unavailable")?;
 let outcome=tokio::time::timeout(budget,async {
  let mut bytes=Vec::new();stdout.take((OUTPUT+1)as u64).read_to_end(&mut bytes).await.map_err(|_|"operator output could not be read")?;
  if bytes.len()>OUTPUT {return Err("operator output exceeds 2 MiB");}
  let status=child.wait().await.map_err(|_|"owned operator command could not be reaped")?;
  let code=status.code().ok_or("owned operator command ended without an exit code")?;
  let report:Value=serde_json::from_slice(&bytes).map_err(|_|"operator command did not return one JSON document; private stderr was not captured")?;
  Ok(json!({"schemaVersion":1,"ok":status.success(),"exitCode":code,"report":report}))
 }).await;
 if !matches!(outcome,Ok(Ok(_))) {let _=child.start_kill();if !matches!(tokio::time::timeout(Duration::from_secs(1),child.wait()).await,Ok(Ok(_))){return Err("owned operator command could not be reaped after termination".into());}}
 outcome.map_err(|_|"operator command reached its 90 second deadline".to_string())?.map_err(str::to_string)
}
pub(crate) fn run(root:&Path,tool:&str,request:&Value)->Result<Value,String>{
 let args=arguments(tool,request)?;
 for key in ["request","from"] {
  if let Some(value)=request.get(key).and_then(Value::as_str) {
   if key=="request" || !value.starts_with("http://")&&!value.starts_with("https://") {
    let path=crate::changes::source_path(root,value).map_err(|_|"operator input file must remain inside the current app without symlinks or hidden paths")?;
    let metadata=std::fs::symlink_metadata(path).map_err(|_|"cannot inspect reviewed operator input file")?;
    if !metadata.is_file() || metadata.file_type().is_symlink(){return Err("reviewed operator input must be a regular current-app file".into());}
   }
  }
 }

 let executable=std::env::current_exe().map_err(|_|"cannot resolve the current compiler executable")?;
 let runtime=crate::http_observer::Runtime::new()?;
 runtime.block_on(execute(root,&executable,&args,Duration::from_secs(90)))
}
#[cfg(test)]
mod tests {
 use super::*;
 #[test]fn mutation_confirmation_and_exact_input_shape_precede_execution(){
  for request in [json!({"operation":"apply","provider":"cloudflare","request":"plan.json"}),json!({"operation":"apply","provider":"cloudflare","request":"plan.json","confirm":false}),json!({"operation":"apply","provider":"cloudflare","request":"../plan.json","confirm":true}),json!({"operation":"plan","confirm":true}),json!({"operation":"publish"})]{assert!(arguments("brain_edge",&request).is_err());}
  assert!(arguments("brain_edge",&json!({"operation":"apply","provider":"cloudflare","request":"plans/edge.json","confirm":true})).is_ok());
  assert!(arguments("brain_data_backfill",&json!({"name":"labels","environment":"development","apply":true,"request":"plan.json"})).is_err());
  assert!(arguments("brain_data_backfill",&json!({"name":"labels","environment":"development","apply":true,"request":"plan.json","confirm":true})).is_ok());
  for(tool,request)in[("brain_db_audit",json!({"limit":201})),("brain_backend_conform",json!({"paths":["/"]})),("brain_seo_diff",json!({"from":"../private.xml"})),("brain_vitals_report",json!({"days":0})),("brain_google_setup",json!({"secret":"private"}))]{assert!(arguments(tool,&request).is_err());}
 }
}
#[cfg(all(test,unix))]
mod lifecycle_tests {
 use super::*;
 use std::{fs,os::unix::fs::PermissionsExt,sync::atomic::{AtomicUsize,Ordering}};
 #[test]
 fn operator_output_and_deadline_failures_kill_and_reap_only_their_owned_child(){
  static NEXT:AtomicUsize=AtomicUsize::new(0);
  let root=std::env::temp_dir().join(format!("brain-operator-child-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));fs::create_dir_all(&root).unwrap();
  struct Cleanup(std::path::PathBuf);impl Drop for Cleanup{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
  let _cleanup=Cleanup(root.clone());let executable=root.join("owned-child");
  for (body,budget,expected) in [("process.stdout.write(JSON.stringify({schemaVersion:1,ok:true}));",Duration::from_secs(5),None),("process.stdout.write('x'.repeat(2*1024*1024+1));setInterval(()=>{},1000);",Duration::from_secs(5),Some("2 MiB")),("setInterval(()=>{},1000);",Duration::from_millis(500),Some("deadline"))] {
   fs::write(&executable,format!("#!/usr/bin/env node\nrequire('node:fs').writeFileSync('owned.pid',String(process.pid));{body}\n")).unwrap();fs::set_permissions(&executable,fs::Permissions::from_mode(0o700)).unwrap();
   let runtime=crate::http_observer::Runtime::new().unwrap();let result=runtime.block_on(execute(&root,&executable,&[],budget));
   if let Some(expected)=expected{assert!(result.unwrap_err().contains(expected));}else{assert_eq!(result.unwrap()["report"]["ok"],true);}
   let pid=fs::read_to_string(root.join("owned.pid")).unwrap();let alive=std::process::Command::new("kill").args(["-0",pid.trim()]).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();assert!(!alive.success(),"owned child must be gone before returning");
  }
 }
}
