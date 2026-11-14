# Contributing

## Checks

Run these before a pull request; CI runs all but `tsc`.

```bash
cd renderer
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo test --release          # tests that need TeX Live print a note and pass without it
cd ..
tsc --noEmit -p .
claude plugin test .          # the hooks' tests, against a fake host
```

`tsc` reads the hooks API's typings in `.claude-plugin/types/`, which Claude
Code writes when a session loads the mod from this folder and which are not
committed, so it runs only on a machine where the mod has been loaded. CI
type-checks nothing and runs the plugin tests under the Claude Code version
pinned in `.github/workflows/ci.yml`; raise it when the hooks use a newer API.

Commit messages follow Conventional Commits (`type(scope): subject`, and a
body that says why).

## Releases

Users get `typeset-render` as a binary the mod downloads and checks against
`renderer/checksums.json` (see the README). A release puts the binaries on
GitHub and their sums in that file.

1. Set the new version in `renderer/Cargo.toml` (and `Cargo.lock`, by
   building) and as `version` in `renderer/checksums.json`, with the four
   sums left empty. Leave `.claude-plugin/plugin.json` at the last
   release's version: the checksums pull request moves it. Merge that to
   `main`.
2. Check that the workflow may open pull requests (step 3):
   `gh api repos/jczhang02/claude-typeset/actions/permissions/workflow --jq .can_approve_pull_request_reviews`
   must print `true`; if not, turn on "Allow GitHub Actions to create and
   approve pull requests" in Settings > Actions > General. Then push the
   tag: `git tag v4.0.0 && git push origin v4.0.0`.
3. `.github/workflows/release.yml` checks that the tag names the crate's
   version, creates the release, builds the four targets and uploads each as
   a bare executable named `typeset-render-<target>`:

   | Platform | Target |
   | --- | --- |
   | Linux x86_64 (static) | `x86_64-unknown-linux-musl` |
   | Linux arm64 (static) | `aarch64-unknown-linux-musl` |
   | macOS Apple silicon | `aarch64-apple-darwin` |
   | macOS Intel | `x86_64-apple-darwin` |

   It then sums the uploaded files and opens a pull request,
   `release/checksums-v<version>`, that writes them into
   `renderer/checksums.json` and sets the new version in
   `.claude-plugin/plugin.json`.
4. Check the sums against the release (`sha256sum typeset-render-*` after
   `gh release download v4.0.0`) and merge the pull request. Only then do
   installed copies update to the new version, and they find its sums.

Claude Code updates an installed copy only when `plugin.json`'s version
changes, and keeps it under that version. Were `plugin.json` to name the
new version in step 1, a copy that updated before step 4 would keep the
empty sums for good ("already at the latest version"). CI checks the rule:
`plugin.json` names the crate's version exactly when `checksums.json` has
all four sums. A fresh install between steps 1 and 4 gets the new code
under the old version and no binary; it updates at step 4. A clone that
running sessions load straight from its folder is not an installed copy:
pulling step 1 there switches its sessions to the new code at once, with
no binary to download yet, so formulas show as text until
`cargo build --release` in its `renderer/` finishes (a few minutes). Build
right after pulling, or pull after step 4.

Pushing the tag and merging the checksums pull request are the owner's to do.

The workflow needs "Allow GitHub Actions to create and approve pull requests"
on in the repository's Actions settings (step 2); its token cannot read the
setting, so it cannot check it first. Without it the checksums job pushes
the branch, fails, and names the link to open the pull request by hand. A
pull request opened with the
workflow's token starts no other workflow, so CI does not run on it; it
changes only `checksums.json` and `plugin.json`'s version. Rerunning a failed release replaces the
uploaded files (`--clobber`) and force-pushes the pull request's branch.
