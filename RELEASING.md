# Releasing Trail

GitHub Releases are the source of truth for Trail binaries. The release workflow
uses `dist` 0.32.0 to build platform archives, checksums, installer scripts,
GitHub attestations, and the Homebrew formula.

## One-time repository setup

1. Add a fine-grained token named `HOMEBREW_TAP_TOKEN` to the `crabbuild/trail`
   GitHub Actions secrets. It needs contents write access to
   `crabbuild/homebrew-tap`.
2. Protect `main` and require the normal CI, Release plan, and Release Readiness
   checks before merging.
3. Keep GitHub Actions workflow permissions able to create releases and artifact
   attestations.

The normal `GITHUB_TOKEN` publishes the release to `crabbuild/trail`. Only the
Homebrew tap requires a separate cross-repository token.

The tap is shared with the Crab release. Trail publication creates or replaces
only `Formula/trail.rb`; the workflow refuses to commit if any other tap file
changes, so the existing `Formula/crab.rb` remains independent.

## Prepare a release

Create a release pull request that contains only release preparation changes:

1. Update `workspace.package.version` in `Cargo.toml`.
2. Run `cargo update -w` so `Cargo.lock` records the workspace version.
3. Move relevant entries from `CHANGELOG.md`'s `Unreleased` section into a
   `X.Y.Z` section with the release date, and update its comparison links.
4. Run:

   ```sh
   make release-check
   dist plan --tag=vX.Y.Z
   dist generate --mode=ci --check
   ```

5. Review and merge the release pull request.

Install the pinned release tool when needed:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/axodotdev/cargo-dist/releases/download/v0.32.0/cargo-dist-installer.sh | sh
```

## Publish

Tag the merge commit. The tag must exactly match the Cargo version:

```sh
git switch main
git pull --ff-only
git tag -a vX.Y.Z -m "Trail X.Y.Z"
git push origin vX.Y.Z
```

Pushing the tag starts `.github/workflows/release.yml`. It will refuse a tag
whose version does not match the package, build all configured targets, and
publish only after every required artifact succeeds.

Publishing a stable GitHub Release triggers `publish-homebrew.yml`, which
updates `Formula/trail.rb` in `crabbuild/homebrew-tap`. Prerelease tags such as
`v0.2.0-rc.1` create a GitHub prerelease but do not replace the stable Homebrew
formula. The Homebrew workflow can be dispatched again with the stable tag if
the cross-repository push needs to be retried.

## Verify

After the workflow succeeds, verify the release from clean environments:

```sh
brew update
brew install crabbuild/tap/trail
trail --version
```

Also test the `trail-installer.sh` and `trail-installer.ps1` commands shown in
the GitHub Release. The reported version must match the tag.

If a release fails, fix the cause and rerun the failed workflow jobs. Do not
move or reuse a published tag. If assets were already exposed, publish a new
patch version.
