use clap::{Parser, Subcommand, ValueEnum};
use perfect_doc::{Config, model::Severity, report};
use std::{
    io::{self, Write},
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Parser)]
#[command(
    name = "perfect-doc",
    version,
    about = "Validate document structure, local references, and collection contracts."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Scan local documentation. External reachability is optional.
    Check {
        #[arg(default_value = ".")]
        roots: Vec<PathBuf>,
        /// Load a TOML contract. The default is perfect-doc.toml in the current directory.
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(long, value_enum, default_value = "human")]
        format: OutputFormat,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long, conflicts_with = "offline")]
        online: bool,
        #[arg(long, conflicts_with = "online")]
        offline: bool,
        #[arg(long, value_enum)]
        fail_on: Option<FailOn>,
        /// Print the effective TOML contract without a scan.
        #[arg(long)]
        show_config: bool,
    },
    /// List supported rule identifiers and their requirement sources.
    Rules {
        #[arg(long)]
        json: bool,
    },
    /// Write a complete default TOML contract. Existing files are protected.
    Init {
        #[arg(default_value = "perfect-doc.toml")]
        path: PathBuf,
    },
}
#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    Junit,
    Sarif,
}
#[derive(Clone, Copy, ValueEnum)]
enum FailOn {
    Info,
    Warning,
    Error,
}

fn run(cli: Cli) -> Result<u8, String> {
    match cli.command {
        Command::Rules { json } => {
            let rules = perfect_doc::rules();
            if json {
                write_output(
                    &serde_json::to_string_pretty(&rules).map_err(|e| e.to_string())?,
                    None,
                )?;
            } else {
                for rule in rules {
                    write_output(
                        &format!(
                            "{} [{:?}, family {}] {}\n",
                            rule.id, rule.requirement, rule.family, rule.description
                        ),
                        None,
                    )?;
                }
            }
            Ok(0)
        }
        Command::Init { path } => {
            let text = toml::to_string_pretty(&Config::default()).map_err(|e| e.to_string())?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|e| format!("Cannot create {}: {e}", path.display()))?;
            file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
            eprintln!("Created {}", path.display());
            Ok(0)
        }
        Command::Check {
            roots,
            config,
            format,
            output,
            online,
            offline,
            fail_on,
            show_config,
        } => {
            let explicit = config.is_some();
            let path = config.unwrap_or_else(|| PathBuf::from("perfect-doc.toml"));
            let mut config = if explicit || path.exists() {
                Config::from_file(&path)?
            } else {
                Config::default()
            };
            if online {
                config.network.enabled = true;
            }
            if offline {
                config.network.enabled = false;
            }
            if let Some(level) = fail_on {
                config.rules.fail_on = match level {
                    FailOn::Info => Severity::Info,
                    FailOn::Warning => Severity::Warning,
                    FailOn::Error => Severity::Error,
                };
            }
            if show_config {
                write_output(
                    &toml::to_string_pretty(&config).map_err(|e| e.to_string())?,
                    output,
                )?;
                return Ok(0);
            }
            let cancelled = Arc::new(AtomicBool::new(false));
            let token = cancelled.clone();
            ctrlc::set_handler(move || token.store(true, Ordering::Relaxed))
                .map_err(|e| format!("Cannot install the cancellation handler: {e}"))?;
            let result = perfect_doc::validate_with_cancel(&roots, &config, &cancelled)
                .map_err(|e| e.to_string())?;
            let text = match format {
                OutputFormat::Human => report::human(&result),
                OutputFormat::Json => report::json_report(&result).map_err(|e| e.to_string())?,
                OutputFormat::Junit => report::junit(&result),
                OutputFormat::Sarif => report::sarif(&result).map_err(|e| e.to_string())?,
            };
            write_output(&text, output)?;
            Ok(result.exit_code())
        }
    }
}
fn write_output(text: &str, path: Option<PathBuf>) -> Result<(), String> {
    if let Some(path) = path {
        std::fs::write(&path, text)
            .map_err(|e| format!("Cannot write report {}: {e}", path.display()))
    } else {
        let mut output = io::stdout().lock();
        output
            .write_all(text.as_bytes())
            .and_then(|_| {
                if text.ends_with('\n') {
                    Ok(())
                } else {
                    output.write_all(b"\n")
                }
            })
            .map_err(|e| format!("Cannot write report: {e}"))
    }
}
fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            if let Err(error) = error.print() {
                eprintln!("Cannot print command help: {error}");
                return ExitCode::from(2);
            }
            return ExitCode::from(code);
        }
    };
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("perfect-doc: {error}");
            ExitCode::from(2)
        }
    }
}
