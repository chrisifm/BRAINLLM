//! Bounded argument handling for focused agent commands; no implicit mutations.
use serde_json::{Value, json};

pub fn entry(args: &[String]) -> std::process::ExitCode {
    match run(args) {
        Ok(blocked) => std::process::ExitCode::from(if blocked { 1 } else { 0 }),
        Err(error) => {
            if args.iter().any(|a| a == "--json") {
                println!("{}", crate::error_json(&error));
            } else {
                eprintln!("error: {error}");
            }
            std::process::ExitCode::from(2)
        }
    }
}
fn run(args: &[String]) -> Result<bool, String> {
    let (command, positional, flags) = parse(args)?;
    let root = crate::app_root()?;
    let number = |key: &str, default: usize| -> Result<usize, String> {
        flags
            .get(key)
            .map(|s| {
                s.parse()
                    .map_err(|_| format!("{key} must be a nonnegative integer"))
            })
            .unwrap_or(Ok(default))
    };
    let value = match command {
        "changes" => {
            if positional.len() != 2 {
                return Err("usage: brain changes <operation> '<json>' [--json]".into());
            }
            let input: Value = serde_json::from_str(positional[1]).map_err(|e| e.to_string())?;
            crate::changes::dispatch(&root, positional[0], &input)?
        }
        "recipe" => match positional.as_slice() {
            [] | ["list"] => crate::recipes::list(),
            [name] => crate::recipes::generate(&root, name, &json!({}), true)?,
            [name, input] if *name != "list" => crate::recipes::generate(
                &root,
                name,
                &serde_json::from_str(input).map_err(|e| e.to_string())?,
                true,
            )?,
            _ => return Err("usage: brain recipe [list|name ['<json>']] [--json]".into()),
        },
        "graph" if positional.is_empty() => {
            crate::intelligence::graph_page(&root, number("--cursor", 0)?, number("--limit", 100)?)?
        }
        "context" if positional.len() == 1 => {
            crate::intelligence::context(&root, positional[0], number("--budget", 8192)?)?
        }
        "why" if positional.len() == 1 => crate::intelligence::why(&root, positional[0])?,
        "preflight" if positional.len() <= 1 => {
            crate::intelligence::preflight(&root, positional.first().copied())?
        }
        _ => {
            return Err(format!(
                "invalid arguments for brain {command}; fix: run brain help"
            ));
        }
    };
    let blocked = command == "preflight"
        && value
            .get("firstBlockingDiagnostic")
            .is_some_and(|v| !v.is_null());
    println!("{}", crate::versioned(value));
    Ok(blocked)
}
fn parse(
    args: &[String],
) -> Result<(&str, Vec<&str>, std::collections::BTreeMap<&str, &str>), String> {
    let command = args.first().ok_or("command is required")?.as_str();
    let mut positional = vec![];
    let mut flags = std::collections::BTreeMap::new();
    let mut i = 1;
    while i < args.len() {
        let value = args[i].as_str();
        if value == "--json" {
            i += 1;
            continue;
        }
        if value.starts_with('-') {
            let allowed = matches!(
                (command, value),
                ("graph", "--cursor" | "--limit") | ("context", "--budget")
            );
            if !allowed || flags.contains_key(value) {
                return Err(format!("unknown or duplicate option {value} for {command}"));
            }
            i += 1;
            let argument = args
                .get(i)
                .ok_or_else(|| format!("{value} requires a value"))?
                .as_str();
            if argument.starts_with('-') {
                return Err(format!("{value} requires a nonnegative integer"));
            }
            flags.insert(value, argument);
        } else {
            positional.push(value);
        }
        i += 1;
    }
    Ok((command, positional, flags))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn json_flags_do_not_become_recipe_parameters_or_graph_targets() {
        let a = args(&["recipe", "--json", "auth-ui"]);
        let (_, p, _) = parse(&a).unwrap();
        assert_eq!(p, ["auth-ui"]);
        let a = args(&["graph", "--cursor", "100", "--json", "--limit", "25"]);
        let (_, p, f) = parse(&a).unwrap();
        assert!(p.is_empty());
        assert_eq!(f["--cursor"], "100");
        assert!(parse(&args(&["graph", "--budget", "1"])).is_err());
        assert!(parse(&args(&["graph", "--limit", "1", "--limit", "2"])).is_err());
        assert!(parse(&args(&["context", "--budget"])).is_err());
    }
}
