use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleCommand {
    Help,
    Status,
    SourceList,
    SourceUse(usize),
    Foxglove(bool),
    Jsonl {
        enabled: bool,
        path: Option<String>,
    },
    PacketLookup {
        struct_name: String,
        field_name: String,
    },
    Quit,
    Unknown(String),
}

pub struct CommandRouter;

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let prefix_len = prefix.len();
    if value.len() < prefix_len {
        return None;
    }
    let head = &value[..prefix_len];
    if head.eq_ignore_ascii_case(prefix) {
        Some(&value[prefix_len..])
    } else {
        None
    }
}

impl CommandRouter {
    pub fn parse(line: &str) -> Option<ConsoleCommand> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        if trimmed.eq_ignore_ascii_case("$help") {
            return Some(ConsoleCommand::Help);
        }
        if trimmed.eq_ignore_ascii_case("$status") {
            return Some(ConsoleCommand::Status);
        }
        if trimmed.eq_ignore_ascii_case("$quit") {
            return Some(ConsoleCommand::Quit);
        }

        if let Some(rest) = strip_prefix_ignore_ascii_case(trimmed, "$source") {
            if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
                return Some(ConsoleCommand::Unknown(trimmed.to_string()));
            }
            let mut parts = rest.split_whitespace();
            match (parts.next(), parts.next(), parts.next()) {
                (Some(command), None, None) if command.eq_ignore_ascii_case("list") => {
                    return Some(ConsoleCommand::SourceList);
                }
                (Some(command), Some(index), None) if command.eq_ignore_ascii_case("use") => {
                    if let Ok(parsed_index) = index.parse::<usize>() {
                        return Some(ConsoleCommand::SourceUse(parsed_index));
                    }
                    return Some(ConsoleCommand::Unknown(trimmed.to_string()));
                }
                _ => return Some(ConsoleCommand::Unknown(trimmed.to_string())),
            }
        }

        if let Some(rest) = strip_prefix_ignore_ascii_case(trimmed, "$foxglove") {
            if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
                return Some(ConsoleCommand::Unknown(trimmed.to_string()));
            }
            let mut parts = rest.split_whitespace();
            return match (parts.next(), parts.next(), parts.next()) {
                (Some(mode), None, None) if mode.eq_ignore_ascii_case("on") => {
                    Some(ConsoleCommand::Foxglove(true))
                }
                (Some(mode), None, None) if mode.eq_ignore_ascii_case("off") => {
                    Some(ConsoleCommand::Foxglove(false))
                }
                _ => Some(ConsoleCommand::Unknown(trimmed.to_string())),
            };
        }

        if let Some(rest) = strip_prefix_ignore_ascii_case(trimmed, "$jsonl") {
            if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
                return Some(ConsoleCommand::Unknown(trimmed.to_string()));
            }
            let rest = rest.trim();
            let mut parts = rest.splitn(2, char::is_whitespace);
            let mode = parts.next().unwrap_or_default();
            let tail = parts.next().map(str::trim);

            if mode.eq_ignore_ascii_case("off") {
                if tail.is_none() || tail == Some("") {
                    return Some(ConsoleCommand::Jsonl {
                        enabled: false,
                        path: None,
                    });
                }
                return Some(ConsoleCommand::Unknown(trimmed.to_string()));
            }

            if mode.eq_ignore_ascii_case("on") {
                return Some(ConsoleCommand::Jsonl {
                    enabled: true,
                    path: tail.and_then(|path| {
                        if path.is_empty() {
                            None
                        } else {
                            Some(path.to_string())
                        }
                    }),
                });
            }
            return Some(ConsoleCommand::Unknown(trimmed.to_string()));
        }

        if let Some(route) = trimmed.strip_prefix("/packet/") {
            let mut parts = route.split('/');
            if let (Some(struct_name), Some(field_name), None) =
                (parts.next(), parts.next(), parts.next())
            {
                if !struct_name.is_empty() && !field_name.is_empty() {
                    return Some(ConsoleCommand::PacketLookup {
                        struct_name: struct_name.to_string(),
                        field_name: field_name.to_string(),
                    });
                }
            }
            return Some(ConsoleCommand::Unknown(trimmed.to_string()));
        }

        Some(ConsoleCommand::Unknown(trimmed.to_string()))
    }
}

pub fn print_help() {
    println!(
        "available commands:\n  $help\n  $status\n  $source list\n  $source use <index>\n  $foxglove on|off\n  $jsonl on|off [path]\n  /packet/<struct>/<field>\n  $quit"
    );
}

pub fn spawn_console_reader(shutdown: CancellationToken) -> mpsc::Receiver<ConsoleCommand> {
    let (tx, rx) = mpsc::channel::<ConsoleCommand>(64);
    tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let mut lines = BufReader::new(stdin).lines();
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => break,
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if let Some(command) = CommandRouter::parse(&line) {
                                if tx.send(command).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_source_use() {
        let cmd = CommandRouter::parse("$source use 3").expect("command");
        assert_eq!(cmd, ConsoleCommand::SourceUse(3));
    }

    #[test]
    fn parse_source_list_accepts_flexible_whitespace() {
        let cmd = CommandRouter::parse("$source\tlist").expect("command");
        assert_eq!(cmd, ConsoleCommand::SourceList);
    }

    #[test]
    fn parse_source_use_accepts_flexible_whitespace() {
        let cmd = CommandRouter::parse("$source   use\t3").expect("command");
        assert_eq!(cmd, ConsoleCommand::SourceUse(3));
    }

    #[test]
    fn parse_source_list_is_case_insensitive() {
        let cmd = CommandRouter::parse("$source LIST").expect("command");
        assert_eq!(cmd, ConsoleCommand::SourceList);
    }

    #[test]
    fn parse_source_use_is_case_insensitive() {
        let cmd = CommandRouter::parse("$source USE 3").expect("command");
        assert_eq!(cmd, ConsoleCommand::SourceUse(3));
    }

    #[test]
    fn parse_source_prefix_is_case_insensitive() {
        let cmd = CommandRouter::parse("$SOURCE use 3").expect("command");
        assert_eq!(cmd, ConsoleCommand::SourceUse(3));
    }

    #[test]
    fn parse_jsonl_on_with_path() {
        let cmd = CommandRouter::parse("$jsonl on out.jsonl").expect("command");
        assert_eq!(
            cmd,
            ConsoleCommand::Jsonl {
                enabled: true,
                path: Some("out.jsonl".to_string())
            }
        );
    }

    #[test]
    fn parse_jsonl_on_with_path_is_case_insensitive() {
        let cmd = CommandRouter::parse("$jsonl ON out.jsonl").expect("command");
        assert_eq!(
            cmd,
            ConsoleCommand::Jsonl {
                enabled: true,
                path: Some("out.jsonl".to_string())
            }
        );
    }

    #[test]
    fn parse_jsonl_on_with_path_accepts_flexible_whitespace() {
        let cmd = CommandRouter::parse("$jsonl\tON\tout.jsonl").expect("command");
        assert_eq!(
            cmd,
            ConsoleCommand::Jsonl {
                enabled: true,
                path: Some("out.jsonl".to_string())
            }
        );
    }

    #[test]
    fn parse_jsonl_prefix_is_case_insensitive() {
        let cmd = CommandRouter::parse("$JSONL on out.jsonl").expect("command");
        assert_eq!(
            cmd,
            ConsoleCommand::Jsonl {
                enabled: true,
                path: Some("out.jsonl".to_string())
            }
        );
    }

    #[test]
    fn parse_foxglove_accepts_flexible_whitespace() {
        let cmd = CommandRouter::parse("$foxglove\toff").expect("command");
        assert_eq!(cmd, ConsoleCommand::Foxglove(false));
    }

    #[test]
    fn parse_packet_lookup() {
        let cmd = CommandRouter::parse("/packet/GyroSample/value").expect("command");
        assert_eq!(
            cmd,
            ConsoleCommand::PacketLookup {
                struct_name: "GyroSample".to_string(),
                field_name: "value".to_string()
            }
        );
    }

    #[test]
    fn parse_rejects_plain_aliases_without_dollar_prefix() {
        let samples = [
            "help",
            "status",
            "source list",
            "source use 1",
            "foxglove on",
            "jsonl off",
            "quit",
            "exit",
        ];

        for raw in samples {
            let cmd = CommandRouter::parse(raw).expect("command");
            assert_eq!(cmd, ConsoleCommand::Unknown(raw.to_string()));
        }
    }

    #[test]
    fn parse_rejects_removed_sync_command() {
        let cmd = CommandRouter::parse("$sync").expect("command");
        assert_eq!(cmd, ConsoleCommand::Unknown("$sync".to_string()));
    }
}
