# Handsoff

Handsoff is a production GitHub issue runner that proposes surgical, scope-faithful changes to real repositories.

## API contract tests

The published API operations are gated on OpenAPI response conformance. Contract tests live alongside the service boundary and validate, for each operation in scope:

- status codes
- required fields
- nullability
- error response shapes

A deliberate schema mismatch fails the test with the operation name and the offending field, so regressions are easy to locate. Generated fixtures are never used as a substitute for implementation-level checks — the tests exercise the real handlers.

### Adding an operation to the contract suite

1. Add the operation to the versioned OpenAPI specification.
2. Add a contract test for the operation next to the existing ones, asserting status codes, required fields, nullability, and error response shapes against the real handler.
3. Run the focused contract suite locally and confirm it passes in CI.

## Contributing

Keep changes surgical and inside the scope of the assigned issue. Do not install dependencies, run builds, or reformat unrelated code as part of a fix.
