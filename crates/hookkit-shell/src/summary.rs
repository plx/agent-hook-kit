//! Best-effort summaries for a deliberately small set of inspection commands.

use crate::bash::CommandOccurrence;

/// Converts one analyzed command occurrence into an application-defined
/// best-effort summary.
pub trait CommandSummarizer {
    /// Summary representation returned by this implementation.
    type Summary;

    /// Summarizes `command` without executing it.
    fn summarize(&self, command: &CommandOccurrence) -> Self::Summary;
}

/// Conservative summarizer for common file reads, listings, and searches.
#[derive(Debug, Clone, Copy, Default)]
pub struct InspectionSummarizer;

impl CommandSummarizer for InspectionSummarizer {
    type Summary = InspectionSummary;

    fn summarize(&self, command: &CommandOccurrence) -> Self::Summary {
        let Some(argv) = command.literal_argv() else {
            return InspectionSummary::Unknown;
        };
        let Some(executable) = argv.first().map(|value| executable_name(value)) else {
            return InspectionSummary::Unknown;
        };

        match executable {
            "cat" | "bat" | "less" | "more" | "nl" => {
                let Some(paths) = plain_path_operands(argv, 1, &[], &[], "", false) else {
                    return InspectionSummary::Unknown;
                };
                if paths.is_empty() {
                    InspectionSummary::Unknown
                } else {
                    InspectionSummary::Read {
                        command: executable.to_owned(),
                        paths,
                    }
                }
            }
            "head" | "tail" => {
                let Some(paths) = plain_path_operands(
                    argv,
                    1,
                    &["-n", "--lines", "-c", "--bytes"],
                    &["-q", "--quiet", "-v", "--verbose"],
                    "qv",
                    true,
                ) else {
                    return InspectionSummary::Unknown;
                };
                if paths.is_empty() {
                    InspectionSummary::Unknown
                } else {
                    InspectionSummary::Read {
                        command: executable.to_owned(),
                        paths,
                    }
                }
            }
            "ls" | "eza" => {
                let Some(paths) = plain_path_operands(
                    argv,
                    1,
                    &[],
                    &[
                        "--all",
                        "--almost-all",
                        "--long",
                        "--human-readable",
                        "--recursive",
                        "--classify",
                    ],
                    "aAlhRFrt1",
                    false,
                ) else {
                    return InspectionSummary::Unknown;
                };
                InspectionSummary::ListFiles {
                    command: executable.to_owned(),
                    paths,
                }
            }
            "tree" => {
                let Some(paths) = plain_path_operands(
                    argv,
                    1,
                    &["-L", "-P", "-I", "--charset", "--filelimit"],
                    &["-a", "-d", "-f", "-i", "--noreport"],
                    "adfi",
                    false,
                ) else {
                    return InspectionSummary::Unknown;
                };
                InspectionSummary::ListFiles {
                    command: executable.to_owned(),
                    paths,
                }
            }
            "du" => {
                let Some(paths) = plain_path_operands(
                    argv,
                    1,
                    &[
                        "-d",
                        "--max-depth",
                        "-B",
                        "--block-size",
                        "--exclude",
                        "--threshold",
                    ],
                    &[
                        "-a",
                        "--all",
                        "-h",
                        "--human-readable",
                        "-s",
                        "--summarize",
                        "-c",
                        "--total",
                    ],
                    "ahsc",
                    false,
                ) else {
                    return InspectionSummary::Unknown;
                };
                InspectionSummary::ListFiles {
                    command: executable.to_owned(),
                    paths,
                }
            }
            "git" if argv.get(1).map(String::as_str) == Some("ls-files") => {
                let Some(paths) = plain_path_operands(
                    argv,
                    2,
                    &[],
                    &[
                        "-c",
                        "--cached",
                        "-d",
                        "--deleted",
                        "-m",
                        "--modified",
                        "-o",
                        "--others",
                        "-i",
                        "--ignored",
                        "-s",
                        "--stage",
                        "-u",
                        "--unmerged",
                    ],
                    "cdmoisuk",
                    false,
                ) else {
                    return InspectionSummary::Unknown;
                };
                InspectionSummary::ListFiles {
                    command: "git ls-files".to_owned(),
                    paths,
                }
            }
            "rg" | "grep" => summarize_search(executable, argv),
            _ => InspectionSummary::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// Best-effort classification of a supported inspection command.
pub enum InspectionSummary {
    /// A recognized file-reading command with explicit path operands.
    Read {
        /// Normalized executable name used for the summary.
        command: String,
        /// Explicit path operands in argv order.
        paths: Vec<PathCandidate>,
    },
    /// A recognized file-listing command.
    ListFiles {
        /// Normalized executable or subcommand name.
        command: String,
        /// Empty means the command's current-directory default was retained.
        paths: Vec<PathCandidate>,
    },
    /// A recognized text-search command.
    Search {
        /// Normalized executable name.
        command: String,
        /// Literal recovered search query.
        query: String,
        /// Explicit search-root operands in argv order.
        paths: Vec<PathCandidate>,
    },
    /// The command, argv, or option combination was not safely summarizable.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
/// Literal path operand recovered from a command argv.
pub struct PathCandidate {
    /// Exact recovered argv value.
    pub value: String,
    /// Zero-based index in the recovered argv.
    pub argument_index: usize,
}

impl PathCandidate {
    fn new(value: &str, argument_index: usize) -> Self {
        Self {
            value: value.to_owned(),
            argument_index,
        }
    }
}

fn summarize_search(executable: &str, argv: &[String]) -> InspectionSummary {
    let mut index = 1;
    let mut query = None;
    let mut paths = Vec::new();
    let mut operands_only = false;
    // rg and GNU grep accept options after positionals, so option parsing
    // continues until `--`.
    while index < argv.len() {
        let argument = argv[index].as_str();
        if !operands_only {
            if argument == "--" {
                operands_only = true;
                index += 1;
                continue;
            }
            if matches!(argument, "-e" | "--regexp") {
                let Some(value) = argv.get(index + 1) else {
                    return InspectionSummary::Unknown;
                };
                if query.is_some() {
                    // Several patterns do not fit one summarized query.
                    return InspectionSummary::Unknown;
                }
                query = Some(value.clone());
                index += 2;
                continue;
            }
            if is_search_boolean_flag(argument) {
                index += 1;
                continue;
            }
            if argument.starts_with('-') && argument != "-" {
                return InspectionSummary::Unknown;
            }
        }
        if query.is_none() {
            query = Some(argument.to_owned());
        } else {
            paths.push(PathCandidate::new(argument, index));
        }
        index += 1;
    }

    let Some(query) = query else {
        return InspectionSummary::Unknown;
    };
    InspectionSummary::Search {
        command: executable.to_owned(),
        query,
        paths,
    }
}

fn plain_path_operands(
    argv: &[String],
    start: usize,
    options_with_values: &[&str],
    boolean_options: &[&str],
    short_boolean_letters: &str,
    allow_numeric_short_option: bool,
) -> Option<Vec<PathCandidate>> {
    let mut paths = Vec::new();
    let mut index = start;
    let mut operands_only = false;
    while index < argv.len() {
        let argument = &argv[index];
        if !operands_only && argument == "--" {
            operands_only = true;
            index += 1;
            continue;
        }
        if !operands_only && argument.starts_with('-') && argument != "-" {
            if options_with_values.contains(&argument.as_str()) {
                argv.get(index + 1)?;
                index += 2;
                continue;
            }
            if boolean_options.contains(&argument.as_str())
                || is_short_boolean_bundle(argument, short_boolean_letters)
                || (allow_numeric_short_option
                    && argument[1..]
                        .chars()
                        .all(|character| character.is_ascii_digit()))
            {
                index += 1;
                continue;
            }
            return None;
        }
        if argument != "-" {
            paths.push(PathCandidate::new(argument, index));
        }
        index += 1;
    }
    Some(paths)
}

fn is_short_boolean_bundle(argument: &str, allowed_letters: &str) -> bool {
    argument.starts_with('-')
        && !argument.starts_with("--")
        && argument.len() > 1
        && argument[1..]
            .chars()
            .all(|character| allowed_letters.contains(character))
}

fn is_search_boolean_flag(argument: &str) -> bool {
    matches!(
        argument,
        "-n" | "--line-number"
            | "-i"
            | "--ignore-case"
            | "-F"
            | "--fixed-strings"
            | "-R"
            | "-r"
            | "--recursive"
            | "--hidden"
            | "--no-ignore"
            | "-l"
            | "--files-with-matches"
    )
}

/// Normalizes executable names exactly as file-access inference does: a bare
/// name, or the basename of a path in a standard system binary directory.
fn executable_name(command: &str) -> &str {
    crate::file_access::builtin_command_name(command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BashAnalysisOutcome, BashAnalyzer};

    fn command(source: &str) -> CommandOccurrence {
        let BashAnalysisOutcome::Complete(mut analysis) = BashAnalyzer::default().analyze(source)
        else {
            panic!("expected complete analysis");
        };
        assert_eq!(analysis.commands.len(), 1);
        analysis.commands.remove(0)
    }

    #[test]
    fn summarizes_literal_reads_lists_and_searches() {
        let summarizer = InspectionSummarizer;
        assert!(matches!(
            summarizer.summarize(&command("cat README.md src/lib.rs")),
            InspectionSummary::Read { ref paths, .. } if paths.len() == 2
        ));
        assert!(matches!(
            summarizer.summarize(&command("ls -la crates")),
            InspectionSummary::ListFiles { ref paths, .. }
                if paths.first().map(|path| path.value.as_str()) == Some("crates")
        ));
        assert!(matches!(
            summarizer.summarize(&command("rg -n needle src")),
            InspectionSummary::Search { ref query, ref paths, .. }
                if query == "needle" && paths.len() == 1
        ));
        assert!(matches!(
            summarizer.summarize(&command("tree -L 2 crates")),
            InspectionSummary::ListFiles { ref paths, .. }
                if paths.iter().map(|path| path.value.as_str()).collect::<Vec<_>>() == ["crates"]
        ));
    }

    #[test]
    fn dynamic_or_ambiguous_commands_remain_unknown() {
        let summarizer = InspectionSummarizer;
        assert_eq!(
            summarizer.summarize(&command("cat $FILE")),
            InspectionSummary::Unknown
        );
        assert_eq!(
            summarizer.summarize(&command("rg --glob '*.rs' needle")),
            InspectionSummary::Unknown
        );
    }

    #[test]
    fn search_options_after_the_query_are_not_paths() {
        let summarizer = InspectionSummarizer;
        assert_eq!(
            summarizer.summarize(&command("rg needle -g '*.rs'")),
            InspectionSummary::Unknown
        );
        assert!(matches!(
            summarizer.summarize(&command("rg needle src -n")),
            InspectionSummary::Search { ref paths, .. }
                if paths.iter().map(|path| path.value.as_str()).collect::<Vec<_>>() == ["src"]
        ));
        assert!(matches!(
            summarizer.summarize(&command("grep -- -n src")),
            InspectionSummary::Search { ref query, ref paths, .. }
                if query == "-n" && paths.len() == 1
        ));
    }

    #[test]
    fn executable_names_match_file_access_normalization() {
        let summarizer = InspectionSummarizer;
        assert!(matches!(
            summarizer.summarize(&command("/bin/cat .env")),
            InspectionSummary::Read { .. }
        ));
        assert_eq!(
            summarizer.summarize(&command("./cat .env")),
            InspectionSummary::Unknown
        );
    }
}
