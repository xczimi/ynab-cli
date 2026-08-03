//! One-shot localhost HTTP listener for the OAuth redirect callback.
//!
//! Deliberately hand-rolled instead of pulling in an HTTP server crate: it
//! needs to accept exactly one request, extract two query params, and reply
//! with a static page — a `std::net::TcpListener` plus a minimal request-line
//! parser is simpler to audit than wiring up a framework for that.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

use crate::error::{Error, Result};

const SUCCESS_BODY: &str = "Authorized — you can close this tab.";
const FAILURE_BODY: &str = "Authorization failed — you can close this tab and try again.";

/// Binds `127.0.0.1:port`, accepts a single connection, and parses the
/// `GET /callback?code=...&state=...` request YNAB's OAuth authorize page
/// redirects the browser to. Returns the authorization code on success.
pub fn wait_for_code(port: u16, expected_state: &str) -> Result<String> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let (mut stream, _) = listener.accept()?;

    let params = {
        let mut reader = BufReader::new(&stream);
        read_callback_params(&mut reader)?
    };

    let outcome = extract_code(&params, expected_state);
    let body = if outcome.is_ok() {
        SUCCESS_BODY
    } else {
        FAILURE_BODY
    };
    write_response(&mut stream, body)?;

    outcome
}

fn extract_code(params: &HashMap<String, String>, expected_state: &str) -> Result<String> {
    if params.contains_key("error") {
        return Err(Error::Config("authorization was denied".into()));
    }

    let code = params
        .get("code")
        .cloned()
        .ok_or_else(|| Error::Config("OAuth callback missing code".into()))?;
    let state = params
        .get("state")
        .cloned()
        .ok_or_else(|| Error::Config("OAuth callback missing state".into()))?;

    if state != expected_state {
        return Err(Error::Config("OAuth state mismatch — try again".into()));
    }

    Ok(code)
}

fn read_callback_params(reader: &mut BufReader<&TcpStream>) -> Result<HashMap<String, String>> {
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;

    // Drain the remaining request headers up to the blank line; we don't
    // need them, but the client expects to be allowed to finish sending.
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
    }

    let path = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| Error::Config("malformed OAuth callback request".into()))?;

    let query = path.split_once('?').map(|x| x.1).unwrap_or("");
    Ok(oauth2::url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect())
}

fn write_response(stream: &mut TcpStream, body: &str) -> Result<()> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::Duration;

    /// Connects with retries — the test spawns `wait_for_code` on a thread
    /// and there's an inherent race between that thread binding the
    /// listener and this one connecting to it.
    fn connect_with_retry(port: u16) -> TcpStream {
        for _ in 0..50 {
            if let Ok(stream) = TcpStream::connect(("127.0.0.1", port)) {
                return stream;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("could not connect to listener on port {port}");
    }

    fn free_port() -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[test]
    fn happy_path_returns_code_and_responds_200() {
        let port = free_port();
        let handle = std::thread::spawn(move || wait_for_code(port, "expected-state"));

        let mut stream = connect_with_retry(port);
        stream
            .write_all(b"GET /callback?code=abc123&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert!(response.contains("Authorized"));

        let code = handle.join().unwrap().unwrap();
        assert_eq!(code, "abc123");
    }

    #[test]
    fn state_mismatch_is_an_error() {
        let port = free_port();
        let handle = std::thread::spawn(move || wait_for_code(port, "expected-state"));

        let mut stream = connect_with_retry(port);
        stream
            .write_all(
                b"GET /callback?code=abc123&state=wrong-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
            )
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();

        let err = handle.join().unwrap().unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("state mismatch")));
    }

    #[test]
    fn access_denied_is_an_error() {
        let port = free_port();
        let handle = std::thread::spawn(move || wait_for_code(port, "expected-state"));

        let mut stream = connect_with_retry(port);
        stream
            .write_all(b"GET /callback?error=access_denied&state=expected-state HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();

        let err = handle.join().unwrap().unwrap_err();
        assert!(matches!(err, Error::Config(msg) if msg.contains("denied")));
    }
}
