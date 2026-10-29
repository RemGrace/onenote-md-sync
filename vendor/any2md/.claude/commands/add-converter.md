Add a new converter for $ARGUMENTS format:
1. Read `src/converter/mod.rs` to understand the Converter trait interface
2. Read an existing converter (e.g., `src/converter/image_ocr/mod.rs`) as a template
3. Create `src/converter/$ARGUMENTS/mod.rs` implementing the Converter trait
4. Register the new module in `src/converter/mod.rs` and `src/lib.rs`
5. Add CLI dispatch logic in `src/main.rs`
6. Add unit tests in the new module with `#[cfg(test)]`
7. Add integration tests in `tests/`
8. Run `cargo test` and `cargo clippy -- -W clippy::all`
9. Update CLAUDE.md architecture section if needed
