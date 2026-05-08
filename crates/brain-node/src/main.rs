mod grpc;
#[cfg(test)]
mod integration_tests;
mod mdns_adv;
mod pair;
mod session;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use grpc::{proto::aether_brain_server::AetherBrainServer, BrainService};
use session::SessionRegistry;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};

#[derive(Parser)]
#[command(name = "brain-node", about = "Aether brain node")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the mTLS gRPC server and advertise via mDNS.
    Serve {
        #[arg(long, env = "BRAIN_GRPC_PORT", default_value = "50051")]
        port: u16,

        #[arg(long, env = "BRAIN_CERTS_DIR", default_value = "/data/certs")]
        certs_dir: PathBuf,
    },

    /// Pairing ceremony — plain (non-TLS) gRPC on a separate port.
    /// Plug the Pi in with a direct cable before running this.
    Pair {
        #[arg(long, env = "BRAIN_PAIR_PORT", default_value = "50052")]
        port: u16,

        #[arg(long, env = "BRAIN_CERTS_DIR", default_value = "/data/certs")]
        certs_dir: PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    match Cli::parse().command {
        Command::Serve { port, certs_dir } => serve(port, certs_dir).await,
        Command::Pair { port, certs_dir } => run_pair_server(port, certs_dir).await,
    }
}

async fn serve(port: u16, certs_dir: PathBuf) -> Result<()> {
    let local_ip = local_ip_address::local_ip().context("detecting local IP")?;
    tracing::info!(ip = %local_ip, "brain local address");

    pair::ensure_certs(&certs_dir, local_ip).context("ensuring certs")?;

    let ca_pem = std::fs::read(certs_dir.join("ca.pem"))?;
    let server_cert_pem = std::fs::read(certs_dir.join("brain.pem"))?;
    let server_key_pem = std::fs::read(certs_dir.join("brain-key.pem"))?;

    let identity = Identity::from_pem(&server_cert_pem, &server_key_pem);
    let client_ca = Certificate::from_pem(&ca_pem);
    let tls = ServerTlsConfig::new()
        .identity(identity)
        .client_ca_root(client_ca);

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let service = BrainService {
        registry: SessionRegistry::new(),
        certs_dir,
    };

    let _mdns = match local_ip {
        IpAddr::V4(v4) => mdns_adv::advertise(port, v4).ok(),
        IpAddr::V6(_) => {
            tracing::warn!("IPv6 local address — skipping mDNS advertisement");
            None
        }
    };

    tracing::info!(%addr, "brain gRPC server starting (mTLS)");
    Server::builder()
        .tls_config(tls)?
        .add_service(AetherBrainServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}

async fn run_pair_server(port: u16, certs_dir: PathBuf) -> Result<()> {
    let local_ip = local_ip_address::local_ip().context("detecting local IP")?;
    pair::ensure_certs(&certs_dir, local_ip).context("ensuring certs")?;

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    let service = BrainService {
        registry: SessionRegistry::new(),
        certs_dir,
    };

    tracing::info!(%addr, "pairing server listening (plain gRPC)");
    println!("Pairing mode active on port {port}.  Connect the Pi with a cable, then run:");
    println!("  edge-node pair --brain-addr <this-machine-ip>:{port} --node-id <name>");

    Server::builder()
        .add_service(AetherBrainServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
