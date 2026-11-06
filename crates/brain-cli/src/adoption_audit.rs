//! Read-only legacy frontend inventory. Metadata and file-name hints never establish behavioral parity.
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, io::Read, path::Path};

const MAX_ENTRIES: usize = 20_000;
const MAX_SOURCES: usize = 2048;
const MAX_DEPTH: usize = 20;
const MAX_PACKAGE: usize = 256 * 1024;
const SKIP: &[&str] = &[
    "node_modules",
    "dist",
    "build",
    "coverage",
    "vendor",
    "target",
];
const SOURCE: &[&str] = &[
    "js", "mjs", "cjs", "jsx", "ts", "tsx", "vue", "svelte", "html", "css", "scss", "brain",
    "jsbrain", "cssbrain",
];

pub fn run(args: &[String]) -> Result<Value, String> {
    let mut root = None;
    let mut audit = false;
    let mut tokens = args.iter();
    while let Some(arg) = tokens.next() {
        match arg.as_str() {
            "--audit" if !audit => audit = true,
            "--json" | "--quiet" | "-q" => (),
            "--progress"
                if tokens
                    .next()
                    .is_some_and(|v| matches!(v.as_str(), "off" | "plain" | "rich")) =>
            {
                ()
            }
            "--progress=off" | "--progress=plain" | "--progress=rich" => (),
            _ if !arg.starts_with('-') && root.is_none() => root = Some(arg.as_str()),
            _ => return Err("usage: brain adopt --audit [source-directory] [--json]".into()),
        }
    }
    if !audit {
        return Err("usage: brain adopt --audit [source-directory] [--json]".into());
    }
    inventory(Path::new(root.unwrap_or(".")))
}

pub fn inventory(root: &Path) -> Result<Value, String> {
    let meta = fs::symlink_metadata(root).map_err(|_| "cannot inspect source directory")?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("audit source must be a regular local directory, not a symlink".into());
    }
    let package = root.join("package.json");
    let mut frameworks = Vec::new();
    let mut package_present = false;
    let mut package_bytes = 0;
    match fs::symlink_metadata(&package) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            if meta.len() > MAX_PACKAGE as u64 {
                return Err("package.json exceeds the 256 KiB inventory limit".into());
            }
            let mut bytes = Vec::new();
            fs::File::open(&package)
                .map_err(|_| "cannot open package.json")?
                .take((MAX_PACKAGE + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "cannot read package.json")?;
            if bytes.len() > MAX_PACKAGE {
                return Err("package.json exceeds the 256 KiB inventory limit".into());
            }
            package_bytes = bytes.len();
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "package.json must be valid JSON")?;
            if !value.is_object() {
                return Err("package.json must be an object".into());
            }
            for (key, label) in [
                ("react", "React"),
                ("@angular/core", "Angular"),
                ("vue", "Vue"),
                ("svelte", "Svelte"),
                ("next", "Next.js"),
                ("nuxt", "Nuxt"),
            ] {
                if ["dependencies", "devDependencies"]
                    .iter()
                    .any(|kind| value.get(kind).and_then(|v| v.get(key)).is_some())
                {
                    frameworks.push(label);
                }
            }
            package_present = true;
        }
        Ok(_) => return Err("package.json must be a regular local file, not a symlink".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(_) => return Err("cannot inspect package.json".into()),
    }
    let mut queue = vec![(String::new(), 0usize)];
    let mut scanned = 0usize;
    let mut skipped_symlinks = 0usize;
    let mut skipped_special = 0usize;
    let mut sources = Vec::new();
    let mut extension_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut source_bytes = 0u64;
    while let Some((directory, depth)) = queue.pop() {
        let iter = fs::read_dir(root.join(&directory))
            .map_err(|_| "cannot inspect source directory entries")?;
        let mut entries = Vec::new();
        for entry in iter {
            scanned += 1;
            if scanned > MAX_ENTRIES {
                return Err(
                    "audit exceeds 20000 entries; audit one frontend package at a time".into(),
                );
            }
            entries.push(entry.map_err(|_| "cannot read source directory entry")?);
        }
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                skipped_special += 1;
                continue;
            };
            if name.starts_with('.') || SKIP.contains(&name) {
                continue;
            }
            let path = if directory.is_empty() {
                name.to_string()
            } else {
                format!("{directory}/{name}")
            };
            if path.len() > 512 || path.chars().any(char::is_control) {
                return Err(
                    "inventory path exceeds 512 bytes or contains control characters".into(),
                );
            }
            let kind = entry
                .file_type()
                .map_err(|_| "cannot inspect source entry type")?;
            if kind.is_symlink() {
                skipped_symlinks += 1;
                continue;
            }
            if kind.is_dir() {
                if depth >= MAX_DEPTH {
                    return Err("inventory depth exceeds 20; audit a smaller package".into());
                }
                queue.push((path, depth + 1));
                continue;
            }
            if !kind.is_file() {
                skipped_special += 1;
                continue;
            }
            let extension = Path::new(name)
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if !SOURCE.contains(&extension) {
                continue;
            }
            if sources.len() >= MAX_SOURCES {
                return Err("inventory exceeds 2048 source files; audit a smaller package".into());
            }
            source_bytes = source_bytes.saturating_add(
                entry
                    .metadata()
                    .map_err(|_| "cannot inspect source size")?
                    .len(),
            );
            *extension_counts.entry(extension.into()).or_default() += 1;
            sources.push(path);
        }
    }
    sources.sort();
    let routes = sources
        .iter()
        .filter(|path| {
            path.split('/')
                .any(|part| matches!(part, "route" | "routes" | "pages" | "views"))
                || Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|name| {
                        ["app.", "router.", "routing."]
                            .iter()
                            .any(|prefix| name.starts_with(prefix))
                    })
        })
        .collect::<Vec<_>>();
    let tests = sources
        .iter()
        .filter(|path| {
            path.split('/')
                .any(|part| matches!(part, "__tests__" | "tests" | "test" | "e2e"))
                || path.contains(".test.")
                || path.contains(".spec.")
        })
        .collect::<Vec<_>>();
    Ok(
        json!({"schemaVersion":1,"ok":true,"mode":"read-only-adoption-audit","packagePresent":package_present,"frameworks":frameworks,"sourceFiles":sources,"sourceCount":sources.len(),"sourceBytes":source_bytes,"extensions":extension_counts,"routeCandidates":routes,"testCandidates":tests,"entriesScanned":scanned,"skippedSymlinks":skipped_symlinks,"skippedSpecialFiles":skipped_special,"limits":{"entries":MAX_ENTRIES,"sourceFiles":MAX_SOURCES,"depth":MAX_DEPTH,"pathBytes":512,"packageBytes":MAX_PACKAGE},"inspection":{"packageBytesRead":package_bytes,"sourceBodiesRead":false,"executedSource":false,"executedScripts":false,"writes":false},"caveat":"Frameworks use known dependency keys. Routes and tests use file-name hints only. No source bodies, dependency versions, package scripts, environment files or configuration values are returned. Unknown frameworks remain unknown.","nextSteps":["Read docs/MIGRATION.md and the target brain spec.","Capture synthetic route, UI, API, authentication and accessibility behavior in the original frontend.","Port one tested leaf component or route into a separate target, preserving states and backend contracts.","Record source, target, behavior, tests, evidence, status and rollback for each slice; inventory is not behavioral acceptance."]}),
    )
}
