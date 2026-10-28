//! Human-facing identity. Protocol output and redirected streams never receive branding.
use std::io::IsTerminal;
use std::time::Instant;

pub struct Session {
    enabled: bool,
    began: Instant,
}

pub fn enabled(args: &[String], terminals: bool, env: &dyn Fn(&str) -> Option<String>) -> bool {
    terminals
        && !args.iter().any(|a| matches!(a.as_str(), "--json" | "--profile" | "--quiet" | "-q" | "--progress=off" | "--progress=plain"))
        && !["CI", "BRAIN_MCP"].iter().any(|k| env(k).is_some_and(|v| !v.is_empty()))
        && env("TERM").as_deref() != Some("dumb")
}

const MARK: &str = r#"      .--. .--.
     / o  V  o \
    |  .--|--.  |   BRAINLLM
    | o   /   o |   The web compiler for LLMs.
     \   /_    /    Your intent. Its language.
      '--' '--'"#;

impl Session {
    pub fn start(args: &[String], message: &str) -> Self {
        let enabled = enabled(args, std::io::stdout().is_terminal() && std::io::stderr().is_terminal(), &crate::term::env_var);
        if enabled {
            // Presence, even with an empty value, opts out of color.
            if std::env::var_os("NO_COLOR").is_none() {
                eprintln!("\x1b[38;5;154m{MARK}\x1b[0m\n\n  > {message}");
            } else {
                eprintln!("{MARK}\n\n  > {message}");
            }
        }
        Self { enabled, began: Instant::now() }
    }

    pub fn finish(&self, message: &str) {
        if self.enabled {
            eprintln!("  > {message} [{:.1} ms]\n", self.began.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_and_redirects_cannot_force_marketing() {
        let args = |a: &[&str]| a.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert!(enabled(&[], true, &|_| None));
        assert!(!enabled(&args(&["--progress=rich"]), false, &|_| None));
        for flag in ["--json", "--profile", "--quiet", "-q", "--progress=off", "--progress=plain"] {
            assert!(!enabled(&args(&[flag, "--progress=rich"]), true, &|_| None), "{flag}");
        }
        for key in ["CI", "BRAIN_MCP"] {
            assert!(!enabled(&args(&["--progress=rich"]), true, &|k| (k == key).then(|| "1".into())));
        }
        assert!(!enabled(&[], true, &|k| (k == "TERM").then(|| "dumb".into())));
    }
    #[test]
    fn monochrome_terminal_keeps_the_identity() {
        assert!(enabled(&[], true, &|k| (k == "NO_COLOR").then(String::new)));
        assert!(MARK.is_ascii());
    }
}
