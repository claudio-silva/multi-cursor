use std::io::{self, Read};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use multi_cursor_core::{
    agent_about_email, bootstrap_cli_state, capture_current, export_account, import_account,
    remove_account, switch_account, AccountExport,
};

#[derive(Parser, Debug)]
#[command(
    name = "multi-cursor-cli",
    about = "Switch Cursor Agent CLI accounts",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Show Multi Cursor active account and cursor-agent about email
    Status,
    /// List known accounts
    List,
    /// Switch cursor-agent to a saved account
    Use {
        /// Email, display name, or account id
        account: String,
    },
    /// Save the current cursor-agent login as an account
    Capture {
        #[arg(long)]
        name: Option<String>,
    },
    /// Export an account snapshot (portable JSON)
    Export {
        account: String,
        #[arg(long, default_value = "-")]
        out: String,
    },
    /// Import an account snapshot from a file or stdin
    Import {
        /// Path to export JSON, or "-" for stdin
        file: String,
    },
    /// Remove a saved account
    Remove { account: String },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Status => cmd_status(),
        Commands::List => cmd_list(),
        Commands::Use { account } => {
            let account = switch_account(&account)?;
            println!(
                "Switched to {} ({})",
                account.name,
                account.email.as_deref().unwrap_or(&account.id)
            );
            if let Some(email) = agent_about_email() {
                println!("cursor-agent about: {email}");
            }
            Ok(())
        }
        Commands::Capture { name } => {
            let account = capture_current(name)?;
            println!(
                "Captured {} ({})",
                account.name,
                account.email.as_deref().unwrap_or(&account.id)
            );
            Ok(())
        }
        Commands::Export { account, out } => cmd_export(&account, &out),
        Commands::Import { file } => {
            let envelope = read_export(&file)?;
            let account = import_account(envelope)?;
            println!(
                "Imported {} ({})",
                account.name,
                account.email.as_deref().unwrap_or(&account.id)
            );
            Ok(())
        }
        Commands::Remove { account } => {
            let account = remove_account(&account)?;
            println!(
                "Removed {} ({})",
                account.name,
                account.email.as_deref().unwrap_or(&account.id)
            );
            Ok(())
        }
    }
}

fn cmd_status() -> Result<(), String> {
    let cfg = bootstrap_cli_state()?;
    let active = cfg
        .active
        .account_id
        .as_ref()
        .and_then(|id| cfg.accounts.iter().find(|a| a.id == *id));

    match active {
        Some(account) => println!(
            "multi-cursor active: {} ({})",
            account.name,
            account.email.as_deref().unwrap_or(&account.id)
        ),
        None => println!("multi-cursor active: (none)"),
    }

    match agent_about_email() {
        Some(email) => println!("cursor-agent about: {email}"),
        None => println!("cursor-agent about: (unavailable)"),
    }
    Ok(())
}

fn cmd_list() -> Result<(), String> {
    let cfg = bootstrap_cli_state()?;
    if cfg.accounts.is_empty() {
        println!("No accounts saved. Run `multi-cursor-cli capture` or `import`.");
        return Ok(());
    }

    for account in &cfg.accounts {
        let marker = if cfg.active.account_id.as_deref() == Some(account.id.as_str()) {
            "*"
        } else {
            " "
        };
        let email = account.email.as_deref().unwrap_or("-");
        println!("{marker} {}  {}  [{}]", account.name, email, account.id);
    }
    Ok(())
}

fn cmd_export(query: &str, out: &str) -> Result<(), String> {
    let envelope = export_account(query)?;
    let raw = serde_json::to_string_pretty(&envelope).map_err(|e| e.to_string())?;
    if out == "-" {
        println!("{raw}");
    } else {
        let path = PathBuf::from(out);
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(&path, format!("{raw}\n")).map_err(|e| e.to_string())?;
        println!("Wrote {}", path.display());
    }
    Ok(())
}

fn read_export(file: &str) -> Result<AccountExport, String> {
    let raw = if file == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("Read stdin: {e}"))?;
        buf
    } else {
        std::fs::read_to_string(file).map_err(|e| format!("Read {file}: {e}"))?
    };
    serde_json::from_str(&raw).map_err(|e| format!("Invalid export JSON: {e}"))
}
