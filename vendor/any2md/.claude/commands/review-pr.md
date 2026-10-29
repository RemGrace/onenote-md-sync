Review pull request #$ARGUMENTS:
1. Fetch PR details with `gh pr view $ARGUMENTS`
2. View the diff with `gh pr diff $ARGUMENTS`
3. Check if tests pass: `cargo test`
4. Check linting: `cargo clippy -- -W clippy::all`
5. Check formatting: `cargo fmt -- --check`
6. Review for:
   - Error handling consistency (uses ConvertError, not panics)
   - Security (SSRF protection, input validation, no command injection)
   - Test coverage for new code
   - Naming conventions match project style
7. Summarize findings with actionable feedback
