//! Unknown flags are errors. A flag nobody reads looks like it took effect: `brain test --port 5280` once read the
//! value as a path, found no tests and reported green. Commands that parse their arguments strictly are not listed.

/// Accepted by every listed command.
const GLOBAL: &[&str] = &["--json", "--quiet", "-q", "--progress"];

/// `(command, flags that take a value, switches)`.
pub(crate) const TABLE: &[(&str, &[&str], &[&str])] = &[
    ("new", &[], &["--backend"]),
    ("add", &["--props", "--store", "--get", "--post", "--fields"], &["--bus", "--key", "--admin-ui"]),
    ("check", &[], &[]),
    ("fix", &[], &[]),
    ("doctor", &[], &["--fix", "--viewer"]),
    ("build", &["--site", "--ext", "--env"], &["--report"]),
    ("run", &["--port", "--host", "--base", "--inspect-port", "--env"], &["--mock", "--profile", "--no-inspect", "--no-trace", "--backend"]),
    ("dev", &["--port", "--host", "--base", "--inspect-port", "--env"], &["--mock", "--profile", "--no-inspect", "--no-trace", "--backend"]),
    ("test", &["--timeout"], &["--heap", "--viewer"]),
    ("e2e", &["--timeout"], &["--heap", "--viewer"]),
    ("explain", &[], &[]),
    ("map", &["--view", "--comp", "--reads", "--limit", "--cursor"], &[]),
    ("card", &[], &[]),
    ("split", &[], &[]),
    ("spec", &["--topic"], &["--all"]),
    ("guide", &[], &[]),
    ("schema", &[], &[]),
    ("theme", &[], &["--list"]),
    ("i18n", &[], &["--missing"]),
    ("seo", &["--since", "--env"], &["--dry-run"]),
    ("stop", &[], &[]),
    ("reload", &[], &[]),
    ("status", &[], &[]),
    ("version", &[], &[]),
    ("find", &["--kind", "--limit", "--cursor"], &["--watch"]),
    ("symbols", &["--kind", "--limit", "--cursor"], &["--watch"]),
    ("refs", &["--kind", "--limit", "--cursor"], &["--watch"]),
    ("lib", &[], &[]),
    ("history", &["--request"], &[]),
    ("integration", &["--version"], &[]),
    ("deps", &["--request", "--file"], &[]),
    ("feedback", &["--request"], &[]),
    ("quality", &[], &[]),
    ("deploy", &["--request"], &[]),
    ("design", &["--request"], &[]),
    ("manifest", &["--request"], &[]),
    (
        "backend",
        &[
            "--env", "--port", "--origin", "--from", "--to", "--user", "--email", "--provider", "--name", "--scopes", "--expires-days", "--daily",
            "--monthly", "--status", "--resource", "--before", "--limit", "--days", "--accept",
        ],
        &["--apply", "--with-workers", "--system", "--create-db"],
    ),
    ("jobs", &["--env", "--payload", "--key", "--priority", "--delay", "--id", "--concurrency"], &["--once"]),
    ("analytics", &["--env", "--since", "--days", "--viewed", "--min-views", "--within", "--not-viewed", "--tracked", "--signed-in", "--limit", "--user"], &["--csv"]),
    ("media", &["--env", "--owner", "--limit", "--offset"], &[]),
];

/// `Err` names the first flag the command does not know and what it does accept. A `--` ends the flags.
pub(crate) fn check(args: &[String]) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else { return Ok(()) };
    let Some((_, valued, switches)) = TABLE.iter().find(|(name, ..)| name == command) else { return Ok(()) };
    let mut tokens = rest.iter();
    while let Some(token) = tokens.next() {
        if token == "--" {
            break;
        }
        // a lone `-` and a negative number are values
        if !token.starts_with('-') || token.len() < 2 || token[1..].starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let (name, inline) = token.split_once('=').map_or((token.as_str(), false), |(name, _)| (name, true));
        if valued.contains(&name) {
            if !inline {
                tokens.next();
            }
        } else if !switches.contains(&name) && !GLOBAL.contains(&name) {
            let known: Vec<&str> = valued.iter().chain(switches.iter()).copied().collect();
            let accepted = if known.is_empty() { "it takes no flags besides --json".to_string() } else { format!("it accepts {}", known.join(", ")) };
            return Err(format!("unknown flag `{name}` for brain {command}; fix: {accepted} (`brain {command} --help` prints its usage)"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<(), String> {
        check(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    /// Commands the documentation shows must parse; a flag added to a command but not to `TABLE` stops working silently.
    #[test]
    fn every_flag_the_documentation_shows_is_accepted() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = vec![root.join("SPEC.md")];
        for dir in ["crates/brain-cli/guides", "docs/backend"] {
            for entry in std::fs::read_dir(root.join(dir)).unwrap() {
                files.push(entry.unwrap().path());
            }
        }
        let mut wrong = vec![];
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            for mention in text.split("brain ").skip(1) {
                let end = mention.find(['`', '\n', '|', ';', '&', '#']).unwrap_or(mention.len());
                let words: Vec<&str> = mention[..end].split_whitespace().collect();
                let Some((command, rest)) = words.split_first() else { continue };
                let Some((_, valued, switches)) = TABLE.iter().find(|(name, ..)| name == command) else { continue };
                for word in rest.iter().filter(|w| w.starts_with("--")) {
                    let name = word.split('=').next().unwrap();
                    if !valued.contains(&name) && !switches.contains(&name) && !GLOBAL.contains(&name) {
                        wrong.push(format!("{}: brain {command} {name}", file.display()));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "documented flags missing from TABLE:\n{}", wrong.join("\n"));
    }

    #[test]
    fn an_unknown_flag_is_named_with_what_the_command_accepts() {
        let error = run(&["test", "--port", "5280"]).unwrap_err();
        assert!(error.contains("`--port`") && error.contains("--timeout"), "{error}");
        assert!(run(&["check", "-x"]).unwrap_err().contains("takes no flags"));
    }

    #[test]
    fn known_flags_values_and_the_end_of_flags_pass() {
        for args in [
            &["test", "comp", "--timeout", "5", "--heap", "--json"][..],
            &["test", "--timeout=5"],
            &["run", "--mock=seed", "--port=5180", "--progress=off"],
            &["add", "comp", "x", "--props", "--looks-like-a-flag"],
            &["find", "x", "--", "--anything"],
            &["seo", "indexnow", "--dry-run", "--since", "2026-01-01"],
            &["map", "-1"],
            &["changes", "--anything"],
        ] {
            assert_eq!(run(args), Ok(()), "{args:?}");
        }
    }
}
