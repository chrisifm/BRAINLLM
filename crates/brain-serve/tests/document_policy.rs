//! Selected error documents keep their own security policy, independently of the requested URL.
use brain_pack::cache::{self,PageRef};
use brain_serve::prod::Static;
use std::{fs,io::{Read,Write},net::{TcpListener,TcpStream},path::{Path,PathBuf},sync::atomic::{AtomicU64,Ordering},time::Duration};
struct App(PathBuf);
impl Drop for App {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
fn ask(dist:&Path,base:&str,path:&str,method:&str,legacy:bool)->String {
    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let peer=listener.accept().unwrap().0;
    let mut gateway=Static::load(dist,base);
    if legacy {gateway.rules.retain(|(path,_)|!path.ends_with("/index.html") && !path.ends_with("/index.brain") && !path.ends_with("/404.brain") && !path.ends_with("/flat.html") && !path.ends_with("/flat.brain"));}
    let worker=std::thread::spawn(move||gateway.handle(peer).unwrap());
    client.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").as_bytes()).unwrap();
    let mut response=String::new();client.read_to_string(&mut response).unwrap();worker.join().unwrap();response
}
#[test]
fn generated_document_policy_covers_error_fallbacks_aliases_bases_and_legacy_headers_without_leaking_to_assets() {
    static NEXT:AtomicU64=AtomicU64::new(0);
    for extension in ["html","brain"] {for base in ["","/site"] {
        let app=App(std::env::temp_dir().join(format!("brain-document-policy-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed))));
        fs::create_dir_all(app.0.join("_b")).unwrap();
        let index=format!("index.{extension}");
        let missing=format!("404.{extension}");
        let docs=format!("docs/index.{extension}");
        let unicode=format!("sample-\u{e9}/index.{extension}");
        let percent=format!("rate%/index.{extension}");
        let flat=format!("flat.{extension}");
        for file in [&index,&missing,&docs,&unicode,&percent,&flat] {
            let path=app.0.join(file);fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path,"<html><head><meta name=\"brain-build\" content=\"policy-proof\"></head><body>Selected document</body></html>").unwrap();
        }
        fs::write(app.0.join("_b/ok.js"),"export const example=1;").unwrap();
        let pages=[PageRef{route:"/",file:&index},PageRef{route:"*",file:&missing},PageRef{route:"/docs",file:&docs},PageRef{route:"/sample-%C3%A9",file:&unicode},PageRef{route:"/rate%25",file:&percent},PageRef{route:"/flat",file:&flat}];
        let headers=cache::base_paths(&cache::headers(&pages,&[],false,"policy-proof",true,"default-src 'self'; object-src 'none'"),base);
        fs::write(app.0.join("_headers"),headers).unwrap();
        for legacy in [false,true] {
            for (path,status) in [("/missing",404),("/missing?query=1",404),("/_b/missing.js",404),("/404",404),("/404.html",404),("/404.brain",404),("/docs",200),("/docs/index.brain",if extension=="brain"{200}else{404}),("/flat.html",if extension=="html"{200}else{404}),("/flat.brain",if extension=="brain"{200}else{404}),("/sample-%C3%A9",200),("/rate%25",200)] {
                for method in ["GET","HEAD"] {
                    let response=ask(&app.0,base,&format!("{base}{path}"),method,legacy);
                    let head=response.split("\r\n\r\n").next().unwrap().to_ascii_lowercase();
                    assert!(response.starts_with(&format!("HTTP/1.1 {status}")),"{extension} {base}{path} {method}: {response}");
                    assert!(head.contains("content-security-policy: default-src 'self'; object-src 'none'\r\n"),"selected document CSP: {response}");
                    assert!(head.contains("cross-origin-opener-policy: same-origin"),"selected document opener policy: {response}");
                    assert!(head.contains("cache-control: no-cache") && !head.contains("immutable"),"an error document must not inherit a missing asset's immutable cache: {response}");
                    if method=="HEAD" {assert_eq!(response.split_once("\r\n\r\n").unwrap().1,"");}
                }
            }
            for path in ["/docs/index.html"] {
                let response=ask(&app.0,base,&format!("{base}{path}"),"GET",legacy);
                assert!(response.starts_with("HTTP/1.1 301") && response.contains(&format!("Location: {base}/docs\r\n")),"canonical aliases must retain their existing redirect: {response}");
            }
            let asset=ask(&app.0,base,&format!("{base}/_b/ok.js"),"GET",legacy).to_ascii_lowercase();
            assert!(asset.starts_with("http/1.1 200") && asset.contains("immutable"),"{asset}");
            assert!(!asset.contains("content-security-policy:") && !asset.contains("cross-origin-opener-policy:"),"document policy must not leak to assets: {asset}");
        }
    }}
}

#[test]
fn a_similar_filename_cannot_inherit_another_documents_popup_policy() {
    let app=App(std::env::temp_dir().join(format!("brain-document-identity-{}",std::process::id())));
    fs::create_dir_all(&app.0).unwrap();
    for file in ["404.html","404.html.html"] {fs::write(app.0.join(file),"<html><body>Example</body></html>").unwrap();}
    fs::write(app.0.join("_headers"),"/*\n  X-Content-Type-Options: nosniff\n/404.html\n  Content-Security-Policy: default-src 'self'\n  Cross-Origin-Opener-Policy: same-origin-allow-popups\n").unwrap();
    let response=ask(&app.0,"","/404.html.html","GET",false);
    assert!(response.starts_with("HTTP/1.1 200"),"only one recognized extension establishes an error status: {response}");
    assert!(!response.to_ascii_lowercase().contains("same-origin-allow-popups"),"a different file must not inherit a guessed canonical identity: {response}");
}
