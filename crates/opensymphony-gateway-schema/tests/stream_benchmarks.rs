use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::mpsc,
};

use opensymphony::opensymphony_gateway_schema::terminal::{
    TerminalEncoding, TerminalFrame, TerminalFrameKind,
};
use opensymphony::opensymphony_gateway_schema::version::SchemaVersion;

const BENCH_PAYLOAD: &str = concat!(
    "[2025-08-17T09:12:00Z] INFO  agent::executor > Starting turn 3\n",
    "[2025-08-17T09:12:01Z] DEBUG agent::planner  > Evaluating tool call: file_write\n",
    "[2025-08-17T09:12:02Z] INFO  agent::tool      > Writing 42 bytes to src/main.rs\n",
    "[2025-08-17T09:12:03Z] INFO  agent::executor > Turn 3 completed in 2.1s\n",
    "[2025-08-17T09:12:04Z] DEBUG agent::runtime  > Awaiting next event\n",
);

fn sample_terminal_frame(sequence: u64) -> TerminalFrame {
    TerminalFrame {
        schema_version: SchemaVersion::v1(),
        frame_sequence: sequence,
        stream_id: "stream-bench".into(),
        run_id: "run-bench".into(),
        terminal_session_id: "term-bench".into(),
        frame_kind: TerminalFrameKind::Log,
        encoding: TerminalEncoding::Utf8,
        content: BENCH_PAYLOAD.into(),
        timestamp: chrono::Utc::now(),
    }
}

fn frame_bytes() -> Vec<u8> {
    serde_json::to_vec(&sample_terminal_frame(0)).expect("serialize frame")
}

/// Benchmark in-process tokio mpsc channel delivery.
#[tokio::test]
async fn bench_in_process_mpsc_throughput() {
    let frame = sample_terminal_frame(0);
    let payload = serde_json::to_vec(&frame).expect("serialize frame");
    let payload_len = payload.len();
    let total_messages: usize = 100_000;

    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let payload_for_producer = payload.clone();
    let producer = tokio::spawn(async move {
        let start = Instant::now();
        for _ in 0..total_messages {
            let _ = tx.send(payload_for_producer.clone());
        }
        start.elapsed()
    });

    let start = Instant::now();
    let mut received = 0;
    while let Some(_item) = rx.recv().await {
        received += 1;
        if received >= total_messages {
            break;
        }
    }
    let elapsed = start.elapsed();
    let _ = producer.await;

    let throughput_mbps =
        (total_messages as f64 * payload_len as f64) / (elapsed.as_secs_f64() * 1_000_000.0);

    eprintln!(
        "in-process mpsc: {} messages in {:?} ({:.2} MB/s)",
        total_messages, elapsed, throughput_mbps
    );

    // Gate: expect > 50 MB/s for tiny payloads on loopback
    assert!(
        throughput_mbps > 50.0,
        "in-process mpsc throughput too low: {:.2} MB/s",
        throughput_mbps
    );
}

/// Benchmark Unix domain socket delivery (macOS/Linux only).
#[cfg(unix)]
#[tokio::test]
async fn bench_unix_domain_socket_throughput() {
    use tokio::net::UnixListener;
    let payload = frame_bytes();
    let payload_len = payload.len();
    let total_messages: usize = 50_000;
    let socket_path = format!("/tmp/opensymphony-bench-{}.sock", std::process::id());

    // Clean up any stale socket
    let _ = std::fs::remove_file(&socket_path);

    let listener = UnixListener::bind(&socket_path).expect("bind unix socket");

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept unix connection");
        let mut buf = vec![0u8; payload_len];
        let mut received = 0;
        loop {
            match stream.read_exact(&mut buf).await {
                Ok(_) => {
                    received += 1;
                    if received >= total_messages {
                        break received;
                    }
                }
                Err(_) => break received,
            }
        }
    });

    let client_socket_path = socket_path.clone();
    let client = tokio::spawn(async move {
        let mut stream = UnixStream::connect(&client_socket_path)
            .await
            .expect("connect unix socket");
        let start = Instant::now();
        for _ in 0..total_messages {
            if stream.write_all(&payload).await.is_err() {
                break;
            }
        }
        let _ = stream.shutdown().await;
        start.elapsed()
    });

    let received = server.await.expect("server task");
    let elapsed = client.await.expect("client task");

    let throughput_mbps =
        (received as f64 * payload_len as f64) / (elapsed.as_secs_f64() * 1_000_000.0);

    eprintln!(
        "unix domain socket: {} messages in {:?} ({:.2} MB/s)",
        received, elapsed, throughput_mbps
    );

    let _ = std::fs::remove_file(&socket_path);

    assert_eq!(received, total_messages, "not all messages delivered");
    // Gate: expect > 10 MB/s for loopback unix socket
    assert!(
        throughput_mbps > 10.0,
        "unix domain socket throughput too low: {:.2} MB/s",
        throughput_mbps
    );
}

/// Benchmark WebSocket loopback delivery using tokio-tungstenite.
#[tokio::test]
async fn bench_websocket_loopback_throughput() {
    use tokio_tungstenite::{accept_async, connect_async};

    let payload = frame_bytes();
    let payload_len = payload.len();
    let total_messages: usize = 10_000;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind tcp");
    let addr = listener.local_addr().expect("local addr");

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept tcp");
        let mut ws = accept_async(stream).await.expect("accept ws");
        let mut received = 0;
        loop {
            match ws.next().await {
                Some(Ok(tokio_tungstenite::tungstenite::Message::Binary(_))) => {
                    received += 1;
                    if received >= total_messages {
                        break received;
                    }
                }
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => break received,
                Some(Ok(_)) => continue,
                Some(Err(_)) => break received,
                None => break received,
            }
        }
    });

    // Give server a moment to start listening
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = tokio::spawn(async move {
        let url = format!("ws://{}/", addr);
        let (mut ws, _) = connect_async(&url).await.expect("connect ws");
        let start = Instant::now();
        for _ in 0..total_messages {
            let msg = tokio_tungstenite::tungstenite::Message::Binary(payload.clone().into());
            if ws.send(msg).await.is_err() {
                break;
            }
        }
        let _ = ws.close(None).await;
        start.elapsed()
    });

    let received = server.await.expect("server task");
    let elapsed = client.await.expect("client task");

    let throughput_mbps =
        (received as f64 * payload_len as f64) / (elapsed.as_secs_f64() * 1_000_000.0);

    eprintln!(
        "websocket loopback: {} messages in {:?} ({:.2} MB/s)",
        received, elapsed, throughput_mbps
    );

    assert_eq!(received, total_messages, "not all messages delivered");
    // Gate: expect > 5 MB/s for loopback websocket with JSON payloads
    assert!(
        throughput_mbps > 5.0,
        "websocket loopback throughput too low: {:.2} MB/s",
        throughput_mbps
    );
}

/// Evaluate JSON-RPC 2.0 envelope overhead by wrapping a terminal frame.
#[test]
fn json_rpc_envelope_overhead_is_acceptable() {
    use serde_json::json;

    let frame = sample_terminal_frame(1);
    let frame_json = serde_json::to_vec(&frame).expect("serialize frame");

    let json_rpc = json!({
        "jsonrpc": "2.0",
        "method": "terminal.frame",
        "params": {
            "stream_id": "stream-bench",
            "cursor": {"sequence": 1, "partition": "terminal:run-bench"},
            "frame": frame,
        }
    });
    let json_rpc_bytes = serde_json::to_vec(&json_rpc).expect("serialize json-rpc");

    let overhead = json_rpc_bytes.len() - frame_json.len();
    let overhead_pct = (overhead as f64 / frame_json.len() as f64) * 100.0;

    eprintln!(
        "JSON-RPC 2.0 envelope overhead: {} bytes ({:.1}%)",
        overhead, overhead_pct
    );

    // Gate: expect overhead < 50% for typical payloads
    assert!(
        overhead_pct < 50.0,
        "JSON-RPC 2.0 envelope overhead too high: {:.1}%",
        overhead_pct
    );
}

/// SSE line overhead evaluation.
#[test]
fn sse_line_overhead_is_acceptable() {
    let frame = sample_terminal_frame(1);
    let frame_json = serde_json::to_string(&frame).expect("serialize frame");

    let sse_event = format!(
        "event: terminal_frame\nid: 1\ndata: {}\n\n",
        frame_json.replace('\n', "")
    );
    let overhead = sse_event.len() - frame_json.len();
    let overhead_pct = (overhead as f64 / frame_json.len() as f64) * 100.0;

    eprintln!(
        "SSE line overhead: {} bytes ({:.1}%)",
        overhead, overhead_pct
    );

    // Gate: expect overhead < 30% for typical payloads
    assert!(
        overhead_pct < 30.0,
        "SSE line overhead too high: {:.1}%",
        overhead_pct
    );
}
