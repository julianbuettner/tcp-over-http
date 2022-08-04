#[macro_use]
extern crate rocket;
use clap::{Parser, Subcommand};
mod code;
mod entry;
mod error;
mod exit;

#[derive(Clone, Debug, Subcommand)]
pub enum CommandMode {
    /// Spin up entry node. Receives incoming TCP and forwards HTTP.
    Entry {
        #[clap(short, long, value_parser, default_value = "127.0.0.1")]
        bind_ip: std::net::IpAddr,

        #[clap(short = 'p', long, value_parser, default_value = "1415")]
        listen_tcp_port: u16,
        /// URL of the exit node.
        #[clap(short, long, value_parser)]
        target_url: String,
    },
    /// Spin up exit node. Receives incoming HTTP and forwards TCP.
    Exit {
        #[clap(short, long, value_parser, default_value = "127.0.0.1")]
        bind_ip: std::net::IpAddr,

        #[clap(short = 'l', long, value_parser, default_value = "8080")]
        listen_http_port: u16,
        #[clap(short = 't', long, value_parser)]
        target_host: String,
        #[clap(short = 'p', long, value_parser)]
        target_port: u16,
        #[clap(short = 'd', long, value_parser)]
        debug: bool,
    },
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct CliArgs {
    #[clap(subcommand)]
    pub mode: CommandMode,
}

#[tokio::main]
async fn main() {
    let args = CliArgs::parse();

    match args.mode {
        CommandMode::Entry {
            bind_ip,
            listen_tcp_port,
            target_url,
        } => entry::entry_main(bind_ip, listen_tcp_port, target_url).await,
        CommandMode::Exit {
            bind_ip,
            listen_http_port,
            target_host,
            target_port,
            debug,
        } => exit::exit_main(bind_ip, listen_http_port, target_host, target_port, debug).await,
    }

    // receiver::receiver_main().await;
}
