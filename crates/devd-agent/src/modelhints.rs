//! What each family of model needs to be told that the others do not.
//!
//! The base prompt says what devd is and how to work in it, and is the same for
//! every model. Models differ in what they get wrong, not in what the work is:
//! one narrates a plan and stops, another edits from memory without reading,
//! another calls five tools where one would do. A line or two aimed at the
//! habit costs a few dozen tokens; a model that does not have the habit is
//! simply not given the line.
//!
//! These are kept short on purpose. A paragraph per model is how a system
//! prompt becomes something no one can reason about.

/// Extra instructions for a model, or nothing.
///
/// Matched on the model id, so it holds for any provider that serves the model
/// (a reseller's `vendor/model` and a snapshot date do not matter).
pub fn addendum(model: &str) -> &'static str {
    let id = model.to_ascii_lowercase();
    let id = id.rsplit('/').next().unwrap_or(&id);
    if id.contains("deepseek") {
        // Strong at long edits, weak at stopping to check: it reports "done" on
        // work it has not run, and re-reads files it has just read.
        "\n## Working notes for this model\n- Run the project's own check before you say a change works, and quote its result. Do not report success from reading your own diff.\n- A file you have just read or written is in your context. Do not read it again unless something else may have changed it.\n- If a tool refuses (permission, missing key, a command not allowed), say so plainly and stop that line of work; do not retry it with small variations.\n"
    } else if id.contains("claude") {
        "\n## Working notes for this model\n- Prefer one `edit_file` call with several `edits` over several calls on the same file.\n- Batch independent reads into one turn.\n"
    } else if id.starts_with("gpt") || id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4") {
        "\n## Working notes for this model\n- Act on the request; do not restate the plan before doing it. Report what changed, briefly, once the work is done.\n- Read a file before editing it, and copy `old` text exactly from what you read.\n"
    } else if id.contains("gemini") {
        "\n## Working notes for this model\n- Call tools with exactly the argument names the schema gives; do not add fields.\n- Keep tool arguments small: pass a path, not a file's contents.\n"
    } else if id.contains("kimi") || id.contains("glm") || id.contains("qwen") || id.contains("llama") || id.contains("mistral") {
        "\n## Working notes for this model\n- Make one tool call at a time unless the calls are plainly independent, and wait for each result before relying on it.\n- Look up a capability's schema with `devd_describe` before the first time you invoke it; do not guess argument names.\n- Read a file before editing it.\n"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_family_is_recognised_whoever_serves_it_and_whatever_its_snapshot() {
        assert_eq!(addendum("deepseek-flash"), addendum("deepseek/deepseek-v4-flash"));
        assert_eq!(addendum("claude-sonnet-5-5"), addendum("anthropic/claude-sonnet-5-5-20260101"));
        assert!(addendum("gpt-6-sol").contains("Act on the request"));
        assert!(addendum("moonshotai/kimi-k3").contains("one tool call at a time"));
    }

    #[test]
    fn an_unknown_model_gets_nothing_rather_than_a_guess() {
        assert_eq!(addendum("my-private-model"), "");
        assert_eq!(addendum(""), "");
    }

    #[test]
    fn each_note_is_short_enough_to_be_read() {
        for model in ["deepseek-x", "claude-x", "gpt-x", "gemini-x", "kimi-x"] {
            let words = addendum(model).split_whitespace().count();
            assert!((1..=90).contains(&words), "{model}: {words} words");
        }
    }
}
