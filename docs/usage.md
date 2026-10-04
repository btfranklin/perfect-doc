# Usage

This guide describes the current command. The [design](design.md) explains the
architecture. The [implementation plan](plan.md) records tested platform and
package evidence. The [check catalog](checks.md) lists the wider accepted scope;
it does not mean that every catalog entry is available.

## Commands

Run commands from the project that owns the documents:

```sh
perfect-doc check .
perfect-doc check docs/manual --format json
perfect-doc check . --online
perfect-doc rules
perfect-doc rules --json
perfect-doc init
```

`check` scans each supplied root. It uses the current directory when no root is
given. It reads `perfect-doc.toml` from the current directory when that file
exists. Use `--config PATH` to select another TOML file. Paths in the file are
relative to the directory that contains that file.

`--show-config` prints the effective configuration and does not scan documents.
`--fail-on info|warning|error` changes the severity that makes an invalid result
fail. The `--online` and `--offline` options override the configuration. They
cannot appear together.

`--no-banner` hides the ASCII banner in human-readable command output and help.
You can put this option before or after the command. The banner follows the
letter shapes of the Perfect Dark (BRK) font. It is built into the executable;
the font is not required at run time.

`init [PATH]` writes a default configuration. It uses `perfect-doc.toml` when no
path is given. It does not replace an existing file. `rules` lists the rule IDs
that this build supports. A rule can be disabled or require an explicit setting.

## Output and exit codes

A check writes its report to standard output by default. It writes status and
error messages to standard error. The check does not write a report file or a
cache by default. Set `--output PATH` to write the selected report to a file. Set
`network.cache` in the TOML file to store online evidence. The cache path is
relative to the TOML file. Offline checks do not use the network cache.

Select `--format human|json|junit|sarif`. JSON, JUnit, and SARIF are machine
reports. When you use `--output`, the report goes to that file and standard
output stays empty.

JSON, JUnit, SARIF, effective configuration, and saved reports do not include
the banner. Version output is also plain. Human-readable scan reports, rule
listings, and help include it unless `--no-banner` is set. The library report
writers do not add a banner.

The command uses these exit codes:

| Code | Meaning |
| --- | --- |
| `0` | The selected required checks passed at the configured failure level. |
| `1` | An invalid result reached the configured failure level. |
| `2` | The command, configuration, input root, or tool could not run. |
| `3` | A required check is unsupported or remains unverified. |

An invalid result at the failure level takes precedence over an incomplete
result. A result with code `0` does not prove a disabled check, an unsupported
format, external reachability in offline mode, or behavior that needs a browser.

Each diagnostic has a rule ID, severity, outcome, requirement source, source
location, message, and repair help. It includes a target when the check has one.
The report includes available rule IDs, enabled rule policies, disabled rules,
input patterns, and selected and parsed document counts. An enabled policy does
not prove that every format feature was present or verified. Related locations
show other source locations, such as the include that supplied the content.
Reports redact user information and query values in URI targets. Lines are
one-based. Columns count Unicode scalar values. Machine reports also retain
the byte offset.

## File selection and profiles

The default file extensions are `.md`, `.markdown`, `.okf`, `.html`, `.htm`, and
`.mdx`. `.okf` uses the Markdown reader. The default Markdown dialect is GFM;
the default heading anchor style is GitHub. HTML uses automatic document or
fragment mode. A scan excludes Git data, `node_modules`, Cargo `target`, Python
`.venv`, `.pytest_cache`, and `dist` directories by default. An empty selection
is incomplete unless `files.allow_empty` is true.

The TOML file can set include and exclude patterns, required paths, extension
mappings, file limits, link boundaries, Markdown rules, HTML mode, metadata
contracts, routes, navigation, assets, network limits, rule severity, and
exceptions. Unknown TOML fields and unknown rule IDs are errors. Use
`perfect-doc init` or `check --show-config` to inspect the available fields and
defaults.

Profiles apply selected format settings to paths that match a glob. For example:

```toml
[[profiles]]
glob = "legacy/**/*.md"
dialect = "commonmark"
anchors = "kramdown"

[[profiles]]
glob = "fragments/**/*.html"
html_mode = "fragment"
```

Profile entries can select a Markdown dialect, a heading anchor style, or an
HTML mode. Later matching entries can set another value. Supported Markdown
dialects are CommonMark, GFM, and MDX. Supported heading styles are GitHub,
Python-Markdown, Kramdown, and explicit IDs only.

A project can declare structural rules with a small contract:

```toml
include = ["README.md", "docs/**"]
required_paths = ["README.md"]

[markdown]
heading_start = 1
single_h1 = true
no_heading_skips = true
closed_fences = true
undefined_references = true

[files]
portable_names = true

[frontmatter]
required_fields = ["id", "language"]
id_field = "id"
language_field = "language"
schema = "doc-schema.json"
```

Use a local JSON Schema for nested field types, patterns, enums, conditional
fields, and value bounds. Schema references stay inside the configuration
directory and are not fetched over the network.

When `markdown.includes` is true, `!include[part.md]` inserts static Markdown.
Use `!include[part.md]{region=example}` with `<!-- region example -->` and
`<!-- endregion example -->`, or `{start=2 end=5}` for a line slice. Includes
combine heading anchors and preserve the original source location. Relative
references in an included file use that file's directory. Fragment-only links
use the combined host. A code fence with `json {file=example.json start=1 end=4}`
checks the file and range; it does not execute or insert that file as Markdown.

The scan checks only configured requirements. For example, it does not require
front matter, a single top-level heading, closed code fences, heading-level
rules, navigation coverage, or online checks unless the configuration enables
those requirements.

## Supported checks and limits

The current build checks supported Markdown syntax and declared heading,
reference, table, code-info, citation, and table-of-contents rules. It checks
front matter in YAML, TOML, and JSON; selected structured JSON, YAML, TOML, and
XML files; and local JSON Schema contracts for JSON, YAML, and TOML. XML
schema validation is reported as unsupported. It checks local links and anchors,
file names, routes, aliases, configured navigation, selected assets, and
static include expansion. Includes are off by default. The include reader keeps
original source locations and builds anchors from the combined content.

HTML checks cover supported parsing, selected structure and attributes, static
references, a subset of ARIA roles and values, selected static accessibility
relationships, language and landmark requirements, `srcset`, and encoding.
These checks do not implement the full HTML standard or the full ARIA model.
They do not compute accessible names, run a browser, execute scripts, or render
the page. HTML fragments that depend on dynamic values can remain unverified.

MDX checks do not execute JavaScript, JSX, or ESM. Dynamic MDX and template
expressions can remain unverified. The tool does not run a site generator. You
can scan output that your project has already generated when you configure its
routes and output directory.

Online mode checks HTTP and HTTPS destinations within configured limits. It
does not run JavaScript or prove that a destination will remain available. It
can check static fragments when enabled. A result is evidence from the time of
the check. Offline mode checks URI syntax only and makes no network request.

The [check catalog](checks.md) includes rules that are not yet available. Use
`perfect-doc rules` to see supported rule IDs for the current build.

## Test integration

Any test system can call `perfect-doc check .` and use its exit code. The
[Python, Node.js, and Make examples](../examples/integration/README.md) show
this process boundary. The examples accept `PERFECT_DOC` when the executable
is not on `PATH`.

Add the local Cargo package as a development dependency, then call the library
from a Rust test:

```rust
let result = perfect_doc::validate(
    &[std::path::PathBuf::from(".")],
    &perfect_doc::Config::default(),
)?;
assert!(result.is_valid(), "{}", perfect_doc::report::human(&result));
```

Use offline scans for repeatable local structure checks. Enable online checks
when the test needs current HTTP evidence. The repository's HTTP tests use
short-lived loopback fixtures; the validator itself is a command and library.

## Build and install

Build and run the native executable with Rust and Cargo:

```sh
cargo build --locked --release --bin perfect-doc
./target/release/perfect-doc check .
```

You can install a local Cargo build with `cargo install --locked --path .`.
Cargo's package is not published.
For a GitHub source install, use the command in the
[README](../README.md#build-and-install).

The project configures a Python wheel with Maturin's `bin` mode. Build a local
wheel with:

```sh
pdm install --dev --no-self
pdm run maturin build --release --locked --out dist/wheels
python3 -m pip install dist/wheels/*.whl
```

Install the wheel for your operating system and CPU. The wheel uses the same
native executable. It still needs a supported Python environment to install
and launch the command.

The npm builder stages a launcher package and one platform package, then runs
`npm pack` for both:

```sh
python3 scripts/package-npm.py
python3 scripts/package-npm.py --target aarch64-apple-darwin
```

The default target is the Rust host. Supported target labels are
`aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`,
`x86_64-unknown-linux-musl`, and `x86_64-pc-windows-msvc`. You need the Rust
target and any required cross linker to build a non-host target. The builder
checks the binary signature and refuses an unsupported or mismatched target.
You can pass `--binary PATH` with `--target` to stage an existing release
binary. This path also checks the binary signature.

For a local macOS Apple Silicon install:

```sh
npm install ./dist/npm/tarballs/perfect-doc-0.1.0.tgz ./dist/npm/tarballs/perfect-doc-darwin-arm64-0.1.0.tgz
npx --no-install perfect-doc check .
```

Use the package version from `Cargo.toml` in these file names. The builder
prints the actual archive paths.

The Python and npm packages are local build outputs. They are not published.
The npm package names and registry availability have not been checked. Only the
Apple Silicon macOS native binary, Python wheel, and npm packages have been run
locally. Other target shapes have package-metadata tests, but no cross-platform
runtime evidence. See the [implementation plan](plan.md) before treating a
target as supported for release.
