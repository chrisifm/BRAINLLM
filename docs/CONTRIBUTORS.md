# Contributor attribution

BRAINLLM credits public GitHub contributor accounts in its README and landing page. Human developers and bot
accounts returned by GitHub are included. A model is not assigned a fictitious profile: a contribution is attributed
to the actual public account GitHub associates with the commits.

## What updates automatically

`scripts/sync-contributors.mjs` queries the public contributors endpoint for `chrisifm/BRAINLLM`, follows numbered
pages, validates accounts and writes two outputs:

- The README section between `<!-- contributors:start -->` and `<!-- contributors:end -->`.
- `docs/site/contributors.json`, consumed as a landing-page snapshot.

The snapshot has `schemaVersion`, `repository` and `contributors`. Each record contains `login`, `profileUrl`,
`avatarUrl`, `contributions` and `type`. Profile labels use GitHub logins; no additional profile lookup is needed.
GitHub's contribution counts are not a score of contribution quality. Accounts are sorted by count, then login.

Run `node scripts/sync-contributors.mjs` locally to refresh from the public API. An optional `GH_TOKEN` or
`GITHUB_TOKEN` is read from the environment for rate limits; never put it in a file or commit. Anonymous email-only
records are not requested or published. The principal author's attribution remains independent of this section.

## Reviewed automation

The `contributors` GitHub Actions workflow runs after pushes to `main`, weekly, or by manual dispatch. It creates a
review branch and pull request, or updates the existing open attribution PR. It never pushes to `main`, force-pushes,
merges, grants permissions or weakens branch protection. A merge conflict stops the workflow for review.

The repository must allow GitHub Actions to create pull requests. The workflow's token requests only repository
contents and pull-request write permissions. The principal author must review the generated PR before merging it.
Current GitHub behavior requires a user with write access to approve workflow runs created by a
`GITHUB_TOKEN`-opened or updated PR. Use the PR's **Approve workflows to run** control. Push-triggered workflows
from this token remain suppressed. If the required `verify` check still does not start, dispatch verification
for that branch or use an explicitly configured, scoped GitHub App integration. Do not remove the required check to make an attribution PR mergeable.

The README and committed landing snapshot update only when the attribution PR is merged. The landing may refresh
public attribution in the browser; that does not bypass the README review gate. GitHub can cache contributor data,
so a just-merged contribution may appear on a later refresh.

## Failure behavior and bounds

The script permits 20 pages of 100 accounts, a two-minute request deadline and at most 2 MiB per page. A pending
GitHub response is retried at most three times. Invalid, empty, failed, over-limit or rate-limited responses fail
without replacing the previous attribution. There is no endless retry or polling loop. Both outputs are validated
before files are replaced; no timestamp is added, so an unchanged list creates no new commit.

References: [GitHub contributor API](https://docs.github.com/en/rest/repos/repos#list-repository-contributors),
[workflow events and GITHUB_TOKEN](https://docs.github.com/en/actions/how-tos/write-workflows/choose-when-workflows-run/trigger-a-workflow).
