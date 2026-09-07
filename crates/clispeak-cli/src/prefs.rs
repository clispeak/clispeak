//! The working agreement: what this user wants spoken, and how.
//!
//! **Why it lives here and not in an agent's memory.** The agreement is the
//! whole point of the skill and it was the part that did not survive. It was
//! stored by telling each agent to remember it, which is per-harness — Claude
//! Code, Codex and Antigravity each remember separately and immediately
//! diverge — invisible to the person it describes, and lost the moment a
//! write is forgotten.
//!
//! The asymmetry is the heart of it: **reading is idempotent, remembering is
//! not.** An agent that forgets to read simply reads again. An agent that
//! forgets to write has lost the preference for good. So the agreement lives
//! in a file next to the groups, one per machine, and every agent reads it
//! with a shell command rather than recalling it (#231).
//!
//! **Nobody is expected to edit this by hand.** It is written by an agent
//! when the user says something that implies a rule, and read back to them in
//! a sentence — that read-back is the only way they ever see it, which is why
//! every mutation here returns the new state for the caller to print.
//!
//! Lists rather than prose, deliberately. Preferences do not change by
//! rewording; they change by gaining a rule or losing one. A prose
//! `speak_when` would mean rewriting the whole sentence on every correction,
//! quietly dropping clauses nobody noticed. As a list, "stop telling me about
//! builds" is a removal and nothing else is at risk.

use serde::{Deserialize, Serialize};

/// One rule, and where it came from.
///
/// The provenance is not decoration. Under this design nobody opens the file,
/// so a rule an agent got wrong is invisible and permanent — three months
/// later a device is oddly quiet and there is nothing to look at. Recording
/// which agent added a rule and when makes "why does it keep doing that?" a
/// question with an answer.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Rule {
    /// What the user asked for, in their terms.
    pub rule: String,
    /// The agent that recorded it.
    #[serde(default)]
    pub by: String,
    /// Unix seconds when it was recorded.
    ///
    /// Seconds rather than a date string because rendering a calendar date
    /// needs civil-date arithmetic or a crate, and this binary's startup is
    /// the premise of the whole thin-client design. An age — "3 days ago" —
    /// is subtraction, needs neither, and is what someone actually asks for
    /// when they say "when did that get added?".
    #[serde(default)]
    pub at: u64,
}

/// Where a response goes: the terminal, the voice, or both.
///
/// Two questions that look like one — which channel carries it, and how much
/// of it the voice gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Output {
    /// Everything written, nothing spoken. Enforced: speaking is refused.
    Terminal,
    /// Full detail written, a short summary spoken. The common case.
    #[default]
    Brief,
    /// The same text in both places.
    Full,
    /// Spoken, with the terminal kept minimal.
    Speech,
}

impl Output {
    /// What to call it in a sentence.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::Brief => "brief",
            Self::Full => "full",
            Self::Speech => "speech",
        }
    }

    /// Read one, or say what the alternatives are.
    ///
    /// The error lists them because the caller is usually an agent acting on
    /// something a person said, and "invalid value" leaves it guessing at a
    /// vocabulary it was never given.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "terminal" => Ok(Self::Terminal),
            "brief" => Ok(Self::Brief),
            "full" => Ok(Self::Full),
            "speech" => Ok(Self::Speech),
            other => Err(format!(
                "'{other}' is not an output mode. Use terminal (write only), \
                 brief (write it all, say a summary), full (say the same \
                 text), or speech (say everything, keep the terminal quiet)"
            )),
        }
    }

    /// How long a spoken message should be under this mode.
    ///
    /// `brief` is the only one that needs a number, and it needs one badly:
    /// "length is your judgement" gives an agent nothing to judge against,
    /// because it has no sense of how long two hundred words is aloud.
    pub fn spoken_length(self) -> Option<&'static str> {
        match self {
            Self::Brief => Some(
                "about forty words — a listener cannot skim, scroll back or \
                 re-read, so say the point and leave the detail in the terminal",
            ),
            _ => None,
        }
    }
}

/// Everything this machine has been told about how to speak to its owner.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Agreement {
    /// What to call the user when speaking to them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address_me_as: Option<String>,
    /// Where a response goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
    /// Moments worth speaking about at all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speak_when: Vec<Rule>,
    /// Things never to say aloud.
    ///
    /// **Advisory, and it cannot be otherwise.** The obvious next step is to
    /// refuse any message containing one of these, and it does not survive
    /// contact: what people want kept quiet are *categories* — "customer
    /// names", "anything I'd not say in a room" — and a substring match
    /// cannot check a category. Making it enforceable would mean listing the
    /// literal secrets in a plaintext config file in order to avoid saying
    /// them aloud, which is worse than the problem. Even literal terms
    /// misfire: `credentials` would refuse "the credentials test passed".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub never_speak: Vec<Rule>,
}

impl Agreement {
    /// Whether anything has ever been recorded.
    ///
    /// An agent needs to tell "this machine is configured" from "nobody has
    /// said anything yet", and it cannot infer that from an empty list — the
    /// old skill asked it to judge exactly that, under the words "until it
    /// exists", and a fresh session always concluded it did not exist and
    /// started an interview (#231).
    pub fn configured(&self) -> bool {
        self.address_me_as.is_some()
            || self.output.is_some()
            || !self.speak_when.is_empty()
            || !self.never_speak.is_empty()
    }

    /// The mode, or the default when nobody has chosen one.
    pub fn output(&self) -> Output {
        self.output.unwrap_or_default()
    }
}

/// Which list a rule belongs to.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum List {
    /// Moments worth speaking about.
    SpeakWhen,
    /// Things never to say aloud.
    NeverSpeak,
}

impl List {
    fn name(self) -> &'static str {
        match self {
            Self::SpeakWhen => "speak-when",
            Self::NeverSpeak => "never-speak",
        }
    }

    fn of(self, a: &mut Agreement) -> &mut Vec<Rule> {
        match self {
            Self::SpeakWhen => &mut a.speak_when,
            Self::NeverSpeak => &mut a.never_speak,
        }
    }
}

/// A scalar the user can state outright.
#[derive(Clone, Copy, clap::ValueEnum)]
pub enum Field {
    /// What to call the user.
    AddressMeAs,
    /// Where responses go.
    Output,
}

/// Unix seconds now, or zero if the clock is before the epoch.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// How long ago, said the way a person would.
///
/// Rounded, not truncated: something added twenty-three hours ago is
/// "yesterday" to everyone except a clock.
pub fn age(at: u64) -> String {
    if at == 0 {
        return "unknown".into();
    }
    let secs = now().saturating_sub(at);
    let days = (secs as f64 / 86_400.0).round() as u64;
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        d if d < 14 => format!("{d} days ago"),
        d if d < 60 => format!("{} weeks ago", (d as f64 / 7.0).round() as u64),
        d => format!("{} months ago", (d as f64 / 30.0).round() as u64),
    }
}

// ------------------------------------------------------------- persistence

/// Read the agreement.
///
/// A malformed or missing file reads as "nothing recorded" rather than an
/// error, for the same reason `config::load` does: the tool has to keep
/// working with no config at all, and refusing to speak because a preferences
/// file has a typo in it would be a worse failure than the one being
/// prevented.
pub fn load() -> Agreement {
    crate::config::load().agent.unwrap_or_default()
}

/// Add a rule and hand back the new state.
///
/// Returns the whole agreement rather than nothing, because the caller has to
/// read it back — under this design that is the only way the user ever sees
/// what was stored.
pub fn add(list: List, text: &str, by: &str) -> anyhow::Result<Agreement> {
    let text = text.trim();
    if text.is_empty() {
        anyhow::bail!("a rule needs some words in it");
    }
    amend(|a| {
        let rules = list.of(a);
        if rules.iter().any(|r| r.rule.eq_ignore_ascii_case(text)) {
            anyhow::bail!("{} already says that", list.name());
        }
        rules.push(Rule {
            rule: text.to_string(),
            by: by.to_string(),
            at: now(),
        });
        Ok(())
    })
}

/// Drop the rule at a one-based index, as `prefs` numbers them.
///
/// One-based because the number the user is told is the number they say back.
pub fn remove(list: List, index: usize) -> anyhow::Result<Agreement> {
    amend(|a| {
        let rules = list.of(a);
        if index == 0 || index > rules.len() {
            anyhow::bail!(
                "{} has {} rule(s); there is no {index}",
                list.name(),
                rules.len()
            );
        }
        rules.remove(index - 1);
        Ok(())
    })
}

/// Set a scalar.
pub fn set(field: Field, value: &str) -> anyhow::Result<Agreement> {
    amend(|a| {
        match field {
            Field::AddressMeAs => {
                let name = value.trim();
                if name.is_empty() {
                    anyhow::bail!("a name needs some letters in it");
                }
                a.address_me_as = Some(name.to_string());
            }
            Field::Output => a.output = Some(Output::parse(value).map_err(anyhow::Error::msg)?),
        }
        Ok(())
    })
}

/// Read, change, write — with the read inside, on purpose.
///
/// **The read has to happen here rather than in the caller.** Several agents
/// share this file; that is the whole point. A caller that loaded the
/// agreement, thought about it, and then wrote it back would lose whatever
/// another agent recorded in between — which is the same shape as the two
/// bugs found in this project on 7 September, both of them a value read at
/// one moment and written at another. Re-reading immediately before the write
/// does not close the window entirely, but it narrows it from "as long as the
/// agent takes to think" to "as long as the write takes".
fn amend(change: impl FnOnce(&mut Agreement) -> anyhow::Result<()>) -> anyhow::Result<Agreement> {
    let mut written = None;
    crate::config::amend(|doc| {
        // Read *inside* the lock, which is the whole point and which the
        // first version of this got wrong while carrying a comment saying not
        // to. It loaded the agreement here, changed it, and then took the
        // lock only for the write — so two agents both read the old list,
        // each appended a rule, and the second write replaced the first.
        // Measured, not imagined: twenty concurrent adds recorded between two
        // and fifteen of them, run to run.
        let mut agreement: Agreement = doc
            .get("agent")
            .cloned()
            .and_then(|v| v.try_into().ok())
            .unwrap_or_default();
        change(&mut agreement)?;
        doc.insert("agent".into(), toml::Value::try_from(&agreement)?);
        written = Some(agreement);
        Ok(())
    })?;
    Ok(written.expect("the closure always sets it when it returns Ok"))
}

// ---------------------------------------------------------------- rendering

/// The agreement, written for whoever reads it — usually an agent.
pub fn render(a: &Agreement) -> String {
    if !a.configured() {
        return "Nothing recorded yet on this machine. Speak when a long task \
                finishes and they have walked away, when a question blocks \
                you, or when something fails they asked to hear about. Say \
                what you are doing in one sentence and let the first \
                correction become the first rule — do not interview them.\n"
            .into();
    }

    let mut out = String::new();
    if let Some(name) = &a.address_me_as {
        out.push_str(&format!("Call them: {name}\n"));
    }
    let mode = a.output();
    out.push_str(&format!("Output: {} — {}\n", mode.as_str(), describe(mode)));
    if let Some(length) = mode.spoken_length() {
        out.push_str(&format!("Spoken length: {length}\n"));
    }
    for (label, rules) in [
        ("Speak when", &a.speak_when),
        ("Never speak", &a.never_speak),
    ] {
        if rules.is_empty() {
            continue;
        }
        out.push_str(&format!("\n{label}\n"));
        for (i, r) in rules.iter().enumerate() {
            let who = if r.by.is_empty() { "unknown" } else { &r.by };
            out.push_str(&format!(
                "  {}. {}   ({who}, {})\n",
                i + 1,
                r.rule,
                age(r.at)
            ));
        }
    }
    out
}

/// What a mode means, in a clause.
fn describe(mode: Output) -> &'static str {
    match mode {
        Output::Terminal => "write everything, speak nothing",
        Output::Brief => "write the detail, speak a short summary of it",
        Output::Full => "write it and speak the same text",
        Output::Speech => "speak everything, keep the terminal minimal",
    }
}

/// One line, for a hook that runs on every prompt.
///
/// Empty when nothing is recorded: a hook that nags on every turn of a
/// machine nobody has configured is noise, and noise is how a hook gets
/// removed.
pub fn brief(a: &Agreement) -> String {
    if !a.configured() {
        return String::new();
    }
    let mut parts = vec![format!("output {}", a.output().as_str())];
    if let Some(name) = &a.address_me_as {
        parts.push(format!("call them {name}"));
    }
    if !a.speak_when.is_empty() {
        parts.push(format!(
            "speak when {}",
            a.speak_when
                .iter()
                .map(|r| r.rule.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if !a.never_speak.is_empty() {
        parts.push(format!(
            "never speak {}",
            a.never_speak
                .iter()
                .map(|r| r.rule.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    format!(
        "clispeak agreement — {}. Full text: clispeak prefs",
        parts.join(", ")
    )
}

/// Drop the agreement entirely.
///
/// Separate from `amend` because it removes the table rather than replacing
/// it: writing an empty `[agent]` would leave a machine that looks configured
/// with nothing in it, which is the one state `configured()` exists to rule
/// out.
pub fn clear() -> anyhow::Result<()> {
    crate::config::remove_agent()?;
    Ok(())
}

// --------------------------------------------------------------- enforcement

/// Why a message was refused, and what to send instead.
pub struct Refusal {
    /// Written for the agent that is about to read it.
    pub message: String,
    /// A line it can send verbatim, when one exists.
    pub instead: Option<String>,
    /// Which kind, so the caller maps it to an exit code.
    ///
    /// Not a `u8` here: the exit codes live with the binary and this is the
    /// library. A module that decided its own exit codes would be a second
    /// place they are written down, and this crate already keeps a test to
    /// stop that happening with `clispeak-core`.
    pub kind: Kind,
}

/// What was refused, in the caller's terms.
pub enum Kind {
    /// The agreement says not to speak at all right now.
    Silenced,
    /// The text needs to open by naming the user.
    NeedsOpener,
}

/// Check a message against the two rules a machine can check.
///
/// **Deliberately two, and not more.** The obvious third is `never_speak`,
/// and it does not survive contact — see the field's own comment. Enforcing
/// judgement produces a tool that refuses correct messages, and a tool that
/// refuses correct messages gets `--raw` and then gets abandoned. The floor
/// is small on purpose.
///
/// `--raw` skips this, like every other check. That is not a hole: the
/// enforcement is here to stop an agent *forgetting* the agreement, not to
/// stop one deciding to override it. An agent that reaches for `--raw` has
/// made a decision, and the user can see it in the command.
pub fn allows(a: &Agreement, text: &str) -> Result<(), Refusal> {
    if a.output() == Output::Terminal {
        return Err(Refusal {
            message: "error: your agreement says output is `terminal` — write \
                      this rather than speaking it.\n       Set by them, not by \
                      you: do not route around it to another device.\n       \
                      They can change it by saying so, which is `clispeak prefs \
                      set output brief`."
                .into(),
            instead: None,
            kind: Kind::Silenced,
        });
    }

    let Some(name) = a.address_me_as.as_deref() else {
        return Ok(());
    };
    // Not under `speech`, where the agent is talking continuously and
    // prefixing every utterance with a name would be absurd. The opener earns
    // its keep when messages are occasional and arrive with no context — it
    // stops being a courtesy and starts being a tic when they are constant.
    if a.output() == Output::Speech || opens_with(text, name) {
        return Ok(());
    }
    Err(Refusal {
        message: "error: your agreement says a message opens by naming them, \
                  and this one does not.\n       A voice from a pocket that does \
                  not say who it is forces them to guess."
            .into(),
        instead: Some(format!("{name}, this is <your name>. {text}")),
        kind: Kind::NeedsOpener,
    })
}

/// Whether the text opens by naming the user.
///
/// Case-insensitive, and the name has to be followed by punctuation or a
/// space rather than merely appearing first — otherwise "Patrickson said the
/// build broke" would pass while naming somebody else entirely.
fn opens_with(text: &str, name: &str) -> bool {
    let text = text.trim_start();
    let Some(rest) = text
        .get(..name.len())
        .filter(|head| head.eq_ignore_ascii_case(name))
        .map(|_| &text[name.len()..])
    else {
        return false;
    };
    rest.chars()
        .next()
        .is_none_or(|c| c.is_whitespace() || c == ',' || c == ':' || c == '!' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(text: &str) -> Rule {
        Rule {
            rule: text.into(),
            by: "Claude".into(),
            at: now(),
        }
    }

    fn named(name: &str, output: Output) -> Agreement {
        Agreement {
            address_me_as: Some(name.into()),
            output: Some(output),
            ..Agreement::default()
        }
    }

    #[test]
    fn nothing_recorded_enforces_nothing() {
        // A machine nobody has said anything about must behave exactly as it
        // did before this feature existed. Enforcement that switches itself
        // on by default would break every install on upgrade.
        let empty = Agreement::default();
        assert!(!empty.configured());
        assert!(allows(&empty, "anything at all").is_ok());
    }

    #[test]
    fn terminal_refuses_and_says_it_is_theirs_to_change() {
        let a = named("Patrick", Output::Terminal);
        let refusal = allows(&a, "Patrick, this is Claude. Hello.").expect_err("refused");
        assert!(matches!(refusal.kind, Kind::Silenced));
        assert!(
            refusal.instead.is_none(),
            "there is no rewrite for this — the message is fine and the \
             channel is closed"
        );
        assert!(
            refusal.message.contains("do not route around it"),
            "an agent that reroutes to another device defeats the point: {}",
            refusal.message
        );
    }

    #[test]
    fn a_message_that_does_not_name_them_is_refused_with_a_rewrite() {
        let a = named("Patrick", Output::Brief);
        let refusal = allows(&a, "The deploy finished.").expect_err("refused");
        assert!(matches!(refusal.kind, Kind::NeedsOpener));
        let instead = refusal.instead.expect("a rewrite");
        assert!(
            instead.starts_with("Patrick,"),
            "the rewrite has to satisfy the rule it is offered for: {instead}"
        );
        assert!(
            instead.ends_with("The deploy finished."),
            "and has to keep what was being said: {instead}"
        );
    }

    #[test]
    fn a_message_that_names_them_passes() {
        let a = named("Patrick", Output::Brief);
        assert!(allows(&a, "Patrick, this is Claude. Done.").is_ok());
        assert!(allows(&a, "patrick, this is Claude. Done.").is_ok(), "case");
        assert!(allows(&a, "  Patrick: done.").is_ok(), "leading space");
    }

    #[test]
    fn a_longer_name_starting_the_same_way_is_not_the_name() {
        // Without the boundary check this passes, and the message is about
        // somebody else entirely.
        let a = named("Patrick", Output::Brief);
        assert!(allows(&a, "Patrickson said the build broke.").is_err());
    }

    #[test]
    fn speech_mode_drops_the_opener() {
        // The opener earns its keep when messages are occasional and arrive
        // with no context. Under `speech` the agent talks continuously and it
        // becomes a tic.
        let a = named("Patrick", Output::Speech);
        assert!(allows(&a, "The deploy finished.").is_ok());
    }

    #[test]
    fn with_no_name_recorded_there_is_nothing_to_check() {
        let a = Agreement {
            output: Some(Output::Brief),
            ..Agreement::default()
        };
        assert!(allows(&a, "The deploy finished.").is_ok());
    }

    #[test]
    fn never_speak_is_not_enforced_and_that_is_deliberate() {
        // Enforcing it would mean listing the literal secrets in a plaintext
        // config in order to avoid saying them, and would refuse correct
        // sentences: `credentials` here must not refuse a message about a
        // test file.
        let mut a = named("Patrick", Output::Brief);
        a.never_speak = vec![rule("credentials")];
        assert!(allows(&a, "Patrick, the credentials test passed.").is_ok());
    }

    #[test]
    fn the_brief_line_is_empty_until_something_is_recorded() {
        // A hook that nags on every turn of an unconfigured machine is noise,
        // and noise is how a hook gets deleted.
        assert!(brief(&Agreement::default()).is_empty());
        assert!(!brief(&named("Patrick", Output::Brief)).is_empty());
    }

    #[test]
    fn an_unknown_output_mode_lists_the_real_ones() {
        let e = Output::parse("loud").expect_err("not a mode");
        for mode in ["terminal", "brief", "full", "speech"] {
            assert!(e.contains(mode), "{mode} missing from: {e}");
        }
    }

    #[test]
    fn ages_are_said_the_way_a_person_would() {
        assert_eq!(age(0), "unknown");
        assert_eq!(age(now()), "today");
        assert_eq!(age(now() - 86_400), "yesterday");
        assert_eq!(age(now() - 3 * 86_400), "3 days ago");
        // Rounded, not truncated: twenty-three hours ago is yesterday to
        // everyone except a clock.
        assert_eq!(age(now() - 82_800), "yesterday");
    }
}
