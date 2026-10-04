# Accepted check catalog

This document owns the validation scope approved by B.T. Franklin. The
[design](design.md) defines the interpretation of format requirements, rendering
profiles, and project contracts. The [implementation plan](plan.md) owns feature
status. Inclusion in this catalog does not mean that a check is implemented or
enabled by default.

All checks use the selected format, reader, profile, and explicit project
contract. Checks that need a project policy are identified below. No check
evaluates prose quality, spelling, facts, topic coverage, or code behavior.

## 1. File discovery and scan coverage

- Find selected documentation files throughout the declared scan roots.
- Recognize the initial Markdown, HTML, and OKF extensions and configured aliases.
- Apply include and exclude patterns and record their effects.
- Report missing scan roots, unreadable directories, and unreadable files.
- Report unsupported formats selected for required validation.
- Detect broken symbolic links and symbolic link loops.
- Apply an explicit policy to directory links and paths outside scan roots.
- Detect an empty scan when the project expects documents.
- Detect source changes during a scan that prevent a consistent result.
- Retain coverage evidence for omitted, failed, and partially processed inputs.

## 2. File encoding and character validity

- Validate the declared or configured text encoding and detect invalid bytes.
- Detect NUL characters and prohibited control characters.
- Check HTML encoding declarations against the input encoding.
- Check byte order marks under a declared format or project requirement.
- Detect ambiguous or hidden characters in names, IDs, and URLs under a project
  policy. Ordinary prose is not subject to this policy.
- Optionally detect unresolved merge conflict markers outside literal examples.
- Preserve source locations when decoding changes byte and character positions.

These checks do not impose a preferred newline or whitespace style. A source
character that a parser can replace must still have a visible result when the
active policy prohibits that replacement.

## 3. File and directory names

- Check declared name patterns, extensions, and directory layout requirements.
- Detect prohibited names and characters on the project's supported platforms.
- Check Windows reserved names and prohibited trailing dots or spaces.
- Detect names that collide when letter case is ignored.
- Detect names that collide after Unicode normalization.
- Check declared path length limits.
- Check declared relationships between a file name, metadata identifier, slug,
  and publication path.
- Detect a file and directory path conflict in the publication layout.

Name style is a project requirement. A particular capitalization or separator
is not a universal document requirement.

## 4. Front matter syntax

- Recognize the selected YAML, TOML, or JSON front matter form.
- Check its permitted location and delimiter structure.
- Detect malformed syntax and duplicate mapping keys.
- Check the required top-level value shape.
- Detect repeated front matter blocks when the project permits only one.
- Handle YAML aliases and references with defined resolution and resource limits.
- Distinguish front matter delimiters from ordinary document constructs.

Presence, absence, and format are declared requirements. Front matter is not
required for every Markdown file by default.

## 5. Front matter contracts

- Check required, optional, and prohibited fields.
- Validate scalar types, null handling, arrays, nested objects, and unknown fields.
- Check allowed values, string patterns, numeric ranges, and collection sizes.
- Check declared array uniqueness requirements.
- Validate dates, times, URLs, local paths, and identifier formats.
- Check conditional field requirements and mutually exclusive fields.
- Check declared relationships, such as a start date preceding an end date.
- Resolve file references and document identifier references in metadata.
- Enforce declared uniqueness across the collection.
- Validate local schemas, schema references, and explicitly enabled format checks.

A metadata string can have a required type or syntax without any judgment of
its prose meaning. Remote schemas must not be fetched during an offline scan.

## 6. Headings and document hierarchy

- Check the permitted first heading level and top-level heading count.
- Check heading level changes under the declared hierarchy policy.
- Detect empty headings under the selected project rule.
- Check permitted nesting and structural shape.
- Apply separate expectations to full documents and included fragments.
- Check the resulting heading structure when includes combine fragments.
- Optionally detect empty sections by their structural node counts.
- Check explicit heading identifier structure through the selected renderer.

Skipping a heading level is a policy choice where the format permits it. Do not
require particular section topics. Repeated heading text is valid when the
renderer creates distinct anchors and no project rule prohibits repetition.

## 7. Markdown block structure

- Interpret headings, paragraphs, lists, block quotes, and code blocks according
  to the selected dialect.
- Check explicit fence closure when the project requires it.
- Check declared container nesting and block placement requirements.
- Check task list marker structure where that extension is enabled.
- Check table recognition, headers, delimiter rows, and alignment markers.
- Optionally check table row column counts as a project contract.
- Handle escaped delimiters, code spans, and literal text correctly.
- Report unsupported selected Markdown extensions rather than claiming coverage.

The reader must follow the format's parsing rules. CommonMark permits arbitrary
text and some unclosed constructs. A candidate marker does not prove that its
author intended markup. Marker heuristics require an explicit project policy.

## 8. Reference definitions

- Detect undefined link and image references where the profile declares that
  reference syntax should be checked.
- Detect duplicate definitions and conflicting destinations.
- Normalize reference labels according to the selected dialect.
- Detect undefined, duplicate, or conflicting footnote definitions.
- Check citations against a supplied bibliography where that extension is used.
- Optionally report unused link, image, footnote, or citation definitions.
- Distinguish literal bracket text from declared reference forms.

An unused definition is not automatically invalid. Duplicate definitions must
be evaluated under the format's precedence rules and the project's contract.

## 9. Anchors and identifiers

- Generate heading anchors with the selected renderer's rules.
- Validate explicit Markdown IDs and HTML IDs.
- Detect duplicate IDs and collisions between explicit and generated IDs.
- Handle repeated headings, Unicode, punctuation, escapes, and inline markup.
- Resolve fragments in the same document and in other documents.
- Check generated footnote and citation anchors.
- Check identifiers after includes combine content.
- Check declared public anchors that must remain available.
- Use renderer-specific matching, case, and fragment decoding rules.

An anchor preservation rule needs a supplied contract or comparison source.
The validator cannot infer an earlier public interface from the current tree.

## 10. Local link destinations

- Check that local target files exist and are readable.
- Resolve relative paths from the correct source location.
- Resolve root-relative paths from a declared source or publication root.
- Detect incorrect letter case on platforms whose filesystems ignore case.
- Handle percent encoding, Unicode, spaces, queries, and fragments correctly.
- Check directory destinations against configured index file rules.
- Apply symbolic link and root-boundary policies.
- Resolve paths under a selected source-to-publication mapping.
- Report ambiguous or unresolved template paths.

Do not treat the existence of a source `.md` file as proof that a linked `.html`
output exists. That relationship needs a declared mapping and the appropriate
source or build-output check.

## 11. URL and URI syntax

- Validate URI structure, schemes, hosts, ports, and escape sequences.
- Handle absolute, relative, and protocol-relative references.
- Check permitted or prohibited schemes under the project policy.
- Validate the syntax of supported `mailto:`, `tel:`, and custom schemes.
- Validate `data:` URI media types and encoding, including Base64 where used.
- Detect unsupported template expressions that prevent complete resolution.
- Apply the relevant international host and path encoding rules.

These checks do not prove email delivery, telephone service, resource
reachability, or intended destination content.

## 12. Publication paths and routes

- Apply declared source-to-publication mappings.
- Check extension changes, index pages, base paths, and trailing slash rules.
- Detect duplicate publication paths, slugs, and aliases.
- Detect conflicts between page routes and directory routes.
- Check redirect target existence and redirect loops.
- Check canonical references against the declared publication map.
- Check alternate page references and language identifier syntax.
- Check declared relationships and duplicate mappings between language variants.

Route checks do not judge whether translated prose is accurate or complete.
Unknown generator behavior requires a supplied map or built output.

## 13. Navigation and collection structure

- Check references in navigation files, indexes, catalogs, and tables of contents.
- Detect references to missing documents or missing anchors.
- Check required navigation membership under a project contract.
- Detect prohibited duplicate entries.
- Detect parent cycles when navigation must be a tree.
- Optionally check reachability from declared entry points.
- Optionally detect documents and assets with no incoming references.
- Check the structural order and coverage of a generated table of contents
  against its declared source headings.

Ordinary document links can form valid cycles. An unreferenced document can be
intentional. Reachability and unused-resource rules need an explicit policy.

## 14. Includes, embeds, and document extensions

- Check include and embed target existence and permitted target types.
- Detect recursive includes and apply depth limits.
- Validate included line ranges and named regions.
- Check region marker pairs and duplicate region names.
- Resolve wiki links and detect ambiguous targets.
- Validate supported directives, admonitions, and shortcode attributes.
- Check extension-specific nesting and block boundaries.
- Parse supported MDX or JSX structure without executing expressions.
- Retain source mappings through supported composition steps.

Do not assume an extension's behavior from its appearance. A selected reader or
profile must define it. Runtime expressions can remain unverified.

## 15. Code block metadata and structured examples

- Check declared code block language names and accepted aliases.
- Validate supported block attributes, identifiers, file names, and line ranges.
- Check references to local external example files.
- Parse explicitly selected JSON, YAML, TOML, XML, or other structured examples.
- Apply a local schema when the project declares one for those examples.
- Respect explicit exceptions for examples that demonstrate invalid structure.
- Preserve literal example boundaries during other reference and markup checks.

These checks do not compile or execute ordinary code examples. They do not
judge whether the example produces correct behavior.

## 16. HTML syntax and conformance

- Check source parse errors, tag syntax, attributes, comments, and character
  references.
- Detect duplicate attributes.
- Check permitted element children, parents, ordering, and occurrences.
- Check required attributes and permitted values.
- Detect prohibited nesting, including nested forms and prohibited interactive
  descendants.
- Apply void-element and raw-text element rules.
- Apply the correct optional start-tag and end-tag rules.
- Distinguish complete pages from fragments and check required page structure.
- Handle declared custom elements and embedded SVG or MathML requirements.
- Apply supported obsolete-element restrictions from the selected standard or
  project policy.

An HTML parser's successful recovery does not establish source conformance.
HTML and XML-style serialization need their respective requirements.

## 17. HTML references and forms

- Check label `for` targets, control `form` targets, and input `list` targets.
- Check table `headers` references and image map references.
- Check ARIA attributes that refer to element identifiers.
- Validate supported roles and permitted attributes, token lists, and values.
- Check required control associations that can be established from static source.
- Validate input type and attribute combinations.
- Check declared uniqueness requirements for form and map names.
- Check reference target types where the applicable standard requires a type.
- Check the combined result of included fragments.

Dynamic DOM changes cannot be inferred from a static source scan.

## 18. Structural accessibility requirements

- Check required image alternative-text attributes and explicit decorative-image
  cases.
- Check the presence of supported accessible-name sources where static source
  establishes the requirement.
- Check table header associations, `scope`, and row or column span structure.
- Check required document language declarations and language identifier syntax.
- Check declared landmark structure.
- Check supported structural control and description relationships.

These checks do not judge alternative-text quality, contrast, visual layout,
keyboard behavior, or overall accessibility. No clean scan establishes full
accessibility compliance.

## 19. Images, media, and linked assets

- Check image, stylesheet, script, font, audio, video, poster, and caption targets.
- Extract URL-bearing attributes according to the supported element rules.
- Parse `srcset` candidates and descriptors.
- Check expected asset types against available file information.
- Parse supported local image or media files to detect malformed structure.
- Check declared image dimensions against local asset dimensions.
- Check SVG symbol and fragment references.
- Resolve URLs in supported CSS, including `url()` and `@import`.
- Check declared local asset integrity hashes.
- Apply corresponding checks to inline HTML and supported embedded resources.
- Decode supported data resources before checking their declared structure.

Asset existence does not establish that an image looks right or a script works.
Remote asset data requires online mode and the applicable network policy.

## 20. Embedded data and supporting manifests

- Parse supported embedded JSON, JSON-LD, YAML, TOML, and XML.
- Check declared local schemas and references.
- Check bibliography, document catalog, and navigation manifest structure.
- Check sitemap syntax and known route references.
- Check schema reference existence, resolution, and declared resource limits.
- Report remote schema dependencies that cannot be resolved offline.

Structured data checks do not prove factual claims in JSON-LD or metadata.

## 21. Generated documentation

- Scan supplied built HTML in addition to source documents.
- Check supplied source-to-output mappings.
- Check expected output files, routes, anchors, and assets.
- Detect references that work in source but fail in the supplied build output.
- Check deployment base paths and directory layout.
- Detect unresolved template values under a declared output policy.
- Preserve source mapping information when the generator supplies it.

The project supplies the output. The validator does not run a generator or
assume that a successful source scan proves a successful deployment.

## 22. External links in online mode

- Check DNS, connections, TLS certificate validation, and HTTP responses.
- Follow redirects within declared limits and detect loops.
- Use a bounded GET fallback when HEAD is not supported.
- Check static fragments in supported returned documents.
- Check expected resource types where declared.
- Use explicitly configured authentication sources.
- Apply timeout, retry, concurrency, host, redirect, and request count limits.
- Expire cached results and retain the time and origin of network evidence.
- Distinguish missing destinations from authentication blocks, rate limits, and
  temporary failures.
- Report anchors that require JavaScript as unverified where static inspection
  cannot establish them.
- Deduplicate network work while retaining each source reference location.

Treat `404` and `410` as missing-resource evidence where the response applies to
the requested destination. Authentication, rate limits, and transient failures
must not be classified as a definite missing resource. A successful response
proves reachability at check time, not intended content or permanent availability.

## 23. Configuration and exceptions

- Reject unknown rule identifiers and malformed settings.
- Validate file patterns, profile mappings, local schemas, and schema references.
- Report required selection patterns that match no inputs.
- Detect malformed or unmatched suppression directives.
- Check exception scope and any declared expiry policy.
- Optionally detect exceptions that no longer suppress a failure.
- Record skipped checks and their reasons.
- Reject contradictory required-check and execution-mode settings.
- Preserve the difference between diagnostic severity and verification state.
- Fail when a required result is unavailable, incomplete, or unsupported.

The coverage report must make it possible to see which enabled checks were
completed and which exceptions affected the result.

## Growth of the catalog

Add a rule only when its structural requirement, evidence, failure behavior,
and supported profiles can be stated clearly. Use a parser for new formats.
Avoid broad regular-expression rules that guess an author's intent. Record
new exclusions and uncertainties in the design or plan before claiming support.

For each implemented rule, supply valid and invalid fixtures and the relevant
ambiguous cases. Check source locations, exceptions, profile differences, and
collection effects. Update the plan's capability status when acceptance passes.
