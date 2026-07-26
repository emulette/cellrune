# Security Policy

## Supported versions

CellRune is at `0.1.2`. Fixes are published on the latest release only.

## Reporting a vulnerability

Report privately through GitHub's private vulnerability reporting on this repository:
open the **Security** tab and choose **Report a vulnerability**. Please do not open a public
issue for a suspected vulnerability.

A useful report includes the affected version and target, the smallest input that reproduces the
problem, what you observed, and what you expected. A workbook that reproduces the problem is the
most useful attachment, so please strip anything confidential from it first. If you cannot share
the file, the relevant part names and the failing formula usually suffice.

## Scope

CellRune parses untrusted spreadsheet input, so the following are in scope:

- memory-safety failures, panics, or non-termination reachable from the public API with a crafted
  `.xlsx` or `.xlsm` file;
- resource exhaustion that survives the configured `ReadLimits`, `CalculationLimits`, or
  `WriteLimits`, including decompression amplification and unbounded allocation;
- reads or writes outside the directory capability granted to the MCP server, or any path
  containment failure in `cellrune-mcp`;
- writes that destroy a destination the write options say must be preserved.

The following are outside scope:

- results that differ from Excel without a safety impact; report those as ordinary issues, since
  complete Excel compatibility is not claimed;
- workbook content returned to a caller that is then treated as trusted input. Cell text, sheet
  names, and defined names are third-party data and are returned verbatim by design, including
  through the MCP server. Treating that content as instructions is the consuming application's
  risk to manage;
- limits configured explicitly by the caller above the defaults;
- vulnerabilities in dependencies, which belong with the dependency, though a report about how
  CellRune exposes one is welcome.

## Handling

Reports are acknowledged, and an assessment with a fix or mitigation plan follows. Reporters are
credited in the release notes unless they ask otherwise.
