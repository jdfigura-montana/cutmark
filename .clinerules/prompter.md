# Prompter task rules

Applies only to tasks whose prompt starts with "Prompter Phase". Other tracks ignore this file.

- Memory bank: read and update only memory-bank/prompter.md, kept under 6 KB. Never read or edit memory-bank/activeContext.md, memory-bank/progress.md or memory-bank/archive/.
- Read only the files and spec sections the prompt names. No repo-wide searches. If you must open something else, say so in the report.
- Stay in the layer the prompt names. If another crate needs a change, stop and report it.
- Edit with targeted replacements; do not rewrite whole files.
- Keep command output to the last 15 lines.
- Test only the packages the prompt names (`cargo test -p <pkg> -q`).
- Ask before adding a crate the prompt does not name. After adding one, check the Cargo.lock diff: no existing version may move.
- Only prompter-platform imports `windows` or any other OS crate. prompter-core never depends on egui, ui-kit or OS crates.
- End every prompt with `cargo fmt --all` and `cargo clippy -p <touched packages> --all-targets -q -- -D warnings`, fixing what they report.
- If the task passes about 200k tokens, commit what compiles, write what remains to memory-bank/prompter.md, and stop.
- Report: files changed, tests added, anything not checked, choices made where the prompt was silent.
- Commit per the Git rules.
