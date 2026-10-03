//! #107: what `run_isolated` touches must outlive it. It used to run each call
//! on a throwaway current-thread runtime: a socket opened inside stayed bound
//! to that runtime's dead I/O driver (Postgres connections left in the pool
//! then held every later acquire until `acquire_timeout`), and a task the
//! library `tokio::spawn`ed died with it. Multi-thread, like the daemon's
//! `#[tokio::main]`.

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::run_isolated;

const PATIENCE: Duration = Duration::from_secs(2);

/// Accept connections and echo back whatever each one sends.
async fn echo_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind echo");
    let addr = listener.local_addr().expect("echo addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 64];
                while let Ok(n) = socket.read(&mut buf).await {
                    if n == 0 || socket.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    addr
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_socket_opened_inside_run_isolated_stays_usable_after_it_returns() {
    let addr = echo_server().await;
    let mut stream: TcpStream =
        run_isolated(
            move || async move { TcpStream::connect(addr).await.map_err(anyhow::Error::from) },
        )
        .await
        .expect("connect inside run_isolated");

    let round_trip = async {
        stream.write_all(b"ping").await?;
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await?;
        Ok::<_, std::io::Error>(buf)
    };
    let echoed = tokio::time::timeout(PATIENCE, round_trip)
        .await
        .expect("the socket must not hang after run_isolated returned")
        .expect("the socket must still work after run_isolated returned");
    assert_eq!(&echoed, b"ping");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_spawned_inside_run_isolated_finishes_after_it_returns() {
    let (tx, rx) = tokio::sync::oneshot::channel::<&'static str>();
    run_isolated(move || async move {
        tokio::spawn(async move {
            // not a sync point: the delay only puts the task's end after run_isolated returns; the oneshot is the sync
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = tx.send("done");
        });
        Ok::<_, anyhow::Error>(())
    })
    .await
    .expect("run_isolated");

    let got = tokio::time::timeout(PATIENCE, rx)
        .await
        .expect("the task must finish within the patience window")
        .expect("the task must not be dropped with run_isolated");
    assert_eq!(got, "done");
}
