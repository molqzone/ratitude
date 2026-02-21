use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsoleCommand {
    Help,
    Status,
    SourceList,
    SourceUse(usize),
    Sync,
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

impl CommandRouter {
    pub fn parse(line: &str) -> Option<ConsoleCommand> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }

        if trimmed == "$help" {
            return Some(ConsoleCommand::Help);
        }
        if trimmed == "$status" {
            return Some(ConsoleCommand::Status);
        }
        if trimmed == "$source list" {
            return Some(ConsoleCommand::SourceList);
        }
        if trimmed == "$sync" {
            return Some(ConsoleCommand::Sync);
        }
        if trimmed == "$quit" {
            return Some(ConsoleCommand::Quit);
        }

        if let Some(rest) = trimmed.strip_prefix("$source use ") {
            if let Ok(index) = rest.trim().parse::<usize>() {
                return Some(ConsoleCommand::SourceUse(index));
            }
            return Some(ConsoleCommand::Unknown(trimmed.to_string()));
        }

        if let Some(rest) = trimmed.strip_prefix("$foxglove ") {
            let mode = rest.trim().to_ascii_lowercase();
            return match mode.as_str() {
                "on" => Some(ConsoleCommand::Foxglove(true)),
                "off" => Some(ConsoleCommand::Foxglove(false)),
                _ => Some(ConsoleCommand::Unknown(trimmed.to_string())),
            };
        }

        if let Some(rest) = trimmed.strip_prefix("$jsonl ") {
            let rest = rest.trim();
            if rest.eq_ignore_ascii_case("off") {
                return Some(ConsoleCommand::Jsonl {
                    enabled: false,
                    path: None,
                });
            }
            if rest.eq_ignore_ascii_case("on") {
                return Some(ConsoleCommand::Jsonl {
                    enabled: true,
                    path: None,
                });
            }
            if let Some(path) = rest.strip_prefix("on ") {
                let path = path.trim();
                return Some(ConsoleCommand::Jsonl {
                    enabled: true,
                    path: if path.is_empty() {
                        None
                    } else {
                        Some(path.to_string())
                    },
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
        "available commands:\n  $help\n  $status\n  $source list\n  $source use <index>\n  $sync\n  $foxglove on|off\n  $jsonl on|off [path]\n  /packet/<struct>/<field>\n  $quit"
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
            "sync",
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
}
