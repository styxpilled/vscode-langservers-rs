mod context;
mod embedded;
#[cfg(feature = "javascript")]
mod javascript;
mod modes;
#[cfg(test)]
mod robustness;
mod server;
mod settings;
#[cfg(test)]
mod tests;

use anyhow::Result;

// Clients pass `--stdio`. Stdio is the only transport, so arguments are ignored.
fn main() -> Result<()> {
    lsp_core::run_server(run)
}

fn run() -> Result<()> {
    let (receiver, writer, io_threads) = lsp_core::stdio();
    let Some((id, params)) = lsp_core::initialize_start(&receiver, &writer) else {
        std::process::exit(1);
    };
    let (server, result) = server::Server::new(writer.clone(), receiver, &params);
    writer.response(id, &result);
    drop(writer);
    let clean_exit = server.run();
    io_threads.join()?;
    if !clean_exit {
        std::process::exit(1);
    }
    Ok(())
}
