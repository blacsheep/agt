use anyhow::Result;
use clap::Subcommand;
use std::{
    io::{self, Write},
    path::Path,
    time::{Duration, Instant},
};
#[derive(Debug, Subcommand)]
pub enum EventCommand {
    Watch {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        conversation: Option<String>,
        #[arg(long)]
        after: Option<String>,
    },
}
pub fn dispatch(command: EventCommand, home: &Path) -> Result<()> {
    let EventCommand::Watch {
        principal,
        conversation,
        mut after,
    } = command;
    let mut listener = agt_storage::wake::Hints::open(home);
    let mut feed = agt_runtime::events::Events::open(home)?;
    let token = crate::signals::cancellation();
    let mut stdout = io::stdout().lock();
    loop {
        let more = feed.project()?;
        let page = feed.page(&principal, conversation.as_deref(), after.as_deref())?;
        after = Some(page.cursor.clone());
        serde_json::to_writer(&mut stdout, &page)?;
        writeln!(stdout)?;
        stdout.flush()?;
        if token.is_canceled() {
            return Ok(());
        }
        if more || page.more {
            continue;
        }
        let waiting = Instant::now();
        let mut deadline = waiting + Duration::from_secs(15);
        while !token.is_canceled() {
            if !listener.available() {
                deadline = deadline.min(waiting + Duration::from_secs(1));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() || listener.wait(remaining.min(Duration::from_millis(250))) {
                break;
            }
        }
        if token.is_canceled() {
            return Ok(());
        }
    }
}
