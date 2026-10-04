# Agent guide

Read [README.md](README.md) first. Use the [usage guide](docs/usage.md) for
commands and configuration, the [design](docs/design.md) for architecture, the
[check catalog](docs/checks.md) for accepted scope, and the
[implementation plan](docs/plan.md) for current status and release evidence.

## Work rules

- The project owner is B.T. Franklin.
- Work on `main`. Create a branch or pull request only if the owner asks.
- Write technical text and comments in ASD-STE100 Simplified Technical English.
- Check document structure. Do not add prose grading, spelling checks, topic
  requirements, AI calls, or document-code execution.
- Keep one Rust package. The library owns validation and reports. The executable
  owns command handling, report output, and exit codes.
- Use stable Rust and verify current crate releases before adding dependencies.
  Keep `Cargo.lock` in version control.
- Keep changes direct. Do not add internal schema versions, compatibility
  layers, legacy paths, or a plugin framework without a demonstrated need.
- Do not report success for missing, unsupported, or incomplete required checks.
- Give each diagnostic its source location, rule, result, message, repair help,
  and target when one applies. Redact user information and query values in URI
  targets.
- A normal scan writes its report to standard output. It does not write a report
  file or cache unless the user selects `--output` or configures
  `network.cache`. `init` writes a configuration file only at its selected path.
- Keep package wrappers thin. They must call the same native executable.
- Keep unrelated user changes intact. Do not commit, publish, create a remote,
  or configure hosted CI unless the user asks.

## Evidence and maintenance

- Keep facts in their authoritative documents. Update the plan when support or
  acceptance evidence changes. Update the design when a contract changes.
- Separate source support, local tests, target runtime tests, and publication
  state. A metadata test does not prove that a package runs on that target.
- Do not describe a check catalog entry as implemented unless code and tests
  provide evidence for the claimed behavior and limits.
- Use self-authored permanent test fixtures. Checks against other repositories
  are for development only. Do not store their paths, content, cases, or results
  in this repository.
- Test implemented behavior and actual failure boundaries. Do not add tests for
  hypothetical behavior that the code does not implement.

## Validation commands

Use the commands in [README.md](README.md#local-checks). They cover Rust source,
the process boundary, local package building, and language integrations. Do not
install tools during a test run.

For a rule change, read its catalog entry and the relevant format standard. Add
tests for valid, invalid, and ambiguous input. Check source locations, profile
behavior, result status, and exit codes where they apply. Record unsupported
cases in the plan.

For packaging, read the [release procedure](docs/releasing.md), installation
design, and release gates. Verify package
names, licenses, and target support before publication. The Cargo package is
currently marked `publish = false`.
