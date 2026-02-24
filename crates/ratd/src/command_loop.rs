use anyhow::Result;

use crate::config_io::save_config;
use crate::console::{print_help, ConsoleCommand};
use crate::daemon::DaemonState;
use crate::output_manager::OutputManager;
use crate::source_scan::render_candidates;
use crate::source_state::refresh_source_candidates;

#[derive(Default)]
pub(crate) struct CommandAction {
    pub(crate) should_quit: bool,
    pub(crate) restart_runtime: bool,
}

pub(crate) async fn handle_console_command(
    command: ConsoleCommand,
    state: &mut DaemonState,
    output_manager: &mut OutputManager,
) -> Result<CommandAction> {
    let mut action = CommandAction::default();

    match command {
        ConsoleCommand::Help => {
            print_help();
        }
        ConsoleCommand::Status => {
            let output = output_manager.snapshot();
            println!("status:");
            println!("  source: {}", state.source().active_addr());
            println!("  packets: {}", state.runtime().schema().packet_count());
            println!(
                "  jsonl: {}",
                if output.jsonl_enabled { "on" } else { "off" }
            );
            println!(
                "  foxglove: {} ({})",
                if output.foxglove_enabled { "on" } else { "off" },
                output.foxglove_ws_addr
            );
        }
        ConsoleCommand::SourceList => {
            let source_cfg = state.config().ratd.source.clone();
            refresh_source_candidates(state.source_mut(), &source_cfg).await;
            render_candidates(state.source().candidates());
        }
        ConsoleCommand::SourceUse(index) => {
            let source_cfg = state.config().ratd.source.clone();
            refresh_source_candidates(state.source_mut(), &source_cfg).await;
            let Some(candidate) = state.source().candidate(index).cloned() else {
                println!("invalid source index: {}", index);
                render_candidates(state.source().candidates());
                return Ok(action);
            };

            let mut next = state.config().clone();
            next.ratd.source.last_selected_addr = candidate.addr.clone();
            save_config(state.config_path(), &next).await?;
            state.replace_config(next);
            state.source_mut().set_active_addr(candidate.addr.clone());
            println!("selected source: {}", state.source().active_addr());
            action.restart_runtime = true;
        }
        ConsoleCommand::Foxglove(enabled) => {
            let mut next = state.config().clone();
            next.ratd.outputs.foxglove.enabled = enabled;
            save_config(state.config_path(), &next).await?;
            state.replace_config(next);
            output_manager.reload_from_config(state.config())?;
            println!("foxglove output: {}", if enabled { "on" } else { "off" });
        }
        ConsoleCommand::Jsonl { enabled, path } => {
            let mut next = state.config().clone();
            next.ratd.outputs.jsonl.enabled = enabled;
            if let Some(path) = path {
                next.ratd.outputs.jsonl.path = path;
            }
            save_config(state.config_path(), &next).await?;
            state.replace_config(next);
            output_manager.reload_from_config(state.config())?;
            println!("jsonl output: {}", if enabled { "on" } else { "off" });
        }
        ConsoleCommand::PacketLookup {
            struct_name,
            field_name,
        } => {
            let packet = state
                .runtime()
                .schema()
                .packets()
                .iter()
                .find(|packet| packet.struct_name.eq_ignore_ascii_case(&struct_name));
            if let Some(packet) = packet {
                let field = packet
                    .fields
                    .iter()
                    .find(|field| field.name.eq_ignore_ascii_case(&field_name));
                if let Some(field) = field {
                    println!(
                        "packet {} field {} => type={}, offset={}, size={}",
                        packet.struct_name, field.name, field.c_type, field.offset, field.size
                    );
                } else {
                    println!("field not found: {}", field_name);
                }
            } else {
                println!("packet not found: {}", struct_name);
            }
        }
        ConsoleCommand::Quit => {
            action.should_quit = true;
        }
        ConsoleCommand::Unknown(raw) => {
            println!("unknown command: {}", raw);
        }
    }

    Ok(action)
}
