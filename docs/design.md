# Perfect Doc design

This document owns product intent and architecture. The [check catalog](checks.md)
owns the full accepted scope. The [usage guide](usage.md) describes the current
command. The [implementation plan](plan.md) owns current status, evidence, and
release gaps.

## Product intent

Perfect Doc checks the structure of a documentation collection. It checks
documents and the relationships between them. Its output can help a test system
in any language decide if a documentation check passed.

The product checks syntax, references, assets, declared metadata, and collection
contracts. It does not judge spelling, grammar, writing style, factual accuracy,
topic coverage, or code behavior. It does not require sections with particular
topics. It does not use an AI service or execute document code.

The name refers to the game Perfect Dark. This choice does not change the
product's technical scope.

## Requirements and profiles

A diagnostic uses one of four requirement sources:

| Source | Meaning |
| --- | --- |
| Format | A requirement of the selected document format or dialect. |
| Profile | A declared interpretation, such as anchor generation or HTML mode. |
| Project | A rule chosen by the project, such as required metadata or heading level. |
| Execution | A read, parse, resource, or network condition that affects completion. |

A report identifies the requirement source. It must not present a project rule
as a universal format rule.

The default document extensions are Markdown (`.md` and `.markdown`), OKF as
Markdown (`.okf`), HTML (`.html` and `.htm`), and MDX (`.mdx`). The default
Markdown dialect is GFM. The default anchor style is GitHub. Users can map other
extensions to a supported reader. A path profile can select a Markdown dialect,
anchor style, or HTML document or fragment mode.

Markdown is permissive. A candidate marker does not prove that the author meant
it as markup. A project can enable rules such as closed code fences, heading
shape, required metadata, or table column counts. The scanner checks only the
rules that apply to the selected format and configured contract.

## Current implementation boundary

The library scans local files, parses supported Markdown and HTML structure,
checks configured metadata and structured data, resolves references, and builds
collection relationships. It can check local assets, routes, aliases,
navigation, static includes, and generated output supplied by the project.
Includes are off by default. They preserve original source locations. Anchor
checks use the combined host content after expansion.

The HTML reader checks a selected subset of syntax, structure, attributes,
references, ARIA values, language, landmarks, and static accessibility
relationships. It does not implement the full HTML standard or full ARIA model.
It does not compute accessible names, run JavaScript, launch a browser, or render
a page. Dynamic values can make a result unverified.

The MDX reader does not execute JavaScript, JSX, or ESM. A structure that needs
execution remains unverified. Template values are not replaced with guessed
destinations.

Online mode checks HTTP and HTTPS destinations within configured limits. It
does not execute remote content. A static fragment check cannot prove content
that a browser creates at run time. Offline mode makes no network request and
does not claim external reachability.

The [check catalog](checks.md) records scope beyond the current implementation.
Use `perfect-doc rules` to inspect the rule IDs available in a build. A listed
catalog item does not establish that the implementation covers every case in
that family.

## Architecture

The project has one Cargo package with a Rust library and a command executable.
The library owns configuration, scan state, diagnostics, rule definitions, and
the report model. It does not print or exit the process. The command parses
options, calls the library, selects a report format, writes output, and returns
an exit code.

The scan follows these steps:

1. Load and validate the TOML configuration.
2. Find the selected documents and supporting files within the declared limits.
3. Decode and parse each selected document with its format profile, then check
   its front matter contract.
4. Index publication routes and check original include targets.
5. Expand enabled static includes while preserving original source locations.
6. Check the combined headings, structured examples, and source characters.
7. Resolve local links and anchors, then check collection rules, assets, and
   supporting data.
8. Run requested online checks, apply rule policy and exceptions, retain scan
   coverage, and sort diagnostics. The command selects the exit code.

Readers parse source. Resolvers find local and external destinations. Rule
modules check the parsed structure and project contracts. Report writers show
the completed result; they do not rescan or change it.

The source modules own these parts of the scan:

| Module | Responsibility |
| --- | --- |
| `config` | Typed TOML settings, defaults, profiles, and configuration checks. |
| `scan` | Discovery, bounded reads, source checks, cancellation, and rule policy. |
| `model` | Sources, source maps, parsed records, diagnostics, coverage, and reports. |
| `markdown_reader`, `html_reader` | Format parsing and static structure extraction. |
| `includes`, `headings` | Static composition and final document hierarchy. |
| `metadata` | Metadata contracts, local schemas, and structured examples. |
| `links`, `collection` | Reference resolution, routes, navigation, and document graphs. |
| `assets`, `online` | Selected local asset checks and bounded HTTP evidence. |
| `report`, `main` | Report rendering and the command process boundary. |

`Source` keeps the original path and UTF-8 bytes. Include composition maps each
inserted source slice back to its original source and include location.
`ParsedDocument` records static structure and reader completion. `Document`
adds metadata and a primary publication route. `Report` keeps findings and
coverage separate. The public `validate` function returns a report or a
`ScanError`; `validate_with_cancel` accepts a cancellation flag.

## Diagnostic and result contract

Every diagnostic identifies a rule, severity, result, requirement source, source
location, message, and repair help. A diagnostic includes a target when the
check has one. Related locations identify additional source locations. URI
targets redact user information and query values. Lines and columns are
one-based. Columns count Unicode scalar values. Machine reports also include
a zero-based byte offset.

Check completion is separate from finding severity. An invalid result can fail
at the configured severity threshold. A required unsupported or unverified
check cannot be reported as complete. A disabled rule or offline external link
is not evidence that the requirement passed.

| Code | Meaning |
| --- | --- |
| `0` | Selected required checks passed at the configured failure level. |
| `1` | An invalid result reached that level. |
| `2` | Invocation, configuration, input setup, or tool failure. |
| `3` | A required check is unsupported or unverified. |

An invalid result at the failure level takes precedence over an incomplete
result. Reports are human-readable, JSON, JUnit, or SARIF. The command writes a
report to standard output unless `--output` selects a file. A normal scan does
not create a report file or cache by default. An online cache is written only
when `network.cache` names a path in the configuration.

## Configuration and execution

The project configuration is TOML. It can select input patterns, formats,
profiles, metadata contracts, routes, navigation, assets, rules, exceptions, and
network policy. Unknown fields and unknown rule IDs are errors. Relative
configuration paths use the directory that contains the configuration file.

The repository applies its own structural rules through
[`perfect-doc.toml`](../perfect-doc.toml).

The default scan is offline. `--online` enables network checks, and `--offline`
disables them. Network requests have limits for timeout, concurrency, count,
redirects, response size, and retries. Host allow and exclude lists can narrow
the policy. Authentication values come from named environment variables and
must not appear in reports or go to an unrelated redirect host. The cache is
optional and has an expiry time.

The scanner never runs a site generator. A project can supply generated files
and configure route checks to inspect them. It does not rewrite source files.

## Build and package boundaries

Build the native executable with Cargo. The Python wheel uses Maturin's `bin`
binding. The npm launcher starts the same native executable. Neither package has
a second validation implementation. A Python installation needs Python to
install and run its command. The npm launcher needs Node.js. The native
executable itself does not need either runtime.

The local package builder creates private npm tarballs in `dist/npm`. It selects
one platform package per target and records operating system, CPU, and Linux
libc constraints. The configured target set is not the same as a verified
release matrix. The [plan](plan.md) records which native and package targets
have been tested. Package names are not confirmed as available in a registry.
Perfect Doc uses the [MIT license](../LICENSE). The native package and the
Python and npm packages declare this license. No package has been published.

See the [usage guide](usage.md) for exact local build commands and installation
limits.
