Fix GitHub issue $ARGUMENTS:
1. Fetch issue details with `gh issue view $ARGUMENTS`
2. Find relevant code in the codebase
3. Implement the fix
4. Run `cargo test` to verify
5. Run `cargo clippy -- -W clippy::all` to lint
6. Commit with a descriptive message referencing the issue
