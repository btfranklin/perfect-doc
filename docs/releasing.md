# Release Perfect Doc

This guide owns the release procedure. `Cargo.toml` owns the package version.
The [implementation plan](plan.md) owns acceptance evidence and publication
state. The [release notes](release-notes.md) describe the current release.

## Prepare the release

1. Update the package version in `Cargo.toml` when a new release needs one. Let
   Cargo update `Cargo.lock` for that change. Python and npm builders read the
   Cargo version.
2. Update the current release notes. Describe supported checks, target systems,
   and limits. Do not claim runtime support from a file signature or metadata
   test.
3. Run the [local checks](../README.md#local-checks). Commit and push the release
   source to `main`.
4. Wait for the push run on `main` to pass all three native jobs in the
   [CI workflow](../.github/workflows/ci.yml). Correct failures before tagging.
   CI stores the tested archives for 14 days. Create the release draft before
   those artifacts expire.

CI uses stable Rust, the latest stable Python and Node.js, and the locked project
dependencies. External actions use the latest verified major version tags.
These tags receive updates within that major version. The Linux runner uses
Ubuntu 24.04 as its build baseline. The Apple Silicon macOS runner uses
macOS 15 and deployment target 15.0. The Windows runner uses Windows Server 2025
and links the C runtime statically.

## Tag and promote

Check that the tag is unused, then tag the tested commit. For the first release:

```sh
git tag -a v0.1.0 -m "Perfect Doc 0.1.0"
git push origin v0.1.0
```

The [release workflow](../.github/workflows/release.yml) checks the tag against
the Cargo version and finds a completed, successful push run of CI on `main`
for the exact tag commit. It downloads the three native archives from that run.
The tag promotes those tested artifacts. The release workflow does not compile
the source or run CI a second time.

Each native CI job builds an archive, extracts it, and runs both a valid scan
and a broken-link scan with the extracted executable. It also checks the
included [native README](../packages/native/README.md) from the extracted
archive. The [archive builder](../scripts/release-artifacts.py) owns archive
layout, binary signature checks, version checks, and SHA-256 files.

CI generates dependency license notices once from the locked Cargo graph with
Cargo-about. It includes them and the Rust standard library MIT license in each
archive. The notice configuration is [about.toml](../about.toml); its text
template is [third-party-notices.hbs](../scripts/third-party-notices.hbs).

After all target checks pass, the workflow verifies the archive checksums and
creates a draft GitHub release with three archives and `SHA256SUMS`. It uses the
release notes from the tagged commit. Only this draft job has repository write
permission. The workflow does not publish a release or write to a package
registry.

Use the workflow's manual dispatch with the existing tag to retry or recover a
draft. The dispatch checks the same tag commit and retained successful CI
artifacts. It does not start a new build or create a new tag.

## Publish and verify

Review the draft notes, target checks, archive names, and checksums. Confirm that
the tag points to the tested source. Publish the reviewed draft:

```sh
gh release edit v0.1.0 --repo btfranklin/perfect-doc --draft=false
```

Download the published archives from GitHub and verify their SHA-256 checksums.
Run the archive for an available native target and check its version and scan
behavior. Record the hosted checks and public download evidence in the plan.

Homebrew formula updates and bottles use the
[tap maintenance procedure](https://github.com/btfranklin/homebrew-tap/blob/main/docs/maintaining.md).
That procedure owns bottle builds, publication, and public installation checks.
Registry publication has separate release gates in the plan.
