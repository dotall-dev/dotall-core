use assert_cmd::Command as AssertCommand;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use tempfile::tempdir;

struct KillChild(Child);

impl Drop for KillChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn viz_serves_tree_and_metrics() {
    let temp = tempdir().expect("temp");
    let ws = temp.path().to_str().expect("utf8");
    AssertCommand::cargo_bin("dotall")
        .unwrap()
        .args(["init", ws])
        .assert()
        .success();

    let child = Command::new(env!("CARGO_BIN_EXE_dotall"))
        .args(["viz", "--port", "0", "--no-open", ws])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut guard = KillChild(child);
    let mut stdout = guard.0.stdout.take().unwrap();
    let mut buf = String::new();
    // Read until a line like "http://127.0.0.1:PORT/"
    let mut bytes = [0u8; 512];
    let n = stdout.read(&mut bytes).unwrap();
    buf.push_str(std::str::from_utf8(&bytes[..n]).unwrap());
    let url = buf
        .lines()
        .find(|l| l.contains("http://127.0.0.1:"))
        .expect("url")
        .trim()
        .to_string();
    let host_port = url.trim_start_matches("http://").trim_end_matches('/');

    let mut stream = TcpStream::connect(host_port).unwrap();
    stream
        .write_all(b"GET /api/metrics HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).unwrap();
    assert!(resp.contains("all_bytes"));

    let mut stream = TcpStream::connect(host_port).unwrap();
    stream
        .write_all(b"GET /api/tree HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut tree_resp = String::new();
    stream.read_to_string(&mut tree_resp).unwrap();
    assert!(
        tree_resp.contains("\"tree\""),
        "expected tree key in /api/tree body, got: {tree_resp}"
    );
    assert!(
        tree_resp.contains("\"history\""),
        "expected history key in /api/tree body, got: {tree_resp}"
    );

    let mut stream = TcpStream::connect(host_port).unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut html = String::new();
    stream.read_to_string(&mut html).unwrap();
    assert!(html.contains("<html"));
    assert!(
        html.contains("lower bound") || html.contains("compressed source size / 4"),
        "HTML should label dump tokens as a lower-bound estimate"
    );
}
