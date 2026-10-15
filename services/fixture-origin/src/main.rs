//! Run the fixture origin server locally.
//!
//! ```text
//! LYNX_FIXTURE_ADDR=127.0.0.1:8080 cargo run -p lynx-fixture-origin
//! ```

use std::net::SocketAddr;

use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr = std::env::var("LYNX_FIXTURE_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    let addr: SocketAddr = addr
        .parse()
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;

    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    println!("lynx-fixture-origin listening on http://{local}");

    lynx_fixture_origin::serve(listener).await
}
