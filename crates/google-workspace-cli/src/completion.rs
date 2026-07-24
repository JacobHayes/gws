// Copyright 2026 Google LLC
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Shell completion support.
//!
//! The command surface is Discovery-driven, so the generated shell functions ask
//! `gws __complete` for candidates at completion time instead of trying to embed
//! every Google API resource and method into a static script.

use std::path::Path;

use crate::discovery::RestDescription;
use crate::error::GwsError;

const SHELLS: &[&str] = &["bash", "zsh", "fish"];
const TOP_LEVEL_COMMANDS: &[&str] = &["auth", "schema", "generate-skills", "completion", "version"];
const TOP_LEVEL_FLAGS: &[&str] = &["--api-version", "--help", "-h", "--version", "-V"];
const GLOBAL_SERVICE_FLAGS: &[&str] = &["--api-version", "--dry-run", "--format", "--sanitize"];
const VALUE_FLAGS: &[&str] = &[
    "--api-version",
    "--filter",
    "--format",
    "--json",
    "--output",
    "-o",
    "--output-dir",
    "--page-delay",
    "--page-limit",
    "--params",
    "--project",
    "--sanitize",
    "--scopes",
    "--services",
    "--upload",
    "--upload-content-type",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Shell {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "bash" => Some(Self::Bash),
            "zsh" => Some(Self::Zsh),
            "fish" => Some(Self::Fish),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct CompleteRequest {
    current: Option<String>,
    shell: Option<Shell>,
    words: Vec<String>,
}

/// Print the shell integration script for `gws completion <shell>`.
pub fn handle_completion_command(args: &[String]) -> Result<(), GwsError> {
    let Some(shell_arg) = args.first().map(String::as_str) else {
        print_completion_usage();
        return Ok(());
    };

    if matches!(shell_arg, "--help" | "-h") {
        print_completion_usage();
        return Ok(());
    }

    let shell = Shell::parse(shell_arg).ok_or_else(|| {
        GwsError::Validation(format!(
            "Unsupported shell '{shell_arg}'. Usage: gws completion <bash|zsh|fish>"
        ))
    })?;

    print!("{}", completion_script(shell));
    Ok(())
}

/// Print completion candidates for the shell helper script.
pub async fn handle_complete_command(args: &[String]) -> Result<(), GwsError> {
    let request = parse_complete_request(args);
    for candidate in completion_candidates(&request.words, request.current.as_deref()).await {
        println!("{candidate}");
    }
    Ok(())
}

fn print_completion_usage() {
    println!("Generate shell completion scripts for gws.");
    println!();
    println!("USAGE:");
    println!("    gws completion <bash|zsh|fish>");
    println!();
    println!("EXAMPLES:");
    println!("    eval \"$(gws completion bash)\"");
    println!("    gws completion zsh > ~/.zfunc/_gws");
    println!("    gws completion fish > ~/.config/fish/completions/gws.fish");
}

fn completion_script(shell: Shell) -> &'static str {
    match shell {
        Shell::Bash => BASH_COMPLETION,
        Shell::Zsh => ZSH_COMPLETION,
        Shell::Fish => FISH_COMPLETION,
    }
}

const BASH_COMPLETION: &str = r#"# bash completion for gws
_gws() {
    local candidate
    COMPREPLY=()
    while IFS= read -r candidate; do
        COMPREPLY+=("$candidate")
    done < <(COMP_CWORD="${COMP_CWORD}" gws __complete --shell bash -- "${COMP_WORDS[@]}" 2>/dev/null)
    return 0
}
complete -o default -o bashdefault -F _gws gws
"#;

const ZSH_COMPLETION: &str = r#"#compdef gws
# zsh completion for gws
local -a completions
completions=("${(@f)$(COMP_CWORD=$((CURRENT - 1)) gws __complete --shell zsh -- "${words[@]}" 2>/dev/null)}")
if (( ${#completions[@]} )); then
    compadd -- "${completions[@]}"
fi
"#;

const FISH_COMPLETION: &str = r#"# fish completion for gws
function __gws_complete
    set -l tokens (commandline -opc)
    set -l current (commandline -ct)
    gws __complete --shell fish --current "$current" -- $tokens 2>/dev/null
end
complete -c gws -f -a "(__gws_complete)"
"#;

fn parse_complete_request(args: &[String]) -> CompleteRequest {
    let mut request = CompleteRequest::default();
    let mut passthrough = false;
    let mut i = 0;

    while i < args.len() {
        let arg = &args[i];
        if passthrough {
            request.words.push(arg.clone());
            i += 1;
            continue;
        }

        match arg.as_str() {
            "--" => {
                passthrough = true;
                i += 1;
            }
            "--shell" => {
                request.shell = args
                    .get(i + 1)
                    .and_then(|value| Shell::parse(value.as_str()));
                i += 2;
            }
            "--current" => {
                request.current = args.get(i + 1).cloned();
                i += 2;
            }
            _ => {
                request.words.push(arg.clone());
                i += 1;
            }
        }
    }

    if request.current.is_none() {
        let current_index = completion_word_index();
        if request.shell == Some(Shell::Bash) {
            let (words, remapped_index) = recombine_bash_wordbreaks(&request.words, current_index);
            request.words = words;
            request.current = current_at_index(&request.words, remapped_index);
        } else {
            request.current = current_at_index(&request.words, current_index);
        }
    }

    request
}

fn completion_word_index() -> Option<usize> {
    std::env::var("COMP_CWORD")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
}

fn current_at_index(words: &[String], index: Option<usize>) -> Option<String> {
    index.map(|index| words.get(index).cloned().unwrap_or_default())
}

fn recombine_bash_wordbreaks(
    words: &[String],
    current_index: Option<usize>,
) -> (Vec<String>, Option<usize>) {
    let mut combined = Vec::<String>::new();
    let mut index_map = Vec::with_capacity(words.len());
    let mut i = 0;

    while i < words.len() {
        if matches!(words[i].as_str(), ":" | "=") && !combined.is_empty() {
            let combined_index = combined.len() - 1;
            combined[combined_index].push_str(&words[i]);
            index_map.push(combined_index);
            i += 1;
            if i < words.len() {
                combined[combined_index].push_str(&words[i]);
                index_map.push(combined_index);
                i += 1;
            }
        } else {
            index_map.push(combined.len());
            combined.push(words[i].clone());
            i += 1;
        }
    }

    let remapped_index =
        current_index.map(|index| index_map.get(index).copied().unwrap_or(combined.len()));
    (combined, remapped_index)
}

async fn completion_candidates(words: &[String], explicit_current: Option<&str>) -> Vec<String> {
    let (completed_words, current) = normalize_completion_words(words, explicit_current);
    let completed_words = strip_leading_global_options(&completed_words);

    if completed_words
        .last()
        .is_some_and(|word| option_expects_value(word))
    {
        return Vec::new();
    }

    let candidates = if completed_words.is_empty() {
        top_level_candidates()
    } else {
        match completed_words[0].as_str() {
            "completion" => completion_command_candidates(&completed_words[1..]),
            "auth" => auth_command_candidates(&completed_words[1..]),
            "schema" => vec![
                "--resolve-refs".to_string(),
                "--help".to_string(),
                "-h".to_string(),
            ],
            "generate-skills" => vec![
                "--output-dir".to_string(),
                "--filter".to_string(),
                "--help".to_string(),
                "-h".to_string(),
            ],
            "version" => Vec::new(),
            service => service_command_candidates(service, &completed_words[1..]).await,
        }
    };

    filter_candidates(candidates, &current)
}

fn normalize_completion_words(
    words: &[String],
    explicit_current: Option<&str>,
) -> (Vec<String>, String) {
    let mut tokens = words.to_vec();
    if tokens.first().is_some_and(|word| is_gws_program(word)) {
        tokens.remove(0);
    }

    let current = explicit_current
        .map(ToOwned::to_owned)
        .or_else(|| tokens.last().cloned())
        .unwrap_or_default();

    if tokens.last().is_some_and(|word| word == &current) {
        tokens.pop();
    }
    if current.is_empty() && tokens.last().is_some_and(String::is_empty) {
        tokens.pop();
    }

    (tokens, current)
}

fn is_gws_program(word: &str) -> bool {
    let file_name = Path::new(word)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(word);
    matches!(file_name, "gws" | "gws.exe" | "run.js")
}

fn strip_leading_global_options(words: &[String]) -> Vec<String> {
    let mut i = 0;
    while i < words.len() {
        let word = &words[i];
        if matches!(word.as_str(), "--help" | "-h" | "--version" | "-V") {
            i += 1;
            continue;
        }
        if option_expects_value(word) {
            i += 2;
            continue;
        }
        if is_equals_option(word) {
            i += 1;
            continue;
        }
        break;
    }
    words[i..].to_vec()
}

fn top_level_candidates() -> Vec<String> {
    let mut candidates: Vec<String> = crate::services::SERVICES
        .iter()
        .flat_map(|entry| entry.aliases.iter().copied())
        .map(ToOwned::to_owned)
        .collect();
    candidates.extend(TOP_LEVEL_COMMANDS.iter().map(|value| value.to_string()));
    candidates.extend(TOP_LEVEL_FLAGS.iter().map(|value| value.to_string()));
    candidates
}

fn completion_command_candidates(words: &[String]) -> Vec<String> {
    if words.is_empty() {
        SHELLS.iter().map(|shell| shell.to_string()).collect()
    } else {
        Vec::new()
    }
}

fn auth_command_candidates(words: &[String]) -> Vec<String> {
    match words.first().map(String::as_str) {
        None => vec![
            "login".to_string(),
            "setup".to_string(),
            "status".to_string(),
            "export".to_string(),
            "logout".to_string(),
            "--help".to_string(),
            "-h".to_string(),
        ],
        Some("login") => vec![
            "--readonly".to_string(),
            "--full".to_string(),
            "--scopes".to_string(),
            "--services".to_string(),
            "--help".to_string(),
            "-h".to_string(),
        ],
        Some("setup") => vec![
            "--project".to_string(),
            "--login".to_string(),
            "--dry-run".to_string(),
            "--help".to_string(),
            "-h".to_string(),
        ],
        Some("export") => vec![
            "--unmasked".to_string(),
            "--help".to_string(),
            "-h".to_string(),
        ],
        Some("status" | "logout") => vec!["--help".to_string(), "-h".to_string()],
        Some(_) => Vec::new(),
    }
}

async fn service_command_candidates(service: &str, words: &[String]) -> Vec<String> {
    let Ok((api_name, version)) = resolve_completion_service(service, words) else {
        return Vec::new();
    };

    let doc = match completion_rest_description(&api_name, &version).await {
        Ok(doc) => doc,
        // Shell completion should stay quiet when Discovery is unavailable; a normal command
        // invocation still fails loudly through the main execution path.
        Err(_) => return Vec::new(),
    };

    let command = crate::commands::build_cli(&doc);
    command_candidates(&command, words)
}

fn resolve_completion_service(
    service: &str,
    words: &[String],
) -> Result<(String, String), GwsError> {
    let (service_name, colon_version) =
        if let Some((service_name, version)) = service.split_once(':') {
            if service_name.is_empty() || version.is_empty() {
                return Err(GwsError::Validation(
                    "Service override must use <api>:<version> syntax".to_string(),
                ));
            }
            (service_name, Some(version.to_string()))
        } else {
            (service, None)
        };

    let (api_name, default_version) = crate::services::resolve_service(service_name)?;
    let flag_version = words.iter().enumerate().find_map(|(index, word)| {
        word.strip_prefix("--api-version=")
            .map(ToOwned::to_owned)
            .or_else(|| {
                (word == "--api-version")
                    .then(|| words.get(index + 1).cloned())
                    .flatten()
            })
    });

    Ok((
        api_name,
        flag_version.or(colon_version).unwrap_or(default_version),
    ))
}

async fn completion_rest_description(
    api_name: &str,
    version: &str,
) -> anyhow::Result<RestDescription> {
    if api_name == "workflow" {
        Ok(RestDescription {
            name: "workflow".to_string(),
            description: Some("Cross-service productivity workflows".to_string()),
            ..Default::default()
        })
    } else {
        crate::discovery::fetch_discovery_document(api_name, version).await
    }
}

fn command_candidates(command: &clap::Command, words: &[String]) -> Vec<String> {
    let mut command = command;
    let mut i = 0;

    while i < words.len() {
        let word = &words[i];
        if word == "--" {
            return Vec::new();
        }
        if command_option_expects_value(command, word) && word.contains('=') {
            i += 1;
            continue;
        }
        if command_option_expects_value(command, word) || option_expects_value(word) {
            if i + 1 == words.len() {
                return Vec::new();
            }
            i += 2;
            continue;
        }
        if word.starts_with('-') || is_equals_option(word) {
            i += 1;
            continue;
        }

        if let Some(subcommand) = command
            .get_subcommands()
            .find(|subcommand| subcommand.get_name() == word)
        {
            command = subcommand;
            i += 1;
        } else {
            break;
        }
    }

    let mut candidates: Vec<String> = command
        .get_subcommands()
        .map(|subcommand| subcommand.get_name().to_string())
        .collect();

    candidates.extend(command_flag_candidates(command));
    candidates.extend(GLOBAL_SERVICE_FLAGS.iter().map(|flag| flag.to_string()));
    candidates.push("--help".to_string());
    candidates.push("-h".to_string());
    candidates
}

fn command_option_expects_value(command: &clap::Command, word: &str) -> bool {
    let option = word.split_once('=').map_or(word, |(option, _)| option);
    command.get_arguments().any(|arg| {
        let matches_long = arg
            .get_long()
            .is_some_and(|long| option == format!("--{long}"));
        let matches_short = arg
            .get_short()
            .is_some_and(|short| option == format!("-{short}"));
        (matches_long || matches_short) && arg.get_action().takes_values()
    })
}

fn command_flag_candidates(command: &clap::Command) -> Vec<String> {
    let mut candidates = Vec::new();
    for arg in command.get_arguments() {
        if let Some(long) = arg.get_long() {
            candidates.push(format!("--{long}"));
        }
        if let Some(short) = arg.get_short() {
            candidates.push(format!("-{short}"));
        }
    }
    candidates
}

fn option_expects_value(word: &str) -> bool {
    VALUE_FLAGS.contains(&word)
}

fn is_equals_option(word: &str) -> bool {
    VALUE_FLAGS
        .iter()
        .filter(|flag| flag.starts_with("--"))
        .any(|flag| word.starts_with(&format!("{flag}=")))
}

fn filter_candidates(mut candidates: Vec<String>, current: &str) -> Vec<String> {
    candidates.retain(|candidate| current.is_empty() || candidate.starts_with(current));
    candidates.sort();
    candidates.dedup();
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_script_contains_hidden_complete_command() {
        assert!(completion_script(Shell::Bash).contains("gws __complete"));
        assert!(completion_script(Shell::Zsh).contains("gws __complete"));
        assert!(completion_script(Shell::Fish).contains("gws __complete"));
    }

    #[test]
    fn zsh_completion_is_an_autoloadable_function_body() {
        let script = completion_script(Shell::Zsh);
        assert!(script.starts_with("#compdef gws"));
        assert!(!script.contains("_gws()"));
        assert!(!script.contains("compdef _gws"));
    }

    #[test]
    fn parse_complete_request_keeps_words_after_separator() {
        let args = vec![
            "--shell".to_string(),
            "bash".to_string(),
            "--current".to_string(),
            "dr".to_string(),
            "--".to_string(),
            "gws".to_string(),
            "dr".to_string(),
        ];

        let request = parse_complete_request(&args);
        assert_eq!(request.current.as_deref(), Some("dr"));
        assert_eq!(request.shell, Some(Shell::Bash));
        assert_eq!(request.words, vec!["gws", "dr"]);
    }

    #[test]
    fn recombine_bash_wordbreaks_preserves_service_and_equals_overrides() {
        let words = vec![
            "gws".to_string(),
            "drive".to_string(),
            ":".to_string(),
            "v3".to_string(),
            "--api-version".to_string(),
            "=".to_string(),
            "v2".to_string(),
            "files".to_string(),
        ];
        let (combined, current_index) = recombine_bash_wordbreaks(&words, Some(7));
        assert_eq!(
            combined,
            vec!["gws", "drive:v3", "--api-version=v2", "files"]
        );
        assert_eq!(current_index, Some(3));
    }

    #[test]
    fn normalize_words_removes_program_and_current_word() {
        let words = vec!["gws".to_string(), "dr".to_string()];
        let (completed, current) = normalize_completion_words(&words, Some("dr"));
        assert!(completed.is_empty());
        assert_eq!(current, "dr");
    }

    #[test]
    fn top_level_candidates_include_services_and_builtins() {
        let candidates = filter_candidates(top_level_candidates(), "dr");
        assert_eq!(candidates, vec!["drive"]);

        let candidates = filter_candidates(top_level_candidates(), "auth");
        assert_eq!(candidates, vec!["auth"]);
    }

    #[test]
    fn completion_command_suggests_supported_shells() {
        let candidates = filter_candidates(completion_command_candidates(&[]), "b");
        assert_eq!(candidates, vec!["bash"]);
    }

    #[test]
    fn auth_command_suggests_login_flags() {
        let candidates = filter_candidates(auth_command_candidates(&["login".to_string()]), "--r");
        assert_eq!(candidates, vec!["--readonly"]);
    }

    #[test]
    fn command_candidates_walk_subcommands_and_flags() {
        let command =
            clap::Command::new("gws").subcommand(clap::Command::new("files").subcommand(
                clap::Command::new("list").arg(clap::Arg::new("params").long("params")),
            ));

        let candidates =
            filter_candidates(command_candidates(&command, &["files".to_string()]), "li");
        assert_eq!(candidates, vec!["list"]);

        let candidates = filter_candidates(
            command_candidates(&command, &["files".to_string(), "list".to_string()]),
            "--p",
        );
        assert!(candidates.contains(&"--params".to_string()));
    }

    #[test]
    fn command_candidates_do_not_suggest_flags_for_helper_option_values() {
        let command = clap::Command::new("gws").subcommand(
            clap::Command::new("+meeting-prep").arg(clap::Arg::new("calendar").long("calendar")),
        );

        let candidates = command_candidates(
            &command,
            &["+meeting-prep".to_string(), "--calendar".to_string()],
        );
        assert!(candidates.is_empty());
    }

    #[test]
    fn completion_service_resolves_alias_before_colon_version_override() {
        assert_eq!(
            resolve_completion_service("wf:v1", &[]).unwrap(),
            ("workflow".to_string(), "v1".to_string())
        );
        assert_eq!(
            resolve_completion_service("reports:reports_v1", &[]).unwrap(),
            ("admin".to_string(), "reports_v1".to_string())
        );
    }

    #[test]
    fn completion_service_applies_api_version_flag_override() {
        assert_eq!(
            resolve_completion_service("drive", &["--api-version".to_string(), "v2".to_string()])
                .unwrap(),
            ("drive".to_string(), "v2".to_string())
        );
        assert_eq!(
            resolve_completion_service("drive:v3", &["--api-version=v2".to_string()]).unwrap(),
            ("drive".to_string(), "v2".to_string())
        );
    }
}
