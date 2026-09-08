use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const API_KEY: &str = "correct-horse-battery-staple";
const START_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_RETRY: Duration = Duration::from_millis(50);

struct Server {
    child: Child,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A port nothing is listening on. The listener is dropped before the server
/// binds it, so this races with anything else grabbing the port meanwhile.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn request(address: SocketAddr, raw: &str, deadline: Instant) -> Option<String> {
    loop {
        if let Ok(mut stream) = TcpStream::connect(address) {
            stream.write_all(raw.as_bytes()).ok()?;
            let mut response = String::new();
            stream.read_to_string(&mut response).ok()?;
            return Some(response);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(CONNECT_RETRY);
    }
}

#[test]
fn serve_answers_health_and_guards_validate_with_the_key() {
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_dcpdoctor"))
        .args([
            "serve",
            "--bind",
            "127.0.0.1",
            "--port",
            &port.to_string(),
            "--api-key",
            API_KEY,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut server = Server { child };
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let deadline = Instant::now() + START_TIMEOUT;

    let health = request(
        address,
        "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        deadline,
    )
    .expect("the server never answered /health");
    assert!(
        health.starts_with("HTTP/1.1 200 OK") && health.contains(r#""status":"ok""#),
        "{health}"
    );

    let unauthorized = request(
        address,
        "POST /validate HTTP/1.1\r\nHost: localhost\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"path\":\"/tmp\"}",
        deadline,
    )
    .expect("the server never answered /validate");
    assert!(
        unauthorized.starts_with("HTTP/1.1 401 Unauthorized"),
        "{unauthorized}"
    );

    server.child.kill().unwrap();
    let status = server.child.wait().unwrap();
    assert!(!status.success(), "the server was not killed");
}
