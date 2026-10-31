#!/usr/bin/env bash
# Fails if one app's domain vocabulary appears in what the framework ships (CONTRIBUTING invariant 11): defaults, messages,
# generated code, templates, guides and docs. Pass a directory to scan it instead of the repo (used to test this gate).
set -euo pipefail
command -v rg >/dev/null || { echo "General check requires ripgrep (rg)." >&2; exit 1; }
cd "$(dirname "$0")/.."
target="${1:-.}"
# Quoted names (roles, tables, fields, functions) and identifiers that only make sense for one kind of app.
# "candidate" and "company" alone are ordinary English ("candidate branch"), so only their quoted or derived forms count.
pattern="[\"'\`](candidates?|compan(y|ies)|vacanc(y|ies)|applicants?|employers?|recruiters?|resumes?)[\"'\`]"
pattern+="|\b(candidate|company|vacancy|applicant|employer|recruiter)_[a-z_]+\b|\b[a-z_]+_(candidate|company|vacancy|applicant|employer|recruiter)\b"
pattern+="|\b(salary_(min|max)|listed_until|closes_on|job_?board|cvAi)\b|\bjob board\b|\bvacanc(y|ies)\b|[^a-zA-Z_]/jobs/|\bjob\.slug\b"
# Allowlist, each with its reason:
#  migrations: immutable published ledger; db.rs: verifies that ledger; auth_flows.rs: the 0.6.1 compatibility constants
#  AUTH-FLOWS.md: its compatibility paragraph; history and test code are not shipped; bench/ measures external references
hits=$(rg -n --hidden -e "$pattern" "$target" \
  -g '!.git' -g '!target' -g '!node_modules' -g '!Cargo.lock' -g '!CHANGELOG.md' -g '!docs/CONTINUATION-CHECKLIST.md' \
  -g '!crates/*/tests/**' -g '!runtime/test/**' -g '!scripts/test/**' -g '!npm/test/**' -g '!bench/**' \
  -g '!crates/brain-cli/releases/**' -g '!scripts/check-general.sh' \
  -g '!crates/brain-backend/migrations/0001_backend.sql' -g '!crates/brain-backend/migrations/0005_auth_flows.sql' \
  -g '!crates/brain-backend/src/db.rs' -g '!crates/brain-backend/src/auth_flows.rs' -g '!docs/AUTH-FLOWS.md' \
  -g '!crates/brain-cli/src/backend.rs' -g '!crates/brain-check/src/baseline_data.rs' -g '!scripts/backend-check.mjs') || { status=$?; [[ "$status" -eq 1 ]] || exit "$status"; }
# backend.rs may name the old roles only inside its compatibility tests.
own=$(awk '/#\[cfg\(test\)\]/{exit} {print FILENAME":"FNR":"$0}' crates/brain-cli/src/backend.rs 2>/dev/null | rg -e "$pattern" || true)
# backend-check.mjs may name them only inside its marked 0.6.1 upgrade fixture.
smoke=$(awk '/check-general: begin/{skip=1} !skip{print FILENAME":"FNR":"$0} /check-general: end/{skip=0}' scripts/backend-check.mjs 2>/dev/null | rg -e "$pattern" || true)
own="$own$smoke"
[[ "$target" == "." ]] || own=""
if [[ -n "$hits$own" ]]; then
  printf '%s\n%s\n' "$hits" "$own" | head -100
  echo "App-domain names found: BRAINLLM is general-purpose; use the app's own declaration or a generic example." >&2
  exit 1
fi
echo "General framework: OK"
