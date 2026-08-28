//! `ctxc completions <SHELL>`.
//!
//! Hidden, because nobody types it twice: a shell reads the script once, at
//! startup, and from then on `ctxc <TAB>` answers out of the same command tree
//! that parses the command line. `ctxc init` prints the one line that installs
//! it, which is where people actually meet this command.
//!
//! The script goes to stdout on its own, with no report around it — every
//! shell expects to source exactly what the command printed.

use std::io::Write;

use anyhow::Result;
use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::Cli;
use crate::output::Printer;

pub fn run<W: Write>(shell: Shell, printer: &mut Printer<W>) -> Result<()> {
    let mut command = Cli::command();
    let name = command.get_name().to_string();

    let mut script = Vec::new();
    clap_complete::generate(shell, &mut command, name, &mut script);

    printer.write_content(&String::from_utf8_lossy(&script))?;
    Ok(())
}

/// The line that installs completions for `shell`, for `ctxc init` to print.
///
/// Every shell wants this in a different place, and guessing wrong writes into
/// somebody's startup file. So CtxC prints the command and leaves running it
/// to the person whose shell it is.
pub fn install_line(shell: Shell) -> String {
    match shell {
        Shell::Bash => {
            "ctxc completions bash > ~/.local/share/bash-completion/completions/ctxc".to_string()
        }
        Shell::Zsh => "ctxc completions zsh > \"${fpath[1]}/_ctxc\"".to_string(),
        Shell::Fish => "ctxc completions fish > ~/.config/fish/completions/ctxc.fish".to_string(),
        Shell::PowerShell => {
            "ctxc completions powershell | Out-String | Invoke-Expression".to_string()
        }
        Shell::Elvish => "ctxc completions elvish > ~/.config/elvish/lib/ctxc.elv".to_string(),
        // `Shell` is non-exhaustive: a shell clap learns about later still gets
        // a working command, just not a hand-written path for it.
        other => format!("ctxc completions {other}"),
    }
}

/// The shell this process was most likely started from.
///
/// A guess, and it says so by returning `None` when it cannot tell. On Windows
/// the answer is PowerShell often enough to be worth assuming; elsewhere
/// `$SHELL` is what the login shell set.
pub fn current_shell() -> Option<Shell> {
    if let Ok(shell) = std::env::var("SHELL") {
        let name = shell.rsplit(['/', '\\']).next().unwrap_or(&shell);
        if let Ok(shell) = name.parse::<Shell>() {
            return Some(shell);
        }
    }

    if cfg!(windows) {
        return Some(Shell::PowerShell);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::OutputFormat;

    #[test]
    fn every_shell_gets_a_script_naming_the_grouped_commands() {
        for shell in [
            Shell::Bash,
            Shell::Zsh,
            Shell::Fish,
            Shell::PowerShell,
            Shell::Elvish,
        ] {
            let mut buffer = Vec::new();
            run(shell, &mut Printer::new(OutputFormat::Human, &mut buffer)).unwrap();
            let script = String::from_utf8(buffer).unwrap();

            for command in ["init", "optimize", "find", "project", "dashboard"] {
                assert!(
                    script.contains(command),
                    "{shell} completions never mention `{command}`"
                );
            }
        }
    }

    #[test]
    fn the_install_line_is_a_command_someone_can_run() {
        assert!(install_line(Shell::Fish).starts_with("ctxc completions fish"));
        assert!(install_line(Shell::PowerShell).contains("Invoke-Expression"));
    }
}
