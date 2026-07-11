# Releasing Trail

GitHub Releases are the source of truth for Trail binaries. The release workflow
uses `dist` 0.32.0 to build platform archives, checksums, installer scripts,
GitHub attestations, and the Homebrew formula.

## One-time repository setup

1. Add a fine-grained token named `HOMEBREW_TAP_TOKEN` to the `crabbuild/trail`
   GitHub Actions secrets. It needs contents write access to
   `crabbuild/homebrew-tap`.
2. Add a fine-grained token named `RELEASE_PLEASE_TOKEN`. It needs contents,
   issues, and pull requests write access to `crabbuild/trail`. A dedicated
   token is required because tags pushed with the built-in `GITHUB_TOKEN` do
   not trigger follow-on GitHub Actions workflows; pull-request checks created
   that way may also require manual approval.
3. Allow GitHub Actions to create pull requests under repository Settings,
   Actions, General.
4. Protect `main` and require the normal CI, Release plan, and Release Readiness
   checks before merging.
5. Keep GitHub Actions workflow permissions able to create releases and artifact
   attestations.

The normal `GITHUB_TOKEN` publishes the release to `crabbuild/trail`. Only the
Homebrew tap requires a separate cross-repository token.

After adding both tokens, dispatch the `Release Automation` workflow once. The
bootstrap run publishes the current manifest version if its tag does not yet
exist, then later pushes maintain the normal release pull request.

The tap is shared with the Crab release. Trail publication creates or replaces
only `Formula/trail.rb`; the workflow refuses to commit if any other tap file
changes, so the existing `Formula/crab.rb` remains independent.

## Automated release flow

Use Conventional Commit prefixes for changes merged to `main`:

- `fix:` creates a patch release.
- `feat:` creates a minor release.
- `feat!:`, `fix!:`, or a `BREAKING CHANGE:` footer creates a major release.
- Other commit types can appear in release notes but do not request a version
  bump on their own.

Every push to `main` runs `release-automation.yml`. Release Please creates or
updates one release pull request containing the Cargo version bump, lockfile,
changelog, and release manifest. Merge that pull request after its required
checks pass.

The merge changes `.release-please-manifest.json`, so the same workflow creates
and pushes the matching annotated `vX.Y.Z` tag. That tag starts the generated
`release.yml` cargo-dist workflow. Cargo-dist builds and attests all platform
artifacts, then publishes the GitHub Release. Publishing the stable release
completes the generated Release workflow, whose `workflow_run` event starts
`publish-homebrew.yml` and updates `Formula/trail.rb` in the shared tap.

No manual version editing or tagging is required during the normal flow.

## Validate a release pull request

Release Please performs the version, lockfile, and changelog updates. CI then
runs the normal checks. To reproduce the release checks locally, run:

```sh
make release-check
dist plan --tag="v$(jq -r '.["."]' .release-please-manifest.json)"
dist generate --mode=ci --check
```

Install the pinned release tool when needed:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/axodotdev/cargo-dist/releases/download/v0.32.0/cargo-dist-installer.sh | sh
```

A successful stable-tag Release workflow triggers `publish-homebrew.yml`, which
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
patch version. For an emergency manual release, update the Cargo version,
lockfile, changelog, and release manifest together, merge those changes, and let
release automation create the tag.
