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
   building), in `.claude-plugin/plugin.json`, and as `version` in
   `renderer/checksums.json`, with the four sums left empty. Merge that to
   `main`.
2. Push the tag: `git tag v4.0.0 && git push origin v4.0.0`.
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
   `renderer/checksums.json`.
4. Check the sums against the release (`sha256sum typeset-render-*` after
   `gh release download v4.0.0`) and merge the pull request. Only then do
   installed copies download the binary: before it, the empty sums read as
   "no release binary yet".

Pushing the tag and merging the checksums pull request are the owner's to do.

The workflow needs "Allow GitHub Actions to create and approve pull requests"
on in the repository's Actions settings. A pull request opened with the
workflow's token starts no other workflow, so CI does not run on it; it
changes only `checksums.json`. Rerunning a failed release replaces the
uploaded files (`--clobber`) and force-pushes the pull request's branch.
