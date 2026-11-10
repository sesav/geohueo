# Releasing geohueo

## Prepare

1. Update `version` in `Cargo.toml`, then run `cargo check` to update `Cargo.lock`.
2. Move the `Unreleased` entries in `CHANGELOG.md` to the new version and update the links at the bottom.
3. Commit and merge the changes. Wait for CI to pass.

Before publishing, open **Release** -> **Run workflow** in GitHub Actions and choose your branch. This tests all five
builds without publishing a release. Download the files from that run and check them.

## Publish

Push a tag that matches the version in `Cargo.toml`, with a `v` prefix. For example, for version `0.3.0`:

```sh
git tag v0.3.0
git push origin v0.3.0
```

The tagged commit must include `.github/workflows/release.yml`. Old tags do not get binaries automatically. After all
builds and tests pass, the workflow publishes a GitHub Release with the binaries, checksums, and `install.sh`.

Versions such as `0.3.0-rc.1` are marked as prereleases. The installer skips them unless you use `--version`. The
install command in the README uses the latest stable release on GitHub.

The workflow does not publish to crates.io. Publish the Cargo package separately if needed.

## If something fails

Rerun the failed jobs. The publish step can fill an existing empty release. It keeps the release as a draft until all
files are uploaded. Releases that already have files are not changed; check a partial draft before retrying.
Do not delete or move a public tag. Use a new version if the code needs to change.
