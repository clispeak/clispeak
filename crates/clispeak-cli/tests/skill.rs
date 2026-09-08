//! The agent skill must describe the CLI that exists.
//!
//! `skills/clispeak/SKILL.md` is installed into agents so they can drive
//! this tool. A skill that has drifted is worse than none: it states, with
//! confidence, that a flag exists, and the agent obeys it. So every command
//! and flag the skill mentions is checked against the binary's own help.
//!
//! This is deliberately a test rather than a note in a contributing guide.
//! The rule "we will keep the skill updated" is only real if something fails
//! when it is not.

use std::process::Command;

/// Path to the built binary, provided by cargo for integration tests.
fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_clispeak")
}

/// The skill's text.
fn skill() -> String {
    // Three levels up from the crate: crates/clispeak-cli -> repo root.
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/clispeak/SKILL.md");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

/// Everything `--help` reports, top level and per subcommand.
fn help_text() -> String {
    let top = Command::new(bin())
        .arg("--help")
        .output()
        .expect("running clispeak --help");
    let mut all = String::from_utf8_lossy(&top.stdout).into_owned();

    for name in subcommands(&all) {
        let sub = Command::new(bin())
            .args([name.as_str(), "--help"])
            .output()
            .expect("running a subcommand's help");
        all.push_str(&String::from_utf8_lossy(&sub.stdout));
    }
    all
}

/// Subcommand names, read out of the `Commands:` block of the top-level help.
fn subcommands(help: &str) -> Vec<String> {
    let Some(block) = help.split("Commands:").nth(1) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for line in block.lines() {
        // The block ends at the next unindented heading, e.g. "Options:".
        if !line.is_empty() && !line.starts_with(' ') {
            break;
        }
        // Command rows are indented exactly two spaces; their wrapped
        // descriptions are indented further.
        if !line.starts_with("  ") || line.starts_with("   ") {
            continue;
        }
        if let Some(name) = line.split_whitespace().next()
            && name.chars().all(|c| c.is_ascii_lowercase())
        {
            names.push(name.to_string());
        }
    }
    names
}

#[test]
fn every_command_the_skill_names_exists() {
    let skill = skill();
    let help = help_text();
    let known = subcommands(&help);

    let mut missing = Vec::new();
    for line in skill.lines() {
        for (at, _) in line.match_indices("clispeak ") {
            let rest = &line[at + "clispeak ".len()..];
            let Some(word) = rest.split_whitespace().next() else {
                continue;
            };
            // Only bare lowercase words are candidates. Flags, placeholders
            // and prose are somebody else's problem.
            if !word.chars().all(|c| c.is_ascii_lowercase()) || word.is_empty() {
                continue;
            }
            if !known.iter().any(|k| k == word) {
                missing.push(word.to_string());
            }
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "skills/clispeak/SKILL.md names commands that do not exist: {missing:?}\n\
         known commands: {known:?}"
    );
}

#[test]
fn every_flag_the_skill_names_exists() {
    let skill = skill();
    let help = help_text();

    // The YAML front matter is not prose about the CLI, and its `---`
    // delimiters would otherwise read as flags.
    let body = skill.split("\n---\n").last().unwrap_or(&skill);

    let mut missing = Vec::new();
    for (at, _) in body.match_indices("--") {
        let rest = &body[at..];
        // A flag's first character after the dashes is a letter; `---` and
        // an em-dash written as `--` are not flags.
        if !rest[2..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let flag: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        // "--" alone is the argument separator, not a flag.
        if flag.len() <= 2 {
            continue;
        }
        if !help.contains(&flag) {
            missing.push(flag);
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "skills/clispeak/SKILL.md names flags that do not exist: {missing:?}"
    );
}

#[test]
fn the_skill_still_tells_an_agent_to_identify_itself() {
    // The one instruction whose loss would be invisible: messages would keep
    // working, and the user would stop knowing which agent was talking.
    let skill = skill();
    assert!(
        skill.contains("this is"),
        "the skill must still show an agent naming itself when it speaks"
    );
    assert!(
        skill.to_lowercase().contains("memory"),
        "the skill must still tell an agent to record the working agreement"
    );
}

/// The hook block is the shape the runner actually accepts.
///
/// **Written because the first version was not, and nothing noticed.** The
/// frontmatter carried `args: []`, copied from a documented example whose
/// `command` was a path to a script with no arguments — where an empty args
/// array is consistent. Generalised to a command *line* it is not: with an
/// explicit `args` the runner treats `command` as a bare executable name and
/// looks up the whole string in `$PATH`, so every prompt produced
///
///   exitCode: 1
///   stderr:   Executable not found in $PATH: "clispeak prefs --brief"
///
/// and the agreement never reached the model. It failed *silently from the
/// inside*: the hook fired, the transcript recorded the error, and nothing in
/// the conversation looked wrong — the agreement appeared to be known because
/// it had been read by hand a few minutes earlier.
///
/// That is the exact failure this hook exists to prevent, so the shape is
/// pinned. Not a full YAML parse, deliberately: the point is to catch the one
/// key whose presence changes how the command is executed.
#[test]
fn the_hook_runs_a_command_line_rather_than_naming_an_executable() {
    let text = skill();
    let front = text
        .split("\n---\n")
        .next()
        .expect("frontmatter before the first ---");

    assert!(
        front.contains("UserPromptSubmit:"),
        "the skill no longer declares the hook that keeps the agreement in \
         context after a compaction"
    );

    let command = front
        .lines()
        .find_map(|l| l.trim().strip_prefix("command:"))
        .map(str::trim)
        .expect("the hook declares a command");
    assert!(
        command.starts_with("clispeak "),
        "the hook should run this tool, not {command:?}"
    );
    assert!(
        command.split_whitespace().count() > 1,
        "a command with no arguments would not need this test; {command:?}"
    );

    assert!(
        !front.contains("args:"),
        "the hook frontmatter has an `args` key. With one present the runner \
         treats `command` as a bare executable name and looks up the whole \
         string in $PATH, so `{command}` becomes a file that does not exist \
         and the hook fails on every prompt — visibly in the transcript and \
         invisibly in the conversation."
    );
}
