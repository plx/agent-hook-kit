//! Denies a small set of destructive Codex `Bash` commands.
//!
//! The command is parsed with `hookkit-shell`'s bounded Bash analyzer, and
//! each rule matches command names, flags, and operands of the simple commands
//! the analyzer recovers. Matching parsed words rather than substrings means
//! `rm -rf /tmp/build` is not mistaken for `rm -rf /`, while `rm -fr /`,
//! `rm -r -f  /`, `sudo rm -rf ~`, `bash -c 'rm -rf /'`, and
//! `git push origin +main` are all recognized.
//!
//! This is still an illustrative guard, not a sandbox. Static analysis cannot
//! see through a command name chosen at runtime (`$CMD -rf /`), a script file,
//! or a program that deletes files itself. Pair it with OS sandboxing or the
//! harness's approval policy for a real boundary.

use hookkit_codex::protocol::{PreToolUse, PreToolUseOutput};
use hookkit_shell::{
    BashAnalyzer, CommandOccurrence, RedirectionKind, RedirectionOperator, ShellWord,
};

/// How deeply `bash -c '...'` and `eval ...` payloads are re-analyzed.
const MAX_NESTING: usize = 3;

/// Commands that run their arguments as another command.
const WRAPPERS: &[&str] = &[
    "sudo", "doas", "env", "nice", "nohup", "time", "command", "exec", "xargs",
];

/// Shells whose `-c` argument is itself a command string.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];

fn main() -> std::process::ExitCode {
    // A guard that cannot read or answer the call blocks it (exit 2 with the
    // diagnostic as the reason) instead of letting it run.
    let options = hookkit_runtime::RunOptions::new().fail_closed();
    hookkit_runtime::run_event_with_options::<PreToolUse, _>(
        options,
        |input, _environment, _ctx| {
            let command = if input.tool_name == "Bash" {
                input
                    .tool_input
                    .get("command")
                    .and_then(serde_json::Value::as_str)
            } else {
                None
            };
            Ok(match command.and_then(deny_reason) {
                Some(reason) => PreToolUseOutput::deny(format!("Denied: {reason}")),
                // No objection: Codex's normal approval flow still applies.
                None => PreToolUseOutput::no_op(),
            })
        },
    )
}

/// Returns why `command` is blocked, or `None` when no rule matches.
fn deny_reason(command: &str) -> Option<String> {
    deny_reason_at_depth(command, 0)
}

fn deny_reason_at_depth(command: &str, depth: usize) -> Option<String> {
    let outcome = BashAnalyzer::default().analyze(command);
    let analysis = outcome.analysis()?;
    analysis
        .commands
        .iter()
        .find_map(|occurrence| command_reason(occurrence, depth))
}

/// Applies every rule to one simple command.
fn command_reason(occurrence: &CommandOccurrence, depth: usize) -> Option<String> {
    if let Some(reason) = device_redirection(occurrence) {
        return Some(reason);
    }
    let name = occurrence.name.as_ref()?;
    let words: Vec<&ShellWord> = std::iter::once(name)
        .chain(occurrence.arguments.iter())
        .collect();
    let words = strip_wrappers(&words);
    let (name, arguments) = words.split_first()?;
    let name = basename(name.literal.as_deref()?);
    match name {
        "rm" => recursive_root_removal(arguments),
        "git" => force_push(arguments),
        "dd" => arguments
            .iter()
            .filter_map(|word| word.literal.as_deref())
            .find(|operand| {
                operand
                    .strip_prefix("of=")
                    .is_some_and(|target| target.starts_with("/dev/"))
            })
            .map(|operand| format!("dd writes directly to a device (`{operand}`)")),
        "chmod" => world_writable_recursive_chmod(arguments),
        _ if name == "mkfs" || name.starts_with("mkfs.") => {
            Some(format!("`{name}` formats a filesystem"))
        }
        "eval" if depth < MAX_NESTING => {
            let source = literal_words(arguments)?.join(" ");
            deny_reason_at_depth(&source, depth + 1)
        }
        _ if SHELLS.contains(&name) && depth < MAX_NESTING => {
            let position = arguments
                .iter()
                .position(|word| word.literal.as_deref() == Some("-c"))?;
            let source = arguments.get(position + 1)?.literal.as_deref()?;
            deny_reason_at_depth(source, depth + 1)
        }
        _ => None,
    }
}

/// Skips leading wrapper commands (`sudo -u root`, `env NAME=value`,
/// `nice -n 5`, ...) so the wrapped command is checked.
///
/// A wrapper's options can take values, so rather than parse each wrapper's
/// syntax the scan resumes at the first later word naming a command a rule
/// inspects. A wrapper that runs nothing this guard knows is left alone.
fn strip_wrappers<'a>(mut words: &'a [&'a ShellWord]) -> &'a [&'a ShellWord] {
    while let Some((first, rest)) = words.split_first() {
        let Some(name) = first.literal.as_deref() else {
            break;
        };
        if !WRAPPERS.contains(&basename(name)) {
            break;
        }
        let Some(start) = rest.iter().position(|word| {
            word.literal
                .as_deref()
                .map(basename)
                .is_some_and(|name| is_inspected(name) || WRAPPERS.contains(&name))
        }) else {
            break;
        };
        words = &rest[start..];
    }
    words
}

/// Command names at least one rule inspects.
fn is_inspected(name: &str) -> bool {
    matches!(name, "rm" | "git" | "dd" | "chmod" | "eval" | "mkfs")
        || name.starts_with("mkfs.")
        || SHELLS.contains(&name)
}

/// `rm` with a recursive flag and a root or home-directory operand.
fn recursive_root_removal(arguments: &[&ShellWord]) -> Option<String> {
    let mut recursive = false;
    let mut operands = Vec::new();
    let mut options_ended = false;
    for word in arguments {
        let text = word.literal.as_deref().unwrap_or(&word.raw);
        if !options_ended && text == "--" {
            options_ended = true;
        } else if !options_ended && text.starts_with("--") {
            recursive |= text == "--recursive";
        } else if !options_ended && text.starts_with('-') && text.len() > 1 {
            recursive |= text.contains(['r', 'R']);
        } else {
            operands.push(word);
        }
    }
    if !recursive {
        return None;
    }
    operands
        .into_iter()
        .find(|word| is_protected_root(word))
        .map(|word| format!("`rm` recursively deletes `{}`", word.raw))
}

/// The filesystem root, the home directory, or everything directly beneath
/// either, however it is spelled.
fn is_protected_root(word: &ShellWord) -> bool {
    let text = word
        .literal
        .as_deref()
        .or(word.pattern.as_deref())
        .unwrap_or(&word.raw);
    let unquoted = text.trim_matches(['"', '\'']);
    let trimmed = unquoted.trim_end_matches(['/', '*']);
    let trimmed = trimmed.trim_end_matches('/');
    matches!(trimmed, "" | "~" | "$HOME" | "${HOME}") && !unquoted.is_empty()
}

/// `git push` with a force flag or a `+refspec`, after any global options.
fn force_push(arguments: &[&ShellWord]) -> Option<String> {
    let literal: Vec<&str> = arguments
        .iter()
        .filter_map(|word| word.literal.as_deref())
        .collect();
    // Skip global options such as `-C <dir>` and `-c <name=value>`.
    let mut index = 0;
    while let Some(argument) = literal.get(index) {
        match *argument {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => index += 2,
            option if option.starts_with('-') => index += 1,
            _ => break,
        }
    }
    if literal.get(index) != Some(&"push") {
        return None;
    }
    literal[index + 1..]
        .iter()
        .find(|argument| {
            argument.starts_with("--force")
                || (argument.starts_with('-')
                    && !argument.starts_with("--")
                    && argument.contains('f'))
                || (argument.starts_with('+') && argument.len() > 1)
        })
        .map(|argument| format!("`git push {argument}` rewrites remote history"))
}

/// `chmod -R 777` (or `a+rwx`) makes a tree world-writable.
fn world_writable_recursive_chmod(arguments: &[&ShellWord]) -> Option<String> {
    let literal: Vec<&str> = arguments
        .iter()
        .filter_map(|word| word.literal.as_deref())
        .collect();
    let recursive = literal.iter().any(|argument| {
        *argument == "--recursive" || (argument.starts_with('-') && argument.contains('R'))
    });
    let world_writable = literal
        .iter()
        .any(|argument| matches!(*argument, "777" | "0777" | "a+rwx" | "ugo+rwx" | "o+w"));
    (recursive && world_writable).then(|| "`chmod -R` makes a tree world-writable".to_owned())
}

/// An output redirection straight onto a block device such as `/dev/sda`.
fn device_redirection(occurrence: &CommandOccurrence) -> Option<String> {
    occurrence
        .redirections
        .iter()
        .filter(|redirection| redirection.kind == RedirectionKind::File)
        .filter(|redirection| {
            matches!(
                redirection.operator,
                Some(
                    RedirectionOperator::Output
                        | RedirectionOperator::Append
                        | RedirectionOperator::Clobber
                        | RedirectionOperator::OutputAndError
                        | RedirectionOperator::AppendOutputAndError
                )
            )
        })
        .filter_map(|redirection| redirection.target.as_ref()?.literal.as_deref())
        .find(|target| {
            ["/dev/sd", "/dev/nvme", "/dev/disk", "/dev/hd", "/dev/vd"]
                .iter()
                .any(|device| target.starts_with(device))
        })
        .map(|target| format!("output is redirected onto the block device `{target}`"))
}

fn literal_words(words: &[&ShellWord]) -> Option<Vec<String>> {
    words.iter().map(|word| word.literal.clone()).collect()
}

fn basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::deny_reason;

    #[test]
    fn destructive_commands_are_denied_however_they_are_spelled() {
        for command in [
            "rm -rf /",
            "rm -fr /",
            "rm -r -f /",
            "rm -rf  /",
            "rm -Rf /*",
            "rm --recursive --force /",
            "rm -rf ~",
            "rm -rf ~/",
            "rm -rf $HOME",
            "rm -rf \"$HOME\"",
            "sudo rm -rf /",
            "env LC_ALL=C nice -n 5 rm -rf /",
            "sudo -u root rm -rf ~",
            "/bin/rm -rf /",
            "cd /tmp && rm -rf /",
            "bash -c 'rm -rf /'",
            "eval rm -rf /",
            "git push --force origin main",
            "git push -f",
            "git push --force-with-lease origin main",
            "git push origin +main",
            "git -C repo push --force",
            "dd if=/dev/zero of=/dev/sda bs=1M",
            "mkfs.ext4 /dev/sdb1",
            "echo boom > /dev/sda",
            "chmod -R 777 .",
        ] {
            assert!(
                deny_reason(command).is_some(),
                "expected a deny for {command:?}"
            );
        }
    }

    #[test]
    fn ordinary_commands_that_merely_look_similar_are_allowed() {
        for command in [
            "cargo test",
            "rm -rf /tmp/build-cache",
            "rm -rf ~/project/target",
            "rm -rf ./target",
            "rm /tmp/file",
            "git push origin main",
            "git push -u origin feature",
            "git pull --force",
            "echo 'rm -rf /'",
            "grep -r 'git push --force' docs",
            "dd if=disk.img of=backup.img",
            "chmod 777 script.sh",
            "cat /dev/sda.log",
        ] {
            assert_eq!(deny_reason(command), None, "{command:?}");
        }
    }
}
