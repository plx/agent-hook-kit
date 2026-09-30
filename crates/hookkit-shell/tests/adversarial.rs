//! Adversarial corpus for guard-relevant shell forms.
//!
//! Every case is a command whose file effect a pre-tool guard must see. The
//! invariant under test is the one a fail-closed guard relies on: the analysis
//! either reports a candidate covering each file the shell touches, or it
//! records an unresolved gap. A complete report that omits a touched file is a
//! bypass.
//!
//! Setting `HOOKKIT_SHELL_DIFFERENTIAL=1` also runs the executable cases under
//! `bash` in a scratch directory full of sentinel files and checks every
//! sentinel the shell actually read, changed, created, or removed against the
//! same analysis.

use std::path::{Path, PathBuf};

use hookkit_core::Utf8Path;
use hookkit_shell::{
    BashAnalysisOutcome, BashAnalyzer, FileAccessAnalyzer, FileAccessCandidate, FileAccessKind,
    FileAccessReport, FileInferenceContext, FileTarget, FileTargetScope, IncompleteReason,
    RedirectionKind, UnknownCommandFallback,
};

const CWD: &str = "/repo";
const HOME: &str = "/home/agent";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Modify,
    Delete,
}

#[derive(Debug, Clone, Copy)]
enum Expect {
    /// A candidate with this access covers the path (relative to [`CWD`], or
    /// absolute).
    Covers(&'static str, Access),
    /// The report records at least one unresolved gap.
    Gap,
}

use Access::{Delete, Modify, Read};
use Expect::{Covers, Gap};

struct Case {
    /// Review finding the case regresses.
    finding: &'static str,
    command: &'static str,
    expect: &'static [Expect],
}

const CASES: &[Case] = &[
    // Words after a redirection target are arguments.
    Case {
        finding: "shell-redirect-extra-destinations",
        command: "rm 2>/dev/null .env",
        expect: &[Covers(".env", Delete)],
    },
    Case {
        finding: "shell-redirect-extra-destinations",
        command: "cat 2>&1 .env",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-redirect-extra-destinations",
        command: "cat </dev/null .env",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-redirect-extra-destinations",
        command: "tee < .env out",
        expect: &[Covers(".env", Read), Covers("out", Modify)],
    },
    Case {
        finding: "shell-redirect-extra-destinations",
        command: "a || rm 2>/dev/null .env",
        expect: &[Covers(".env", Delete)],
    },
    // Here-document-owned redirections and arguments.
    Case {
        finding: "shell-heredoc-nested-redirects-args",
        command: "cat <<EOF > .env\nhello\nEOF\n",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-heredoc-nested-redirects-args",
        command: "rm <<EOF .env\nEOF\n",
        expect: &[Covers(".env", Delete)],
    },
    Case {
        finding: "shell-heredoc-nested-redirects-args",
        command: "cat <<EOF .env\nEOF\n",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-heredoc-nested-redirects-args",
        command: "cat <<EOF >> .env && echo ok\nhello\nEOF\n",
        expect: &[Covers(".env", Modify)],
    },
    // Comma brace expansion.
    Case {
        finding: "shell-brace-expansion-literal",
        command: "cat .e{n,}v",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-brace-expansion-literal",
        command: "rm -f {.env,x}",
        expect: &[Covers(".env", Delete)],
    },
    Case {
        finding: "access-activity-oob-brace-expansion-literal",
        command: "cat {.env,x}",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-brace-expansion-literal",
        command: "cat {.e,.f}{nv,x}",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-brace-expansion-literal",
        command: "cat .e{n,}v$SUFFIX",
        expect: &[Gap],
    },
    // Redirections on statements that are not simple commands.
    Case {
        finding: "shell-compound-statement-redirects",
        command: "{ cat; } < .env",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "(echo x) > .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "if true; then echo x; fi > .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "for f in a; do echo; done > .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "case x in x) echo;; esac > .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "[ -f x ] > .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "declare -p > .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "export X=1 > .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "f() { echo x; } > .env; f",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "while read l; do echo \"$l\"; done < .env",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "! cat > .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-compound-statement-redirects",
        command: "a && { b; } > .env",
        expect: &[Covers(".env", Modify)],
    },
    // Body-less redirections.
    Case {
        finding: "shell-bare-redirect-and-dollar-lt",
        command: "echo \"$(< .env)\"",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-bare-redirect-and-dollar-lt",
        command: "echo $(<.env)",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-bare-redirect-and-dollar-lt",
        command: "x=$(< .env)",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-bare-redirect-and-dollar-lt",
        command: "> .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-bare-redirect-and-dollar-lt",
        command: "< .env",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-bare-redirect-and-dollar-lt",
        command: "> /dev/null rm .env",
        expect: &[Covers(".env", Delete)],
    },
    // Backticks in unquoted here-document bodies.
    Case {
        finding: "shell-heredoc-backticks-invisible",
        command: "cat <<EOF\n`cat .env`\nEOF\n",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-heredoc-backticks-invisible",
        command: "cat <<EOF\nnote: `rm -rf secrets` done\nEOF\n",
        expect: &[Covers("secrets/key", Delete)],
    },
    Case {
        finding: "shell-heredoc-backticks-invisible",
        command: "cat <<EOF\n`cat \\.env`\nEOF\n",
        expect: &[Gap],
    },
    Case {
        finding: "shell-heredoc-backticks-invisible",
        command: "cat <<EOF\n`cat .env\nEOF\n",
        expect: &[Gap],
    },
    // Backslash-newline inside words.
    Case {
        finding: "shell-line-continuation-splits-words",
        command: "cat .e\\\nnv",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-line-continuation-splits-words",
        command: "echo x > .e\\\nnv",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-line-continuation-splits-words",
        command: "ca\\\nt .e\\\nnv",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-line-continuation-splits-words",
        command: "cat '.e'\\\nnv",
        expect: &[Covers(".env", Read)],
    },
    // sed scripts that open files or run commands.
    Case {
        finding: "shell-sed-script-io",
        command: "sed 'r .env' README.md",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-sed-script-io",
        command: "sed -n '1w .env' x",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-sed-script-io",
        command: "sed 's/a/b/w .env' x",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-sed-script-io",
        command: "sed '1e cat .env' x",
        expect: &[Gap],
    },
    Case {
        finding: "shell-sed-script-io",
        command: "sed -i '' 'w .env' x",
        expect: &[Covers(".env", Modify)],
    },
    // Directory semantics of mv, cp, and ln.
    Case {
        finding: "shell-mv-cp-directory-scope",
        command: "mv secrets /tmp/x",
        expect: &[Covers("secrets/key", Modify)],
    },
    Case {
        finding: "shell-mv-cp-directory-scope",
        command: "cp /tmp/x secrets",
        expect: &[Covers("secrets/x", Modify)],
    },
    Case {
        finding: "shell-mv-cp-directory-scope",
        command: "cp /tmp/.env .",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-mv-cp-directory-scope",
        command: "ln -s /etc/passwd .",
        expect: &[Covers("passwd", Modify)],
    },
    // Dynamic search words.
    Case {
        finding: "shell-search-dynamic-words",
        command: "O='-f .env'; grep $O foo",
        expect: &[Gap],
    },
    Case {
        finding: "shell-search-dynamic-words",
        command: "grep \"$O\" foo",
        expect: &[Gap],
    },
    Case {
        finding: "shell-search-dynamic-words",
        command: "X='-e root /etc/shadow'; grep $X",
        expect: &[Gap],
    },
    // zsh-only effects under an unknown dialect.
    Case {
        finding: "shell-dialect-mismatch",
        command: "echo PWNED >! .env",
        expect: &[Covers(".env", Modify), Gap],
    },
    Case {
        finding: "shell-dialect-mismatch",
        command: "echo PWNED >>!.env",
        expect: &[Covers(".env", Modify), Gap],
    },
    Case {
        finding: "shell-explicit-fd-dup-to-word-silent",
        command: "ls /x 2>& .env",
        expect: &[Covers(".env", Modify)],
    },
    Case {
        finding: "shell-explicit-fd-dup-to-word-silent",
        command: "echo x 2>&$FD",
        expect: &[Gap],
    },
    // Globs with quoted parts.
    Case {
        finding: "shell-glob-raw-quotes",
        command: "cat \".en\"v*",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-glob-raw-quotes",
        command: "cat '.e'n*",
        expect: &[Covers(".env", Read)],
    },
    // Home-relative operands.
    Case {
        finding: "shell-tilde-operands-no-candidate",
        command: "cat ~/.ssh/id_rsa",
        expect: &[Covers("/home/agent/.ssh/id_rsa", Read), Gap],
    },
    Case {
        finding: "shell-tilde-operands-no-candidate",
        command: "cp ~/.ssh/id_rsa /tmp/k",
        expect: &[Covers("/home/agent/.ssh/id_rsa", Read)],
    },
    Case {
        finding: "shell-tilde-assignment-words",
        command: "dd if=~/.ssh/id_rsa of=/tmp/k",
        expect: &[Covers("/home/agent/.ssh/id_rsa", Read), Gap],
    },
    // Long chains and deep nesting.
    Case {
        finding: "shell-depth-limit-drops-siblings",
        command: concat!(
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "true && true && true && true && true && true && true && true && true && true && ",
            "cat .env"
        ),
        expect: &[Covers(".env", Read)],
    },
    // Directory changes in deferred code.
    Case {
        finding: "shell-cwd-source-order",
        command: "for d in a b; do cat key; cd secrets; done",
        expect: &[Gap],
    },
    Case {
        finding: "shell-cwd-source-order",
        command: "builtin cd secrets; cat key",
        expect: &[Gap],
    },
    // Interpreters and wrappers.
    Case {
        finding: "shell-interpreters-unknown",
        command: "python3 -c 'print(open(\".env\").read())'",
        expect: &[Gap],
    },
    Case {
        finding: "shell-interpreters-unknown",
        command: "env cat .env",
        expect: &[Covers(".env", Read)],
    },
    Case {
        finding: "shell-interpreters-unknown",
        command: "sudo -u root rm -rf secrets",
        expect: &[Covers("secrets/key", Delete)],
    },
    Case {
        finding: "shell-interpreters-unknown",
        command: "find . -name x -exec cat {} +",
        expect: &[Gap],
    },
    // Separately lexed descriptors.
    Case {
        finding: "shell-fd-zero-parsed-as-argument",
        command: "cat 0<.env",
        expect: &[Covers(".env", Read)],
    },
    // Interactive pagers.
    Case {
        finding: "shell-pager-interactive-escape",
        command: "less README.md",
        expect: &[Gap],
    },
];

fn report_for(command: &str) -> (BashAnalysisOutcome, FileAccessReport) {
    let outcome = BashAnalyzer::default().analyze(command);
    let report = FileAccessAnalyzer::default()
        .with_unknown_command_fallback(UnknownCommandFallback::LiteralPathOperands)
        .infer(
            &outcome,
            FileInferenceContext::new(Some(Utf8Path::new(CWD)))
                .with_home(Some(Utf8Path::new(HOME))),
        );
    (outcome, report)
}

fn absolute(path: &str) -> PathBuf {
    if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        Path::new(CWD).join(path)
    }
}

fn access_matches(candidate: &FileAccessCandidate, access: Access) -> bool {
    match access {
        Access::Read => candidate.access.may_read(),
        Access::Modify => candidate.access.may_modify(),
        Access::Delete => matches!(
            candidate.access,
            FileAccessKind::Delete | FileAccessKind::MoveSource
        ),
    }
}

/// Whether a candidate's target covers `path`, using lexical containment for
/// directory scopes and a small glob matcher for glob scopes.
fn covers(candidate: &FileAccessCandidate, path: &Path) -> bool {
    let FileTarget::Path { expression, scope } = &candidate.target else {
        return true;
    };
    let Some(resolved) = expression.resolved.as_deref() else {
        return false;
    };
    let resolved = Path::new(resolved.as_str());
    match scope {
        FileTargetScope::Exact => resolved == path,
        FileTargetScope::Descendants => path.starts_with(resolved) && path != resolved,
        FileTargetScope::ExactOrDescendants => path.starts_with(resolved),
        FileTargetScope::Glob => {
            glob_matches(resolved.to_str().unwrap_or(""), path.to_str().unwrap_or(""))
        }
        _ => true,
    }
}

/// Matches `*`, `?`, and bracket-escaped single characters, which is all the
/// corpus needs.
fn glob_matches(pattern: &str, text: &str) -> bool {
    fn tokens(pattern: &str) -> Vec<Option<char>> {
        // `None` is `*`; `Some('\0')` is `?`.
        let characters = pattern.chars().collect::<Vec<_>>();
        let mut tokens = Vec::new();
        let mut index = 0;
        while index < characters.len() {
            match characters[index] {
                '*' => tokens.push(None),
                '?' => tokens.push(Some('\0')),
                '[' if characters.get(index + 2) == Some(&']') => {
                    tokens.push(Some(characters[index + 1]));
                    index += 2;
                }
                character => tokens.push(Some(character)),
            }
            index += 1;
        }
        tokens
    }
    fn matches(tokens: &[Option<char>], text: &[char]) -> bool {
        match tokens.split_first() {
            None => text.is_empty(),
            Some((None, rest)) => (0..=text.len()).any(|skip| matches(rest, &text[skip..])),
            Some((Some(expected), rest)) => text.split_first().is_some_and(|(first, tail)| {
                (*expected == '\0' || expected == first) && matches(rest, tail)
            }),
        }
    }
    matches(&tokens(pattern), &text.chars().collect::<Vec<_>>())
}

fn check(case: &Case) -> Vec<String> {
    let (_, report) = report_for(case.command);
    let mut failures = Vec::new();
    for expectation in case.expect {
        match expectation {
            Expect::Covers(path, access) => {
                let path = absolute(path);
                if !report
                    .candidates
                    .iter()
                    .any(|candidate| access_matches(candidate, *access) && covers(candidate, &path))
                {
                    failures.push(format!(
                        "[{}] {:?}: no {access:?} candidate covers {}\n{report:#?}",
                        case.finding,
                        case.command,
                        path.display()
                    ));
                }
            }
            Expect::Gap => {
                if report.is_fully_resolved() {
                    failures.push(format!(
                        "[{}] {:?}: expected an unresolved gap\n{report:#?}",
                        case.finding, case.command
                    ));
                }
            }
        }
    }
    failures
}

#[test]
fn every_adversarial_form_is_covered_or_unresolved() {
    let failures = CASES.iter().flat_map(check).collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn deep_nesting_keeps_shallow_siblings() {
    let command = format!("echo {}x{}; cat .env", "$(".repeat(150), ")".repeat(150));
    let (outcome, report) = report_for(&command);
    assert!(matches!(
        outcome,
        BashAnalysisOutcome::Partial {
            reason: IncompleteReason::DepthLimit { .. },
            ..
        }
    ));
    let env = absolute(".env");
    assert!(
        report
            .candidates
            .iter()
            .any(|candidate| access_matches(candidate, Read) && covers(candidate, &env))
    );
    assert!(!report.is_fully_resolved());
}

#[test]
fn escaped_heredoc_patch_bodies_are_not_literal() {
    // Bash joins `.e\<newline>nv` into `.env` before `apply_patch` reads it.
    let (outcome, _) = report_for("apply_patch <<EOF\n*** Delete File: .e\\\nnv\nEOF\n");
    let analysis = outcome.analysis().unwrap();
    let document = analysis.commands[0].redirections[0]
        .here_document
        .as_ref()
        .unwrap();
    assert_eq!(document.literal_body, None);
}

#[test]
fn enclosing_statement_heredocs_are_exposed_for_patch_consumers() {
    let (outcome, _) =
        report_for("(cd pkg && apply_patch) <<'EOF'\n*** Begin Patch\n*** End Patch\nEOF\n");
    let analysis = outcome.analysis().unwrap();
    let statement = &analysis.statement_redirections[0];
    let names = analysis.commands[statement.commands.clone()]
        .iter()
        .filter_map(|command| command.name.as_ref()?.literal.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(names, ["cd", "apply_patch"]);
    assert_eq!(
        statement.redirections[0].kind,
        RedirectionKind::HereDocument
    );
    assert_eq!(
        statement.redirections[0]
            .here_document
            .as_ref()
            .and_then(|document| document.literal_body.as_deref()),
        Some("*** Begin Patch\n*** End Patch\n")
    );

    // The common Codex form attaches the heredoc to `apply_patch` itself.
    let (outcome, _) =
        report_for("cd pkg && apply_patch <<'EOF'\n*** Begin Patch\n*** End Patch\nEOF\n");
    let analysis = outcome.analysis().unwrap();
    assert_eq!(
        analysis.commands[1].redirections[0].kind,
        RedirectionKind::HereDocument
    );
}

/// Benign commands must stay precise: exactly these candidates, no gaps.
#[test]
fn benign_commands_remain_fully_resolved() {
    for (command, expected) in [
        (
            "cat > src/main.rs <<'EOF'\nfn main() {}\nEOF\n",
            &[("src/main.rs", FileAccessKind::Modify)][..],
        ),
        ("ls | grep foo", &[(".", FileAccessKind::Enumerate)][..]),
        ("git log | grep -i fix || true", &[][..]),
        ("echo {} HEAD@{1} '{a,b}'", &[][..]),
        (
            "head -n 5 README.md 2>&1",
            &[("README.md", FileAccessKind::Read)][..],
        ),
        (
            "cat a.txt b.txt | tee out.txt >/dev/null",
            &[
                ("a.txt", FileAccessKind::Read),
                ("b.txt", FileAccessKind::Read),
                ("out.txt", FileAccessKind::Modify),
                ("/dev/null", FileAccessKind::Modify),
            ][..],
        ),
    ] {
        let (_, report) = report_for(command);
        if command.starts_with("git") {
            // `git` has no bundled semantics; only the search must be quiet.
            assert!(report.candidates.is_empty(), "{command}: {report:#?}");
            continue;
        }
        assert!(report.is_fully_resolved(), "{command}: {report:#?}");
        let mut actual = report
            .candidates
            .iter()
            .map(|candidate| match &candidate.target {
                FileTarget::Path { expression, .. } => (expression.raw.as_str(), candidate.access),
                _ => ("<workspace>", candidate.access),
            })
            .collect::<Vec<_>>();
        actual.sort_by_key(|(raw, _)| *raw);
        let mut expected = expected.to_vec();
        expected.sort_by_key(|(raw, _)| *raw);
        assert_eq!(actual, expected, "{command}");
    }
}

/// Optional differential check against a real `bash`.
mod differential {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;
    use std::process::Command;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Cases that are safe to execute inside a scratch directory.
    const EXECUTABLE: &[&str] = &[
        "rm 2>/dev/null .env",
        "cat 2>&1 .env",
        "cat </dev/null .env",
        "tee < .env out",
        "cat <<EOF > .env\nhello\nEOF\n",
        "rm <<EOF .env\nEOF\n",
        "cat <<EOF .env\nEOF\n",
        "cat .e{n,}v",
        "rm -f {.env,x}",
        "{ cat; } < .env",
        "(echo x) > .env",
        "if true; then echo x; fi > .env",
        "for f in a; do echo; done > .env",
        "f() { echo x; } > .env; f",
        "! cat > .env",
        "echo \"$(< .env)\"",
        "x=$(< .env); echo \"$x\"",
        "> .env",
        "cat <<EOF\n`cat .env`\nEOF\n",
        "cat <<EOF\nnote: `rm -rf secrets` done\nEOF\n",
        "cat .e\\\nnv",
        "echo x > .e\\\nnv",
        "sed 'r .env' README.md",
        "sed -n '1w .env' x",
        "sed 's/a/b/w .env' x",
        "mv secrets moved",
        "cp x secrets",
        "O='-f .env'; grep $O foo",
        "cat \".en\"v*",
        "cat 0<.env",
        "env cat .env",
        "a || rm 2>/dev/null .env",
    ];

    const SENTINELS: &[(&str, &str)] = &[
        (".env", "SENTINEL-ENV\n"),
        ("secrets/key", "SENTINEL-KEY\n"),
        ("README.md", "a readme\n"),
        ("x", "abc\n"),
        ("foo", "SENTINEL-FOO\n"),
        ("out", "old\n"),
    ];

    fn scratch_directory() -> PathBuf {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "hookkit-shell-differential-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(directory.join("secrets")).unwrap();
        for (path, contents) in SENTINELS {
            fs::write(directory.join(path), contents).unwrap();
        }
        directory
    }

    fn snapshot(directory: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::new();
        let mut pending = vec![directory.to_path_buf()];
        while let Some(current) = pending.pop() {
            let Ok(entries) = fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if let Ok(contents) = fs::read(&path) {
                    files.insert(
                        path.strip_prefix(directory).unwrap().to_path_buf(),
                        contents,
                    );
                }
            }
        }
        files
    }

    #[test]
    fn analysis_covers_every_effect_bash_performs() {
        if std::env::var_os("HOOKKIT_SHELL_DIFFERENTIAL").is_none() {
            return;
        }
        let mut failures = Vec::new();
        for command in EXECUTABLE {
            let directory = scratch_directory();
            let before = snapshot(&directory);
            let output = Command::new("bash")
                .arg("-c")
                .arg(command)
                .current_dir(&directory)
                .env("HOME", &directory)
                .output()
                .expect("bash runs");
            let after = snapshot(&directory);
            let printed = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );

            let mut touched = Vec::new();
            for (path, contents) in SENTINELS {
                if contents.starts_with("SENTINEL") && printed.contains(contents.trim()) {
                    touched.push((PathBuf::from(path), Access::Read));
                }
            }
            for (path, contents) in &before {
                match after.get(path) {
                    None => touched.push((path.clone(), Access::Delete)),
                    Some(new) if new != contents => touched.push((path.clone(), Access::Modify)),
                    _ => {}
                }
            }
            for path in after.keys().filter(|path| !before.contains_key(*path)) {
                touched.push((path.clone(), Access::Modify));
            }
            // Guard against a vacuous harness: every case has an observable
            // effect except the pattern-file read, whose patterns match nothing.
            if touched.is_empty() && !command.contains("grep $O") {
                failures.push(format!("{command:?}: bash showed no observable effect"));
            }

            let (_, report) = report_for(command);
            for (path, access) in touched {
                let absolute = Path::new(CWD).join(&path);
                let covered = report.candidates.iter().any(|candidate| {
                    covers(candidate, &absolute)
                        && (access_matches(candidate, access)
                            || (access == Access::Delete && candidate.access.may_modify()))
                });
                if !covered && report.is_fully_resolved() {
                    failures.push(format!(
                        "{command:?}: bash {access:?} {} but the complete report omits it\n{report:#?}",
                        path.display()
                    ));
                }
            }
            let _ = fs::remove_dir_all(&directory);
        }
        assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    }
}
