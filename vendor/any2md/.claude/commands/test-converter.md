Test the $ARGUMENTS converter thoroughly:
1. Read the converter source in `src/converter/$ARGUMENTS/`
2. Read existing tests (both unit tests in the module and integration tests in `tests/`)
3. Run `cargo test` to see current test status
4. Identify untested edge cases and error paths
5. Write new tests to improve coverage
6. Run `cargo test` again to verify all tests pass
7. Run `cargo clippy -- -W clippy::all` to ensure no warnings
