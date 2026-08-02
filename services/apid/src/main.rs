//! API Gateway Daemon entry point.

use tokio::net::TcpListener;

#[granola::service("apid")]
#[tokio::main]
async fn main(notifier: NotifyClient) -> Result<()> {
    config::init()?;

    let args = apid::parse_args(&std::env::args().collect::<Vec<_>>());

    let addr: core::net::SocketAddr = args.listen_addr.parse()?;
    let listener = TcpListener::bind(addr).await?;
    let tls_acceptor = apid::setup_tls(args.maintenance_mode)?;

    kmsg::info!("API daemon ready, listening on {}", addr);
    notifier.ready()?;

    apid::run(
        &listener,
        &tls_acceptor,
        granola::shutdown_signal(),
        args.maintenance_mode,
    )
    .await;

    Ok(())
}
