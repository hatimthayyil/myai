use crate::{
    master::{Master, mcp_config},
    mcp,
    prompt::system_prompt,
    session::{Event, Session, Spawner},
    tui::Tui,
    ui::Plain,
};
use ai_memory::{ClaudeCode, Compactor, Options, Store, Who, default_dir, prov};
use anyhow::Result;
use clap::{Args, Subcommand};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGTERM};
use std::{
    io::{self, BufRead, IsTerminal},
    path::PathBuf,
    process::ExitCode,
    sync::{
        Arc,
        mpsc::{Sender, channel},
    },
    thread,
    time::Duration,
};
#[derive(Args)]
pub struct Cli {
    #[arg(long, default_value = "opus")]
    model: String,
    #[arg(long, default_value = "high", value_parser = ["low", "medium", "high", "xhigh", "max"])]
    effort: String,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    #[command(about = "Serve the read-only zoom and date MCP tools over stdio.")]
    Mcp,
}
impl Cli {
    pub fn exec(self) -> Result<ExitCode> {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let dir = default_dir()?;
        let store = Store::open(&dir)?;
        if matches!(self.command, Some(Command::Mcp)) {
            mcp::serve(&store, io::stdin().lock(), &mut io::stdout().lock())?;
            return Ok(ExitCode::SUCCESS);
        }
        let cwd = std::env::current_dir()?;
        let program = ai_memory::claude::find()?;
        let master = Master::new(
            program.clone(),
            &self.model,
            &self.effort,
            &system_prompt(home.as_deref(), &cwd)?,
            mcp_config(&std::env::current_exe()?, &dir),
        )?;
        let backend = Arc::new(ClaudeCode::compactor_at(program, "sonnet")?);
        let mut waited = false;
        let mut compact = loop {
            if let Some(c) = Compactor::new(&store, backend.clone(), Options::default())? {
                break c;
            }
            if !waited {
                eprintln!("Waiting for the running compactor (ai memory nap or another chat) …");
                waited = true;
            }
            thread::sleep(Duration::from_secs(1));
        };
        let (tx, rx) = channel();
        let input = tx.clone();
        let tui = io::stdin().is_terminal() && io::stdout().is_terminal();
        let mut signals = signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP])?;
        let handle = signals.handle();
        let sigtx = tx.clone();
        thread::spawn(move || {
            for signal in signals.forever() {
                let event = if signal == SIGINT {
                    Event::Cancel
                } else {
                    Event::Exit
                };
                if sigtx.send(event).is_err() {
                    break;
                }
            }
        });
        let mut session = Session {
            compact: &mut compact,
            master: &master,
            spawn: Spawner::new(tx),
            place: prov::place(&cwd),
            who: Who {
                agent: "ai-chat".into(),
                model: self.model.clone(),
                session: std::process::id().to_string(),
            },
            prime: true,
        };
        let result = match tui {
            true => {
                let mut ui = Tui::start(input, &self.model, &self.effort)?;
                let result = session.run(rx, &mut ui);
                result.and(ui.finish())
            }
            false => {
                read_lines(input);
                session.run(rx, &mut Plain::new(io::stdout().lock()))
            }
        };
        handle.close();
        if let Err(e) = result
            && !e.chain().any(|cause| {
                cause
                    .downcast_ref::<io::Error>()
                    .is_some_and(|e| e.kind() == io::ErrorKind::BrokenPipe)
            })
        {
            return Err(e);
        }
        Ok(ExitCode::SUCCESS)
    }
}

/// Plain input: each stdin line is a message; its end exits a terminal, or ends a pipe once idle.
fn read_lines(input: Sender<Event>) {
    let tty = io::stdin().is_terminal();
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            match line {
                Ok(line) => {
                    if input.send(Event::Input(line)).is_err() {
                        return;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = input.send(if tty { Event::Exit } else { Event::End });
    });
}
