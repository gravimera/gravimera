use anyhow::Result;
use clap::Parser;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = soundtest::cli::Cli::parse();
    soundtest::run(cli).await
}
