use anyhow::{anyhow, Result};
use clap::Parser;
use rat_config::DEFAULT_CONFIG_PATH;

#[derive(Parser, Debug, Clone)]
#[command(name = "ratd", about = "Ratitude interactive daemon")]
pub struct Cli {
    #[arg(
        long,
        default_value = DEFAULT_CONFIG_PATH,
        help = "Path to rat.toml config file"
    )]
    pub config: String,
}

pub fn parse_cli() -> Result<Cli> {
    let args = std::env::args().collect::<Vec<String>>();
    reject_positional_subcommand(&args)?;
    Ok(Cli::parse())
}

#[cfg(test)]
fn parse_cli_from<I, T>(args: I) -> Result<Cli>
where
    I: IntoIterator<Item = T>,
    T: Into<String>,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<String>>();
    reject_positional_subcommand(&args)?;

    Cli::try_parse_from(args).map_err(|err| anyhow!(err.to_string()))
}

fn reject_positional_subcommand(args: &[String]) -> Result<()> {
    if let Some(first) = args.get(1) {
        if !first.starts_with('-') {
            return Err(anyhow!(
                "positional subcommands are not supported; start `ratd` and use console commands"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_accepts_default_entry() {
        let cli = parse_cli_from(["ratd"]).expect("parse");
        assert_eq!(cli.config, "rat.toml");
    }

    #[test]
    fn cli_rejects_removed_positional_mode() {
        let err = parse_cli_from(["ratd", "old_mode"]).expect_err("must reject");
        assert!(err
            .to_string()
            .contains("positional subcommands are not supported"));
    }

    #[test]
    fn cli_accepts_config_flag() {
        let cli = parse_cli_from(["ratd", "--config", "examples/mock/rat.toml"]).expect("parse");
        assert_eq!(cli.config, "examples/mock/rat.toml");
    }
}
