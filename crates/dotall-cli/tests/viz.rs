use assert_cmd::Command as AssertCommand;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use tempfile::tempdir;

#[test]
fn viz_serves_tree_and_metrics() {
    let temp = tempdir().expect("temp");
    let ws = temp.path().to_str().expect("utf8");
    AssertCommand::cargo_bin("dotall")
        .unwrap()
        .args(["init", ws])
        .assert()
        .success();

    let mut child = Command::new(env!("CARGO_BIN_EXE_dotall"))
        .args(["viz", "--port", "0", "--no-open", ws])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
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
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut html = String::new();
    stream.read_to_string(&mut html).unwrap();
    assert!(html.contains("<html"));

    let _ = child.kill();
}
