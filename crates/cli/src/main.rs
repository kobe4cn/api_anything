use clap::Parser;

#[derive(Parser)]
#[command(name = "api-anything", about = "AI-powered legacy system API gateway generator")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Generate REST API from legacy system
    Generate {
        /// Path to source contract (WSDL, help output, etc.)
        #[arg(short, long)]
        source: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _cli = Cli::parse();
    println!("API-Anything CLI — not yet implemented");
    Ok(())
}
