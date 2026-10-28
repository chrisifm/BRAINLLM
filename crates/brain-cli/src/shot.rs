//! Bounded screenshots and browser-decoded visual baselines.
use std::path::{Component, Path, PathBuf};

pub fn safe_output(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if relative.is_empty() || relative.contains('\\') || path.components().any(|c| !matches!(c, Component::Normal(_))) { return Err("shot output must stay inside the project".into()); }
    let mut dest=root.to_path_buf();
    for part in path.components() {
        dest.push(part);
        if std::fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink()) { return Err("shot output refuses symlinks".into()); }
    }
    std::fs::create_dir_all(dest.parent().ok_or("missing output directory")?).map_err(|e|e.to_string())?;
    Ok(dest)
}
/// Validate the PNG envelope before any browser decoder can allocate pixel buffers.
pub fn png_size(bytes: &[u8]) -> Result<(u32, u32), String> {
    if bytes.len() > 8 * 1024 * 1024 { return Err("PNG input exceeds 8 MiB".into()); }
    if bytes.len() < 33 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[8..16] != b"\0\0\0\rIHDR" { return Err("invalid PNG header".into()); }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    if width == 0 || height == 0 || width > 4096 || height > 4096 || u64::from(width) * u64::from(height) > 4_194_304 { return Err("PNG dimensions exceed the 4 megapixel/4096 edge limit".into()); }
    let depth = bytes[24];
    let valid_depth = match bytes[25] { 0 => [1,2,4,8,16].contains(&depth), 2|4|6 => [8,16].contains(&depth), 3 => [1,2,4,8].contains(&depth), _ => false };
    if !valid_depth || bytes[26] != 0 || bytes[27] != 0 || bytes[28] > 1 { return Err("unsupported PNG header".into()); }
    let mut pos = 8usize;
    let mut data = false;
    while pos + 12 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[pos..pos+4].try_into().unwrap()) as usize;
        let end = pos.checked_add(12).and_then(|n| n.checked_add(len)).filter(|n| *n <= bytes.len()).ok_or("truncated PNG chunk")?;
        let kind = &bytes[pos+4..pos+8];
        if kind == b"acTL" { return Err("animated PNG baselines are unsupported".into()); }
        if kind == b"IHDR" && pos != 8 { return Err("duplicate PNG header".into()); }
        if kind == b"IDAT" { data = true; }
        if kind == b"IEND" { return if data && len == 0 && end == bytes.len() { Ok((width,height)) } else { Err("invalid PNG end".into()) }; }
        pos = end;
    }
    Err("PNG has no complete image data/end".into())
}
fn read_png(path: &Path) -> Result<Vec<u8>,String> {
    use std::io::Read;
    let mut bytes=Vec::new();
    std::fs::File::open(path).map_err(|e|e.to_string())?.take(8*1024*1024+1).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
    png_size(&bytes)?;
    Ok(bytes)
}
/// Atomic replacement prevents readers from observing a partial screenshot/baseline.
pub fn write_output(path: &Path, bytes: &[u8]) -> Result<(),String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64,Ordering};
    static NEXT: AtomicU64=AtomicU64::new(0);
    if std::fs::symlink_metadata(path).is_ok_and(|m|m.file_type().is_symlink()) { return Err("shot refuses symlink output".into()); }
    let temporary=path.with_extension(format!("shot-{}-{}.tmp",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|e|e.to_string())?;
    let result=file.write_all(bytes).and_then(|_|file.sync_all()).and_then(|_|std::fs::rename(&temporary,path));
    if result.is_err(){let _=std::fs::remove_file(&temporary);}
    result.map_err(|e|e.to_string())
}
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8]=b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out=String::with_capacity(bytes.len().div_ceil(3)*4);
    for c in bytes.chunks(3) {
        let n=(u32::from(c[0])<<16)|(u32::from(*c.get(1).unwrap_or(&0))<<8)|u32::from(*c.get(2).unwrap_or(&0));
        out.push(ABC[((n>>18)&63)as usize]as char);out.push(ABC[((n>>12)&63)as usize]as char);
        out.push(if c.len()>1{ABC[((n>>6)&63)as usize]as char}else{'='});
        out.push(if c.len()>2{ABC[(n&63)as usize]as char}else{'='});
    }
    out
}

use brain_mcp::Json;
use crate::browser::Browser;

#[derive(Default)]
pub struct Options {
    pub baseline: Option<String>,
    pub update: bool,
    pub max_diff: f64,
}
fn baseline_name(name: &str) -> bool { !name.is_empty() && name.len() <= 64 && name.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'-'||b==b'_') }

pub fn capture(browser: &mut Browser, root: &Path, route: &str, width: u32, height: u32, opts: &Options) -> Result<Json,String> {
    if !opts.max_diff.is_finite() || !(0.0..=1.0).contains(&opts.max_diff) { return Err("maxDiff must be a ratio between 0 and 1".into()); }
    if opts.update && opts.baseline.is_none() { return Err("updating a baseline requires its explicit name".into()); }
    if opts.baseline.as_deref().is_some_and(|n|!baseline_name(n)) { return Err("baseline names use 1-64 letters, digits, dashes or underscores".into()); }
    let (_,diags)=crate::run_check(Some(root.to_path_buf()))?;
    if diags.iter().any(|d|!d.is_warning()) { return Err("shot requires a project without compiler errors; run brain check first".into()); }
    let (file, issues, page)=browser.shot(root,route,width,height)?;
    let text=browser.accessibility_text()?;
    let text_path=file.with_extension("ax.txt");
    if std::fs::symlink_metadata(&text_path).is_ok_and(|m|m.file_type().is_symlink()) { return Err("shot refuses symlink output".into()); }
    write_output(&text_path,text.as_bytes())?;
    let mut ok=issues.is_empty();
    let mut comparison=Json::Null;
    if let Some(name)=&opts.baseline {
        let baseline=safe_output(root,&format!("baselines/{name}-{width}x{height}.png"))?;
        if opts.update {
            write_output(&baseline,&read_png(&file)?)?;
            comparison=Json::obj([("updated",Json::Bool(true)),("file",Json::str(baseline.display().to_string()))]);
        } else if !baseline.is_file() {
            ok=false;
            comparison=Json::obj([("ok",Json::Bool(false)),("error",Json::str("baseline missing; inspect the screenshot and explicitly run --update-baseline"))]);
        } else {
            if std::fs::metadata(&baseline).map_err(|e|e.to_string())?.len()>8*1024*1024 {return Err("baseline exceeds 8 MiB".into());}
            comparison=browser.compare_png(&read_png(&baseline)?,&read_png(&file)?,opts.max_diff)?;
            ok &= comparison.get("ok")==Some(&Json::Bool(true));
            if let Json::Obj(ref mut entries)=comparison {
                if let Some(i)=entries.iter().position(|(k,_)|k=="diff") {
                    let (_,encoded)=entries.remove(i);
                    if let Some(encoded)=encoded.as_str(){
                        if encoded.len()>12*1024*1024 { return Err("diff PNG exceeds encoded size limit".into()); }
                        let pixels=crate::browser::base64_decode(encoded).ok_or("invalid diff PNG")?;
                        png_size(&pixels)?;
                        let diff=file.with_extension("diff.png");
                        if std::fs::symlink_metadata(&diff).is_ok_and(|m|m.file_type().is_symlink()) { return Err("shot refuses symlink output".into()); }
                        write_output(&diff,&pixels)?;
                        entries.push(("diffFile".into(),Json::str(diff.display().to_string())));
                    }
                }
            }
        }
    }
    let a11y=Json::Arr(issues.iter().map(|i|Json::obj([("code",Json::str(i.code)),("role",Json::str(&i.role)),("node",Json::str(&i.node)),("msg",Json::str(&i.msg)),("fix",Json::str(i.fix))])).collect());
    Ok(Json::obj([("ok",Json::Bool(ok)),("file",Json::str(file.display().to_string())),("textFile",Json::str(text_path.display().to_string())),("text",Json::str(text)),("width",Json::Num(width as f64)),("height",Json::Num(height as f64)),("page",page),("a11y",a11y),("comparison",comparison)]))
}

pub fn dev(root: &Path) -> Result<brain_serve::Dev,String> {
    brain_serve::set_quiet(true);
    let opts=brain_serve::Options {root:root.to_path_buf(),host:"127.0.0.1".into(),port:0,head_prepend:crate::head_prepend(),runtime:crate::runtime_files(),env:Some(crate::env::loader(root,"development")),base:"/".into(),diagnose:None,problems:None,upstream:None,mock:None};
    brain_serve::start_dev(opts,crate::compile_fn(root.to_path_buf(),true),false,None).map_err(|e|e.to_string())
}

pub fn run(root: &Path,args:&[String])->Result<Json,String>{
    let mut route=None;let mut viewport="both".to_string();let mut opts=Options::default();let mut width=None;let mut height=None;let mut i=0;
    while i<args.len(){
        let a=args[i].as_str();
        match a {
            "--json"|"--text"=>{},"--update-baseline"=>opts.update=true,
            "--baseline"|"--max-diff"|"--viewport"|"--width"|"--height"=>{
                i+=1;let v=args.get(i).ok_or_else(||format!("missing value for {a}"))?;
                match a {"--baseline"=>opts.baseline=Some(v.clone()),"--max-diff"=>opts.max_diff=v.parse().map_err(|_|"invalid --max-diff")?,"--viewport"=>viewport=v.clone(),"--width"=>width=Some(v.parse::<u32>().map_err(|_|"invalid --width")?),_=>height=Some(v.parse::<u32>().map_err(|_|"invalid --height")?)}
            }
            _ if !a.starts_with('-') && route.is_none()=>route=Some(a.to_string()),
            _=>return Err(format!("unknown shot argument: {a}")),
        }i+=1;
    }
    let route=route.unwrap_or_else(||"/".into());
    let sizes=match(width,height){(Some(w),Some(h))=>vec![(w,h)],(None,None)=>match viewport.as_str(){"mobile"=>vec![(390,844)],"desktop"=>vec![(1280,800)],"both"=>vec![(390,844),(1280,800)],_=>return Err("viewport must be mobile, desktop or both".into())},_=>return Err("provide both --width and --height".into())};
    let server=dev(root)?;let mut browser=Browser::open(&server.url())?;
    let mut shots=Vec::new();for(w,h)in sizes{shots.push(capture(&mut browser,root,&route,w,h,&opts)?);}
    let ok=shots.iter().all(|s|s.get("ok")==Some(&Json::Bool(true)));
    Ok(Json::obj([("schemaVersion",Json::Num(1.0)),("ok",Json::Bool(ok)),("route",Json::str(route)),("shots",Json::Arr(shots))]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn png_headers_reject_decoder_allocation_bombs_and_incomplete_data() {
        let mut png=b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(8192u32.to_be_bytes());png.extend(8192u32.to_be_bytes());png.extend([8,6,0,0,0,0,0,0,0]);
        assert!(png_size(&png).unwrap_err().contains("dimensions"));
        png[16..20].copy_from_slice(&1u32.to_be_bytes());png[20..24].copy_from_slice(&1u32.to_be_bytes());
        assert!(png_size(&png).is_err());
        png.extend(b"\0\0\0\0IDAT\0\0\0\0\0\0\0\0IEND\0\0\0\0");
        assert_eq!(png_size(&png).unwrap(),(1,1)); // Pixel validity remains Chrome's responsibility.
        png[37..41].copy_from_slice(b"acTL");assert!(png_size(&png).unwrap_err().contains("animated"));
        assert!(png_size(b"not an image").is_err());
    }
    #[test]fn encoding_and_names(){for bytes in [b"".as_slice(),b"a",b"ab",b"abc",b"abcdef"]{assert_eq!(crate::browser::base64_decode(&base64(bytes)).unwrap(),bytes);}assert!(baseline_name("home-1"));assert!(!baseline_name("../secret"));assert!(!baseline_name(""));}
}
