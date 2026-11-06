//! Opt-in private error artifacts. No DSN or original .brain source enters this module.
use std::{collections::BTreeMap, io::Read, path::Path};
use brain_lex::Json;

#[derive(Clone)]
pub(crate) struct Mapping {
    pub input: String,
    pub positions: Vec<(usize, usize)>,
}
pub(crate) struct Artifact {
    pub asset: String,
    pub id: String,
    pub javascript: String,
    pub map: String,
    pub source_name: String,
    pub generated_source: String,
}
pub(crate) fn minify(source: &str, enabled: bool) -> Result<(brain_min::Minified, Vec<(usize, usize)>), String> {
    if enabled { return brain_min::minify_module_with_positions(source,true).map_err(|_| "Sentry module mapping failed; check generated browser JavaScript".into()) }
    let positions=brain_min::lex::lex(source).map_err(|_| "Sentry generated JavaScript could not be scanned")?.into_iter().map(|token|(token.start,token.start)).collect();
    Ok((brain_min::Minified{code:source.into(),report:Default::default()},positions))
}
pub(crate) fn enabled(root: &Path) -> Result<bool, String> {
    let path = root.join("brain.json");
    let meta = match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err("Cannot inspect brain.json Sentry configuration".into()),
        Ok(meta) => meta,
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 2*1024*1024 { return Err("Sentry configuration requires a regular brain.json file of at most 2 MiB".into()) }
    let mut source = String::new();
    std::fs::File::open(path).map_err(|_| "Cannot open brain.json Sentry configuration")?.take(2*1024*1024+1).read_to_string(&mut source).map_err(|_| "Cannot read brain.json Sentry configuration")?;
    if source.len() > 2*1024*1024 { return Err("brain.json exceeds the Sentry configuration read limit".into()) }
    let config = Json::parse(&source).map_err(|_| "Sentry configuration requires valid brain.json JSON")?;
    let Json::Obj(fields) = config else { return Err("brain.json must be an object".into()) };
    let entries: Vec<_> = fields.iter().filter(|(key, _)| key == "sentry").collect();
    if entries.len() > 1 { return Err("brain.json sentry must be declared once".into()) }
    match entries.first().map(|(_, value)| value) {
        None | Some(Json::Bool(false)) => Ok(false),
        Some(Json::Obj(fields)) if fields.len() == 1 && fields[0].0 == "enabled" => match fields[0].1 {
            Json::Bool(value) => Ok(value),
            _ => Err("sentry.enabled must be an explicit boolean".into()),
        },
        _ => Err("sentry requires {\"enabled\":true} or false; keep BRAIN_SENTRY_DSN in the private server environment".into()),
    }
}
impl Mapping {
    /// Import/asset references are rewritten after minification. Preserve their start anchor and shift later tokens.
    pub fn replace(&mut self, start: usize, end: usize, length: usize) {
        self.positions.retain(|(output, _)| *output <= start || *output >= end);
        for (output, _) in &mut self.positions {
            if *output >= end { *output = *output - (end - start) + length; }
        }
    }
}
fn hash(bytes: &[u8]) -> String { crate::sha::sha256(bytes).iter().map(|b| format!("{b:02x}")).collect() }
fn uuid(bytes: &[u8]) -> String {
    let mut b = crate::sha::sha256(bytes);
    b[6] = (b[6] & 15) | 0x50; b[8] = (b[8] & 63) | 0x80;
    let s: String = b[..16].iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &s[..8], &s[8..12], &s[12..16], &s[16..20], &s[20..])
}
fn vlq(value: i64, out: &mut String) {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut n = (value.unsigned_abs() << 1) | u64::from(value < 0);
    loop { let mut digit = (n & 31) as usize; n >>= 5; if n != 0 { digit |= 32; } out.push(CHARS[digit] as char); if n == 0 { break; } }
}
struct Index<'a> { source: &'a str, lines: Vec<usize>, unicode: Vec<(usize, usize)> }
impl<'a> Index<'a> {
    fn new(source: &'a str) -> Self {
        let mut lines = vec![0]; let mut unicode = vec![(0,0)]; let mut saved = 0;
        for (offset, ch) in source.char_indices() {
            if ch == '\n' { lines.push(offset+1); }
            if !ch.is_ascii() { saved += ch.len_utf8()-ch.len_utf16(); unicode.push((offset+ch.len_utf8(),saved)); }
        }
        Self { source, lines, unicode }
    }
    fn location(&self, offset: usize) -> Option<(usize, usize)> {
        if offset > self.source.len() || !self.source.is_char_boundary(offset) { return None }
        let line = self.lines.partition_point(|start| *start <= offset)-1;
        let start = self.lines[line];
        let saved = |at| self.unicode[self.unicode.partition_point(|(position,_)| *position <= at)-1].1;
        Some((line, offset-start-(saved(offset)-saved(start))))
    }
}
pub(crate) fn artifact(javascript: &mut String, mapping: &Mapping) -> Result<Artifact, String> {
    let source_name = format!("generated/{}.js", hash(mapping.input.as_bytes()));
    let mut mappings = String::new();
    let (mut line, mut column, mut previous_line, mut previous_column) = (0, 0, 0, 0);
    let mut first = true;
    let output_index = Index::new(javascript);
    let input_index = Index::new(&mapping.input);
    for &(output, input) in &mapping.positions {
        let Some((output_line, output_column)) = output_index.location(output) else { return Err("Sentry output mapping is not on a UTF-8 boundary".into()) };
        let Some((input_line, input_column)) = input_index.location(input) else { return Err("Sentry input mapping is not on a UTF-8 boundary".into()) };
        while line < output_line { mappings.push(';'); line += 1; column = 0; first = true; }
        if !first { mappings.push(','); }
        vlq(output_column as i64 - column as i64, &mut mappings); vlq(0, &mut mappings);
        vlq(input_line as i64 - previous_line as i64, &mut mappings); vlq(input_column as i64 - previous_column as i64, &mut mappings);
        column = output_column; previous_line = input_line; previous_column = input_column; first = false;
    }
    if mappings.is_empty() { return Err("Sentry requires a non-empty generated JavaScript source map".into()) }
    let seed = format!("{javascript}\n{source_name}\n{mappings}");
    let id = uuid(seed.as_bytes());
    javascript.push_str(&format!("\n//# debugId={id}\n"));
    let map = format!("{{\"version\":3,\"debug_id\":{},\"debugId\":{},\"sources\":[{}],\"names\":[],\"mappings\":{},\"x_brain_precision\":\"generated-JavaScript token anchors; no original BrainScript mapping\"}}\n", crate::prerender_json(&id), crate::prerender_json(&id), crate::prerender_json(&source_name), crate::prerender_json(&mappings));
    Ok(Artifact { asset: String::new(), id, javascript: javascript.clone(), map, source_name, generated_source: mapping.input.clone() })
}
pub(crate) fn validate_catalog(artifacts: &[Artifact]) -> Result<(), String> {
    if artifacts.len() > 4096 {
        Err("Sentry supports at most 4096 mapped browser modules per build; fix: reduce the optional reporting build graph or disable sentry".into())
    } else { Ok(()) }
}
pub(crate) fn catalog(artifacts: &[Artifact]) -> String {
    let entries: BTreeMap<&str, &str> = artifacts.iter().map(|artifact| (artifact.asset.as_str(), artifact.id.as_str())).collect();
    format!("{{{}}}", entries.into_iter().map(|(asset,id)| format!("{}:{}", crate::prerender_json(asset), crate::prerender_json(id))).collect::<Vec<_>>().join(","))
}
pub(crate) fn browser_catalog(artifacts: &[Artifact]) -> String {
    // Relative catalog keys are data, not resource references. Keep conservative byte accounting from
    // treating every build asset as a download on every route; the reporter restores the leading slash.
    let entries: BTreeMap<&str, &str> = artifacts.iter().map(|artifact| (artifact.asset.trim_start_matches('/'), artifact.id.as_str())).collect();
    format!("{{{}}}", entries.into_iter().map(|(asset,id)| format!("{}:{}", crate::prerender_json(asset), crate::prerender_json(id))).collect::<Vec<_>>().join(","))
}
pub(crate) fn write_private(root: &Path, build: &str, artifacts: &[Artifact]) -> Result<(), String> {
    fn error<T>(_: T) -> String { "Cannot write private Sentry artifacts; check the owned .brain/sentry directory permissions and remove symlinks".into() }
    let mut dir = root.to_path_buf();
    for part in [".brain", "sentry", build] {
        dir.push(part);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => return Err(error(())),
            Ok(_) => {},
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(&dir).map_err(error)?,
            Err(_) => return Err(error(())),
        }
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(error)?; }
    }
    let generated = dir.join("generated");
    if std::fs::symlink_metadata(&generated).is_ok_and(|m| !m.is_dir() || m.file_type().is_symlink()) { return Err(error(())) }
    std::fs::create_dir_all(&generated).map_err(error)?;
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(&generated, std::fs::Permissions::from_mode(0o700)).map_err(error)?; }
    let write = |name: &str, bytes: &[u8]| -> Result<(), String> {
        let path = dir.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.is_file() || m.file_type().is_symlink()) { return Err(error(())) }
        std::fs::write(&path, bytes).map_err(error)?;
        #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(error)?; }
        Ok(())
    };
    for artifact in artifacts {
        write(&format!("{}.js", artifact.id), artifact.javascript.as_bytes())?;
        write(&format!("{}.js.map", artifact.id), artifact.map.as_bytes())?;
        // Only already-public browser JS, never the original .brain text or a server function.
        // Development inline maps can contain the original .brain, including private server code.
        // Such input still has valid source-less coordinates, but never gets copied as a source artifact.
        if !artifact.generated_source.contains("sourceMappingURL") && !artifact.generated_source.contains("sourcesContent") {
            write(&artifact.source_name, artifact.generated_source.as_bytes())?;
        }
    }
    write("catalog.json", catalog(artifacts).as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    #[test]
    fn actual_throw_maps_to_generated_javascript_after_unicode_and_import_replacement() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("brain-sentry-map-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir_all(dir.join("nested")).unwrap();
        struct Cleanup(std::path::PathBuf); impl Drop for Cleanup { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
        let _cleanup = Cleanup(dir.clone());
        let source = "import { seed } from './dependency.mjs';\nexport function fail() { globalThis.label = '😀'; if (seed) throw new TypeError('synthetic private value'); }\n";
        let (result, positions) = brain_min::minify_module_with_positions(source,true).unwrap();
        let mut code = result.code;
        let mut mapping = Mapping { input: source.into(), positions };
        let start = code.find("'./dependency.mjs'").unwrap();
        let old_len = "'./dependency.mjs'".len();
        let replacement = "'./nested/dependency.mjs'";
        mapping.replace(start,start+old_len,replacement.len()); code.replace_range(start..start+old_len,replacement);
        let initial = artifact(&mut code,&mapping).unwrap();
        assert!(!initial.map.contains("sourcesContent")); assert!(!initial.map.contains("synthetic private value"));
        assert!(code.ends_with(&format!("//# debugId={}\n",initial.id)));
        let mut repeat = code.split("\n//# debugId=").next().unwrap().to_string();
        assert_eq!(artifact(&mut repeat,&mapping).unwrap().id,initial.id);
        let mut different = code.replace("nested/dependency.mjs","nested/dependencY.mjs");
        assert_ne!(artifact(&mut different,&mapping).unwrap().id,initial.id);
        std::fs::write(dir.join("nested/dependency.mjs"),"export const seed=1").unwrap();
        std::fs::write(dir.join("entry.mjs"),&code).unwrap();
        std::fs::write(dir.join("entry.mjs.map"),&initial.map).unwrap();
        let script = r#"import {SourceMap} from 'node:module';import{readFileSync}from'node:fs';import{pathToFileURL}from'node:url';const path=process.argv[1];const m=await import(pathToFileURL(path).href);try{m.fail()}catch(e){const at=/entry\.mjs:(\d+):(\d+)/.exec(e.stack);if(!at)process.exit(2);const map=JSON.parse(readFileSync(path+'.map','utf8'));const p=new SourceMap(map).findEntry(+at[1]-1,+at[2]-1);console.log(JSON.stringify({line:p.originalLine,column:p.originalColumn,id:map.debug_id,alias:map.debugId}))}"#;
        let output = std::process::Command::new("node").args(["--input-type=module","-e",script]).arg(dir.join("entry.mjs")).output().unwrap();
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
        let parsed = Json::parse(std::str::from_utf8(&output.stdout).unwrap()).unwrap();
        assert_eq!(parsed.get("line"),Some(&Json::Num(1.0)));
        let expected = source.lines().nth(1).unwrap().split("new TypeError").next().unwrap().encode_utf16().count();
        assert_eq!(parsed.get("column"),Some(&Json::Num(expected as f64)),"{}",String::from_utf8_lossy(&output.stdout));
        assert_eq!(parsed.get("id"),Some(&Json::Str(initial.id.clone())));assert_eq!(parsed.get("alias"),Some(&Json::Str(initial.id)));
    }
    #[test]
    fn utf16_locations_and_private_defaults_are_explicit() {
        let index=Index::new("😀x\n\u{e9}z");
        assert_eq!(index.location(4),Some((0,2)));assert_eq!(index.location(8),Some((1,1)));assert_eq!(index.location(1),None);
    }
    #[test]
    fn private_maps_never_copy_development_inline_source_contents() {
        let root=std::env::temp_dir().join(format!("brain-sentry-private-{}",std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        struct Cleanup(std::path::PathBuf);impl Drop for Cleanup{fn drop(&mut self){let _=std::fs::remove_dir_all(&self.0);}}
        let _cleanup=Cleanup(root.clone());
        let source="export const kept=1\n//# sourceMappingURL=data:application/json;base64,synthetic_private_source";
        let(result,positions)=minify(source,true).unwrap();let mut code=result.code;
        let artifact=artifact(&mut code,&Mapping{input:source.into(),positions}).unwrap();
        let source_name=artifact.source_name.clone();
        write_private(&root,"synthetic-build",&[artifact]).unwrap();
        let dir=root.join(".brain/sentry/synthetic-build");
        assert!(!dir.join(source_name).exists());
        for entry in std::fs::read_dir(&dir).unwrap().flatten().filter(|entry|entry.path().is_file()) {
            let text=std::fs::read_to_string(entry.path()).unwrap();assert!(!text.contains("synthetic_private_source"));assert!(!text.contains("sourcesContent"));
        }
        let (unminified,positions)=minify("export const retained = '😀';",false).unwrap();
        assert_eq!(unminified.code,"export const retained = '😀';");assert!(!positions.is_empty());
    }
    #[test]
    fn mapped_catalog_limit_is_rejected_during_build_not_server_startup() {
        let mut artifacts: Vec<Artifact> = (0..4096).map(|index| Artifact {asset:format!("/_b/{index:08x}.js"),id:String::new(),javascript:String::new(),map:String::new(),source_name:String::new(),generated_source:String::new()}).collect();
        assert!(validate_catalog(&artifacts).is_ok());
        artifacts.push(Artifact {asset:"/_b/ffffffff.js".into(),id:String::new(),javascript:String::new(),map:String::new(),source_name:String::new(),generated_source:String::new()});
        assert!(validate_catalog(&artifacts).unwrap_err().contains("disable sentry"));
    }
    #[test]
    fn core_observer_stays_within_the_real_minified_gzip_budget() {
        let source=include_str!("../../../runtime/brain.js");
        let min=brain_min::minify_runtime_core(source).unwrap();
        let gzip=crate::gzip::gzip(min.as_bytes(),15);
        eprintln!("Sentry observer: core raw={} minified={} gzip={}",source.len(),min.len(),gzip.len());
        assert!(gzip.len()<=4096,"core gzip budget exceeded: {}",gzip.len());
        if let Ok(path)=std::env::var("BRAIN_CORE_BASELINE") {
            let baseline=std::fs::read_to_string(path).unwrap();let old=brain_min::minify_runtime_core(&baseline).unwrap();let old_gzip=crate::gzip::gzip(old.as_bytes(),15);
            eprintln!("Sentry observer baseline: minified={} gzip={}; delta minified={} gzip={}",old.len(),old_gzip.len(),min.len() as isize-old.len() as isize,gzip.len() as isize-old_gzip.len() as isize);
        }
    }
}
