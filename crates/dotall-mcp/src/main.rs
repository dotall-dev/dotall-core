use clap::Parser;
use dotall_mcp::server::ServerOptions;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = ServerOptions::parse();
    let workspace = std::env::current_dir()?;
    let server =
        dotall_mcp::server::DotallServer::open_or_init(workspace, options.flush_on_close())?;
    let service = server.clone().serve(stdio()).await?;

    let session_result = service.waiting().await;
    server.flush_staged();
    session_result?;
    Ok(())
}
