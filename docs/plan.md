# Implementation plan

This document owns current status, acceptance evidence, release gates, and open
decisions. The [design](design.md) owns architecture. The [usage guide](usage.md)
owns command details. The [check catalog](checks.md) owns the wider accepted
scope. Catalog entries are not proof of implementation.

## Current state

Perfect Doc has a working validator and command. The implementation covers a
useful subset of the accepted structural checks. It does not implement full HTML
conformance, computed accessibility, browser rendering, or all catalog rules.

| Area | State |
| --- | --- |
| Rust library and command | Implemented; the library returns reports and the command handles output and exit codes. |
| Offline scan | Implemented for supported Markdown, HTML, metadata, references, assets, and collection settings. |
| Static includes | Implemented as an opt-in feature with original source locations and combined anchors. |
| Online HTTP checks | Implemented as an opt-in feature with limits and an optional explicit cache. |
| Reports | Human, JSON, JUnit, and SARIF formats are available. |
| Python and npm packages | Local build paths are implemented. They are not published. |
| Full accepted check catalog | Incomplete; see [usage limits](usage.md) and [checks](checks.md). |
| Native target runtime checks | Passed on Linux x64, Windows x64, and Apple Silicon macOS. |
| Remote repository | Public source is hosted at `btfranklin/perfect-doc` on GitHub. |
| Hosted CI | Passed on Linux x64, Windows x64, and Apple Silicon macOS. |
| GitHub native release | [v0.1.0](https://github.com/btfranklin/perfect-doc/releases/tag/v0.1.0) is public with four native archives and SHA-256 checksums. |
| Homebrew | Public tap and bottles for Apple Silicon macOS and Linux x64; see [verification](#homebrew-verification). |

Each diagnostic has a source location, rule, result, message, and repair help.
It has a target when the check has one. Reports redact user information and
query values in URI targets. The output contract is described in the
[design](design.md).

## Local verification

The local host is Apple Silicon macOS. The native executable, Python wheel, and
npm packages have been built and run on this host. Package metadata tests cover
the configured target shapes, but they do not prove that other targets run.
Do not claim Windows or Linux runtime support from these checks.

The local source checks passed:

- 210 Rust tests, including property tests, controlled HTTP tests, source map
  checks, report checks, and public library and command tests.
- 15 Python package, launcher, and native archive tests after Intel macOS support was removed.
- 140 repeated HTTP fixture runs with concurrent test processes after the
  accepted socket handling fix.
- Python, Node.js, and Make integration examples with the native executable.
- Rust formatting, Clippy with warnings denied, and API documentation.
- The repository's own documentation scan.
- GitHub source installation in a temporary prefix, followed by an offline
  scan with the installed executable.
- A native archive with dependency notices, an extracted executable, a scan
  of its README, a valid document scan, and an actionable broken-link result.

The locally verified tool versions were:

| Tool | Version |
| --- | --- |
| Rust and Cargo | `1.99.0` |
| Python | `3.14.8` |
| PDM | `2.29.2` |
| pytest | `9.1.1` |
| Maturin | `1.15.0` |
| Node.js | `26.10.0` |

These are local observations from 2026-10-03 in `America/Phoenix`. Check tool
versions again before a release.

Use the dependency setup and validation commands in the
[README](../README.md#local-checks). Run them from the repository root. The
README owns these commands; this plan records their acceptance evidence.

The language examples call the same native executable. They do not contain a
second validator. The Python and npm build instructions are in the
[usage guide](usage.md). Build artifacts stay in ignored `dist/` and `target/`
directories.

## Hosted verification

The native source and archive checks passed in
[CI run 37173813097](https://github.com/btfranklin/perfect-doc/actions/runs/37173813097)
for commit `cd05572c55cfdc6ced9bd2b55f889a01a45a20b0`. Each target ran the Rust
checks, repository scan, Python, Node.js, and Make examples, package tests, and
an extracted native release executable. The archive checks verified its version,
included README, a valid scan, and an actionable broken-link result. Each archive
contains the project license and generated dependency notices.

The runners were Ubuntu 24.04 x64, Windows Server 2025 x64, macOS 15 Apple
Silicon, and macOS 15 Intel. The Windows executable links the C runtime
statically. Its imported libraries do not include Visual C++ runtime DLLs.
These checks prove the native executable on those systems. They do not prove
Python wheel or npm installation on all four targets.

The release executable build steps in the initial tagged workflow took 55
seconds on Linux, 77 seconds on Windows, 92 seconds on Apple Silicon, and 123
seconds on Intel macOS. Environment setup, cache restore, and source checks add
time. The release workflow now copies the tested archives from a successful CI
run for the exact tag commit. It does not compile that commit a second time.

The [archive transfer run](https://github.com/btfranklin/perfect-doc/actions/runs/37175029397)
passed with an 11-second draft job. It selected the source CI run above, checked
the tag against Cargo, checked the complete target archive set, and verified
all checksums. The `v0.1.0` tag still points to that tested source commit.

The release was published on 2026-10-03 in `America/Phoenix`. All five public
assets were downloaded without credentials. All four archive checksums and all
five GitHub asset digests matched. The public Apple Silicon archive also passed
version, README, valid scan, and actionable broken-link checks on local macOS
27.0. The public Windows executable's imported libraries confirmed static C
runtime linking. Linux, Windows, and Intel macOS runtime evidence comes from
the native CI jobs that produced those exact released archives.

The results above describe the original v0.1.0 release. Current macOS support
requires Apple Silicon. Intel macOS is no longer supported and has no active
build or package target. The original public release assets remain available.

The [current native CI run](https://github.com/btfranklin/perfect-doc/actions/runs/37180310147)
passed for commit `c3ed1049ab701d763c423d11d404d8eaf80280f6` on Apple Silicon
macOS 15, Ubuntu 24.04 x64, and Windows Server 2025 x64. It checked Rust source,
language integrations, package behavior, and extracted native archives. These
new archives are CI outputs; the published v0.1.0 assets remain unchanged.

## Homebrew verification

The [public tap](https://github.com/btfranklin/homebrew-tap) provides the stable
Perfect Doc 0.1.0 formula and prebuilt bottles. The formula uses the public
v0.1.0 source archive. Its downloaded SHA-256 matched the formula checksum.
macOS requires Apple Silicon; Intel macOS is not supported.

The [bottle build run](https://github.com/btfranklin/homebrew-tap/actions/runs/37180069167)
passed for the reviewed formula commit
`02d6d5a620644445480a99c57c0b1c3770b070cb`. Both target jobs used Homebrew
test-bot to build the source, create bottles, install those bottles, check
linkage, and run the formula tests. The tested targets were Apple Silicon macOS
15 (`arm64_sequoia`) and Ubuntu 24.04 x64 (`x86_64_linux`).

The [publication run](https://github.com/btfranklin/homebrew-tap/actions/runs/37180565946)
passed. Homebrew published those tested files, added their checksums to the
formula, and created attestations. Both
[public bottles](https://github.com/btfranklin/homebrew-tap/releases/tag/perfect-doc-0.1.0)
were downloaded without credentials. Their SHA-256 checksums matched the formula
and GitHub asset digests. Both attestations verified against the tap repository.

The [public installation run](https://github.com/btfranklin/homebrew-tap/actions/runs/37180985423)
passed on both targets on 2026-10-03 in `America/Phoenix`. Both fresh runners
started with no registered tap or installed Perfect Doc. The checks confirmed
the public tap origin and required `poured_from_bottle` in Homebrew's installed
metadata. Both targets passed version output, a valid scan, a broken-link scan
with repair help and source location, `brew test`, reinstall by the short name,
and uninstall. The public bottles need no Rust compiler. Python is a test-helper
dependency in the Linux workflow, not a Perfect Doc runtime dependency.

The [tap maintenance guide](https://github.com/btfranklin/homebrew-tap/blob/main/docs/maintaining.md)
owns the bottle and public installation procedure. Upgrade verification needs
the next real source release. It has not been run for the first release.

On 2026-10-04 in `America/Phoenix`, the tap release process changed to use
`main` directly. The [new bottle build run](https://github.com/btfranklin/homebrew-tap/actions/runs/37232849181)
passed both target jobs for commit
`a0f43843343ca62bbc5c9cdfc4d3aad8748365c3`. Eight bottle validation tests passed
locally and on the macOS runner. They cover missing artifacts, wrong release
metadata, checksum failures, duplicate targets, and Homebrew rebuild filenames.

The [publication preview](https://github.com/btfranklin/homebrew-tap/actions/runs/37233527862)
passed. It checked the current tested commit, downloaded both artifacts,
verified their metadata and SHA-256 checksums, prepared the bottle formula,
and passed Homebrew style and strict online audit checks. The
[existing-version check](https://github.com/btfranklin/homebrew-tap/actions/runs/37233653620)
stopped as required because the 0.1.0 bottle release already exists. All upload,
attestation, and push steps were skipped. The public asset digests stayed
unchanged. The new publication workflow will upload and commit directly to
`main`; that path needs a new source release for its first complete runtime
check. No branch, pull request, or new release was created for this change.

## Capability work

The original staged plan is now a status map. Core scanning, configuration,
reports, collection checks, includes, and bounded HTTP checks exist. Work remains
to expand format coverage and test each accepted rule against valid, invalid,
and ambiguous structures. Do not use the catalog family count as a measure of
implementation coverage.

Known limits include:

- HTML checks cover selected syntax and static relationships. They do not cover
  the full HTML or ARIA standards, computed accessible names, or rendering.
- MDX and template expressions are not executed. Dynamic structure can remain
  unverified.
- Online checks use HTTP and HTTPS. They do not execute remote code or prove
  future availability.
- Includes require configuration. Their source maps and combined anchors have
  local tests, but further format and nested edge cases remain part of ongoing
  validation.
- Source maps support UTF-8. A selected other HTML encoding is reported as
  unsupported. Includes require static local file targets and UTF-8 slices.
- Image decoding supports the formats enabled in `Cargo.toml`. Other detected
  image formats are reported as unsupported. Audio, video, and font checks cover
  selected headers and reference types, not full media decoding.
- CSS checks parse tokens and references. They do not validate every property or
  stylesheet rule against the complete CSS specifications.
- XML syntax checks do not implement XML schema validation. A JSON Schema
  contract for XML is reported as unsupported.
- The accepted catalog contains checks that the current implementation does
  not provide.

Update this plan when support or evidence changes. Record the command, host
scope, and result. Do not turn a package shape check into runtime evidence.

## Distribution and release gates

The Python wheel uses Maturin `bin` mode. The npm builder creates a private
launcher package and one private native package per selected target. Both call
the same executable. The configured targets are:

| Rust target | npm platform package suffix | Native runtime evidence |
| --- | --- | --- |
| `aarch64-apple-darwin` | `darwin-arm64` | Local macOS and macOS 15 CI. |
| `x86_64-unknown-linux-gnu` | `linux-x64-gnu` | Ubuntu 24.04 x64 CI. |
| `x86_64-unknown-linux-musl` | `linux-x64-musl` | Not run. |
| `x86_64-pc-windows-msvc` | `win32-x64` | Windows Server 2025 x64 CI. |

The package names are candidates. Registry availability is unknown. The Cargo
manifest sets `publish = false`. The owner selected the [MIT license](../LICENSE).
Cargo and Python metadata declare MIT. Both npm packages declare MIT and include
the license text. No wheel or npm package has been published.

Follow the [release procedure](releasing.md) for native GitHub releases. Before
registry publication, verify package names, select release identities, and obtain
the owner's authorization for registry access. Build and run each
claimed platform package on its target system. Verify offline use, dependency
requirements, package contents, and recovery instructions. Do not configure a
remote, hosted CI, or publication workflow without a user request.
